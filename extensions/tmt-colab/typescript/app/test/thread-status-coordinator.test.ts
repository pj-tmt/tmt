import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import {
  freezeThreadRecipients,
  ThreadStatusCoordinator,
  type ThreadStatusCoordinatorOptions,
} from '../src/thread-status-coordinator.js';
import type { AgentDestination } from '../src/live-ask.js';
import type { ThreadView } from '../src/thread-records.js';
import type { PageAsk } from '../src/ask-panel.js';
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
it('confirms publication before notifications finish, keeping one serial task and the click fence', async () => {
  const events: string[] = [];
  let release!: () => void;
  let finish!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const finished = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: async () => [agent('alpha', '1'), agent('beta', '2')],
    binding: {
      setStatus: async (_, previous, resolved, recipients) => {
        expect(previous).toBe(null);
        expect(resolved).toBe(true);
        events.push('published');
        return {
          changed: true,
          status: {
            ...f.recipientCases[0].record,
            thread: ref,
            recipients: [...(await recipients)!],
            ref: { writer: f.thread.senderDevice, id: f.recipientCases[0].record.actionId },
            depth: 1,
          },
        };
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
      if (recipient.agent === agent('alpha', '1').agent) {
        await held;
        return { adopted: false, reason: 'RECIPIENT_UNAVAILABLE' };
      }
      finish();
      throw new Error('uncertain ledger outcome');
    },
  });
  const one = coordinator.change(thread(), true);
  expect(coordinator.change(thread(), true)).toBe(one);
  // A blocked notification must not block the confirmed status result.
  expect((await one).changed).toBe(true);
  expect(events[0]).toBe('published');
  expect(events.filter((value) => value.startsWith('attempt:'))).toHaveLength(1);
  expect(coordinator.change(thread(), true)).toBe(one);
  release();
  await finished;
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

it('a reading close freezes only its named current agent, never title, reason or prose mentions', () => {
  const a = agent('alpha', '1');
  const b = agent('beta', '2');
  const captured = thread('Read “@beta”\nReading: reading-id\nPage: @beta\n@alpha\nProse @missing');
  const frozen = freezeThreadRecipients(captured, [ask()], [a, b], 'alpha');
  expect(Object.keys(frozen.recipients[0]).sort()).toEqual([
    'agent',
    'agentName',
    'machine',
    'operationId',
  ]);
  expect(
    frozen.recipients.map(({ machine, agent, agentName }) => ({ machine, agent, agentName })),
  ).toEqual([{ machine: a.machine, agent: a.agent, agentName: 'alpha' }]);
  expect(frozen.warnings).toEqual([]);
  expect(freezeThreadRecipients(captured, [ask()], [], 'alpha')).toEqual({
    recipients: [],
    warnings: [{ name: 'alpha', reason: 'unknown' }],
  });
  expect(freezeThreadRecipients(captured, [], [a, agent('alpha', '3')], 'alpha')).toEqual({
    recipients: [],
    warnings: [{ name: 'alpha', reason: 'ambiguous' }],
  });
});

it('immediate Reopen succeeds and supersedes the unfinished Resolve notifications', async () => {
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let drained!: () => void;
  const settled = new Promise<void>((resolve) => {
    drained = resolve;
  });
  const attempts: string[] = [];
  const writes: boolean[] = [];
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: async () => [agent('alpha', '1'), agent('beta', '2')],
    binding: {
      setStatus: async (_, previous, resolved, recipients) => {
        writes.push(resolved);
        return {
          changed: true,
          status: {
            ...f.recipientCases[0].record,
            thread: ref,
            resolved,
            previous,
            recipients: [...(await recipients)!],
            ref: { writer: f.thread.senderDevice, id: crypto.randomUUID() },
            depth: writes.length,
          },
        };
      },
      notificationFailed: async () => {
        drained();
      },
    },
    notify: async (_, recipient) => {
      attempts.push(recipient.agentName);
      await held;
      return { adopted: false, reason: 'RECIPIENT_UNAVAILABLE' };
    },
  });
  const resolved = await coordinator.change(thread(), true);
  if (!resolved.changed) throw new Error('Expected Resolve');
  const reopened = await coordinator.change(
    { ...thread(), resolved: true, status: resolved.status },
    false,
  );
  expect(reopened.changed).toBe(true);
  expect(writes).toEqual([true, false]);
  release();
  await settled;
  await Promise.resolve();
  expect(attempts).toEqual(['alpha']);
});

function batchThread(body: string) {
  const value = thread(body);
  value.ref = { ...value.ref, id: crypto.randomUUID() };
  value.threadId = value.ref.id;
  return value;
}

