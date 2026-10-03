import { readFileSync } from 'node:fs';
import { beforeEach, expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import {
  ManagementClient,
  managementLog,
  newLink,
  project,
  selectionBytes,
  type Pending,
  type Selection,
} from '../src/management.js';
import type { Registration } from '../src/registration.js';
import type { Bootstrap, PageInfo } from '../src/bootstrap.js';
const records = new Map<string, unknown>();
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) => {
    if (values.length) records.set(key, structuredClone(values[0]));
    else return records.get(key);
  },
}));
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
const json = (x: unknown) => c.text(JSON.stringify(x));
const mount = new URL('http://localhost/r/abcd/x/colab/');
const page: PageInfo = {
  pageId: v.page,
  epoch: '1',
  sharing: 'private',
  history: 'shared',
  archived: false,
};
async function fixture() {
  const signer = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const issuer = c.statement.Envelope.fromJson(json(v.statement));
  const g = await issuer.verifyNext(v.space, hex(v.public), null);
  const certificate = c.certificate.input({
    space: v.space,
    issuerKind: 'member',
    issuerId: g.head.ownerMember.id,
    deviceId: v.device,
    signingKey: hex(v.public),
    encryptionKey: c.wrap.Envelope.fromJson(json(v.wrap)).header().recipientKey,
    membershipRevision: '1',
    issuedAt: Date.now() - 1000,
    expiresAt: Date.now() + 86400000,
  });
  const chain = c.certificate.Chain.fromJson(
    json({
      version: 1,
      issuerStatement: c.encodeBinary(g.head.hash),
      deviceCertificate: c.encodeBinary(certificate),
      issuerSignature: c.encodeBinary(await c.sign(signer, certificate)),
    }),
  );
  const registration = {
    deviceId: v.device,
    issuer,
    chain,
    keys: { sign: signer, signPublic: hex(v.public) },
  } as Registration;
  const log = [g];
  const raw = [c.encodeBinary(issuer.toJson())];
  async function append(operation: string, value: unknown) {
    const bytes = json(value),
      prev = log[log.length - 1].head;
    const input = c.statement.input({
      space: v.space,
      operation,
      revision: String(prev.revision + 1n),
      previousHash: prev.hash,
      payloadDigest: await c.digest(bytes),
    });
    const envelope = c.statement.Envelope.fromJson(
      json({
        statement: c.encodeBinary(input),
        payload: c.encodeBinary(bytes),
        signature: c.encodeBinary(await c.sign(signer, input)),
      }),
    );
    log.push(await envelope.verifyNext(v.space, hex(v.public), prev));
    raw.push(c.encodeBinary(envelope.toJson()));
  }
  await append('page.share', { pageId: v.page, mode: 'private', epoch: '1' });
  const client = new ManagementClient(mount, registration);
  const boot: Bootstrap = { space: v.space, owner: hex(v.public), revision: '2', pages: [page] };
  const view = project(page, log);
  return { client, registration, log, raw, append, boot, view };
}
beforeEach(() => {
  records.clear();
  vi.restoreAllMocks();
  vi.stubGlobal('navigator', { locks: { request: async (_: string, fn: () => unknown) => fn() } });
});
it('freezes user selections, seed, operation ID and signed bytes across an explicit retry', async () => {
  const f = await fixture(),
    link = newLink('viewer', [v.page]),
    original = { ...link };
  const preparing = f.client.prepare(f.view, { operation: 'link.add', value: link });
  link.seed = c.encodeBinary(new Uint8Array(32));
  const pending = await preparing,
    envelope = JSON.parse(pending.body),
    payload = c.binary(envelope.payload, 16384);
  expect(JSON.parse(c.decodeText(payload))).toEqual(original);
  expect(pending.artifact?.seed).toBe(original.seed);
  const bytes = c.binary(envelope.request, 1024);
  expect(await c.strictVerify(hex(v.public), c.binary(envelope.signature, 64, 64), bytes)).toBe(
    true,
  );
  const fields = c.fields(bytes, 11);
  expect(c.decodeText(fields[4])).toBe(f.view.revision);
  expect(c.equal(fields[7], await c.digest(payload))).toBe(true);
  const fetcher = vi
    .fn()
    .mockRejectedValueOnce(new Error('lost receipt'))
    .mockResolvedValue(
      new Response(
        JSON.stringify({
          operationId: pending.id,
          membershipHead: { revision: '3', statementHash: c.encodeBinary(new Uint8Array(32)) },
        }),
      ),
    );
  vi.stubGlobal('fetch', fetcher);
  await expect(f.client.send(pending)).rejects.toMatchObject({ code: 'UNKNOWN' });
  await f.client.send(pending);
  expect(fetcher.mock.calls.map((args) => args[1].body)).toEqual([pending.body, pending.body]);
  vi.spyOn(Date, 'now').mockReturnValue(pending.expiresAt);
  await expect(f.client.send(pending)).rejects.toMatchObject({ code: 'EXPIRED' });
  expect(fetcher).toHaveBeenCalledTimes(2);
  expect([...records.values()].some((x) => JSON.stringify(x).includes(original.seed))).toBe(false);
});
it('rejects computed fields, wrong page, incomplete assignment and owner-member removal', async () => {
  const { view } = await fixture();
  const member = {
    id: crypto.randomUUID(),
    role: 'editor' as const,
    pages: [v.page, 'ffffffff-ffff-4fff-bfff-ffffffffffff'].sort(),
  };
  view.members.push(member);
  const selections = [
    { operation: 'page.share', value: { pageId: v.page, mode: 'public', publishedKeys: [] } },
    { operation: 'epoch.advance', value: { pageId: member.id } },
    { operation: 'member.remove', value: { memberId: member.id, pages: [v.page] } },
    { operation: 'member.remove', value: { memberId: view.ownerMember, pages: [v.page] } },
    { operation: 'link.add', value: { ...newLink('viewer', [v.page]), seed: 'invalid' } },
  ];
  for (const s of selections) expect(() => selectionBytes(view, s as Selection)).toThrow();
  expect(
    JSON.parse(
      c.decodeText(
        selectionBytes(view, {
          operation: 'member.role',
          value: { memberId: member.id, role: 'viewer', pages: member.pages },
        }),
      ),
    ),
  ).toEqual({ memberId: member.id, role: 'viewer', pages: member.pages });
});
it.each([
  ['INVALID', 400],
  ['DENIED', 403],
  ['EXPIRED', 403],
  ['STALE_HEAD', 409],
  ['CONFLICT', 409],
  ['CAPACITY', 503],
  ['UNAVAILABLE', 503],
])('preserves definite rejection %s without mutation or automatic retry', async (code, status) => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, { operation: 'epoch.advance', value: { pageId: v.page } });
  const fetcher = vi.fn(
    async () => new Response(JSON.stringify({ code }), { status: Number(status) }),
  );
  vi.stubGlobal('fetch', fetcher);
  await expect(f.client.send(p)).rejects.toMatchObject({ code });
  expect(fetcher).toHaveBeenCalledTimes(1);
  expect(records.size).toBe(0);
});
it('does not accept a wrong operation, malformed or oversized acknowledgment', async () => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, { operation: 'epoch.advance', value: { pageId: v.page } });
  for (const body of [
    JSON.stringify({
      operationId: crypto.randomUUID(),
      membershipHead: { revision: '3', statementHash: c.encodeBinary(new Uint8Array(32)) },
    }),
    '{',
    'x'.repeat(8193),
  ]) {
    vi.stubGlobal('fetch', async () => new Response(body));
    await expect(f.client.send(p)).rejects.toMatchObject({ code: 'UNKNOWN' });
  }
});
it('verifies the acknowledged position and requested change even when a later head exists', async () => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, {
      operation: 'page.history',
      value: { pageId: v.page, mode: 'current' },
    });
  await f.append('page.history', { pageId: v.page, mode: 'current' });
  const ack = {
    operationId: p.id,
    membershipHead: { revision: '3', statementHash: c.encodeBinary(f.log[2].head.hash) },
  };
  await f.append('page.history', { pageId: v.page, mode: 'shared' });
  vi.spyOn(f.client, 'snapshot').mockResolvedValue({ boot: f.boot, log: f.log });
  await f.client.read(v.page);
  expect((await f.client.verify(p, ack)).page.history).toBe('shared');
  await expect(
    f.client.verify(p, {
      ...ack,
      membershipHead: { ...ack.membershipHead, statementHash: c.encodeBinary(new Uint8Array(32)) },
    }),
  ).rejects.toThrow();
  await expect(
    f.client.verify(p, {
      ...ack,
      membershipHead: { revision: '2', statementHash: c.encodeBinary(f.log[1].head.hash) },
    }),
  ).rejects.toThrow();
  const other = { ...p, operation: 'page.share' } as Pending;
  await expect(f.client.verify(other, ack)).rejects.toThrow();
});
it('admits paged signed metadata without opening content; forged log, wrong scope and cancellation close the socket', async () => {
  const f = await fixture();
  let mode = 'valid',
    closed = 0,
    sent: string[] = [];
  class Socket {
    protocol = 'colab-sync-v1';
    onopen: (() => void) | null = null;
    onclose: (() => void) | null = null;
    onerror: (() => void) | null = null;
    onmessage: ((e: { data: string }) => void) | null = null;
    constructor() {
      queueMicrotask(() => this.onopen?.());
    }
    close() {
      closed++;
    }
    send(raw: string) {
      sent.push(raw);
      const request = JSON.parse(raw);
      if (mode === 'silent') return;
      const first = request.type === 'hello',
        head = f.log[1].head;
      const frame = {
        version: 1,
        type: 'catchup',
        space: f.boot.space,
        page: mode === 'scope' ? crypto.randomUUID() : v.page,
        epoch: '1',
        streams: [],
        more: true,
        ...(first
          ? {
              baseline: 'unsupported-content-baseline',
              membershipHead: {
                revision: '2',
                statementHash: c.encodeBinary(head.hash),
                ownerKey: c.encodeBinary(f.boot.owner),
                statements: [
                  mode === 'forged'
                    ? c.encodeBinary(
                        json({
                          ...JSON.parse(c.decodeText(c.binary(f.raw[0], 16384))),
                          signature: c.encodeBinary(new Uint8Array(64)),
                        }),
                      )
                    : f.raw[0],
                ],
                more: true,
              },
            }
          : { membership: { statements: [f.raw[1]], more: false } }),
      };
      queueMicrotask(() => this.onmessage?.({ data: JSON.stringify(frame) }));
    }
  }
  vi.stubGlobal('WebSocket', Socket);
  const log = await managementLog(mount, f.boot, page, f.registration);
  expect(log.length).toBe(2);
  expect(closed).toBe(1);
  expect(sent.map((x) => JSON.parse(x).type)).toEqual(['hello', 'ack']);
  expect(records.get(`log:${v.space}`)).toEqual(f.raw);
  for (const invalid of ['forged', 'scope']) {
    records.clear();
    mode = invalid;
    await expect(managementLog(mount, f.boot, page, f.registration)).rejects.toThrow();
  }
  mode = 'silent';
  const controller = new AbortController();
  const loading = managementLog(mount, f.boot, page, f.registration, controller.signal);
  controller.abort();
  await expect(loading).rejects.toMatchObject({ code: 'UNAVAILABLE' });
  expect(closed).toBe(4);
});
