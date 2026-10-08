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
