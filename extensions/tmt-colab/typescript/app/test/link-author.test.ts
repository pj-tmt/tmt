import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import type { FoldCommand, FoldResult, DecoderCommand } from '../src/fold-protocol.js';
import { Admission } from '../src/admission.js';
import { Objects, type ObjectEntry } from '../src/objects.js';
import type { Registration } from '../src/registration.js';

vi.mock('../src/storage.js', () => ({ record: async () => undefined }));
vi.stubGlobal('navigator', { locks: { request: async (_key: string, fn: () => unknown) => fn() } });
afterEach(() => vi.unstubAllGlobals());

// The same frozen bytes native `tests/link_author_vectors.rs` replays in a real store: each case
// marks the link-issued own envelopes both sides admit, and the ones both reject. A rejection may
// come when the device chain is presented or when the envelope is admitted; it must come.
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/link-author-v1.json', import.meta.url), 'utf8'),
);
interface Sealed {
  stream: string;
  namespace: string;
  seq: string;
  hash: string;
  envelope: string;
}
interface Row {
  name: string;
  log: string;
  chains: string[];
  envelopes: Sealed[];
  expect: string;
}
async function admission(
  log: string,
  epoch: string = v.epoch,
  registration: Registration = {} as Registration,
): Promise<Admission> {
  const statements: string[] = v.logs[log];
  const last = c.statement.Envelope.fromJson(c.binary(statements.at(-1)!, 32 * 1024));
  const a = new Admission(v.spaceId, v.pageId, epoch, c.binary(v.ownerKey, 32, 32), registration, {
    linkId: '70000000-0000-4000-8000-000000000001',
    principal: 'principal',
    expiresAt: Number.MAX_SAFE_INTEGER,
  });
  await a.membership(
    {
      revision: String(statements.length),
      statementHash: c.encodeBinary(await last.hash()),
      ownerKey: v.ownerKey,
      statements,
      more: false,
    },
    true,
  );
  a.root = await crypto.subtle.importKey('raw', c.binary(v.epochSecret, 32, 32), 'HKDF', false, [
    'deriveBits',
  ]);
  return a;
}
const entry = (e: Sealed): ObjectEntry => ({
  seq: e.seq,
  envelopeHash: e.hash,
  envelope: e.envelope,
});
const presented = (names: string[]) => names.map((name) => v.chains[name]);
const row = (prefix: string): Row => v.cases.find((r: Row) => r.name.startsWith(prefix));

for (const r of v.cases as Row[]) {
  it(`${r.expect === 'admit' ? 'admits' : 'rejects'}: ${r.name}`, async () => {
    const a = await admission(r.log),
      objects = new Objects(a);
    const run = async () => {
      await a.chains(presented(r.chains));
      for (const e of r.envelopes) await objects.admit(e.stream, entry(e), 'update');
    };
    if (r.expect === 'admit') {
      await run();
      return;
    }
    // Everything before the last envelope is admitted; the last one is the refusal, unless the
    // chain itself is refused first.
    await a.chains(presented(r.chains)).then(
      async () => {
        for (const e of r.envelopes.slice(0, -1)) await objects.admit(e.stream, entry(e), 'update');
        const last = r.envelopes.at(-1)!;
        await expect(objects.admit(last.stream, entry(last), 'update')).rejects.toThrow();
      },
      () => undefined,
    );
  });
}

it('refuses at the chain every chain whose issuer does not stand', async () => {
  for (const name of [
    'forged',
    'wrongHash',
    'belowAdd',
    'memberClaimingLink',
    'linkClaimingMember',
  ])
    await expect((await admission('base')).chains([v.chains[name]]), name).rejects.toThrow();
});

it('admits a link device as a visible writer that holds no owner-device status authority', async () => {
  const r = row('removed link: envelope inside'),
    a = await admission(r.log),
    objects = new Objects(a),
    e = r.envelopes[0];
  await a.chains(presented(r.chains));
  await objects.admit(e.stream, entry(e), 'update');
  expect(objects.statusWriter(e.stream)).toBe(false);
  expect(a.ownerDevice(e.stream)).toBe(false);
  // A link chain never authorizes the member path: fresh owner-device checks refuse it.
  expect(() => a.author(e.stream, a.head!.revision.toString())).toThrow();
  // The removal's signed cut names exactly the admitted tail head.
  expect(() => objects.finish()).not.toThrow();
});

