import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { readThreads, discussionKey, type DiscussionRecord } from '../src/thread-records.js';
import {
  openThreadCount,
  projectThreadStatus,
  ThreadStatusSeen,
} from '../src/thread-status-view.js';
import type { OwnState, JsonValue } from '../src/fold-protocol.js';
const fixture = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/discussion-v1.json', import.meta.url), 'utf8'),
);
function thread() {
  const row = fixture.statusCases.find((value: { name: string }) => value.name === 'agent resolve');
  const own: OwnState = {};
  for (const record of [row.thread, ...row.actions] as DiscussionRecord[]) {
    own[record.senderDevice] ??= { threads: {}, messages: {}, intents: {}, replies: {} };
    own[record.senderDevice][record.kind === 'thread' ? 'threads' : 'messages'][
      discussionKey(record)
    ] = record as unknown as JsonValue;
  }
  return readThreads(
    own,
    fixture.scope,
    () => new Uint8Array(32),
    () => true,
  )[0];
}
it('keeps an agent resolution unseen until an explicit open, including reload and new winning actions', () => {
  const value = thread();
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
  };
  const seen = () => new ThreadStatusSeen(fixture.scope, fixture.thread.senderDevice, storage);
  expect(projectThreadStatus(value, seen())).toMatchObject({
    resolved: true,
    unseen: true,
    actor: 'agent',
    actorName: value.status?.agentName,
  });
  expect(values.size).toBe(0);
  expect(seen().unseen(value)).toBe(true);
  seen().opened(value);
  expect(seen().unseen(value)).toBe(false);
  expect(values.size).toBe(1);
  const changed = structuredClone(value);
  changed.status!.ref.id = fixture.comment.messageId;
  expect(seen().unseen(changed)).toBe(true);
  expect(
    new ThreadStatusSeen(
      { ...fixture.scope, epoch: '2' },
      fixture.thread.senderDevice,
      storage,
    ).unseen(value),
  ).toBe(true);
  expect(
    new ThreadStatusSeen(fixture.scope, fixture.comment.messageId, storage).unseen(value),
  ).toBe(true);
});
it('counts only open live non-Chat threads and keeps status independent of attention storage', () => {
  const resolved = thread();
  const open = { ...resolved, resolved: false, status: undefined };
  const chat = { ...open, anchor: null, ref: { ...open.ref, id: open.ref.writer } };
  const deleted = { ...open, deleted: true };
  expect(openThreadCount([resolved, open, chat, deleted])).toBe(1);
  expect(projectThreadStatus(chat, undefined, true).controllable).toBe(false);
  // Resolve/Reopen needs owner-member provenance on the local device, not just a live thread.
  expect(projectThreadStatus(open, undefined, true).controllable).toBe(true);
  expect(projectThreadStatus(open).controllable).toBe(false);
  expect(projectThreadStatus(open, undefined, false).controllable).toBe(false);
  const unavailable = {
    getItem: () => {
      throw new Error('unavailable');
    },
    setItem: () => {
      throw new Error('unavailable');
    },
  };
  const broken = new ThreadStatusSeen(fixture.scope, fixture.thread.senderDevice, unavailable);
  expect(projectThreadStatus(resolved, broken).resolved).toBe(true);
  expect(() => broken.opened(resolved)).not.toThrow();
  expect(broken.unseen(resolved)).toBe(false);
  expect(
    new ThreadStatusSeen(fixture.scope, fixture.thread.senderDevice, unavailable).unseen(resolved),
  ).toBe(true);
});

it('combines undecided proposals and annotations once, independent of decision delivery and resolution', () => {
  const annotation = { ...thread(), resolved: false, status: undefined };
  const proposal = {
    ...annotation,
    proposal: {
      proposalId: annotation.threadId,
      title: 'Review',
      body: 'Review this',
      proposer: {
        machineId: annotation.senderDevice,
        agentId: annotation.senderDevice,
        label: 'Agent',
      },
    },
  };
  const approved = {
    ...proposal,
    decision: fixture.decisionCases
      .flatMap((value: { actions: unknown[] }) => value.actions)
      .find((value: { decision: string }) => value.decision === 'approved'),
  };
  const declined = {
    ...approved,
    decision: { ...approved.decision, decision: 'declined' as const },
  };
  const resolved = { ...proposal, resolved: true };
  const chat = {
    ...annotation,
    anchor: null,
    ref: { ...annotation.ref, id: annotation.ref.writer },
  };
  expect(
    openThreadCount([
      annotation,
      proposal,
      approved,
      declined,
      resolved,
      chat,
      { ...proposal, deleted: true },
    ]),
  ).toBe(2);
  expect(openThreadCount([{ ...resolved, resolved: false }])).toBe(1);
  expect(openThreadCount([{ ...approved, resolved: false }])).toBe(0);
});
