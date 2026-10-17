import { expect, it } from 'vite-plus/test';
import type { Question, Review } from '../src/questions.js';
import {
  answeredComment,
  canonicalReviewThread,
  confirmComment,
  needsChangeComment,
  resolvedByPerson,
  reviewState,
  sectionAction,
  sectionAnswered,
  sectionNote,
} from '../src/reviews.js';
import type { CommentView, ThreadView } from '../src/thread-records.js';

const review: Review = {
  id: '10000000-0000-4000-8000-000000000301',
  title: 'Hosting plan',
  agent: 'Agent 1',
  questions: [],
};
const id = '10000000-0000-4000-8000-000000000001';

function comment(writer: string, messageId: string, sequence: string, body: string, device = true) {
  return {
    version: 1,
    kind: 'comment',
    spaceId: id,
    pageId: id,
    epoch: '1',
    senderDevice: writer,
    deviceName: 'You',
    revision: '1',
    deleted: false,
    at: '0',
    messageId,
    ref: { writer, id: messageId },
    thread: { writer, id },
    sequence,
    body,
    deviceAuthor: device,
  } as CommentView;
}
function thread(
  ref: string,
  comments: CommentView[],
  status?: { resolved: boolean; actor: 'person' | 'agent' },
): ThreadView {
  return {
    ref: { writer: ref, id: ref },
    threadId: ref,
    comments,
    resolved: status?.resolved ?? false,
    deleted: false,
    ...(status
      ? { status: { ...status, deviceName: 'Browser', at: '1', agentName: null } as never }
      : {}),
  } as unknown as ThreadView;
}

it('formats fixed, bounded comments and recognizes only its own head', () => {
  expect(confirmComment(review, '  Looks   right ', 'Page', '@Agent 1')).toBe(
    'Confirmed “Hosting plan”: Looks right\nPage: Page\n@Agent 1',
  );
  expect(confirmComment(review, '', 'Page', '')).toBe('Confirmed “Hosting plan”\nPage: Page');
  expect(needsChangeComment(review, 'First\nSecond', 'Page', '@Agent 1')).toBe(
    'Needs change to “Hosting plan”: First\nSecond\nPage: Page\n@Agent 1',
  );
  expect(answeredComment(review, 'Page')).toBe('Answered “Hosting plan”\nPage: Page');
  // A confirm note is one short line; a change needs a note.
  expect(confirmComment(review, 'x'.repeat(201), 'Page', '')).toBeUndefined();
  expect(confirmComment(review, 'bell\u0007', 'Page', '')).toBeUndefined();
  expect(needsChangeComment(review, '   ', 'Page', '')).toBeUndefined();
  expect(needsChangeComment(review, 'é'.repeat(4097), 'Page', '')).toBeUndefined();
  expect(confirmComment(review, '', 'x'.repeat(16384), '')).toBeUndefined();
  expect(sectionAction(review, 'Confirmed “Hosting plan”: ok')).toBe('confirmed');
  expect(sectionAction(review, 'Needs change to “Hosting plan”: fix')).toBe('changes');
  expect(sectionAction(review, 'Answered “Hosting plan”\nPage: P')).toBe('answered');
  // Another section's head, a longer title and plain text are not this section's action.
  expect(sectionAction(review, 'Confirmed “Hosting plan v2”: ok')).toBeUndefined();
  expect(sectionAction(review, 'Confirmed “Other”: ok')).toBeUndefined();
  expect(sectionAction(review, 'I confirmed “Hosting plan”')).toBeUndefined();
  expect(sectionNote(review, 'Confirmed “Hosting plan”: ok\nPage: P\n@A')).toBe('ok');
  expect(sectionNote(review, 'Confirmed “Hosting plan”\nPage: P')).toBeUndefined();
});

