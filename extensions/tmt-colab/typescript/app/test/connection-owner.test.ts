import { describe, expect, it } from 'vite-plus/test';
import {
  backoffMs,
  ConnectionOwner,
  reconnectAttempt,
  type AttemptOutcome,
} from '../src/connection-owner.js';

const immediate = () => Promise.resolve();

describe('backoff', () => {
  it('grows 1, 2, 4, 8 s then holds at 15 s, spread by 20 percent', () => {
    expect([0, 1, 2, 3, 4, 9].map((index) => backoffMs(index, () => 0.5))).toEqual([
      1000, 2000, 4000, 8000, 15000, 15000,
    ]);
    expect(backoffMs(0, () => 0)).toBe(800);
    expect(backoffMs(0, () => 0.999)).toBe(1200);
  });
});

describe('connection owner', () => {
  const owner = (outcomes: AttemptOutcome[], waits: number[] = []) =>
    new ConnectionOwner({
      attempt: () => Promise.resolve(outcomes.shift() ?? 'ended'),
      wait: (ms) => {
        waits.push(ms);
        return immediate();
      },
      visible: immediate,
      jitter: () => 0.5,
    });

  it('retries with growing waits until connected, with no click', async () => {
    const waits: number[] = [];
    expect(await owner(['retry', 'retry', 'connected'], waits).reconnect()).toBe('connected');
    expect(waits).toEqual([1000, 2000, 4000]);
  });

  it('stops retrying when the attempt reports ended', async () => {
    const waits: number[] = [];
    expect(await owner(['retry', 'ended'], waits).reconnect()).toBe('ended');
    expect(waits).toEqual([1000, 2000]);
  });

  it('makes no attempt while hidden and resumes when visible', async () => {
    let shown!: () => void;
    const attempts: string[] = [];
    const subject = new ConnectionOwner({
      attempt: () => {
        attempts.push('try');
        return Promise.resolve('connected');
      },
      wait: immediate,
      visible: () => new Promise<void>((resolve) => (shown = resolve)),
    });
    const done = subject.reconnect();
    await immediate();
    expect(attempts).toEqual([]);
    shown();
    expect(await done).toBe('connected');
    expect(attempts).toEqual(['try']);
  });

  it('stop cancels the series: no further attempt and no late outcome', async () => {
    let attempts = 0;
    let release!: () => void;
    const subject = new ConnectionOwner({
      attempt: () => {
        attempts++;
        return new Promise<AttemptOutcome>((resolve) => (release = () => resolve('retry')));
      },
      wait: immediate,
      visible: immediate,
    });
    const done = subject.reconnect();
    await immediate();
    await immediate();
    subject.stop();
    release();
    expect(await done).toBe('stopped');
    expect(attempts).toBe(1);
    expect(subject.reconnecting).toBe(false);
  });

  it('ignores a second call while a series runs: one loop only', async () => {
    const subject = owner(['retry', 'connected']);
    const first = subject.reconnect();
    expect(await subject.reconnect()).toBe('stopped');
    expect(await first).toBe('connected');
  });
});

describe('reconnect attempt outcome', () => {
  const recoverable = (failure: Error | null) => failure?.name === 'Recoverable';
  const lost = Object.assign(new Error('lost'), { name: 'Recoverable' });
  const make = (reconnect: () => Promise<boolean>, state: { failure: Error | null }) => {
    const cleared: Array<Error | null> = [];
    const attempt = reconnectAttempt({
      reconnect,
      failure: () => state.failure,
      recovered: (interrupted) => cleared.push(interrupted),
      recoverable,
    });
    return { attempt, cleared };
  };

  it('connects and reports the interruption it cleared', async () => {
    const { attempt, cleared } = make(async () => true, { failure: lost });
    expect(await attempt()).toBe('connected');
    expect(cleared).toEqual([lost]);
  });

  it('retries after a failed reconnect that left a fresh recoverable failure', async () => {
    const state = { failure: lost as Error | null };
    const { attempt } = make(async () => {
      state.failure = Object.assign(new Error('again'), { name: 'Recoverable' });
      return false;
    }, state);
    expect(await attempt()).toBe('retry');
  });

  it('ends when a failed reconnect left no new failure or a terminal one', async () => {
    expect(await make(async () => false, { failure: lost }).attempt()).toBe('ended');
    const state = { failure: lost as Error | null };
    const { attempt } = make(async () => {
      state.failure = new Error('DENIED');
      return false;
    }, state);
    expect(await attempt()).toBe('ended');
  });

  it('keeps retrying when the reconnect rejects while the page is still recoverable', async () => {
    const rejected = make(async () => Promise.reject(new Error('boom')), { failure: lost });
    expect(await rejected.attempt()).toBe('retry');
    const terminal = make(async () => Promise.reject(new Error('boom')), {
      failure: new Error('DENIED'),
    });
    expect(await terminal.attempt()).toBe('ended');
  });
});
