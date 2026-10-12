import { expect, it } from 'vite-plus/test';
import {
  askedMessageIds,
  batchAnswersText,
  batchFits,
  commentsForAnswerBatch,
  sendAnswers,
  type AdmittedQuestions,
  type BatchAnswer,
} from '../src/answer-batch.js';
import type { AskBinding, PageAsk } from '../src/ask-panel.js';
import type { AgentDestination } from '../src/live-ask.js';
import type { Question } from '../src/questions.js';
import type { CommentView, DiscussionRef, ThreadView } from '../src/thread-records.js';
import {
  asksAbout,
  captureConversation,
  conversationAsks,
  type AnswerBatchContext,
  type CommentContext,
  type ThreadBinding,
} from '../src/thread-store.js';
import { destination, id } from './ask-fixtures.js';

const ME = id(4);
const OTHER = id(5);

const question = (n: number, title: string): Question => ({
  id: `q${n}`,
  title,
  kind: 'short',
  choices: [],
  other: false,
  agent: '',
  group: 'g1',
  label: null,
});
const questions = [question(1, 'Scope'), question(2, 'Owner'), question(3, 'Notes')];
const page: AdmittedQuestions = {
  questions,
  groups: [{ id: 'g1', title: 'Kickoff', questions: ['q1', 'q2', 'q3'] }],
};
const answerBody = (q: Question, answer: string) => `Answer to “${q.title}”: ${answer}\nPage: Doc`;

/** A thread of one question with its own answer comment, as the verified projection reports it. */
function threadWith(
  n: number,
  q: Question,
  answer: string,
  options: {
    writer?: string;
    deleted?: boolean;
    deviceAuthor?: boolean;
    body?: string;
    anchored?: boolean;
    proposal?: boolean;
  } = {},
): ThreadView {
  const ref: DiscussionRef = { writer: ME, id: id(100 + n) };
  const comment = {
    kind: 'comment',
    version: 1,
    messageId: id(30 - n * 9),
    ref: { writer: options.writer ?? ME, id: id(30 - n * 9) },
    thread: ref,
    senderDevice: options.writer ?? ME,
    revision: '1',
    deleted: options.deleted ?? false,
    deviceName: 'Browser',
    at: '1',
    body: options.body ?? answerBody(q, answer),
    deviceAuthor: options.deviceAuthor ?? true,
  } as unknown as CommentView;
  return {
    kind: 'thread',
    version: 1,
    threadId: ref.id,
    ref,
    senderDevice: ME,
    revision: '1',
    deleted: false,
    deviceName: 'Browser',
    at: '1',
    anchor: options.anchored === false ? null : { exact: `${q.title}?`, prefix: '', suffix: '' },
    resolved: false,
    ...(options.proposal ? { proposal: { proposalId: id(9) } } : {}),
    comments: [comment],
  } as unknown as ThreadView;
}
const contextOf = (thread: ThreadView, revision = '1'): CommentContext => ({
  thread: thread.ref,
  message: thread.comments[0].ref,
  threadRevision: thread.revision,
  messageRevision: revision,
});
const threads = () => [
  threadWith(1, questions[0], 'Only the API'),
  threadWith(2, questions[1], 'Ada'),
  threadWith(3, questions[2], 'Line one\nLine two'),
];
const batchOf = (
  list: ThreadView[],
  ids = ['q1', 'q2', 'q3'],
  group = 'g1',
): AnswerBatchContext => ({
  group,
  answers: list.map((thread, index) => ({ question: ids[index], context: contextOf(thread) })),
});

it('a batch origin is the first answer’s thread, every comment ID sorted, and all answers in tab order', () => {
  const list = threads();
  const origin = commentsForAnswerBatch(list, batchOf(list), page, ME);
  expect(origin.thread).toBe(list[0].threadId);
  // Tab order is q1, q2, q3 while the comment IDs sort the other way round.
  expect(list.map((thread) => thread.comments[0].messageId)).toEqual([id(21), id(12), id(3)]);
  expect(origin.messageIds).toEqual([id(3), id(12), id(21)]);
  expect(origin.quote).toBe('Scope?');
  expect(origin.comment).toBe(
    'Answers to 3 questions:\nScope: Only the API\nOwner: Ada\nNotes: Line one\nLine two',
  );
  // A subset of the group, still in tab order, is a batch too.
  const two = [list[0], list[2]];
  expect(commentsForAnswerBatch(list, batchOf(two, ['q1', 'q3']), page, ME).comment).toBe(
    'Answers to 2 questions:\nScope: Only the API\nNotes: Line one\nLine two',
  );
});

