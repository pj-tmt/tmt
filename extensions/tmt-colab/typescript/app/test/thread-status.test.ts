import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import {
  discussionKey,
  readThreads,
  validateDiscussionRecord,
  type ThreadRecord,
} from '../src/thread-records.js';
import {
  foldThreadStatus,
  type ThreadStatusRecord,
  type ThreadNotificationRecord,
} from '../src/thread-status.js';
import type { OwnState, JsonValue } from '../src/fold-protocol.js';
import { projectConversations, renderConversationsMarkdown } from '../src/conversations.js';
const fixture = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/discussion-v1.json', import.meta.url), 'utf8'),
);
interface Case {
  name: string;
  thread: ThreadRecord;
  threadHistory?: ThreadRecord[];
  actions: ThreadStatusRecord[];
  notifications?: ThreadNotificationRecord[];
  expected: {
    resolved: boolean;
    status: { writer: string; id: string; depth: number } | null;
    threadsJson: string;
    markdown: string;
  };
}
for (const row of fixture.statusCases as Case[]) {
  it(`causal status vector: ${row.name}`, () => {
    const fold = (actions: ThreadStatusRecord[]) =>
      foldThreadStatus(
        row.thread,
        row.thread.senderDevice,
        actions.map((record) => ({
          ...record,
          ref: { writer: record.senderDevice, id: record.actionId },
        })),
      );
    for (const actions of [row.actions, [...row.actions].reverse()]) {
      const value = fold(actions);
      expect(value.resolved).toBe(row.expected.resolved);
      expect(value.status ? { ...value.status.ref, depth: value.status.depth } : null).toEqual(
        row.expected.status,
      );
    }
    // A wall clock and a display label cannot change the winning action.
    const clocks = row.actions.map((r, index) => ({
      ...r,
      at: String(8640000000000000 - index),
      deviceName: 'Another label',
    }));
    expect(fold(clocks).status?.ref).toEqual(fold(row.actions).status?.ref);
  });
}
it('admits immutable action and pre-dispatch notification grammar, refusing extra fields and substituted bindings', () => {
  const action = fixture.statusCases[2].actions[0];
  for (const record of [action, fixture.notification]) {
    const key = discussionKey(record);
    expect(() => validateDiscussionRecord('messages', key, record)).not.toThrow();
    for (const change of [
      { revision: '2' },
      { deleted: true },
      { at: '01' },
      { senderDevice: 'invalid' },
      { unexpected: true },
    ])
      expect(() => validateDiscussionRecord('messages', key, { ...record, ...change })).toThrow();
    expect(() => validateDiscussionRecord('threads', key, record)).toThrow();
    expect(() => validateDiscussionRecord('messages', 'wrong', record)).toThrow();
  }
  for (const change of [
    { previous: {} },
    { actor: 'editor' },
    { actor: 'person', agentName: 'Pretend' },
    { previous: undefined },
    { agentName: undefined },
    { recipients: [{}] },
  ])
    expect(() =>
      validateDiscussionRecord('messages', discussionKey(action), { ...action, ...change }),
    ).toThrow();
  expect(() =>
    validateDiscussionRecord('messages', discussionKey(fixture.notification), {
      ...fixture.notification,
      status: { ...fixture.notification.status, writer: fixture.thread.senderDevice },
    }),
  ).toThrow();
});
it('projects only historically authenticated, writer-bound page/epoch actions without changing thread ownership', () => {
  const thread = fixture.thread,
    action = fixture.statusCases[2].actions[0];
  const own: OwnState = {};
  for (const record of [thread, action]) {
    own[record.senderDevice] ??= { threads: {}, messages: {}, intents: {}, replies: {} };
    own[record.senderDevice][record.kind === 'thread' ? 'threads' : 'messages'][
      discussionKey(record)
    ] = record as JsonValue;
  }
  const read = (keys: (writer: string) => Uint8Array | undefined = () => new Uint8Array(32)) =>
    readThreads(own, fixture.scope, keys, (writer) => keys(writer) !== undefined)[0];
  expect(read()).toMatchObject({
    resolved: true,
    ref: { writer: thread.senderDevice, id: thread.threadId },
  });
  expect(
    read((writer?: string) => (writer === action.senderDevice ? undefined : new Uint8Array(32)))
      .resolved,
  ).toBe(false);
  // A keyed writer without owner-device provenance, a bridge, stays readable but cannot resolve.
  expect(
    readThreads(
      own,
      fixture.scope,
      () => new Uint8Array(32),
      (writer) => writer !== action.senderDevice,
    )[0],
  ).toMatchObject({ resolved: false, ref: { writer: thread.senderDevice, id: thread.threadId } });
  for (const change of [
    { pageId: fixture.comment.messageId },
    { epoch: '2' },
    { senderDevice: thread.senderDevice },
  ]) {
    own[action.senderDevice].messages[discussionKey(action)] = { ...action, ...change };
    expect(read().resolved).toBe(false);
  }
});

for (const row of fixture.statusCases as Case[]) {
  it(`exported effective status/provenance bytes: ${row.name}`, async () => {
    const own: OwnState = {};
    for (const record of [
      ...(row.threadHistory ?? []),
      row.thread,
      ...row.actions,
      ...(row.notifications ?? []),
    ]) {
      own[record.senderDevice] ??= { threads: {}, messages: {}, intents: {}, replies: {} };
      // Deliberately reverse field insertion order; export owns canonical order.
      own[record.senderDevice][record.kind === 'thread' ? 'threads' : 'messages'][
        discussionKey(record)
      ] = Object.fromEntries(Object.entries(record).reverse()) as JsonValue;
    }
    const output = await projectConversations({
      ...fixture.scope,
      title: 'Status vectors',
      membershipHead: { revision: '1', statementHash: '00'.repeat(32) },
      own,
      signingKey: () => new Uint8Array(32),
      statusWriter: () => true,
    });
    expect(JSON.stringify(output.threads)).toBe(row.expected.threadsJson);
    expect(renderConversationsMarkdown(output)).toBe(row.expected.markdown);
  });
}

for (const row of fixture.recipientCases) {
  it(`shared recipient grammar: ${row.name}`, () => {
    const validate = () =>
      validateDiscussionRecord('messages', discussionKey(row.record), row.record);
    if (row.valid) expect(validate).not.toThrow();
    else expect(validate).toThrow();
  });
}
it('bounds frozen recipients before admitting a status action', () => {
  const record = fixture.recipientCases[0].record;
  expect(() =>
    validateDiscussionRecord('messages', discussionKey(record), {
      ...record,
      recipients: Array(1001).fill(record.recipients[0]),
    }),
  ).toThrow();
});
