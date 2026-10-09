import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { expect, it } from 'vite-plus/test';
import { attachment } from '@tmt/colab-client';
import { AttachmentReadError } from '../src/attachments.js';
import { gatherAttachments, type GatherScope } from '../src/export-attachments.js';
import type { OwnState } from '../src/fold-protocol.js';

const vectors = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/attachment-v1.json', import.meta.url), 'utf8'),
);
const base = (name: string) =>
  JSON.parse(vectors.cases.find((row: { name: string }) => row.name === name).input);
const SPACE = base('document').space as string;
const PAGE = base('document').page as string;
const WRITER = base('message').authorDevice as string;
const REVISION = 'v1:' + 'a'.repeat(64);
const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const sha = (value: Uint8Array) => createHash('sha256').update(value).digest('hex');
/** A structurally valid descriptor of the shared vector, with its own ID and declared size. */
const descriptor = (kind: 'document' | 'message', n: number, bytes: number, messageId?: number) => {
  const d = base(kind);
  d.attachmentId = id(n);
  d.objectId = n.toString(16).padStart(2, '0').repeat(32);
  d.plaintextBytes = String(bytes);
  d.payloadBytes = String(bytes + 16);
  d.filename = `file-${n}.bin`;
  if (messageId !== undefined) d.source.messageId = id(messageId);
  return d as attachment.AttachmentDescriptor;
};
const message = (n: number, revision: string, attachments: unknown[], edit: object = {}) => ({
  kind: 'comment',
  deleted: false,
  senderDevice: WRITER,
  messageId: id(n),
  revision,
  spaceId: SPACE,
  pageId: PAGE,
  epoch: '3',
  attachments,
  ...edit,
});
const own = (messages: Record<string, unknown>): OwnState =>
  ({
    [WRITER]: { threads: {}, messages, intents: {}, replies: {} },
  }) as OwnState;
const scope = (extra: Partial<GatherScope> = {}): GatherScope => ({
  spaceId: SPACE,
  pageId: PAGE,
  epoch: '3',
  revision: REVISION,
  document: [],
  own: {},
  ...extra,
});
const idOf = (selector: attachment.AttachmentSelector) => selector.attachmentId;

it('lists document attachments in list order, then live messages by message and revision, each read under its reference', async () => {
  const asked: attachment.AttachmentSelector[] = [];
  const entries = await gatherAttachments(
    scope({
      document: [descriptor('document', 21, 3), descriptor('document', 20, 3)],
      own: own({
        [`${id(32)}:1`]: message(32, '1', [descriptor('message', 23, 3, 32)]),
        [`${id(31)}:1`]: message(31, '1', [
          descriptor('message', 22, 3, 31),
          descriptor('message', 24, 3, 31),
        ]),
      }),
    }),
    async (selector) => {
      asked.push(selector);
      return Uint8Array.of(7, 7, 7);
    },
  );
  expect(entries.map((entry) => [entry.attachmentId, entry.source, entry.state])).toEqual([
    [id(21), 'document', 'included'],
    [id(20), 'document', 'included'],
    [id(22), 'message', 'included'],
    [id(24), 'message', 'included'],
    [id(23), 'message', 'included'],
  ]);
  expect(asked.map(idOf)).toEqual([id(21), id(20), id(22), id(24), id(23)]);
  expect(asked[0]).toMatchObject({ kind: 'document-current', contentRevision: REVISION });
  expect(asked[2]).toMatchObject({
    kind: 'message',
    writerId: WRITER,
    messageId: id(31),
    messageRevision: '1',
  });
  expect(entries.every((entry) => sha(entry.bytes!) === sha(Uint8Array.of(7, 7, 7)))).toBe(true);
});

it('lists only live comments of the exported epoch with matching provenance', async () => {
  const live = (n: number, edit: object = {}) => [
    `${id(n)}:1`,
    message(n, '1', [descriptor('message', n + 100, 3, n)], edit),
  ];
  const entries = await gatherAttachments(
    scope({
      own: own(
        Object.fromEntries([
          live(1),
          live(2, { deleted: true }),
          live(3, { epoch: '2' }),
          live(4, { senderDevice: id(99) }),
          live(5, { pageId: id(98) }),
          live(6, { kind: 'thread' }),
          live(7, { revision: '2' }),
        ]),
      ),
    }),
    async () => Uint8Array.of(0, 0, 0),
  );
  expect(entries.map((entry) => entry.attachmentId)).toEqual([id(101)]);
});

it('says why a read failed without disclosing anything', async () => {
  const reasons = new Map<string, unknown>([
    [id(1), new AttachmentReadError('not-found')],
    [id(2), new AttachmentReadError('denied')],
    [id(3), new AttachmentReadError('changed')],
    [id(4), new Error('storage')],
  ]);
  const entries = await gatherAttachments(
    scope({ document: [1, 2, 3, 4].map((n) => descriptor('document', n, 3)) }),
    async (selector) => {
      throw reasons.get(idOf(selector));
    },
  );
  expect(entries.map((entry) => [entry.state, entry.reason])).toEqual([
    ['missing', undefined],
    ['unavailable', 'denied'],
    ['unavailable', 'changed'],
    ['unavailable', 'unavailable'],
  ]);
  expect(entries.every((entry) => entry.bytes === undefined)).toBe(true);
});

it('leaves later entries unread once the total cap or the time budget is spent', async () => {
  const document = [
    descriptor('document', 1, 4),
    descriptor('document', 2, 4),
    descriptor('document', 3, 2),
  ];
  let reads = 0;
  const read = async () => {
    reads++;
    return new Uint8Array(4);
  };
  const capped = await gatherAttachments(scope({ document, totalBytes: 8 }), read);
  expect(capped.map((entry) => [entry.state, entry.reason])).toEqual([
    ['included', undefined],
    ['included', undefined],
    ['unavailable', 'too-large'],
  ]);
  expect(reads).toBe(2);
  reads = 0;
  const idle = await gatherAttachments(scope({ document, budgetMs: 0 }), read);
  expect(
    idle.every((entry) => entry.state === 'unavailable' && entry.reason === 'unavailable'),
  ).toBe(true);
  expect(reads).toBe(0);
});

it('never discloses bytes that disagree with the descriptor length', async () => {
  const [entry] = await gatherAttachments(
    scope({ document: [descriptor('document', 1, 4)] }),
    async () => new Uint8Array(5),
  );
  expect([entry.state, entry.reason, entry.bytes]).toEqual([
    'unavailable',
    'unavailable',
    undefined,
  ]);
});

it('refuses the whole export for a descriptor of another page or space', async () => {
  for (const foreign of [{ page: id(77) }, { space: 'a'.repeat(32) }]) {
    let reads = 0;
    await expect(
      gatherAttachments(
        scope({ document: [{ ...descriptor('document', 1, 3), ...foreign }] }),
        async () => {
          reads++;
          return new Uint8Array(3);
        },
      ),
    ).rejects.toThrow();
    expect(reads).toBe(0);
  }
});
