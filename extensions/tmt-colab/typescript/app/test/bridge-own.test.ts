import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Admission } from '../src/admission.js';
import { Objects, type ObjectEntry } from '../src/objects.js';
import type { Registration } from '../src/registration.js';

vi.mock('../src/storage.js', () => ({ record: async () => undefined }));
vi.stubGlobal('navigator', { locks: { request: async (_key: string, fn: () => unknown) => fn() } });

// The same frozen bytes native `tests/bridge_vectors.rs` replays in a real store: each case
// marks the bridge-signed own envelopes both sides admit, and the ones both reject.
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/bridge-own-v1.json', import.meta.url), 'utf8'),
);
interface Sealed {
  stream: string;
  namespace: string;
  seq: string;
  hash: string;
  envelope: string;
}
async function admission(log: string, epoch: string = v.epoch): Promise<Admission> {
  const statements: string[] = v.logs[log];
  const last = c.statement.Envelope.fromJson(c.binary(statements.at(-1)!, 32 * 1024));
  const a = new Admission(
    v.spaceId,
    v.pageId,
    epoch,
    c.binary(v.ownerKey, 32, 32),
    {} as Registration,
    {
      linkId: '70000000-0000-4000-8000-000000000001',
      principal: 'principal',
      expiresAt: Number.MAX_SAFE_INTEGER,
    },
  );
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

for (const row of v.cases as { name: string; log: string; envelopes: Sealed[]; expect: string }[]) {
  it(`${row.expect === 'admit' ? 'admits' : 'rejects'}: ${row.name}`, async () => {
    const objects = new Objects(await admission(row.log));
    const settled = row.expect === 'admit' ? row.envelopes : row.envelopes.slice(0, -1);
    for (const e of settled) {
      expect(await objects.admit(e.stream, entry(e), 'update')).toMatchObject({
        namespace: e.namespace,
        writer: e.stream,
      });
    }
    if (row.expect === 'reject') {
      const e = row.envelopes.at(-1)!;
      await expect(objects.admit(e.stream, entry(e), 'update')).rejects.toThrow();
    }
  });
}

it('admits a bridge as a visible writer that holds no owner-device status authority', async () => {
  const row = v.cases.find((r: { name: string }) =>
    r.name.startsWith('revoked bridge: envelope inside'),
  );
  const a = await admission(row.log),
    objects = new Objects(a),
    e = row.envelopes[0] as Sealed;
  await objects.admit(e.stream, entry(e), 'update');
  expect(objects.ownSigningKey(e.stream)).toEqual(c.binary(v.bridge.signKey, 32, 32));
  expect(objects.statusWriter(e.stream)).toBe(false);
  expect(a.ownerDevice(e.stream)).toBe(false);
  // The signed cut names exactly the admitted tail head.
  expect(() => objects.finish()).not.toThrow();
});

it('grants a bridge only the epoch its membership state had at the envelope revision', async () => {
  const e = v.cases[0].envelopes[0] as Sealed,
    context = c.decodeHeader(c.Envelope.fromJson(c.binary(e.envelope, 64 * 1024)).header()).context,
    hash = c.binary(e.hash, 32, 32);
  expect((await admission('base')).readAuthor(context, hash).ownerDevice).toBe(false);
  // Historical creator admission follows the envelope's epoch, while the
  // current Objects owner still refuses an old envelope in its new namespace.
  const advanced = await admission('base', '2');
  expect(advanced.readAuthor(context, hash).ownerDevice).toBe(false);
  expect(() => advanced.readAuthor({ ...context, epoch: '2' }, hash)).toThrow();
  await expect(new Objects(advanced).admit(e.stream, entry(e))).rejects.toThrow();
});