it('a batch origin refuses anything but this device’s answers to one group, in order', () => {
  const ok = threads();
  const refuse = (list: ThreadView[], batch: AnswerBatchContext, admitted = page, device = ME) =>
    expect(() => commentsForAnswerBatch(list, batch, admitted, device)).toThrow();
  // another group, an unknown question, a question twice, out of tab order, one answer, too many
  refuse(ok, batchOf(ok, ['q1', 'q2', 'q3'], 'g2'));
  refuse(ok, batchOf(ok, ['q1', 'q2', 'qx']));
  refuse(ok, batchOf(ok, ['q1', 'q1', 'q3']));
  refuse(ok, batchOf(ok, ['q2', 'q1', 'q3']));
  refuse(ok, batchOf(ok.slice(0, 1), ['q1']));
  refuse(ok, {
    group: 'g1',
    answers: [...batchOf(ok).answers, { question: 'q3', context: contextOf(ok[2]) }],
  });
  // the same answers submitted out of tab order, or two answers to one question
  refuse([ok[1], ok[0], ok[2]], batchOf([ok[1], ok[0], ok[2]], ['q2', 'q1', 'q3']));
  const twice = [ok[0], threadWith(4, questions[0], 'Again'), ok[2]];
  refuse(twice, batchOf(twice, ['q1', 'q1', 'q3']));
  // a question outside the admitted group
  refuse(ok, batchOf(ok), {
    ...page,
    groups: [{ id: 'g1', title: null, questions: ['q1', 'q2'] }],
  });
  // another writer, a device that did not author it, a deleted comment, a comment that is no answer
  refuse([ok[0], threadWith(2, questions[1], 'Ada', { writer: OTHER }), ok[2]], batchOf(ok));
  refuse([ok[0], threadWith(2, questions[1], 'Ada', { deviceAuthor: false }), ok[2]], batchOf(ok));
  refuse([ok[0], threadWith(2, questions[1], 'Ada', { deleted: true }), ok[2]], batchOf(ok));
  refuse(
    [ok[0], threadWith(2, questions[1], 'Ada', { body: 'A plain comment' }), ok[2]],
    batchOf(ok),
  );
  // an answer to a different question than the one it is submitted for
  refuse([ok[0], threadWith(2, questions[2], 'Ada'), ok[2]], batchOf(ok));
  // a thread that is a proposal or has no anchor is not a question thread
  refuse([ok[0], threadWith(2, questions[1], 'Ada', { proposal: true }), ok[2]], batchOf(ok));
  refuse([ok[0], threadWith(2, questions[1], 'Ada', { anchored: false }), ok[2]], batchOf(ok));
  // a stale comment revision, and another device asking
  const stale = batchOf(ok);
  stale.answers = stale.answers.map((answer, index) =>
    index === 1 ? { ...answer, context: contextOf(ok[1], '2') } : answer,
  );
  refuse(ok, stale);
  refuse(ok, batchOf(ok), page, OTHER);
});

it('asked message IDs are the batch’s sorted comments, or the one comment', () => {
  const list = threads();
  expect(askedMessageIds({ context: contextOf(list[0]), batch: batchOf(list) })).toEqual([
    id(3),
    id(12),
    id(21),
  ]);
  expect(askedMessageIds({ context: contextOf(list[1]) })).toEqual([id(12)]);
});

it('batch text is one line per answer under one header', () => {
  expect(
    batchAnswersText([
      { title: 'A', answer: '1' },
      { title: 'B', answer: '2' },
    ]),
  ).toBe('Answers to 2 questions:\nA: 1\nB: 2');
});