it('Resolve all freezes one snapshot and directory, commits before sending, and notifies aliases once per agent', async () => {
  const first = batchThread('@alpha and @alias');
  const second = batchThread('@alias and @beta');
  const a = agent('alpha', '1'),
    b = { ...agent('beta', '1'), machine: '60000000-0000-4000-8000-000000000002' };
  const events: string[] = [];
  const writes: { id: string; recipients: string[] }[] = [];
  let lists = 0;
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let finished!: () => void;
  const drained = new Promise<void>((resolve) => {
    finished = resolve;
  });
  let attempts = 0;
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: async () => {
      lists++;
      return [a, { ...a, agentName: 'alias' }, b];
    },
    binding: {
      setStatus: async (ref, previous, resolved, recipients) => {
        expect(previous).toBe(null);
        expect(resolved).toBe(true);
        const frozen = [...(await recipients)!];
        writes.push({ id: ref.id, recipients: frozen.map((value) => value.agent) });
        events.push(`commit:${ref.id}`);
        return {
          changed: true,
          status: {
            ...f.recipientCases[0].record,
            thread: ref,
            recipients: frozen,
            ref: { writer: ref.writer, id: crypto.randomUUID() },
            depth: 1,
          },
        };
      },
      notificationFailed: async () => {},
    },
    notify: async (status, recipient, captured) => {
      expect(events).toContain(`commit:${captured.ref.id}`);
      expect(captured.comments[0].body).not.toContain('mutated');
      expect(status.recipients).toContainEqual(recipient);
      attempts++;
      events.push(`send:${recipient.agent}`);
      if (recipient.machine === a.machine && recipient.agent === a.agent) await held;
      if (attempts === 2) finished();
      return { adopted: true };
    },
  });
  const batch = coordinator.resolveAll([first, second, first]);
  expect(coordinator.resolveAll([first, second])).toBe(batch);
  second.comments[0].body = '@mutated';
  const result = await batch;
  let settled = false;
  void result.settled.then(() => {
    settled = true;
  });
  expect(settled).toBe(false);
  expect(result.results).toHaveLength(2);
  expect(writes).toEqual([
    { id: first.ref.id, recipients: [a.agent] },
    { id: second.ref.id, recipients: [b.agent] },
  ]);
  expect(lists).toBe(1);
  expect(coordinator.resolveAll([first, second])).toBe(batch);
  release();
  await drained;
  await result.settled;
  expect(settled).toBe(true);
  expect(events.filter((value) => value.startsWith('send:')).sort()).toEqual([
    `send:${a.agent}`,
    `send:${b.agent}`,
  ]);
});

it('Resolve all reports partial publication and assigns a recipient only after a successful commit', async () => {
  const noop = batchThread('@alpha');
  const refused = batchThread('@alpha');
  const committed = batchThread('@alpha @missing');
  const attempts: string[] = [],
    failures: string[] = [];
  let notified!: () => void;
  const drained = new Promise<void>((resolve) => {
    notified = resolve;
  });
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: async () => [agent('alpha', '1')],
    binding: {
      setStatus: async (ref, _previous, _resolved, recipients) => {
        if (ref.id === noop.ref.id) return { changed: false };
        if (ref.id === refused.ref.id) throw new Error('writer admission changed');
        return {
          changed: true,
          status: {
            ...f.recipientCases[0].record,
            thread: ref,
            recipients: [...(await recipients)!],
            ref: { writer: ref.writer, id: crypto.randomUUID() },
            depth: 1,
          },
        };
      },
      notificationFailed: async (status, _operation, reason) => {
        failures.push(`${status.thread.id}:${reason}`);
        notified();
      },
    },
    notify: async (_status, recipient, captured) => {
      attempts.push(`${captured.ref.id}:${recipient.agentName}`);
      return { adopted: false, reason: 'RECIPIENT_UNAVAILABLE' };
    },
  });
  const { results } = await coordinator.resolveAll([noop, refused, committed]);
  expect(results[0]).toMatchObject({ thread: noop.ref, result: { changed: false } });
  expect(results[1]).toMatchObject({ thread: refused.ref, error: expect.any(Error) });
  expect(results[2]).toMatchObject({
    thread: committed.ref,
    result: { changed: true, warnings: [{ name: 'missing', reason: 'unknown' }] },
  });
  await drained;
  expect(attempts).toEqual([`${committed.ref.id}:alpha`]);
  expect(failures).toEqual([`${committed.ref.id}:RECIPIENT_UNAVAILABLE`]);
});

it('an empty Resolve all does not read the directory or write a status', async () => {
  const unexpected = async () => {
    throw new Error('unexpected work');
  };
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: unexpected,
    binding: { setStatus: unexpected, notificationFailed: unexpected },
    notify: unexpected,
  });
  const result = await coordinator.resolveAll([]);
  expect(result.results).toEqual([]);
  await result.settled;
});

