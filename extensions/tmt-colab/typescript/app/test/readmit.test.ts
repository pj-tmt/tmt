import { expect, it, vi } from 'vite-plus/test';
import {
  PageReopener,
  ReadmitBudget,
  readmittable,
  reopenWithBackoff,
  retryableReopen,
} from '../src/readmit.js';
import { FAILURE_CAUSES } from '../src/terminal-failure.js';

it.each(['STALE_EPOCH', 'DENIED', 'EXPIRED'])('%s on an open page is re-admittable', (code) => {
  expect(readmittable(new Error(code))).toBe(true);
});

it.each(['Page unavailable', 'CAPACITY', 'REMOTE_DEVICE_REVOKED', 'Access ended', 'GAP'])(
  '%s is not re-admitted automatically',
  (code) => {
    expect(readmittable(new Error(code))).toBe(false);
  },
);

it('allows three reopens per minute, then stops until the window passes', () => {
  const budget = new ReadmitBudget();
  expect([0, 1000, 2000].map((t) => budget.take(t))).toEqual([true, true, true]);
  expect(budget.take(3000)).toBe(false);
  expect(budget.take(59_999)).toBe(false);
  expect(budget.take(60_001)).toBe(true);
});

const retryable = (error: Error) => retryableReopen(error, FAILURE_CAUSES);

it('retries a lagging epoch with growing pauses until the reopen succeeds', async () => {
  const waits: number[] = [];
  let calls = 0;
  const result = await reopenWithBackoff(
    async () => {
      calls += 1;
      if (calls < 3) throw new Error('STALE_EPOCH');
      return 'page';
    },
    {
      signal: new AbortController().signal,
      retryable,
      wait: async (ms) => void waits.push(ms),
    },
  );
  expect(result).toEqual({ value: 'page' });
  expect(waits).toEqual([1000, 3000]);
});

it('stops at once when the access ended, and after the last delay otherwise', async () => {
  const waits: number[] = [];
  const wait = async (ms: number) => void waits.push(ms);
  const signal = new AbortController().signal;
  expect(
    await reopenWithBackoff(
      async () => {
        throw new Error('REMOTE_DEVICE_REVOKED');
      },
      { signal, retryable, wait },
    ),
  ).toEqual({ error: new Error('REMOTE_DEVICE_REVOKED') });
  expect(waits).toEqual([]);
  expect(
    await reopenWithBackoff(
      async () => {
        throw new Error('DENIED');
      },
      { signal, retryable, wait },
    ),
  ).toEqual({ error: new Error('DENIED') });
  expect(waits).toEqual([1000, 3000, 8000, 15_000]);
});

it('does not open again once aborted', async () => {
  const controller = new AbortController();
  let calls = 0;
  await reopenWithBackoff(
    async () => {
      calls += 1;
      throw new Error('STALE_EPOCH');
    },
    {
      signal: controller.signal,
      retryable,
      wait: async () => controller.abort(),
    },
  );
  expect(calls).toBe(1);
});

const noWait = async () => {};

it('a reset closes the reopened binding and a late result never lands', async () => {
  const reopener = new PageReopener<string>();
  const signals: AbortSignal[] = [];
  const done: unknown[] = [];
  let finish: (value: string) => void = () => {};
  reopener.start(
    (signal) => {
      signals.push(signal);
      return new Promise((resolve) => (finish = resolve));
    },
    { retryable, wait: noWait, done: (result) => done.push(result) },
  );
  expect(reopener.busy).toBe(true);
  // The route loads a newer binding while the reopen is still in flight.
  reopener.reset();
  expect(signals[0]?.aborted).toBe(true);
  finish('late');
  await Promise.resolve();
  await Promise.resolve();
  expect(done).toEqual([]);
  expect(reopener.busy).toBe(false);
});

it('a reset closes a binding that an earlier reopen produced and frees the next reopen', async () => {
  const reopener = new PageReopener<string>();
  const signals: AbortSignal[] = [];
  const done: unknown[] = [];
  const run = () =>
    reopener.start(
      async (signal) => {
        signals.push(signal);
        return 'binding';
      },
      { retryable, wait: noWait, done: (result) => done.push(result) },
    );
  run();
  await vi.waitFor(() => expect(done).toHaveLength(1));
  expect(signals[0]?.aborted).toBe(false);
  reopener.reset();
  expect(signals[0]?.aborted).toBe(true);
  run();
  await vi.waitFor(() => expect(done).toHaveLength(2));
  expect(signals[1]?.aborted).toBe(false);
});

it('a second successful reopen closes the binding of the first', async () => {
  const reopener = new PageReopener<string>();
  const signals: AbortSignal[] = [];
  let count = 0;
  const run = () =>
    reopener.start(
      async (signal) => {
        signals.push(signal);
        return 'binding';
      },
      { retryable, wait: noWait, done: () => (count += 1) },
    );
  run();
  await vi.waitFor(() => expect(count).toBe(1));
  run();
  await vi.waitFor(() => expect(count).toBe(2));
  expect(signals.map((signal) => signal.aborted)).toEqual([true, false]);
});