/** A card’s answers for a send, in tab order. */
function answers(list: ThreadView[], existing = false, texts = ['Only the API', 'Ada', 'Done']) {
  return list.map((thread, index): BatchAnswer => ({
    question: questions[index],
    body: answerBody(questions[index], texts[index]),
    selector: { exact: `${questions[index].title}?`, prefix: '', suffix: '' },
    ...(existing ? { thread } : {}),
  }));
}
function discussion(fail: ReadonlySet<string> = new Set()) {
  const calls: { kind: 'reply' | 'create'; body: string }[] = [];
  const binding = {
    deviceId: ME,
    async reply(thread: DiscussionRef, body: string, revision?: string) {
      calls.push({ kind: 'reply', body });
      if ([...fail].some((part) => body.includes(part))) throw new Error('refused');
      return {
        thread,
        message: { writer: ME, id: id(60 + calls.length) },
        threadRevision: revision ?? '1',
        messageRevision: '1',
      } satisfies CommentContext;
    },
    async create(body: string) {
      calls.push({ kind: 'create', body });
      if ([...fail].some((part) => body.includes(part))) throw new Error('refused');
      return {
        thread: { writer: ME, id: id(80 + calls.length) },
        message: { writer: ME, id: id(60 + calls.length) },
        threadRevision: '1',
        messageRevision: '1',
      } satisfies CommentContext;
    },
  };
  return { calls, binding: binding as unknown as ThreadBinding };
}
type Prepared = Parameters<AskBinding['prepare']>[0];
function asker(
  outcome: (recipient: string) => 'adopted' | 'unadopted' | 'uncertain' | 'noprepare',
) {
  const prepared: Prepared[] = [];
  const binding = {
    async destinations() {
      return [];
    },
    async prepare(input: Prepared) {
      prepared.push(input);
      const result = outcome(input.destination.agentName);
      if (result === 'noprepare') throw new Error('not prepared');
      return {
        preview: {},
        available: true,
        state: { state: 'preview' },
        send: async () =>
          result === 'adopted'
            ? { state: 'accepted', adopted: true }
            : result === 'unadopted'
              ? { state: 'failed', adopted: false }
              : { state: 'uncertain' },
      };
    },
  };
  return { prepared, binding: binding as unknown as AskBinding };
}
const recipients = (...names: string[]): AgentDestination[] =>
  names.map((name, index) => ({ ...destination(), agent: id(40 + index), agentName: name }));
const group = page.groups[0];
const base = { group, title: 'Doc', url: 'https://example.test/x/colab/' };

it('a Send publishes one comment per answered question in tab order, then one Ask per recipient over them', async () => {
  const { calls, binding } = discussion();
  const { prepared, binding: ask } = asker(() => 'adopted');
  const result = await sendAnswers(
    { discussion: binding, ask },
    { ...base, answers: answers(threads()), recipients: recipients('alpha', 'beta') },
    [],
  );
  expect(calls.map((call) => [call.kind, call.body.split('\n')[0]])).toEqual([
    ['create', 'Answer to “Scope”: Only the API'],
    ['create', 'Answer to “Owner”: Ada'],
    ['create', 'Answer to “Notes”: Done'],
  ]);
  expect(result).toMatchObject({
    answers: [
      { question: 'q1', state: 'recorded' },
      { question: 'q2', state: 'recorded' },
      { question: 'q3', state: 'recorded' },
    ],
    canonical: { question: 'q1' },
    failures: [],
  });
  // Two recipients, each asked once about all three comments, the first one's thread canonical.
  expect(prepared.map((input) => input.destination.agentName)).toEqual(['alpha', 'beta']);
  for (const input of prepared) {
    expect(input.batch?.group).toBe('g1');
    expect(input.batch?.answers.map((answer) => answer.question)).toEqual(['q1', 'q2', 'q3']);
    expect(input.context?.message.id).toBe(input.batch?.answers[0].context.message.id);
    expect(input.retryOf).toBeUndefined();
  }
});

