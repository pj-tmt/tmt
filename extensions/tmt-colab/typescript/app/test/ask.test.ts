import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import {
  binary,
  decodeText,
  digest,
  encodeBinary,
  fields,
  strictVerify,
  text,
} from '@tmt/colab-client';
import { FrozenAsk, REQUEST_BYTES, escapedPreview } from '../src/ask-intent.js';
import { storeAskDraft } from '../src/ask-record-store.js';
import { destination, id, selection } from './ask-fixtures.js';

const records = new Map<string, unknown>();
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) => {
    if (values.length) records.set(key, structuredClone(values[0]));
    else return records.get(key);
  },
}));
vi.stubGlobal('navigator', {
  locks: { request: async (_key: string, run: () => unknown) => run() },
});
const vector = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/send-preview-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (v) => parseInt(v, 16));
async function key() {
  return crypto.subtle.importKey(
    'pkcs8',
    hex('302e020100300506032b657004220420' + vector.seed),
    'Ed25519',
    false,
    ['sign'],
  );
}
function preview() {
  return FrozenAsk.capture(selection(), destination());
}

it('matches independently framed bytes, SHA-256 and Ed25519 over exact frozen UTF-8', async () => {
  const frozen = FrozenAsk.capture(selection(), destination(), {
    operationId: id(9),
    issuedAt: vector.issuedAt,
    validityMs: vector.validityMs,
  });
  const signed = await frozen.signed(await key(), vector.issuedAt);
  expect(signed.input).toBe(vector.input);
  expect(signed.signature).toBe(vector.signature);
  expect(signed.finalBytes).toBe(vector.finalBytes);
  expect(encodeBinary(await digest(frozen.finalBytes()))).toBe(vector.finalDigest);
  expect(
    await strictVerify(
      hex(vector.publicKey),
      binary(signed.signature, 64, 64),
      binary(signed.input, 16384),
    ),
  ).toBe(true);
  const framed = fields(binary(signed.input, 16384), 15, 16384);
  expect(decodeText(framed[0])).toBe('tmt-colab-send-v1');
  expect(framed[9]).toEqual(await digest(frozen.finalBytes()));
  for (let i = 0; i < framed.length; i++) {
    const changed = binary(signed.input, 16384);
    const offset = framed.slice(0, i).reduce((n, f) => n + f.length + 4, 0) + 4;
    changed[offset] ^= 1;
    expect(
      await strictVerify(hex(vector.publicKey), binary(signed.signature, 64, 64), changed),
    ).toBe(false);
  }
});
it('copies scopes and bytes before asynchronous signing, including paused-source inputs', async () => {
  const s = selection(),
    d = destination(),
    ids = s.messageIds as string[];
  const frozen = FrozenAsk.capture(s, d),
    signing = frozen.signed(await key());
  s.quote = 'a newer live source';
  ids[0] = id(99);
  d.agent = id(99);
  frozen.finalBytes().fill(0);
  const signed = await signing;
  expect(decodeText(binary(signed.finalBytes, REQUEST_BYTES))).toBe(frozen.view.message);
  expect(frozen.view.message).toContain(selection().quote);
  expect(frozen.view.message).not.toContain('#secret');
  expect(frozen.view.agent).toBe(id(6));
  expect(
    await strictVerify(
      hex(vector.publicKey),
      binary(signed.signature, 64, 64),
      binary(signed.input, 16384),
    ),
  ).toBe(true);
  expect(escapedPreview(frozen.finalBytes())).toContain('\\u{d}\\u{a}😀\\u{0}\\u{202e}');
});
it('rejects credentials, invalid Unicode, noncanonical scope/list and over-limit composed messages', () => {
  for (const change of [
    { url: 'https://user:secret@example.test/page' },
    { url: 'javascript:alert(1)' },
    { quote: '\ud800' },
    { url: 'https://example.test/\ud800' },
    { page: id(1).toUpperCase().replace('4000', '400A') },
    { messageIds: [id(3), id(3)] },
    { messageIds: [id(8), id(3)] },
  ])
    expect(() => FrozenAsk.capture({ ...selection(), ...change }, destination())).toThrow();
  expect(() =>
    FrozenAsk.capture(selection(), {
      ...destination(),
      agent: '00000000-0000-0000-0000-000000000000',
    }),
  ).toThrow();
  expect(() => FrozenAsk.capture(selection(), { ...destination(), grantRevision: '01' })).toThrow();
  const base = preview().deliveredBytes().length;
  const exact = { ...selection(), comment: selection().comment + 'x'.repeat(REQUEST_BYTES - base) };
  expect(FrozenAsk.capture(exact, destination()).deliveredBytes()).toHaveLength(REQUEST_BYTES);
  expect(() =>
    FrozenAsk.capture({ ...exact, comment: exact.comment + 'x' }, destination()),
  ).toThrow();
  expect(() => FrozenAsk.capture(selection(), destination(), { validityMs: 86400001 })).toThrow();
});
it('rejects expired/future intents and extractable keys before signing or adoption', async () => {
  const frozen = FrozenAsk.capture(selection(), destination(), { issuedAt: 100, validityMs: 100 });
  await expect(frozen.signed(await key(), 99)).rejects.toThrow();
  await expect(frozen.signed(await key(), 200)).rejects.toThrow();
  const extractable = (await crypto.subtle.generateKey('Ed25519', true, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  await expect(preview().signed(extractable.privateKey)).rejects.toThrow();
});
it('persists only signed metadata, never the plaintext quote, comment or encoded final bytes', async () => {
  records.clear();
  const frozen = preview(),
    signed = await frozen.signed(await key());
  expect(await storeAskDraft(signed)).toBe('created');
  expect([...records.entries()]).toEqual([
    [
      `ask:${signed.senderDevice}:${signed.operationId}`,
      { input: signed.input, signature: signed.signature },
    ],
  ]);
  const stored = JSON.stringify([...records.values()]);
  for (const plaintext of [
    selection().quote,
    selection().comment,
    frozen.view.message,
    signed.finalBytes,
  ])
    expect(stored).not.toContain(JSON.stringify(plaintext));
});
it('reservation conflicts preserve immutable metadata', async () => {
  records.clear();
  const signed = await preview().signed(await key());
  expect(await storeAskDraft(signed)).toBe('created');
  expect(await storeAskDraft(signed)).toBe('existing');
  const stored = structuredClone([...records.values()]);
  await expect(storeAskDraft({ ...signed, input: encodeBinary(text('changed')) })).rejects.toThrow(
    'INTENT_CONFLICT',
  );
  expect([...records.values()]).toEqual(stored);
});
