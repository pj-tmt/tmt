import { expect, it } from 'vite-plus/test';
import type {
  SealedSyncPort,
  SyncCall,
  SyncChange,
  SyncSubscription,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { HostedReshareRequired } from '../src/hosted-authority.js';
import { HostedSession } from '../src/hosted-session.js';
import type { PageView } from '../src/transport.js';

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
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
function fixture(
  options: {
    refuse?: unknown;
    refresh?: (call: SyncCall) => Promise<void>;
    drain?: () => Promise<void>;
    wait?: (ms: number, signal: AbortSignal) => Promise<void>;
  } = {},
) {
  const events: string[] = [],
    views: PageView[] = [],
    failures: unknown[] = [],
    calls: SyncCall[] = [];
  const terminal = deferred<void>(),
    recovered = deferred<void>();
  let active = 0,
    peak = 0;
  const watchers: { fail(error: unknown): void; changed(): void }[] = [];
  let opened = 0;
  const session = new HostedSession(
    {
      wait: options.wait ?? (async () => {}),
      async open(call) {
        calls.push(call);
        const name = String(++opened);
        events.push(`open:${name}`);
        if (options.refuse) throw options.refuse;
        const port = {
          async subscribe() {
            let pending: ReturnType<typeof deferred<SyncChange>> | undefined;
            let closed = false;
            active++;
            peak = Math.max(peak, active);
            const watcher = {
              fail: (error: unknown) => pending?.reject(error),
              changed: () => pending?.resolve({ kind: 'changed' }),
            };
            watchers.push(watcher);
            return {
              next(call: SyncCall) {
                pending = deferred<SyncChange>();
                const selected = pending;
                const abort = () =>
                  selected.reject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
                call.signal.addEventListener('abort', abort, { once: true });
                if (call.signal.aborted) abort();
                return selected.promise.finally(() =>
                  call.signal.removeEventListener('abort', abort),
                );
              },
              close() {
                if (closed) return;
                closed = true;
                active--;
                pending?.reject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
              },
            } satisfies SyncSubscription;
          },
          close() {
            events.push(`port-close:${name}`);
            expect(active).toBe(0);
          },
        } as unknown as SealedSyncPort;
        return {
          port,
          interests: ['log', 'checkpoints', 'authority'].map((resource) => ({
            resource,
            scope: {},
          })),
          reader: {
            async refresh(call: SyncCall) {
              events.push(`read:${name}`);
              await options.refresh?.(call);
              return { source: name, title: 'Verified' };
            },
            close() {
              events.push(`reader-close:${name}`);
            },
          },
        };
      },
      async drain() {
        events.push('drain');
        await options.drain?.();
      },
      publish(view) {
        views.push(view);
        if (view.source !== '1') recovered.resolve();
      },
      failed(error) {
        failures.push(error);
        terminal.resolve();
      },
    },
    call(),
  );
  return {
    session,
    events,
    views,
    failures,
    calls,
    watchers,
    terminal: terminal.promise,
    recovered: recovered.promise,
    active: () => active,
    peak: () => peak,
  };
}

it('subscribes once and joins all interests and charges before releasing its reader and port', async () => {
  const f = fixture();
  await f.session.ready;
  expect(f.active()).toBe(3);
  expect(f.views).toEqual([{ source: '1', title: 'Verified' }]);
  await f.session.close();
  expect(f.active()).toBe(0);
  expect(f.failures).toEqual([]);
  expect(f.events.slice(-3)).toEqual(['drain', 'reader-close:1', 'port-close:1']);
});

it('a terminal catch-up hint retires and joins the old group before a fresh verified binding', async () => {
  const f = fixture();
  try {
    await f.session.ready;
    f.watchers[0].fail({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'catch-up-required' });
    await f.recovered;
    expect(f.peak()).toBe(3);
    expect(f.events.indexOf('port-close:1')).toBeLessThan(f.events.indexOf('open:2'));
    expect(f.events.indexOf('drain')).toBeLessThan(f.events.indexOf('open:2'));
    expect(f.views.at(-1)?.source).toBe('2');
  } finally {
    await f.session.close();
  }
});

it.each(['REMOTE_SCOPE_DENIED', 'REMOTE_BUDGET_EXHAUSTED', 'REMOTE_RATE_LIMITED'])(
  'never reopens after %s',
  async (code) => {
    const f = fixture();
    await f.session.ready;
    const refusal = { code, resetAtMs: 9000, retryAfterMs: 10 };
    f.watchers[0].fail(refusal);
    await f.terminal;
    await f.session.close();
    expect(f.failures).toEqual([refusal]);
    expect(f.calls).toHaveLength(1);
    expect(f.active()).toBe(0);
  },
);

it('allows at most three finite initial attempts under one unchanged deadline', async () => {
  const refusal = { code: 'REMOTE_STATE_UNAVAILABLE' },
    f = fixture({ refuse: refusal });
  await expect(f.session.ready).rejects.toEqual(refusal);
  await f.terminal;
  await f.session.close();
  expect(f.calls).toHaveLength(3);
  expect(new Set(f.calls.map((call) => call.deadlineAtMs)).size).toBe(1);
  expect(f.failures).toEqual([refusal]);
});

it('does not reinterpret the typed re-share refusal as a recoverable transport failure', async () => {
  const refusal = new HostedReshareRequired(),
    f = fixture({ refuse: refusal });
  await expect(f.session.ready).rejects.toBe(refusal);
  await f.session.close();
  expect(f.calls).toHaveLength(1);
  expect(f.failures).toEqual([refusal]);
});

it('close fences an in-flight refresh and waits for both it and durable charging before finishing', async () => {
  const reading = deferred<void>(),
    releaseRead = deferred<void>(),
    draining = deferred<void>(),
    releaseDrain = deferred<void>();
  let reads = 0;
  const f = fixture({
    async refresh(call) {
      if (++reads > 1) {
        reading.resolve();
        await releaseRead.promise;
        expect(call.signal.aborted).toBe(true);
      }
    },
    async drain() {
      draining.resolve();
      await releaseDrain.promise;
    },
  });
  await f.session.ready;
  f.watchers[0].changed();
  await reading.promise;
  const close = f.session.close();
  expect(f.active()).toBe(0);
  expect(f.events).not.toContain('reader-close:1');
  releaseRead.resolve();
  await draining.promise;
  expect(f.events).not.toContain('reader-close:1');
  releaseDrain.resolve();
  await close;
  expect(f.views).toHaveLength(1);
  expect(f.failures).toEqual([]);
  expect(f.calls).toHaveLength(1);
});

it('recovery backoff is cancellation-aware and shares the finite overall attempt bound', async () => {
  const waiting = deferred<void>();
  const f = fixture({
    refuse: { code: 'REMOTE_STATE_UNAVAILABLE' },
    wait: async (_ms, signal) => {
      waiting.resolve();
      await new Promise<void>((resolve) =>
        signal.addEventListener('abort', () => resolve(), { once: true }),
      );
    },
  });
  const ready = f.session.ready.catch((error) => error);
  try {
    await Promise.race([
      waiting.promise,
      f.terminal.then(() => {
        throw new Error('Recovery skipped its backoff');
      }),
    ]);
    expect(f.calls).toHaveLength(1);
  } finally {
    await f.session.close();
  }
  expect(await ready).toMatchObject({ state: 'cancelled' });
  expect(f.calls).toHaveLength(1);
  expect(f.failures).toEqual([]);
});

it('a terminal budget refusal observed before cancellation stays terminal through joined teardown', async () => {
  const entered = deferred<void>(),
    release = deferred<void>(),
    refusal = { code: 'REMOTE_BUDGET_EXHAUSTED', resetAtMs: 9999, retryAfterMs: 1000 };
  const f = fixture({
    drain: async () => {
      entered.resolve();
      await release.promise;
    },
  });
  await f.session.ready;
  f.watchers[0].fail(refusal);
  await entered.promise;
  const close = f.session.close();
  release.resolve();
  await close;
  expect(f.failures).toEqual([refusal]);
  expect(f.calls).toHaveLength(1);
});