it('Resolve all never retries an uncertain first adoption on another thread', async () => {
  const first = batchThread('@alpha'),
    second = batchThread('@alpha');
  const attempts: string[] = [];
  let failures = 0;
  const options: ThreadStatusCoordinatorOptions = {
    asks: () => [],
    destinations: async () => [agent('alpha', '1')],
    binding: {
      setStatus: async (ref, _previous, _resolved, recipients) => ({
        changed: true as const,
        status: {
          ...f.recipientCases[0].record,
          thread: ref,
          recipients: [...(await recipients)!],
          ref: { writer: ref.writer, id: crypto.randomUUID() },
          depth: 1,
        },
      }),
      notificationFailed: async () => {
        failures++;
      },
    },
    notify: async (_status, _recipient, captured) => {
      attempts.push(captured.ref.id);
      throw new Error('unknown ledger outcome');
    },
  };
  const { results } = await new ThreadStatusCoordinator(options).resolveAll([first, second]);
  expect(results).toHaveLength(2);
  expect(attempts).toEqual([first.ref.id]);
  expect(failures).toBe(0);
});

it('Reopen supersedes the remaining notifications of a committed Resolve-all thread', async () => {
  const captured = batchThread('@alpha @beta');
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let finish!: () => void;
  const drained = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const attempts: string[] = [];
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: async () => [agent('alpha', '1'), agent('beta', '2')],
    binding: {
      setStatus: async (ref, previous, resolved, recipients) => ({
        changed: true,
        status: {
          ...f.recipientCases[0].record,
          thread: ref,
          previous,
          resolved,
          recipients: [...(await recipients)!],
          ref: { writer: ref.writer, id: crypto.randomUUID() },
          depth: 1,
        },
      }),
      notificationFailed: async () => {
        finish();
      },
    },
    notify: async (_status, recipient) => {
      attempts.push(recipient.agentName);
      await held;
      return { adopted: false, reason: 'RECIPIENT_UNAVAILABLE' };
    },
  });
  const { results } = await coordinator.resolveAll([captured]);
  const result = results[0];
  if (!('result' in result) || !result.result.changed) throw new Error('Expected committed status');
  await coordinator.change({ ...captured, resolved: true, status: result.result.status }, false);
  release();
  await drained;
  expect(attempts).toEqual(['alpha']);
});

it('Resolve all skips an already-pending explicit action and pins later status revisions at confirmation', async () => {
  const first = batchThread('First'),
    second = batchThread('Second');
  const previous = { writer: second.ref.writer, id: crypto.randomUUID() };
  second.status = {
    ...f.recipientCases[0].record,
    ref: { ...previous },
    thread: second.ref,
    depth: 1,
    recipients: [],
  };
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const writes: string[] = [];
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: async () => [],
    binding: {
      setStatus: async (ref, prior, _resolved, recipients) => {
        writes.push(ref.id);
        if (ref.id === first.ref.id) await held;
        else expect(prior).toEqual(previous);
        return {
          changed: true,
          status: {
            ...f.recipientCases[0].record,
            thread: ref,
            recipients: [...(await recipients)!],
            ref: { writer: ref.writer, id: crypto.randomUUID() },
            depth: 1,
          },
        };
      },
      notificationFailed: async () => {},
    },
    notify: async () => {
      throw new Error('No mentions to notify');
    },
  });
  const single = coordinator.change(first, true);
  const batch = coordinator.resolveAll([first, second]);
  second.status!.ref.id = crypto.randomUUID();
  const { results } = await batch;
  expect(results[0]).toMatchObject({ thread: first.ref, result: { changed: false } });
  expect(results[1]).toMatchObject({ thread: second.ref, result: { changed: true } });
  expect(writes).toEqual([first.ref.id, second.ref.id]);
  release();
  await single;
});

it('Resolve all freezes a synchronous directory failure once and still admits status-only writes', async () => {
  let lists = 0,
    writes = 0;
  const coordinator = new ThreadStatusCoordinator({
    asks: () => [],
    destinations: () => {
      lists++;
      throw new Error('directory unavailable');
    },
    binding: {
      setStatus: async (ref, _previous, _resolved, recipients) => {
        writes++;
        return {
          changed: true,
          status: {
            ...f.recipientCases[0].record,
            thread: ref,
            recipients: [...(await recipients)!],
            ref: { writer: ref.writer, id: crypto.randomUUID() },
            depth: 1,
          },
        };
      },
      notificationFailed: async () => {},
    },
    notify: async () => ({ adopted: true }),
  });
  const { results } = await coordinator.resolveAll([
    batchThread('@missing'),
    batchThread('@missing'),
  ]);
  expect(lists).toBe(1);
  expect(writes).toBe(2);
  for (const result of results)
    expect(result).toMatchObject({
      result: { changed: true, warnings: [{ name: 'missing', reason: 'unknown' }] },
    });
});
