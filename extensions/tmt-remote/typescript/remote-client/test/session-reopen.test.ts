import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';
import {
  boundedAdmission,
  ReopenSessionError,
  type AdmissionRuntime,
} from '../src/session-reopen.js';
function clock(jitter = 0): AdmissionRuntime {
  return { now: () => Date.now(), later: setTimeout, clear: clearTimeout, jitter: () => jitter };
}
for (const reason of ['unconfirmed', 'unreachable', 'transient'] as const) {
  test(`four attempts exhaust with typed ${reason}, never terminal authority`, async () => {
    vi.useFakeTimers();
    try {
      const began = Date.now();
      const calls: number[] = [];
      const pending = boundedAdmission(
        async () => {
          calls.push(Date.now() - began);
          throw new ReopenSessionError(reason, 'admission-unconfirmed');
        },
        undefined,
        clock(),
      );
      const checked = assert.rejects(
        pending,
        (error) =>
          error instanceof ReopenSessionError &&
          error.reason === reason &&
          error.detail === 'budget-exhausted',
      );
      await vi.runAllTimersAsync();
      await checked;
      assert.deepEqual(calls, [0, 250, 750, 1750]);
      assert.equal(vi.getTimerCount(), 0);
    } finally {
      vi.useRealTimers();
    }
  });
}
test('hung opens abort at 4 s and total budget stays below 20 s with maximum jitter', async () => {
  vi.useFakeTimers();
  try {
    const began = Date.now();
    const calls: number[] = [];
    const signals: AbortSignal[] = [];
    const pending = boundedAdmission(
      (signal) => {
        signals.push(signal);
        calls.push(Date.now() - began);
        return new Promise<never>(() => {});
      },
      undefined,
      clock(1),
    );
    const checked = assert.rejects(
      pending,
      (error) => error instanceof ReopenSessionError && error.reason === 'unreachable',
    );
    await vi.runAllTimersAsync();
    await checked;
    assert.deepEqual(calls, [0, 4312, 8937, 14187]);
    assert.ok(Date.now() - began <= 20_000);
    assert.ok(signals.every((signal) => signal.aborted));
    assert.equal(vi.getTimerCount(), 0);
  } finally {
    vi.useRealTimers();
  }
});
test('opaque authority evidence is not erased by later network failures', async () => {
  vi.useFakeTimers();
  try {
    let calls = 0;
    const pending = boundedAdmission(
      async () => {
        throw new ReopenSessionError(
          calls++ === 0 ? 'unconfirmed' : 'unreachable',
          'admission-unconfirmed',
        );
      },
      undefined,
      clock(),
    );
    const checked = assert.rejects(
      pending,
      (error) => error instanceof ReopenSessionError && error.reason === 'unconfirmed',
    );
    await vi.runAllTimersAsync();
    await checked;
  } finally {
    vi.useRealTimers();
  }
});
test('owner cancel clears timers and ignores a late successful result', async () => {
  vi.useFakeTimers();
  try {
    const owner = new AbortController();
    let finish!: (value: string) => void;
    let child!: AbortSignal;
    const pending = boundedAdmission(
      (signal) => {
        child = signal;
        return new Promise<string>((resolve) => {
          finish = resolve;
        });
      },
      owner.signal,
      clock(),
    );
    const checked = assert.rejects(
      pending,
      (error) => error instanceof ReopenSessionError && error.detail === 'cancelled',
    );
    await vi.advanceTimersByTimeAsync(0);
    owner.abort();
    finish('late');
    await checked;
    assert.equal(child.aborted, true);
    assert.equal(vi.getTimerCount(), 0);
  } finally {
    vi.useRealTimers();
  }
});

test('cancel during backoff clears both wait and deadline timers and makes no second attempt', async () => {
  vi.useFakeTimers();
  try {
    const owner = new AbortController();
    let calls = 0;
    const pending = boundedAdmission(
      async () => {
        calls++;
        throw new ReopenSessionError('unconfirmed', 'admission-unconfirmed');
      },
      owner.signal,
      clock(),
    );
    const checked = assert.rejects(
      pending,
      (error) => error instanceof ReopenSessionError && error.detail === 'cancelled',
    );
    await vi.advanceTimersByTimeAsync(0);
    assert.equal(calls, 1);
    assert.equal(vi.getTimerCount(), 2);
    owner.abort();
    await checked;
    assert.equal(vi.getTimerCount(), 0);
    assert.equal(calls, 1);
  } finally {
    vi.useRealTimers();
  }
});
test('an elapsed absolute budget preserves unconfirmed even when it expires during backoff', async () => {
  vi.useFakeTimers();
  try {
    const began = Date.now();
    let offset = 0;
    let calls = 0;
    const time = clock();
    time.now = () => Date.now() + offset;
    const pending = boundedAdmission(
      async () => {
        calls++;
        offset = 19_900;
        throw new ReopenSessionError('unconfirmed', 'admission-unconfirmed');
      },
      undefined,
      time,
    );
    const checked = assert.rejects(
      pending,
      (error) => error instanceof ReopenSessionError && error.reason === 'unconfirmed',
    );
    await vi.runAllTimersAsync();
    await checked;
    assert.equal(calls, 1);
    assert.equal(Date.now() + offset - began, 20_000);
    assert.equal(vi.getTimerCount(), 0);
  } finally {
    vi.useRealTimers();
  }
});