it('ends the removed link device stream at the cut that the removal signed', async () => {
  const r = row('removed link: envelope beyond'),
    a = await admission(r.log),
    objects = new Objects(a);
  await a.chains(presented(r.chains));
  await objects.admit(r.envelopes[0].stream, entry(r.envelopes[0]), 'update');
  // The reader holds the cut's tail, so finishing succeeds; a stream short of it does not.
  expect(() => objects.finish()).not.toThrow();
  const short = new Objects(await admission(r.log));
  expect(() => short.finish()).toThrow();
});

const seedOf = (byte: number) => new Uint8Array(32).fill(byte);
const EDITOR_LINK = '50000000-0000-4000-8000-000000000001';
const COMMENTER_LINK = '50000000-0000-4000-8000-000000000002';
const statementHash = async (log: string, revision: number) =>
  c.statement.Envelope.fromJson(c.binary(v.logs[log][revision - 1], 32 * 1024)).hash();
/** The chain the browser's own link code mints for a device derived from the link's seed. */
async function mintedChain(
  linkId: string,
  byte: number,
  revision: number,
  validity: { issuedAt: number; expiresAt: number } = {
    issuedAt: c.link.DEVICE_ISSUED_AT,
    expiresAt: c.link.DEVICE_EXPIRES_AT,
  },
) {
  const link = await c.link.deriveLink(seedOf(byte), v.spaceId, linkId),
    device = await c.link.deriveDevice(seedOf(byte), v.spaceId, linkId);
  const chain = await c.link.certifyDevice(link, {
    deviceId: device.id,
    signingPublic: device.signingPublic,
    encryptionPublic: new Uint8Array(32).fill(7),
    membershipRevision: String(revision),
    issuerStatement: await statementHash('base', revision),
    ...validity,
  });
  return { device, value: { deviceId: device.id, chain: c.encodeBinary(chain) } };
}

it('keeps a device known under one link from being re-presented under another issuer', async () => {
  const a = await admission('base'),
    editor = await mintedChain(EDITOR_LINK, 0x51, 3);
  await a.chains([editor.value]);
  // The same device and key, certified by the commenter link at its own link.add: a valid chain
  // of its own, but not the issuer this admission already holds for the device.
  const commenterLink = await c.link.deriveLink(seedOf(0x52), v.spaceId, COMMENTER_LINK);
  const other = await c.link.certifyDevice(commenterLink, {
    deviceId: editor.device.id,
    signingPublic: editor.device.signingPublic,
    encryptionPublic: new Uint8Array(32).fill(7),
    membershipRevision: '4',
    issuerStatement: await statementHash('base', 4),
    issuedAt: c.link.DEVICE_ISSUED_AT,
    expiresAt: c.link.DEVICE_EXPIRES_AT,
  });
  await expect(
    a.chains([{ deviceId: editor.device.id, chain: c.encodeBinary(other) }]),
  ).rejects.toThrow();
});

async function foldWorker() {
  vi.resetModules();
  let result: FoldResult & { error?: string };
  const host = {
    onmessage: null as unknown as (event: unknown) => Promise<void>,
    postMessage(value: typeof result) {
      result = structuredClone(value);
    },
  };
  vi.stubGlobal('self', host);
  await import('../src/fold.worker.js');
  let id = 0;
  return async (command: DecoderCommand | FoldCommand) => {
    await host.onmessage({ data: { id: ++id, command } });
    if (result.error) throw new Error(result.error);
    return result;
  };
}

it('shows an edit request that a browser-derived link device sealed into its own stream', async () => {
  // A device derived and certified by the browser's link code, writing through the browser's
  // sealing: the admitted original is the one the reader folds into the page's `own` root.
  const editor = await mintedChain(EDITOR_LINK, 0x51, 3);
  const request = JSON.parse(
    readFileSync(
      new URL('../../../contracts/vectors/edit-request-v1.json', import.meta.url),
      'utf8',
    ),
  ).cases.find((x: { valid: boolean; digestOnly?: boolean }) => x.valid && !x.digestOnly);
  const run = await foldWorker();
  const prepared = (await run({
    type: 'prepare-own',
    writer: editor.device.id,
    records: [{ root: request.root, key: request.key, value: request.record }],
  })) as unknown as { update: Uint8Array };
  const envelope = await c.Envelope.seal(
    {
      space: v.spaceId,
      page: v.pageId,
      epoch: v.epoch,
      kind: 'update',
      namespace: 'own',
      authorDevice: editor.device.id,
      membershipRevision: '6',
      streamSeq: '1',
      prevHash: new Uint8Array(32),
    },
    c.binary(v.epochSecret, 32, 32),
    editor.device.sign,
    prepared.update,
  );
  const a = await admission('base'),
    objects = new Objects(a);
  await a.chains([editor.value]);
  const admitted = await objects.admit(
    editor.device.id,
    {
      seq: '1',
      envelopeHash: c.encodeBinary(await envelope.hash()),
      envelope: c.encodeBinary(envelope.toJson()),
    },
    'update',
  );
  expect(admitted).toMatchObject({ namespace: 'own', writer: editor.device.id });
  const view = await run({
    type: 'apply',
    updates: [],
    own: [{ writer: editor.device.id, update: admitted!.update }],
  });
  expect(view.own[editor.device.id].intents[request.key]).toEqual(request.record);
  expect(objects.statusWriter(editor.device.id)).toBe(false);
});

