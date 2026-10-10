import { expect, it } from 'vite-plus/test';
import { ReadmitBudget, readmittable, reopenWithBackoff, retryableReopen } from '../src/readmit.js';
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
