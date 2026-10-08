import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { attachment as a, binary, decodeText, equal, text } from '../src/index.js';
const corpus = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/attachment-v1.json', import.meta.url), 'utf8'),
);
const bytes = (value: string) => binary(value, 1024 * 1024);
const hex = (value: Uint8Array) =>
  Array.from(value, (b) => b.toString(16).padStart(2, '0')).join('');
it('admits the same independent descriptor/manifest/asset corpus as Rust', async () => {
  for (const c of corpus.cases) {
    if (c.operation === 'document' || c.operation === 'comment') continue;
    let accepted = false;
    try {
      if (c.operation === 'manifest') {
        const manifest = a.decodeAttachmentManifest(text(c.input));
        if (c.inputBytes) {
          expect(equal(a.attachmentManifestInput(manifest), bytes(c.inputBytes))).toBe(true);
          expect(hex(await a.attachmentManifestHash(manifest))).toBe(c.hash);
        }
      } else {
        const d = a.decodeAttachment(text(c.input));
        if (c.operation === 'open') {
          const plain = await a.openAttachment(
            d,
            bytes(c.payload),
            { ...a.attachmentContext(d), ...c.context },
            bytes(c.secret ?? corpus.secret),
            bytes(c.publicKey ?? corpus.publicKey),
          );
          expect(equal(plain, bytes(corpus.plaintext))).toBe(true);
        } else if (c.admit) {
          expect(decodeText(a.attachmentJson(d))).toBe(c.canonical);
          expect(equal(a.attachmentInput(d), bytes(c.inputBytes))).toBe(true);
          expect(hex(await a.attachmentHash(d))).toBe(c.hash);
        }
      }
      accepted = true;
    } catch (error) {
      if (c.admit)
        throw new Error(`Positive attachment vector failed: ${c.name}`, { cause: error });
    }
    expect(accepted, c.name).toBe(c.admit);
  }
});
it('snapshots the descriptor, source, payload, admitted context and keys before awaiting', async () => {
  const c = corpus.cases.find((v: { name: string }) => v.name === 'document-asset');
  const d = a.decodeAttachment(text(c.input)),
    payload = bytes(c.payload),
    secret = bytes(corpus.secret),
    key = bytes(corpus.publicKey),
    context = a.attachmentContext(d);
  const opening = a.openAttachment(d, payload, context, secret, key);
  d.filename = 'changed';
  if (d.source.kind === 'document') d.source.sourceDigest = '00'.repeat(32);
  payload.fill(0);
  secret.fill(0);
  key.fill(0);
  context.prevHash.fill(1);
  context.epoch = '2';
  expect(equal(await opening, bytes(corpus.plaintext))).toBe(true);
});
