import { expect, it } from 'vite-plus/test';
import {
  answerComment,
  canonicalQuestionThread,
  currentAnswer,
  type Question,
} from '../src/questions.js';
import type { CommentView } from '../src/thread-records.js';
const question: Question = {
  id: '10000000-0000-4000-8000-000000000001',
  title: 'Which colour?',
  kind: 'single',
  choices: [{ label: 'Blue', description: 'A cool colour' }, { label: 'Green' }],
  other: true,
  agent: 'Agent 1',
  group: null,
  label: null,
};
it('bounds fixed plain text answers for every kind and keeps choices in authored order', () => {
  expect(answerComment(question, ['Blue'], '', false, 'Page', '@Agent 1')).toBe(
    'Answer to “Which colour?”: Blue\nPage: Page\n@Agent 1',
  );
  expect(
    answerComment({ ...question, kind: 'multiple' }, ['Green', 'Blue'], '', false, 'Page', ''),
  ).toBe('Answer to “Which colour?”: Blue; Green\nPage: Page');
  expect(answerComment(question, [], 'Yellow', true, 'Page', '@Agent 1')).toContain(
    'Other: Yellow',
  );
  expect(
    answerComment(
      { ...question, kind: 'short', choices: [], other: false },
      [],
      'Yes',
      false,
      'Page',
      '',
    ),
  ).toContain('Yes');
  expect(
    answerComment(
      { ...question, kind: 'long', choices: [], other: false },
      [],
      'First\nSecond',
      false,
      'Page',
      '',
    ),
  ).toContain('First\nSecond');
  for (const [selected, value, other] of [
    [[], '', false],
    [['Forged'], '', false],
    [['Blue', 'Blue'], '', false],
    [['Blue', 'Green'], '', false],
    [[], '', true],
    [['Blue'], 'Yellow', true],
  ] as const)
    expect(answerComment(question, selected, value, other, 'Page', '')).toBeUndefined();
  expect(
    answerComment({ ...question, kind: 'short' }, [], '🐈'.repeat(201), false, 'Page', ''),
  ).toBeUndefined();
  expect(
    answerComment({ ...question, kind: 'long' }, [], 'é'.repeat(4097), false, 'Page', ''),
  ).toBeUndefined();
  expect(answerComment(question, ['Blue'], '', false, 'x'.repeat(16384), '')).toBeUndefined();
});
function comment(
  writer: string,
  id: string,
  sequence: string,
  body: string,
  deviceAuthor = true,
): CommentView {
  return {
    version: 1,
    kind: 'comment',
    spaceId: question.id,
    pageId: question.id,
    epoch: '1',
    senderDevice: writer,
    deviceName: 'You',
    revision: '1',
    deleted: false,
    at: '0',
    messageId: id,
    ref: { writer, id },
    thread: { writer, id: question.id },
    sequence,
    body,
    deviceAuthor,
  };
}
it('the latest verified device answer wins by Lamport order, never time, replies or bridge comments', () => {
  const first = comment('a', 'a', '1', 'Answer to “Which colour?”: Blue');
  const later = comment('b', 'b', '2', 'Answer to “Which colour?”: Green');
  const bridge = comment('c', 'c', '3', 'Answer to “Which colour?”: forged', false);
  const reply = comment('c', 'd', '4', 'Agent explanation');
  const removed = { ...later, ref: { writer: 'd', id: 'd' }, sequence: '5', deleted: true };
  for (const comments of [
    [first, bridge, later, reply, removed],
    [removed, reply, later, bridge, first],
  ])
    expect(currentAnswer(question, comments)).toBe(later);
  expect(currentAnswer(question, [{ ...later, deviceAuthor: undefined }])).toBeUndefined();
});

it('a later empty sibling with a smaller id cannot take over an answered thread', () => {
  const answered = {
    ref: { writer: 'b', id: 'b' },
    comments: [comment('b', 'c', '1', 'Answer to “Which colour?”: Blue')],
  };
  const empty = { ref: { writer: 'a', id: 'a' }, comments: [] };
  const agent = {
    ...empty,
    comments: [comment('a', 'd', '9', 'Answer to “Which colour?”: forged', false)],
  };
  for (const threads of [
    [answered, empty],
    [empty, answered],
    [agent, answered],
  ])
    expect(canonicalQuestionThread(question, threads)).toBe(answered);
  const unanswered = { ...answered, comments: [] };
  expect(canonicalQuestionThread(question, [unanswered, empty])).toBe(empty);
});
