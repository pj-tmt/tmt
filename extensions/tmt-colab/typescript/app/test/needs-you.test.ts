import { expect, it } from 'vite-plus/test';
import { pendingNeedsYou, unansweredQuestions, type NeedsYouCandidate } from '../src/needs-you.js';
import type { Question } from '../src/questions.js';
import type { CommentView } from '../src/thread-records.js';

const candidate = (
  id: string,
  state: NeedsYouCandidate['state'],
  kind: NeedsYouCandidate['kind'],
) => ({ id, title: id, agents: ['Agent 1'], kind, state }) as NeedsYouCandidate;

it('counts only unread readings, unanswered blocks and reviews waiting for the reader', () => {
  expect(
    pendingNeedsYou([
      candidate('change', 'changes', 'review'),
      candidate('review', 'waiting', 'review'),
      candidate('answer', 'unanswered', 'answer'),
      candidate('closed', 'closed', 'review'),
      candidate('read', 'read', 'reading'),
      candidate('reading', 'unread', 'reading'),
      candidate('answered', 'answered', 'answer'),
    ]).map(({ id }) => id),
  ).toEqual(['reading', 'answer', 'review']);
  expect(pendingNeedsYou([candidate('read', 'read', 'reading')])).toEqual([]);
});

it('counts a target ID once, preserving order within each kind', () => {
  const first = candidate('first', 'unread', 'reading');
  expect(
    pendingNeedsYou([
      first,
      candidate('second', 'unread', 'reading'),
      first,
      candidate('first', 'waiting', 'review'),
    ]).map(({ id }) => id),
  ).toEqual(['first', 'second']);
});

const question = (id: string): Question => ({
  id,
  title: id,
  kind: 'short',
  choices: [],
  other: false,
  agent: 'Agent 1',
  group: 'group',
  label: null,
});
const answer = (body: string, deviceAuthor = true, deleted = false) =>
  ({
    body,
    deviceAuthor,
    deleted,
    ref: { writer: 'device', id: 'comment' },
    sequence: '1',
  }) as CommentView;

it('uses the canonical device answer; an empty sibling or agent prose does not hide pending questions', () => {
  const first = question('First'),
    second = question('Second');
  const threads = [
    { ref: { writer: 'a', id: 'empty' }, comments: [] },
    {
      ref: { writer: 'b', id: 'answered' },
      comments: [answer('Answer to “First”: yes\nPage: Page')],
    },
  ];
  expect(
    unansweredQuestions([first, second], (id) => (id === 'First' ? threads : [])).map(
      ({ id }) => id,
    ),
  ).toEqual(['Second']);
  for (const comment of [
    answer('Answer to “First”: yes', false),
    answer('Answer to “First”: yes', true, true),
  ]) {
    expect(
      unansweredQuestions([first], () => [
        { ref: { writer: 'a', id: 'thread' }, comments: [comment] },
      ]),
    ).toEqual([first]);
  }
});

it('registered questions own their count independently of a legacy passage with the same ID', () => {
  const reading = candidate('same', 'unread', 'reading');
  const answer = candidate('same', 'unanswered', 'answer');
  expect(
    pendingNeedsYou([answer, reading, answer, candidate('same', 'waiting', 'review')], true),
  ).toEqual([reading, answer]);
});

it('keeps handled attention in the admitted viewer stream and matches the exact item version', async () => {
  const { needsYouKey, readNeedsYouAcknowledgements, validateNeedsYouAcknowledgement } =
    await import('../src/needs-you-acknowledgement.js');
  const { id } = await import('./ask-fixtures.js');
  const scope = { spaceId: 'a'.repeat(32), pageId: id(1), epoch: '1' };
  const item = {
    ...candidate('question-1', 'unanswered', 'answer'),
    version: 'authored-version-one',
  };
  const actionId = id(8);
  const value = {
    version: 1,
    kind: 'needs-you-handled',
    ...scope,
    senderDevice: id(4),
    actionId,
    at: '1',
    items: [{ kind: item.kind, id: item.id, version: item.version }],
  };
  expect(() =>
    validateNeedsYouAcknowledgement(`${actionId}:needs-you-handled`, value),
  ).not.toThrow();
  const own = {
    [id(4)]: {
      threads: {},
      intents: {},
      replies: {},
      messages: { [`${actionId}:needs-you-handled`]: value },
    },
  };
  const handled = readNeedsYouAcknowledgements(own, scope, id(4));
  expect(handled.has(needsYouKey(item))).toBe(true);
  expect(handled.has(needsYouKey({ ...item, version: 'authored-version-two' }))).toBe(false);
  expect(readNeedsYouAcknowledgements(own, scope, id(5)).size).toBe(0);
  expect(readNeedsYouAcknowledgements(own, { ...scope, epoch: '2' }, id(4)).size).toBe(0);
  expect(pendingNeedsYou([item], true, handled)).toEqual([]);
  expect(
    pendingNeedsYou([{ ...item, version: 'authored-version-two' }], true, handled),
  ).toHaveLength(1);
});
