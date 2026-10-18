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
