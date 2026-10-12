/** Page/Remote doubles for real parent chrome; no production import or native stores. */
import { mount as mountPage, notificationSources, discussionProof } from './ask-page-browser.js';
import { id, selection } from './ask-fixtures.js';
import type { PageAsk } from '../src/ask-panel.js';
import type { ThreadView } from '../src/thread-records.js';
import type { ManagementPort } from '../src/management.js';

let threads: ThreadView[];
let asks: PageAsk[];
let next = 200;
const own = id(4);
function thread(chat: boolean, writer = own): ThreadView {
  const threadId = chat ? writer : id(41);
  const value: ThreadView = {
    version: 1,
    kind: 'thread',
    spaceId: selection().space,
    pageId: id(1),
    epoch: '1',
    senderDevice: writer,
    deviceName: writer === own ? 'You' : 'Other person',
    revision: '1',
    deleted: false,
    at: '1',
    resolved: false,
    threadId,
    ref: { writer, id: threadId },
    anchor: chat ? null : { exact: 'Exact selected text', prefix: '', suffix: '' },
    comments: [],
  };
  value.comments.push({
    version: 1,
    kind: 'comment',
    spaceId: value.spaceId,
    pageId: value.pageId,
    epoch: '1',
    senderDevice: writer,
    deviceName: 'You',
    revision: '1',
    deleted: false,
    at: '1',
    messageId: id(chat ? 32 : 42),
    ref: { writer, id: id(chat ? 32 : 42) },
    thread: value.ref,
    body: 'Own recorded turn',
    deviceAuthor: true,
  });
  return value;
}
const publish = () => notificationSources({ threads, asks });
export async function mount() {
  const management: ManagementPort = {
    async read(pageId) {
      return {
        revision: '1',
        ownerMember: own,
        members: [],
        links: [],
        page: {
          pageId,
          epoch: '1',
          sharing: 'private',
          history: 'shared',
          archived: false,
          deleted: false,
          retentionDays: 30,
          lastUpdateAtMs: null,
          expiresAtMs: null,
          warnings: [],
        },
      };
    },
    async prepare() {
      throw new Error('No management command is admitted by this fixture');
    },
    async send() {
      throw new Error('No management command is admitted by this fixture');
    },
    async verify() {
      throw new Error('No management command is admitted by this fixture');
    },
  };
  await mountPage({ management, attachments: true, strict: true });
  next = 200;
  threads = [thread(true), thread(false)];
  asks = [];
  publish();
}
export function arrive(options: {
  destination: 'Chat' | 'Thread';
  human?: boolean;
  foreign?: boolean;
  pending?: boolean;
  body?: string;
}) {
  // Preserve comments committed through the real fixture composer (including
  // attachment preview triggers) when appending the next admitted arrival.
  threads = [...(discussionProof() ?? [])];
  const t = threads.find((value) =>
    options.destination === 'Chat' ? value.anchor === null : value.anchor !== null,
  )!;
  const index = next++;
  const body = options.body ?? `${options.destination} reply ${index}\nSecond reply line`;
  if (options.human) {
    const writer = options.foreign ? id(70) : id(7);
    t.comments.push({
      ...t.comments[0],
      senderDevice: writer,
      deviceName: 'Other person',
      messageId: id(index),
      ref: { writer, id: id(index) },
      body,
      deviceAuthor: true,
    });
  } else
    asks.push({
      writer: options.foreign ? id(70) : own,
      operationId: id(index),
      requestId: `req_${id(index + 1000)}`,
      thread: t.threadId,
      messageIds: [t.comments[0].messageId],
      message: 'Own Ask',
      agent: id(6),
      agentName: 'Atlas',
      deviceName: 'You',
      machine: id(5),
      issuedAt: index,
      state: 'accepted',
      canTrack: !options.foreign,
      reply: options.pending ? undefined : body,
    });
  publish();
  return index;
}
export function complete({ index, body }: { index: number; body: string }) {
  const ask = asks.find((record) => record.operationId === id(index) && record.reply === undefined);
  if (!ask) throw new Error('No admitted pending Ask to complete');
  ask.reply = body;
  publish();
}
export function repeat() {
  publish();
}
export function withdraw(destination: 'Chat' | 'Thread') {
  threads = threads.map((t) =>
    (t.anchor === null) === (destination === 'Chat') ? { ...t, deleted: true } : t,
  );
  publish();
}
export function foreignConversation() {
  threads = [thread(false, id(70))];
  publish();
}
export function history(destination: 'Chat' | 'Thread') {
  const t = threads.find((value) => (value.anchor === null) === (destination === 'Chat'))!;
  t.comments.push(
    ...Array.from({ length: 40 }, (_, index) => ({
      ...t.comments[0],
      messageId: id(500 + index),
      ref: { writer: own, id: id(500 + index) },
      body: `Earlier turn ${index}.\n${'Historical context. '.repeat(5)}`,
      sequence: String(index + 2),
    })),
  );
  publish();
}
export function snapshot() {
  return structuredClone({ threads, asks });
}