it('a question whose comment failed stays out of the Ask and out of the batch', async () => {
  const { binding } = discussion(new Set(['“Owner”']));
  const { prepared, binding: ask } = asker(() => 'adopted');
  const result = await sendAnswers(
    { discussion: binding, ask },
    { ...base, answers: answers(threads()), recipients: recipients('alpha') },
    [],
  );
  if ('refused' in result) throw new Error('unexpected refusal');
  expect(result.answers.map((value) => [value.question, value.state])).toEqual([
    ['q1', 'recorded'],
    ['q2', 'failed'],
    ['q3', 'recorded'],
  ]);
  expect(prepared).toHaveLength(1);
  expect(prepared[0].batch?.answers.map((answer) => answer.question)).toEqual(['q1', 'q3']);
  expect(result.canonical?.question).toBe('q1');
});

it('the canonical answer is the first one recorded, even when an earlier one failed', async () => {
  const { binding } = discussion(new Set(['“Scope”']));
  const { prepared, binding: ask } = asker(() => 'adopted');
  const result = await sendAnswers(
    { discussion: binding, ask },
    { ...base, answers: answers(threads()), recipients: recipients('alpha') },
    [],
  );
  if ('refused' in result) throw new Error('unexpected refusal');
  expect(result.answers.map((value) => value.state)).toEqual(['failed', 'recorded', 'recorded']);
  expect(result.canonical?.question).toBe('q2');
  expect(result.canonical?.context).toEqual(result.answers[1].context);
  expect(prepared[0].context?.message.id).toBe(result.answers[1].context?.message.id);
});

it('one recorded answer is the single-comment Ask it was before batches', async () => {
  for (const list of [threads().slice(0, 1), threads()]) {
    const { binding } = discussion(new Set(list.length > 1 ? ['“Owner”', '“Notes”'] : []));
    const { prepared, binding: ask } = asker(() => 'adopted');
    await sendAnswers(
      { discussion: binding, ask },
      { ...base, answers: answers(list), recipients: recipients('alpha') },
      [],
    );
    expect(prepared).toHaveLength(1);
    expect(prepared[0].batch).toBeUndefined();
    expect(prepared[0].comment).toBe(answerBody(questions[0], 'Only the API'));
    expect(prepared[0].quote).toBe('Scope?');
    expect(prepared[0].context?.conversation).toEqual({ comments: [], replies: [] });
  }
});

it('no recipient keeps the answers on the page and asks nobody', async () => {
  const { calls, binding } = discussion();
  const { prepared, binding: ask } = asker(() => 'adopted');
  const result = await sendAnswers(
    { discussion: binding, ask },
    { ...base, answers: answers(threads()), recipients: [] },
    [],
  );
  expect(calls).toHaveLength(3);
  expect(prepared).toEqual([]);
  expect(result).toMatchObject({ canonical: { question: 'q1' }, failures: [] });
});

it('nothing recorded asks nobody; an Ask that cannot be prepared or sent is reported with its batch', async () => {
  const none = discussion(new Set(['Answer to']));
  const asked = asker(() => 'adopted');
  const failed = await sendAnswers(
    { discussion: none.binding, ask: asked.binding },
    { ...base, answers: answers(threads()), recipients: recipients('alpha') },
    [],
  );
  expect(asked.prepared).toEqual([]);
  expect(failed).toMatchObject({
    answers: [{ state: 'failed' }, { state: 'failed' }, { state: 'failed' }],
    failures: [],
  });
  expect(failed).not.toHaveProperty('canonical');
  const { binding } = discussion();
  const mixed = asker((agent) =>
    agent === 'alpha' ? 'noprepare' : agent === 'beta' ? 'unadopted' : 'uncertain',
  );
  const result = await sendAnswers(
    { discussion: binding, ask: mixed.binding },
    { ...base, answers: answers(threads()), recipients: recipients('alpha', 'beta', 'gamma') },
    [],
  );
  if ('refused' in result) throw new Error('unexpected refusal');
  expect(result.failures.map((failure) => [failure.agent, failure.uncertain])).toEqual([
    ['alpha', false],
    ['beta', false],
    ['gamma', true],
  ]);
  // Each failure retries the same batch for its recipient, and names the canonical comment.
  for (const failure of result.failures) {
    expect(failure.input.batch?.answers).toHaveLength(3);
    expect(failure.message).toBe(failure.input.batch?.answers[0].context.message.id);
    expect(askedMessageIds(failure.input)).toHaveLength(3);
  }
});

