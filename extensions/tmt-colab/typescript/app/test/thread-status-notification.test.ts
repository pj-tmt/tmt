import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { readThreads, discussionKey, type DiscussionRecord } from '../src/thread-records.js';
import {
  statusNotificationForAsk,
  projectStatusNotifications,
} from '../src/thread-status-notification.js';
import type { OwnState, JsonValue } from '../src/fold-protocol.js';
import type { PageAsk } from '../src/ask-panel.js';
import { projectThreadPresentation } from '../src/thread-status-presentation.js';
import { ThreadStatusSeen } from '../src/thread-status-view.js';
const f = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/discussion-v1.json', import.meta.url), 'utf8'),
);
function fixture() {
  const own: OwnState = {};
  const action = f.recipientCases[0].record;
  for (const record of [f.thread, f.comment, action] as DiscussionRecord[]) {
    own[record.senderDevice] ??= { threads: {}, messages: {}, intents: {}, replies: {} };
    own[record.senderDevice][record.kind === 'thread' ? 'threads' : 'messages'][
      discussionKey(record)
    ] = record as unknown as JsonValue;
  }
  const read = () => readThreads(own, f.scope, () => new Uint8Array(32));
  const thread = read()[0];
  const context = {
    status: thread.status!.ref,
    recipient: thread.status!.recipients[0],
    thread: structuredClone(thread),
  };
  return { own, read, thread, context };
}
it('readmits a committed person Resolve and exact frozen discussion without inventing a comment or operation', () => {
  const x = fixture();
  const origin = statusNotificationForAsk(x.read(), x.context, x.thread.status!.senderDevice);
  expect(origin).toMatchObject({
    thread: x.thread.threadId,
    messageIds: [],
    operationId: x.context.recipient.operationId,
    quote: x.thread.anchor?.exact,
  });
  expect(origin.comment).toContain('Thread resolved by');
  expect(origin.comment).toContain(x.thread.comments[0].body);
  for (const mutate of [
    () => {
      x.context.status.id = crypto.randomUUID();
    },
    () => {
      x.context.recipient.agent = crypto.randomUUID();
    },
    () => {
      x.context.thread.revision = '2';
    },
    () => {
      x.context.thread.comments[0].body = 'replaced frozen body';
    },
  ]) {
    const pristine = structuredClone(x.context);
    mutate();
    expect(() =>
      statusNotificationForAsk(x.read(), x.context, x.thread.status!.senderDevice),
    ).toThrow();
    x.context = pristine;
  }
  expect(() => statusNotificationForAsk(x.read(), x.context, crypto.randomUUID())).toThrow();
  expect(() =>
    statusNotificationForAsk([x.thread, x.thread], x.context, x.thread.status!.senderDevice),
  ).toThrow();
});
it('projects admitted pre-ledger failure, held, refused and uncertain outcomes after reload without dispatch capability', () => {
  const x = fixture();
  expect(projectStatusNotifications(x.thread, [])).toMatchObject([
    { state: 'uncertain', canTrack: false },
  ]);
  const failure = {
    ...f.notification,
    operationId: x.context.recipient.operationId,
    status: x.context.status,
  };
  x.own[failure.senderDevice].messages[discussionKey(failure)] = failure;
  expect(projectStatusNotifications(x.read()[0], [])).toMatchObject([
    { state: 'unavailable', reason: failure.reason, canTrack: false },
  ]);
  const ask: PageAsk = {
    operationId: x.context.recipient.operationId,
    writer: x.context.status.writer,
    thread: x.thread.threadId,
    message: 'Resolution notification',
    agent: x.context.recipient.agent,
    agentName: 'Renamed agent',
    deviceName: 'Browser',
    issuedAt: 1,
    machine: x.context.recipient.machine,
    state: 'held',
    canTrack: true,
  };
  for (const state of ['held', 'refused', 'uncertain'] as const)
    expect(projectStatusNotifications(x.read()[0], [{ ...ask, state }])).toMatchObject([
      { state, canTrack: true },
    ]);
  expect(
    projectStatusNotifications(x.read()[0], [{ ...ask, writer: crypto.randomUUID() }]),
  ).toMatchObject([{ state: 'unavailable' }]);
  x.own[failure.senderDevice].messages[discussionKey(failure)] = {
    ...failure,
    status: { ...failure.status, id: crypto.randomUUID() },
  };
  expect(projectStatusNotifications(x.read()[0], [])).toMatchObject([{ state: 'uncertain' }]);
});

it('supplies the window one parent-owned status/outcome view without storage or render-side writes', () => {
  const x = fixture();
  const failure = {
    ...f.notification,
    operationId: x.context.recipient.operationId,
    status: x.context.status,
  };
  x.own[failure.senderDevice].messages[discussionKey(failure)] = failure;
  const thread = x.read()[0];
  let writes = 0;
  const seen = new ThreadStatusSeen(f.scope, f.thread.senderDevice, {
    getItem: () => null,
    setItem: () => {
      writes++;
    },
  });
  const presentation = projectThreadPresentation(thread, [], seen);
  expect(Object.keys(presentation)).toEqual(['thread', 'status', 'notificationOutcomes']);
  expect(presentation.status).toMatchObject({ resolved: true, controllable: true });
  expect(presentation.notificationOutcomes).toMatchObject([
    { state: 'unavailable', reason: failure.reason, canTrack: false },
  ]);
  expect(writes).toBe(0);
  expect(structuredClone(presentation)).toEqual(presentation);
  expect(projectThreadPresentation(thread, [], seen)).toEqual(presentation);
  expect(writes).toBe(0);
});

it('uses the admitted Ask reason including null without inheriting a stale pre-ledger failure', () => {
  for (const reason of ['PREPARATION_FAILED', 'RECIPIENT_UNAVAILABLE'] as const) {
    const x = fixture();
    const failure = {
      ...f.notification,
      operationId: x.context.recipient.operationId,
      status: x.context.status,
      reason,
    };
    x.own[failure.senderDevice].messages[discussionKey(failure)] = failure;
    const thread = x.read()[0];
    const ask: PageAsk = {
      operationId: x.context.recipient.operationId,
      writer: x.context.status.writer,
      thread: x.thread.threadId,
      message: 'Resolution notification',
      agent: x.context.recipient.agent,
      agentName: 'Renamed agent',
      deviceName: 'Browser',
      issuedAt: 1,
      machine: x.context.recipient.machine,
      state: 'accepted',
      reason: null,
      canTrack: true,
    };
    const original = structuredClone({ thread, ask });
    for (const state of ['accepted', 'held', 'uncertain'] as const) {
      const outcomes = projectStatusNotifications(thread, [{ ...ask, state }]);
      expect(outcomes).toMatchObject([{ state, reason: null, canTrack: true }]);
      expect(Object.values(outcomes[0]).some((value) => typeof value === 'function')).toBe(false);
    }
    expect({ thread, ask }).toEqual(original);
    expect(thread.notifications).toMatchObject([{ reason }]);
    expect(projectStatusNotifications(thread, [])).toMatchObject([
      { state: 'unavailable', reason, canTrack: false },
    ]);
  }
});
