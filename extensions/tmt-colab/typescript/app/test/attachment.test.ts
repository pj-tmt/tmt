import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { validateProjection } from '../src/fold-protocol.js';
import { validateDiscussionRecord } from '../src/thread-records.js';
const corpus = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/attachment-v1.json', import.meta.url), 'utf8'),
);
it('admits bounded document and Chat/annotation descriptors from the shared native corpus', () => {
  for (const c of corpus.cases) {
    if (c.operation !== 'document' && c.operation !== 'comment') continue;
    const v = JSON.parse(c.input);
    const admit = () =>
      c.operation === 'document'
        ? validateProjection({
            source: v.html,
            title: v.meta.title,
            attachments: v.meta.attachments,
          })
        : validateDiscussionRecord('messages', `${v.messageId}:${v.revision}`, v);
    if (c.admit) expect(admit, c.name).not.toThrow();
    else expect(admit, c.name).toThrow();
  }
});

it('matches native page tokens and object namespaces from independent signed position vectors', async () => {
  const c = await import('@tmt/colab-client'),
    { Objects } = await import('../src/objects.js'),
    { attachmentNamespace } = await import('../src/attachments.js');
  for (const vector of corpus.referenceRevisions) {
    // This vector isolates byte/position parity. Runtime membership and reference
    // authority are exercised with actual Admission/Objects/Worker elsewhere.
    const root = await crypto.subtle.importKey(
      'raw',
      c.binary(corpus.secret, 32, 32),
      'HKDF',
      false,
      ['deriveBits'],
    );
    const a = {
      space: vector.space,
      page: vector.page,
      epoch: vector.epoch,
      head: { revision: BigInt(vector.revision), hash: c.binary(vector.headHash, 32, 32) },
      root,
      readAuthor: () => ({ key: c.binary(corpus.publicKey, 32, 32), ownerDevice: true }),
      readRoot() {
        return root;
      },
    } as unknown as import('../src/admission.js').Admission;
    const objects = new Objects(a);
    for (const entry of vector.entries)
      await objects.admit(
        vector.writer,
        { seq: entry.seq, envelopeHash: entry.envelopeHash, envelope: entry.envelope },
        entry.kind,
        entry.namespace,
      );
    expect(await objects.revision(), vector.name).toBe(vector.token);
    const namespace = await attachmentNamespace(vector.space, vector.page);
    expect([...namespace].map((b) => b.toString(16).padStart(2, '0')).join('')).toBe(
      corpus.namespace,
    );
  }
});
