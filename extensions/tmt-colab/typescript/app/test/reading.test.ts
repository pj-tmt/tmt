import { expect, it } from 'vite-plus/test';
import type { Reading } from '../src/questions.js';
import {
  canonicalReadingThread,
  currentReadingComment,
  isReadingComment,
  readingComment,
  readingState,
  readingNotification,
} from '../src/reading.js';
import type { CommentView, ThreadView } from '../src/thread-records.js';

const reading: Reading = {
  id: '10000000-0000-4000-8000-000000000401',
  title: 'Hosting',
  agent: 'Agent 1',
  why: 'Read before deciding',
};
function comment(body = readingComment(reading, 'Page', '@Agent 1')!): CommentView {
  return {
    ref: { writer: 'device', id: 'comment' },
    body,
    deviceAuthor: true,
    deleted: false,
    sequence: '1',
  } as CommentView;
}
function thread(comments: CommentView[], actor: 'person' | 'agent' = 'person'): ThreadView {
  return {
    ref: { writer: 'device', id: 'thread' },
    comments,
    deleted: false,
    resolved: true,
    status: { resolved: true, actor, deviceName: 'Browser', at: '123' },
  } as ThreadView;
}
it('records a bounded fixed comment with an exact reading ID, not an arbitrary prose match', () => {
  const body = readingComment(reading, 'Page', '@Agent 1')!;
  expect(body).toBe(
    'Read “Hosting”\nReading: 10000000-0000-4000-8000-000000000401\nPage: Page\n@Agent 1',
  );
  expect(isReadingComment(reading, body)).toBe(true);
  expect(isReadingComment({ ...reading, id: '10000000-0000-4000-8000-000000000402' }, body)).toBe(
    false,
  );
  for (const other of [
    'I read this',
    body + '\nextra',
    body.replace('Reading:', 'reading:'),
    body.replace('Page: ', ''),
  ])
    expect(isReadingComment(reading, other)).toBe(false);
  expect(readingComment(reading, 'x'.repeat(16384), '')).toBeUndefined();
});
it('requires a nondeleted verified device record and a person Resolve before showing read', () => {
  const record = comment();
  expect(readingState(reading, thread([record]))).toEqual({
    kind: 'read',
    by: 'Browser',
    at: '123',
  });
  expect(readingState(reading, thread([record], 'agent'))).toEqual({ kind: 'unread' });
  for (const invalid of [
    { ...record, deleted: true },
    { ...record, deviceAuthor: false },
    { ...record, deviceAuthor: undefined },
  ]) {
    expect(currentReadingComment(reading, [invalid])).toBeUndefined();
    expect(readingState(reading, thread([invalid]))).toEqual({ kind: 'unread' });
  }
  expect(readingState(reading, { ...thread([record]), deleted: true })).toEqual({ kind: 'unread' });
  expect(readingState(reading, { ...thread([record]), resolved: false })).toEqual({
    kind: 'unread',
  });
});
it('identical headings with distinct reading IDs never share state; siblings have a stable canonical thread', () => {
  const first = thread([comment()]);
  const empty = { ...first, ref: { writer: 'a', id: 'a' }, comments: [] };
  const sibling = { ...first, ref: { writer: 'b', id: 'b' } };
  for (const list of [
    [first, empty, sibling],
    [sibling, first, empty],
  ])
    expect(canonicalReadingThread(reading, list)).toBe(sibling);
  const another = { ...reading, id: '10000000-0000-4000-8000-000000000402' };
  expect(canonicalReadingThread(another, [first])).toBeUndefined();
  expect(readingState(another, first)).toEqual({ kind: 'unread' });
});

it('notification wording reads the existing ledger outcome, not a successful Resolve or adoption', () => {
  const statusRef = { writer: 'device', id: 'status' };
  const recipient = {
    machine: 'machine',
    agent: 'agent',
    agentName: 'alpha',
    operationId: 'operation',
  };
  const status = { ...thread([comment()]).status!, ref: statusRef, recipients: [recipient] };
  const saved = { ...thread([comment()]), threadId: 'thread', status, statuses: [status] };
  expect(readingNotification(saved, [])?.state).toBe('uncertain');
  for (const state of [
    'accepted',
    'held',
    'refused',
    'uncertain',
    'dispatching',
    'failed',
    'cancelled',
    'expired',
    'abandoned',
  ] as const) {
    const ask = {
      writer: 'device',
      operationId: 'operation',
      machine: 'machine',
      agent: 'agent',
      thread: 'thread',
      state,
    } as import('../src/ask-panel.js').PageAsk;
    expect(readingNotification(saved, [ask])?.state).toBe(state);
  }
  const replied = {
    writer: 'device',
    operationId: 'operation',
    machine: 'machine',
    agent: 'agent',
    thread: 'thread',
    state: 'accepted',
    reply: 'Done',
  } as import('../src/ask-panel.js').PageAsk;
  expect(readingNotification(saved, [replied])).toMatchObject({ state: 'accepted', reply: 'Done' });
  const failed = {
    ...saved,
    notifications: [
      { status: statusRef, operationId: 'operation', reason: 'RECIPIENT_UNAVAILABLE' },
    ],
  } as ThreadView;
  expect(readingNotification(failed, [])).toMatchObject({
    state: 'unavailable',
    reason: 'RECIPIENT_UNAVAILABLE',
  });
});
