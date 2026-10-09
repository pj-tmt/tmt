import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import {
  freezeThreadRecipients,
  ThreadStatusCoordinator,
} from '../src/thread-status-coordinator.js';
import type { AgentDestination } from '../src/live-ask.js';
import type { ThreadView } from '../src/thread-records.js';
import type { PageAsk } from '../src/ask-panel.js';
import type { ThreadStatusView } from '../src/thread-status.js';
const f = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/discussion-v1.json', import.meta.url), 'utf8'),
);
const ref = { writer: f.thread.senderDevice, id: f.thread.threadId };
function thread(body = '@alpha and @beta; @alpha again'): ThreadView {
  return {
    ...f.thread,
    ref,
    comments: [
      {
        ...f.comment,
        body,
        deleted: false,
        ref: { writer: f.comment.senderDevice, id: f.comment.messageId },
      },
    ],
  };
}
function agent(name: string, digit: string): AgentDestination {
  return {
    machine: '60000000-0000-4000-8000-000000000001',
    machineName: 'Test machine',
    agent: `70000000-0000-7000-8000-00000000000${digit}`,
    agentName: name,
    online: 'online',
    grantExpiresAt: null,
    deviceName: 'Browser',
    grantRevision: '1',
    mode: 'direct',
  };
}
function ask(): PageAsk {
  return {
    thread: ref.id,
    messageIds: [f.comment.messageId],
    operationId: crypto.randomUUID(),
    writer: f.comment.senderDevice,
    message: '@alpha prior turn',
    agent: agent('alpha', '1').agent,
    agentName: 'alpha',
    deviceName: 'Browser',
    issuedAt: 1,
    machine: agent('alpha', '1').machine,
    state: 'accepted',
    canTrack: true,
  };
}
it('freezes every unique mention including non-repliers, deduplicates and retains signed IDs after a rename', () => {
  const a = agent('renamed', '1'),
    b = agent('beta', '2');
  const frozen = freezeThreadRecipients(thread(), [ask()], [a, b]);
  expect(frozen.recipients.map((value) => value.agent)).toEqual([a.agent, b.agent]);
  expect(frozen.recipients.map((value) => value.agentName)).toEqual(['alpha', 'beta']);
  expect(new Set(frozen.recipients.map((value) => value.operationId)).size).toBe(2);
  expect(frozen.warnings).toEqual([]);
  expect(freezeThreadRecipients(thread('mail@alpha @missing @beta'), [], [b]).warnings).toEqual([
    { name: 'missing', reason: 'unknown' },
  ]);
  const ambiguous = freezeThreadRecipients(
    thread('@alpha'),
    [],
    [agent('alpha', '1'), agent('alpha', '2')],
  );
  expect(ambiguous.recipients).toEqual([]);
  expect(ambiguous.warnings).toEqual([{ name: 'alpha', reason: 'ambiguous' }]);
  expect(
    freezeThreadRecipients(
      { ...thread(), comments: thread().comments.map((value) => ({ ...value, deleted: true })) },
      [ask()],
      [a, b],
    ).recipients,
  ).toEqual([]);
});
it('publishes operation IDs before one sequential attempt per recipient, coalesces clicks and preserves partial outcomes', async () => {
  const events: string[] = [];
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: async () => [agent('alpha', '1'), agent('beta', '2')],
    binding: {
      setStatus: async (_, previous, resolved, recipients) => {
        expect(previous).toBe(null);
        expect(resolved).toBe(true);
        events.push('published');
        await held;
        const record: ThreadStatusView = {
          ...f.recipientCases[0].record,
          thread: ref,
          recipients: [...recipients!],
          ref: { writer: f.thread.senderDevice, id: f.recipientCases[0].record.actionId },
          depth: 1,
        };
        return { changed: true, status: record };
      },
      notificationFailed: async (_, id, reason) => {
        expect(reason).toBe('RECIPIENT_UNAVAILABLE');
        events.push(`failed:${id}`);
      },
    },
    notify: async (status, recipient, captured) => {
      expect(status.recipients).toContainEqual(recipient);
      expect(captured.comments[0].body).toContain('@beta');
      events.push(`attempt:${recipient.agent}`);
      if (recipient.agent === agent('alpha', '1').agent)
        return { adopted: false, reason: 'RECIPIENT_UNAVAILABLE' };
      throw new Error('uncertain ledger outcome');
    },
  });
  const one = coordinator.change(thread(), true),
    two = coordinator.change(thread(), true);
  expect(one).toBe(two);
  release();
  expect((await one).changed).toBe(true);
  expect(events[0]).toBe('published');
  expect(events.filter((value) => value.startsWith('attempt:'))).toHaveLength(2);
  expect(events.filter((value) => value.startsWith('failed:'))).toHaveLength(1);
});
it('no-op, reopen and failed status publication cannot notify or enumerate recipients unnecessarily', async () => {
  let sends = 0,
    lists = 0,
    writes = 0;
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: async () => {
      lists++;
      return [];
    },
    binding: {
      setStatus: async () => {
        writes++;
        throw new Error('publication refused');
      },
      notificationFailed: async () => {},
    },
    notify: async () => {
      sends++;
      return { adopted: true };
    },
  });
  expect((await coordinator.change(thread(), false)).changed).toBe(false);
  await expect(coordinator.change({ ...thread(), resolved: true }, false)).rejects.toThrow(
    'publication refused',
  );
  expect([sends, lists, writes]).toEqual([0, 0, 1]);
});