it('closes only on a person Resolve, never on an agent label, a legacy flag or a deleted thread', () => {
  const confirmed = comment('w', 'a', '1', 'Confirmed “Hosting plan”: ok\nPage: P');
  const person = thread('w', [confirmed], { resolved: true, actor: 'person' });
  expect(resolvedByPerson(person)).toBe(true);
  expect(reviewState(review, person)).toEqual({
    kind: 'closed',
    by: 'Browser',
    at: '1',
    note: 'ok',
  });
  const agent = thread('w', [confirmed], { resolved: true, actor: 'agent' });
  expect(resolvedByPerson(agent)).toBe(false);
  expect(reviewState(review, agent)).toEqual({ kind: 'waiting' });
  // A thread flagged resolved with no status action (legacy initial state) is not a person's.
  const legacy = { ...thread('w', [confirmed]), resolved: true } as ThreadView;
  expect(reviewState(review, legacy)).toEqual({ kind: 'waiting' });
  expect(reviewState(review, { ...person, deleted: true })).toEqual({ kind: 'waiting' });
  // Reopen by anyone makes it waiting again, keeping the old confirmation as history only.
  const reopened = thread('w', [confirmed], { resolved: false, actor: 'agent' });
  expect(reviewState(review, reopened)).toEqual({ kind: 'waiting' });
  expect(reviewState(review, undefined)).toEqual({ kind: 'waiting' });
});

it('shows changes requested until an agent replies to the newest note', () => {
  const change = comment('w', 'a', '1', 'Needs change to “Hosting plan”: fix it\nPage: P');
  const open = thread('w', [change]);
  expect(reviewState(review, open)).toEqual({ kind: 'changes', note: 'fix it' });
  expect(reviewState(review, open, new Set(['a']))).toEqual({ kind: 'waiting' });
  // A later reader note re-opens the request even when the first one was answered.
  const again = comment('w', 'b', '2', 'Needs change to “Hosting plan”: and this\nPage: P');
  expect(reviewState(review, thread('w', [change, again]), new Set(['a']))).toEqual({
    kind: 'changes',
    note: 'and this',
  });
  // A forged bridge comment or a plain agent comment never counts.
  const forged = comment('x', 'c', '3', 'Needs change to “Hosting plan”: forged', false);
  expect(reviewState(review, thread('w', [forged]))).toEqual({ kind: 'waiting' });
});

it('picks the canonical section thread like an answer block does', () => {
  const recorded = thread('b', [comment('b', 'c', '1', 'Confirmed “Hosting plan”\nPage: P')]);
  const empty = thread('a', []);
  const forged = thread('a', [comment('a', 'd', '9', 'Confirmed “Hosting plan”', false)]);
  for (const threads of [
    [recorded, empty],
    [empty, recorded],
    [forged, recorded],
  ])
    expect(canonicalReviewThread(review, threads)).toBe(recorded);
  expect(canonicalReviewThread(review, [thread('b', []), empty])).toBe(empty);
});

it('a section is answered only when every question inside has a verified device answer', () => {
  const second = '10000000-0000-4000-8000-000000000002';
  const question = (questionId: string, title: string): Question => ({
    id: questionId,
    title,
    kind: 'short',
    choices: [],
    other: false,
    agent: '',
    group: null,
    label: null,
  });
  const questions = [question(id, 'First?'), question(second, 'Second?')];
  const section: Review = { ...review, questions: [id, second] };
  const answers = new Map<string, ThreadView[]>([
    [id, [thread('a', [comment('a', 'a', '1', 'Answer to “First?”: Yes')])]],
  ]);
  const associated = (questionId: string) => answers.get(questionId) ?? [];
  expect(sectionAnswered(section, questions, associated)).toBe(false);
  answers.set(second, [thread('b', [comment('b', 'b', '1', 'Answer to “Second?”: No', false)])]);
  expect(sectionAnswered(section, questions, associated)).toBe(false);
  answers.set(second, [thread('b', [comment('b', 'b', '1', 'Answer to “Second?”: No')])]);
  expect(sectionAnswered(section, questions, associated)).toBe(true);
  expect(sectionAnswered({ questions: [] }, questions, associated)).toBe(false);
});
