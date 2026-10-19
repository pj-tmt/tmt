import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { describe, expect, it } from 'vite-plus/test';
import { processState, type ProcessState } from '../e2e/harness/cleanup.js';
import { waitForProcessExit } from '../e2e/harness/readiness.js';

type Kill = (target: number, signal: 0) => unknown;

function failing(code: string): Kill {
  return () => {
    throw Object.assign(new Error(code), { code });
  };
}

const probeCases: [name: string, kill: Kill, expected: ProcessState][] = [
  ['a signalled process', () => undefined, 'running'],
  ['ESRCH', failing('ESRCH'), 'absent'],
  ['EPERM', failing('EPERM'), 'unknown'],
  ['EIO', failing('EIO'), 'unknown'],
  [
    'an error without a code',
    () => {
      throw new Error('no code');
    },
    'unknown',
  ],
  [
    'a thrown non-error',
    () => {
      throw 'ESRCH';
    },
    'unknown',
  ],
];

describe('E2E process state probe', () => {
  it.each(probeCases)('classifies %s as %s', (_name, kill, expected) => {
    expect(processState(42, { kill })).toBe(expected);
  });

  it('probes the process group through the negated pid', () => {
    const targets: number[] = [];
    const kill = (target: number) => {
      targets.push(target);
    };
    processState(42, { kill });
    processState(42, { kill, group: true });
    expect(targets).toEqual([42, -42]);
  });

  it.each([0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY])(
    'leaves pid %s unknown without signalling anything',
    (pid) => {
      const kill = (): never => {
        throw new Error('an invalid pid must not be signalled');
      };
      expect(processState(pid, { kill })).toBe('unknown');
      expect(processState(pid, { kill, group: true })).toBe('unknown');
    }
  );

  it('sees a child this test spawned run and then be proven absent after it is reaped', async () => {
    const child = spawn(process.execPath, ['-e', 'setTimeout(() => {}, 30_000)'], {
      stdio: 'ignore',
    });
    try {
      expect(child.pid).toBeDefined();
      expect(processState(child.pid!)).toBe('running');
      child.kill('SIGKILL');
      await once(child, 'close');
      expect(processState(child.pid!)).toBe('absent');
    } finally {
      child.kill('SIGKILL');
    }
  });
});

describe('E2E wait for process exit', () => {
  const sequence = (states: Record<number, ProcessState[]>) => {
    const polls = new Map<number, number>();
    return (pid: number): ProcessState => {
      const seen = polls.get(pid) ?? 0;
      polls.set(pid, seen + 1);
      const list = states[pid]!;
      return list[Math.min(seen, list.length - 1)]!;
    };
  };

  it('reports no processes as exited without probing', async () => {
    const probe = (): never => {
      throw new Error('nothing to probe');
    };
    await expect(waitForProcessExit([], probe, 50)).resolves.toEqual({ outcome: 'exited' });
  });

  it('reports exited once every pid is proven absent, including one that exits while waiting', async () => {
    const probe = sequence({ 1: ['absent'], 2: ['running', 'running', 'absent'] });
    await expect(waitForProcessExit([1, 2], probe, 1_000)).resolves.toEqual({
      outcome: 'exited',
    });
  });

  it('reports a pid that is still running at the deadline as timed out', async () => {
    const probe = sequence({ 1: ['absent'], 2: ['running'], 3: ['unknown'] });
    await expect(waitForProcessExit([1, 2, 3], probe, 60)).resolves.toEqual({
      outcome: 'timed-out',
      running: [2],
      unknown: [3],
    });
  });

  it('reports a pid whose state stays unknown as unknown, never as exited', async () => {
    const probe = sequence({ 1: ['absent'], 2: ['unknown'] });
    await expect(waitForProcessExit([1, 2], probe, 60)).resolves.toEqual({
      outcome: 'unknown',
      unknown: [2],
    });
  });

  it('keeps waiting through a transient unknown state', async () => {
    const probe = sequence({ 1: ['unknown', 'unknown', 'absent'] });
    await expect(waitForProcessExit([1], probe, 1_000)).resolves.toEqual({ outcome: 'exited' });
  });
});