/** A session registered with the chain of the vector's link, and the revision its head is at. */
const registered = (name: string) =>
  ({
    deviceId: v.chains[name].deviceId,
    keys: {},
    chain: c.certificate.Chain.fromJson(c.binary(v.chains[name].chain, 16 * 1024)),
  }) as unknown as Registration;
const head = (log: string) => String(v.logs[log].length);

it('names the signing key of a link device that may write its own stream now', async () => {
  for (const name of ['editor', 'commenter']) {
    const a = await admission('base', v.epoch, registered(name));
    const key = await a.linkAuthor(v.chains[name].deviceId, head('base'));
    expect(key, name).toEqual(
      c.certificate.Chain.fromJson(c.binary(v.chains[name].chain, 16 * 1024)).certificate()
        .signingKey,
    );
    await expect(a.ownAuthor(head('base')), name).resolves.toEqual(key);
  }
});

it('refuses a link device whose link may not write, or has ended, or whose page is no longer open to it', async () => {
  const refused: [string, string, string?][] = [
    ['viewer link', 'base', 'viewer'],
    ['link that does not cover the page', 'base', 'other'],
    ['forged chain', 'base', 'forged'],
    ['chain citing another statement', 'base', 'wrongHash'],
    ['chain below the link.add revision', 'base', 'belowAdd'],
    ['member chain claiming link issuance', 'base', 'memberClaimingLink'],
    ['link removed', 'removed', 'editor'],
    ['page no longer link-shared', 'unshared', 'editor'],
  ];
  for (const [label, log, name] of refused) {
    const a = await admission(log, v.epoch, registered(name!));
    await expect(a.ownAuthor(head(log)), label).rejects.toThrow();
  }
});

it('refuses a link device at a revision the log has not reached, in another epoch, past its expiry, and unknown', async () => {
  const device = v.chains.editor.deviceId;
  const a = await admission('base', v.epoch, registered('editor'));
  await expect(a.linkAuthor(device, String(v.logs.base.length + 1))).rejects.toThrow();
  await expect(
    a.linkAuthor('90000000-0000-4000-8000-000000000009', head('base')),
  ).rejects.toThrow();
  const later = await admission('base', '2', registered('editor'));
  await expect(later.ownAuthor(head('base'))).rejects.toThrow();
  // A chain whose validity has ended no longer writes, though it still names the link's history.
  const expired = await mintedChain(EDITOR_LINK, 0x51, 3, { issuedAt: 1, expiresAt: 2 });
  const registration = {
    deviceId: expired.device.id,
    keys: {},
    chain: c.certificate.Chain.fromJson(c.binary(expired.value.chain, 16 * 1024)),
  } as unknown as Registration;
  const old = await admission('base', v.epoch, registration);
  await expect(old.ownAuthor(head('base'))).rejects.toThrow();
  // The same chain with the window open writes: nothing else differs.
  const open = await mintedChain(EDITOR_LINK, 0x51, 3);
  const live = await admission('base', v.epoch, {
    deviceId: open.device.id,
    keys: {},
    chain: c.certificate.Chain.fromJson(c.binary(open.value.chain, 16 * 1024)),
  } as unknown as Registration);
  await expect(live.ownAuthor(head('base'))).resolves.toEqual(open.device.signingPublic);
});

it('keeps the member path closed to a link device, and the link path to everything else', async () => {
  const a = await admission('base', v.epoch, registered('editor'));
  const device = v.chains.editor.deviceId;
  await a.linkAuthor(device, head('base'));
  // Once the link's chain is known to the admission, the member path still refuses it.
  expect(() => a.author(device, head('base'))).toThrow();
});
