/** Page/Remote doubles for real parent chrome; no production import or native stores. */
import { mount as mountPage, notificationSources, discussionProof } from './ask-page-browser.js';
// Read the mounted module's state even when Vite gives its dependency a timestamp.
export { proof, agentResolves, seenProof, discussionProof } from './ask-page-browser.js';
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
    createdAt: '1',
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
    async setLabels() {
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
  destination: 'Chat' | 'Thread' | 'Answer' | 'Proposal';
  human?: boolean;
  foreign?: boolean;
  pending?: boolean;
  body?: string;
}) {
  // Preserve comments committed through the real fixture composer (including
  // attachment preview triggers) when appending the next admitted arrival.
  threads = [...(discussionProof() ?? [])];
  const t = threads.find((value) =>
    options.destination === 'Chat'
      ? value.anchor === null && !value.proposal
      : options.destination === 'Proposal'
        ? !!value.proposal
        : options.destination === 'Answer'
          ? value.anchor?.exact === 'Question for you?'
          : value.anchor?.exact === 'Exact selected text',
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
/** A second unresolved annotation keeps the total nonzero after an agent resolution. */
export function secondThread() {
  const value = thread(false);
  value.threadId = id(43);
  value.ref = { writer: own, id: id(43) };
  value.comments = value.comments.map((comment) => ({
    ...comment,
    thread: value.ref,
    messageId: id(44),
    ref: { writer: own, id: id(44) },
    body: 'A separate unresolved annotation',
  }));
  threads = [...(discussionProof() ?? []), value];
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

/** Admitted block projections, separate from transport and Send results. */
export async function mountBlocks(
  options: { detached?: boolean; folded?: boolean; grouped?: boolean } = {},
) {
  await mount();
  const answer = thread(false);
  answer.threadId = id(91);
  answer.ref = { writer: own, id: answer.threadId };
  answer.anchor = { exact: 'Question for you?', prefix: '', suffix: '' };
  answer.comments = [
    {
      ...answer.comments[0],
      thread: answer.ref,
      body: 'Answer to “Question for you?”: My answer',
      deviceAuthor: true,
    },
  ];
  const proposal = thread(false);
  proposal.threadId = id(81);
  proposal.ref = { writer: own, id: proposal.threadId };
  proposal.anchor = null;
  proposal.comments = [
    {
      ...proposal.comments[0],
      thread: proposal.ref,
      messageId: id(82),
      ref: { writer: own, id: id(82) },
      body: 'My proposal follow-up',
    },
  ];
  proposal.proposal = {
    proposalId: proposal.threadId,
    title: 'A proposal for the page',
    body: 'Please review this proposal.',
    proposer: { machineId: id(5), agentId: id(6), label: 'Atlas' },
  };
  proposal.resolved = !!options.folded;
  threads = [thread(true), thread(false), answer, proposal];
  notificationSources({
    threads,
    asks,
    source: `<style>body{margin:24px;font:16px/1.5 sans-serif}tmt-question,tmt-proposal{display:block}</style><p id="selected">Exact selected text</p><div style="height:1100px">Page context above the blocks.</div>${options.grouped ? `<tmt-questions data-id="${id(88)}"><p data-title>Two questions</p><tmt-question data-id="${id(89)}" data-kind="short"><p data-question>First unanswered question</p></tmt-question>` : ''}<tmt-question data-id="${id(90)}" data-kind="short"><p data-question>Question for you?</p></tmt-question>${options.grouped ? '</tmt-questions>' : ''}<div style="height:500px">Between blocks</div>${options.detached ? '' : `<tmt-proposal data-id="${id(81)}"></tmt-proposal>`}<div style="height:1000px">Page context below the blocks.</div>`,
  });
}
export function withdrawBlock(destination: 'Answer' | 'Proposal') {
  threads = [...(discussionProof() ?? [])].map((t) =>
    (destination === 'Proposal' ? !!t.proposal : t.anchor?.exact === 'Question for you?')
      ? { ...t, deleted: true }
      : t,
  );
  publish();
}
