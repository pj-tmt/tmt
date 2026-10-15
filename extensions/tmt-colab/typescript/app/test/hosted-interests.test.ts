import { expect, it, vi } from 'vite-plus/test';
import type {
  SealedSyncPort,
  SyncCall,
  SyncChange,
  SyncInterest,
  SyncSubscription,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { HostedInterests } from '../src/hosted-interests.js';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const call = (): SyncCall => ({
  signal: new AbortController().signal,
  deadlineAtMs: Date.now() + 30_000,
});
const interests = (page: string): SyncInterest[] =>
  ['log', 'checkpoints', 'authority'].map((resource) => ({
    resource,
    scope: { page: { stringValue: page } },
  }));
function fixture(drain: () => Promise<void> = async () => {}) {
  const events: string[] = [],
    failures: unknown[] = [];
  const terminal = deferred<void>();
  let active = 0,
    peak = 0;
  const listeners: {
    changed(): void;
    error(error: unknown): void;
    calls: number;
    stopped: boolean;
  }[] = [];
  const port = {
    async subscribe(interest: SyncInterest) {
      const page = (interest.scope.page as { stringValue: string }).stringValue;
      events.push(`subscribe:${page}`);
      active++;
      peak = Math.max(peak, active);
      let pending: ReturnType<typeof deferred<SyncChange>> | undefined;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const listener = {
        changed: () => pending?.resolve({ kind: 'changed' }),
        error: (error: unknown) => pending?.reject(error),
        calls: 0,
        stopped: false,
      };
      listeners.push(listener);
      const subscription: SyncSubscription = {
        async next(budget) {
          listener.calls++;
          pending = deferred<SyncChange>();
          const abort = () =>
            pending?.reject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
          budget.signal.addEventListener('abort', abort, { once: true });
          timer = setTimeout(
            () => pending?.reject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' }),
            Math.max(0, budget.deadlineAtMs - Date.now()),
          );
          try {
            return await pending.promise;
          } finally {
            clearTimeout(timer);
            budget.signal.removeEventListener('abort', abort);
            pending = undefined;
          }
        },
        close() {
          if (listener.stopped) return;
          listener.stopped = true;
          active--;
          events.push(`close:${page}`);
          pending?.reject({ code: 'REMOTE_STATE_UNAVAILABLE' });
        },
      };
      return subscription;
    },
  } as unknown as SealedSyncPort;
  const group = new HostedInterests(port, drain, (error) => {
    failures.push(error);
    terminal.resolve();
  });
  return {
    group,
    events,
    listeners,
    failures,
    terminal: terminal.promise,
    active: () => active,
    peak: () => peak,
  };
}

it('subscribes before initial discovery and never retains more than three page interests', async () => {
  const f = fixture();
  await f.group.replace(
    interests('first'),
    async () => {
      f.events.push('discover:first');
    },
    call(),
  );
  expect(f.events).toEqual([
    'subscribe:first',
    'subscribe:first',
    'subscribe:first',
    'discover:first',
  ]);
  await f.group.replace(
    interests('second'),
    async () => {
      f.events.push('discover:second');
    },
    call(),
  );
  expect(f.peak()).toBe(3);
  expect(f.events.indexOf('subscribe:second')).toBeGreaterThan(f.events.lastIndexOf('close:first'));
  await f.group.close();
  expect(f.active()).toBe(0);
});

it('refuses a fourth interest before disturbing the existing group', async () => {
  const f = fixture();
  await f.group.replace(interests('first'), async () => {}, call());
  await expect(
    f.group.replace([...interests('second'), interests('second')[0]], async () => {}, call()),
  ).rejects.toMatchObject({ code: 'REMOTE_RATE_LIMITED' });
  expect(f.active()).toBe(3);
  expect(f.events).toHaveLength(3);
  await f.group.close();
});

it('joins old in-flight discovery and delayed budget writes before opening the next page', async () => {
  const read = deferred<void>(),
    entered = deferred<void>(),
    write = deferred<void>(),
    draining = deferred<void>();
  const f = fixture(() => {
    draining.resolve();
    return write.promise;
  });
  const old = f.group.replace(
    interests('old'),
    async (budget) => {
      entered.resolve();
      await read.promise;
      expect(budget.signal.aborted).toBe(true);
    },
    call(),
  );
  await entered.promise;
  let switched = false;
  const next = f.group
    .replace(interests('new'), async () => {}, call())
    .then(() => {
      switched = true;
    });
  await Promise.resolve();
  expect(f.listeners.every((listener) => listener.stopped)).toBe(true);
  expect(f.events.some((event) => event === 'subscribe:new')).toBe(false);
  read.resolve();
  await draining.promise;
  expect(switched).toBe(false);
  expect(f.events.some((event) => event === 'subscribe:new')).toBe(false);
  write.resolve();
  await expect(old).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
  await next;
  expect(f.peak()).toBe(3);
  await f.group.close();
});

it('coalesces hints during discovery into serialized refreshes without losing the later wake', async () => {
  const entered = deferred<void>(),
    read = deferred<void>(),
    refreshed = deferred<void>();
  const f = fixture();
  let runs = 0;
  const start = f.group.replace(
    interests('page'),
    async () => {
      if (++runs === 1) {
        entered.resolve();
        await read.promise;
      } else refreshed.resolve();
    },
    call(),
  );
  await entered.promise;
  f.listeners.forEach((listener) => listener.changed());
  await Promise.resolve();
  read.resolve();
  await start;
  await refreshed.promise;
  expect(runs).toBe(2);
  await f.group.close();
});

it('an idle deadline awaits the same interests and still observes a later change', async () => {
  vi.useFakeTimers();
  const f = fixture();
  try {
    let reads = 0;
    const refreshed = deferred<void>();
    await f.group.replace(
      interests('page'),
      async () => {
        if (++reads === 2) refreshed.resolve();
      },
      call(),
    );
    await vi.advanceTimersByTimeAsync(30_000);
    expect(f.events).toHaveLength(3);
    expect(f.listeners.map((listener) => listener.calls)).toEqual([2, 2, 2]);
    f.listeners[0].changed();
    await refreshed.promise;
    expect(reads).toBe(2);
    await f.group.close();
  } finally {
    vi.useRealTimers();
  }
});

it.each([
  'REMOTE_SCOPE_DENIED',
  'REMOTE_BUDGET_EXHAUSTED',
  'REMOTE_RATE_LIMITED',
  'REMOTE_STATE_UNAVAILABLE',
])('a terminal %s closes the whole group without retrying any dead interest', async (code) => {
  const f = fixture();
  await f.group.replace(interests('page'), async () => {}, call());
  const error = { code, resetAtMs: 99, retryAfterMs: 1 };
  f.listeners[0].error(error);
  await f.terminal;
  await f.group.close();
  expect(f.failures).toEqual([error]);
  expect(f.active()).toBe(0);
  expect(f.events.filter((event) => event.startsWith('subscribe:'))).toHaveLength(3);
  f.listeners.forEach((listener) => listener.changed());
  expect(f.failures).toHaveLength(1);
});
