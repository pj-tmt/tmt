import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import * as Y from 'yjs';
import type { Admission } from '../src/admission.js';
import { sealOwnUpdate } from '../src/own-seal.js';
import { hostedLog, hostedOwnLog } from '../src/hosted-documents.js';

const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const requests = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/edit-request-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (b) => parseInt(b, 16));
async function fixture() {
  const root = await crypto.subtle.importKey('raw', new Uint8Array(32).fill(7), 'HKDF', false, [
    'deriveBits',
  ]);
  const sign = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const a = {
    space: v.space,
    page: v.page,
    epoch: '1',
    root,
    head: { revision: 3n },
    registration: { deviceId: v.page, keys: { sign } },
  } as unknown as Admission;
  return { a, root };
}
it('seals the C3a own edit-request vector through the shared Colab envelope path and exact log mapping', async () => {
  const { a, root } = await fixture(),
    row = requests.cases.find((r: { name: string }) => r.name === 'with-hunks');
  const record = await c.editRequest.verifiedEditRequest(row.record, row.root, row.key),
    doc = new Y.Doc();
  doc.getMap('intents').set(row.key, record);
  const update = new Uint8Array(Y.encodeStateAsUpdate(doc)),
    author = vi.fn(() => hex(v.public));
  const entry = await sealOwnUpdate(a, { seq: 0n, hash: new Uint8Array(32) }, update, author);
  expect(author).toHaveBeenCalledExactlyOnceWith(v.page, '3');
  const sealed = await hostedOwnLog(
    entry,
    { space: a.space, page: a.page, epoch: '1' },
    'holder-uid',
    Date.parse('2027-01-01T00:00:00Z'),
  );
  expect(sealed.target).toEqual({ resource: 'log', id: `${a.space}_${a.page}_1_${v.page}_1` });
  const original = await hostedLog(sealed, { space: a.space, page: a.page, epoch: '1' });
  expect(original.header.context).toMatchObject({
    namespace: 'own',
    streamSeq: '1',
    membershipRevision: '3',
    authorDevice: v.page,
  });
  const opened = await original.envelope.open(original.header.context, root, hex(v.public)),
    roundtrip = new Y.Doc();
  try {
    Y.applyUpdate(roundtrip, opened);
    expect(roundtrip.getMap('intents').get(row.key)).toEqual(record);
    expect(roundtrip.getText('html').toString()).toBe('');
    expect(c.decodeText(sealed.bytes)).not.toContain('<p>edited</p>');
    expect(sealed.bytes.length).toBeLessThanOrEqual(393216);
  } finally {
    opened.fill(0);
    roundtrip.destroy();
    doc.destroy();
  }
});
it('never seals when the injected existing author admission refuses', async () => {
  const { a } = await fixture(),
    seal = vi.spyOn(c.Envelope, 'seal');
  try {
    await expect(
      sealOwnUpdate(a, { seq: 0n, hash: new Uint8Array(32) }, new Uint8Array([1]), () => {
        throw { code: 'REMOTE_SCOPE_DENIED' };
      }),
    ).rejects.toEqual({ code: 'REMOTE_SCOPE_DENIED' });
    expect(seal).not.toHaveBeenCalled();
  } finally {
    seal.mockRestore();
  }
});
it('does not project a content envelope into the own-log publisher', async () => {
  const { a } = await fixture();
  const envelope = await c.Envelope.seal(
    {
      space: a.space,
      page: a.page,
      epoch: '1',
      kind: 'update',
      namespace: 'content',
      authorDevice: v.page,
      membershipRevision: '3',
      streamSeq: '1',
      prevHash: new Uint8Array(32),
    },
    a.root!,
    a.registration.keys.sign,
    new Uint8Array([1]),
  );
  await expect(
    hostedOwnLog(
      {
        seq: '1',
        envelopeHash: c.encodeBinary(await envelope.hash()),
        envelope: c.encodeBinary(envelope.toJson()),
      },
      { space: a.space, page: a.page, epoch: '1' },
      'uid',
      Date.now() + 10000,
    ),
  ).rejects.toThrow();
});