it('a batch over the Ask size cap refuses before anything is published', async () => {
  const { calls, binding } = discussion();
  const { prepared, binding: ask } = asker(() => 'adopted');
  const long = 'x'.repeat(30 * 1024);
  const big = answers(threads(), false, [long, long, long]);
  expect(batchFits({ ...base, answers: big })).toBe(false);
  expect(batchFits({ ...base, answers: answers(threads()) })).toBe(true);
  const result = await sendAnswers(
    { discussion: binding, ask },
    { ...base, answers: big, recipients: recipients('alpha') },
    [],
  );
  expect(result).toEqual({ refused: 'too-large' });
  expect(calls).toEqual([]);
  expect(prepared).toEqual([]);
});

it('an answered question replies in its own thread, with that thread’s earlier conversation for a single Ask', async () => {
  const list = threads();
  const earlier = captureConversation(list[0], []);
  expect(earlier.comments).toHaveLength(1);
  const { calls, binding } = discussion();
  const { prepared, binding: ask } = asker(() => 'adopted');
  await sendAnswers(
    { discussion: binding, ask },
    { ...base, answers: answers(list.slice(0, 1), true), recipients: recipients('alpha') },
    [],
  );
  expect(calls).toEqual([{ kind: 'reply', body: answerBody(questions[0], 'Only the API') }]);
  expect(prepared[0].context?.conversation).toEqual(earlier);
});

const ask = (overrides: Partial<PageAsk>): PageAsk => ({
  operationId: id(70),
  writer: ME,
  message: 'm',
  agent: id(6),
  agentName: 'alpha',
  deviceName: 'You',
  issuedAt: 1,
  machine: id(5),
  state: 'accepted',
  canTrack: true,
  ...overrides,
});

it('a batch Ask belongs to every question thread it covers, and its reply to the canonical one', () => {
  const list = threads();
  const batch = ask({
    thread: list[0].threadId,
    messageIds: [id(3), id(12), id(21)],
    reply: 'Thanks',
  });
  const single = ask({
    operationId: id(71),
    thread: list[1].threadId,
    messageIds: [id(12)],
  });
  const stranger = ask({ operationId: id(72), thread: id(999), messageIds: [id(12)] });
  const asks = [batch, single, stranger];
  expect(conversationAsks(list[0], asks)).toEqual([batch]);
  expect(conversationAsks(list[1], asks)).toEqual([batch, single]);
  expect(conversationAsks(list[2], asks)).toEqual([batch]);
  expect(asksAbout(batch, list[2], list[2].comments[0])).toBe(true);
  expect(asksAbout(single, list[0], list[1].comments[0])).toBe(false);
  // An Ask is only about comments its own writer wrote: another writer's Ask naming the same
  // comment IDs is no association, for a batch or a single comment.
  const foreign = { ...batch, writer: OTHER };
  expect(asksAbout(foreign, list[2], list[2].comments[0])).toBe(false);
  expect(conversationAsks(list[0], [foreign, { ...single, writer: OTHER }])).toEqual([]);
  expect(conversationAsks(list[1], [foreign, { ...single, writer: OTHER }])).toEqual([]);
  // The earlier conversation a thread captures carries only the replies it is the canonical thread of.
  expect(captureConversation(list[0], asks).replies).toHaveLength(1);
  expect(captureConversation(list[1], asks).replies).toEqual([]);
  expect(captureConversation(list[2], asks).replies).toEqual([]);
  // A deleted comment no longer ties its thread to the Ask unless deleted ones are asked for.
  list[2].comments[0] = { ...list[2].comments[0], deleted: true };
  expect(conversationAsks(list[2], asks)).toEqual([]);
  expect(conversationAsks(list[2], asks, true)).toEqual([batch]);
});
