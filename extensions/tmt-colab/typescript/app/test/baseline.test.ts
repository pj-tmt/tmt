import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Admission } from '../src/admission.js';
import { openBaseline } from '../src/baseline.js';
import type { Registration } from '../src/registration.js';
vi.mock('../src/storage.js', () => ({ record: async () => undefined }));
vi.stubGlobal('navigator', { locks: { request: async (_key: string, fn: () => unknown) => fn() } });
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const b = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/baseline-v1.json', import.meta.url), 'utf8'),
)[1];
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
const json = (value: unknown) => c.text(JSON.stringify(value));
async function fixture(
  change: Record<string, unknown> = {},
  body = { source: b.source, update: b.update },
  contextChange: Partial<c.Context> = {},
) {
  const signer = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const owner = hex(v.public),
    g = c.statement.Envelope.fromJson(json(v.statement));
  const genesis = await g.verifyNext(v.space, owner, null);
  const env = await c.Envelope.seal(
    {
      space: v.space,
      page: v.page,
      epoch: '2',
      kind: 'html',
      namespace: 'content',
      authorDevice: genesis.head.ownerMember.id,
      membershipRevision: '3',
      streamSeq: '0',
      prevHash: new Uint8Array(32),
      ...contextChange,
    },
    hex(v.epochKey),
    signer,
    json(body),
  );
  const hash = c.encodeBinary(await env.hash());
  const descriptor = {
    pageId: v.page,
    epoch: '2',
    sourceDigest: b.sourceDigest,
    baselineCommitment: b.commitment,
    title: b.title,
    objectEnvelopeHash: hash,
    membershipRevision: '3',
    ...change,
  };
  const sign = async (operation: string, value: unknown, head: c.statement.Head) => {
    const bytes = json(value),
      input = c.statement.input({
        space: v.space,
        operation,
        revision: String(head.revision + 1n),
        previousHash: head.hash,
        payloadDigest: await c.digest(bytes),
      });
    return c.statement.Envelope.fromJson(
      json({
        statement: c.encodeBinary(input),
        payload: c.encodeBinary(bytes),
        signature: c.encodeBinary(await c.sign(signer, input)),
      }),
    );
  };
  const shared = await sign(
    'page.share',
    { pageId: v.page, mode: 'private', epoch: '1' },
    genesis.head,
  );
  const before = await shared.verifyNext(v.space, owner, genesis.head);
  const advance = await sign(
    'epoch.advance',
    { pageId: v.page, epoch: '2', cuts: [], baseline: descriptor, wraps: [] },
    before.head,
  );
  const last = await advance.verifyNext(v.space, owner, before.head);
  const a = new Admission(v.space, v.page, '2', owner, {} as Registration);
  await a.membership(
    {
      revision: '3',
      statementHash: c.encodeBinary(last.head.hash),
      ownerKey: c.encodeBinary(owner),
      statements: [g, shared, advance].map((x) => c.encodeBinary(x.toJson())),
      more: false,
    },
    true,
  );
  a.root = await crypto.subtle.importKey('raw', hex(v.epochKey), 'HKDF', false, ['deriveBits']);
  return {
    a,
    descriptor,
    object: { envelopeHash: hash, envelope: c.encodeBinary(env.toJson()) },
    env,
  };
}
it('opens the exact signed reset object independently of the device stream', async () => {
  const f = await fixture(),
    result = await openBaseline(f.a, c.encodeBinary(json(f.descriptor)), f.object);
  expect(result.source).toBe(b.source);
  expect(result.title).toBe(b.title);
  expect(result.update).toEqual(c.binary(b.update, 4 * 1024 * 1024));
});
it('rejects descriptor substitutions, object substitutions and strict body errors', async () => {
  const f = await fixture();
  for (const change of [
    { title: 'substituted' },
    { baselineCommitment: c.encodeBinary(new Uint8Array(32)) },
    { epoch: '1' },
    { membershipRevision: '2' },
  ])
    await expect(
      openBaseline(f.a, c.encodeBinary(json({ ...f.descriptor, ...change })), f.object),
    ).rejects.toThrow();
  await expect(
    openBaseline(f.a, c.encodeBinary(json(f.descriptor)), {
      ...f.object,
      envelopeHash: c.encodeBinary(new Uint8Array(32)),
    }),
  ).rejects.toThrow();
  const forged = JSON.parse(c.decodeText(f.env.toJson()));
  forged.signature = c.encodeBinary(new Uint8Array(64));
  const env = c.Envelope.fromJson(json(forged));
  const hash = c.encodeBinary(await env.hash());
  await expect(
    openBaseline(f.a, c.encodeBinary(json(f.descriptor)), {
      envelopeHash: hash,
      envelope: c.encodeBinary(env.toJson()),
    }),
  ).rejects.toThrow();
  const malformed = await fixture({}, { source: b.source, update: b.update, extra: true } as {
    source: string;
    update: string;
  });
  await expect(
    openBaseline(malformed.a, c.encodeBinary(json(malformed.descriptor)), malformed.object),
  ).rejects.toThrow();
});

it('rejects signed baseline objects with device author, wrong revision or old epoch', async () => {
  // The vector's v.device equals its management-member ID; v.page is the distinct
  // certified-device ID in v.chain, so this is an actual author substitution.
  for (const change of [{ authorDevice: v.page }, { membershipRevision: '2' }, { epoch: '1' }]) {
    const f = await fixture({}, undefined, change);
    await expect(openBaseline(f.a, c.encodeBinary(json(f.descriptor)), f.object)).rejects.toThrow();
  }
});
