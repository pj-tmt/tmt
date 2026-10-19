import { execFileSync, type ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import type { MockEvent } from './types.js';

export async function killAndWait(
  pids: number[],
  label: string,
  onError: (error: Error) => void
): Promise<number[]> {
  for (const pid of pids) {
    try {
      process.kill(-pid, 'SIGKILL');
    } catch (error) {
      if (!(error instanceof Error) || !('code' in error) || error.code !== 'ESRCH') {
        onError(
          new Error(`Could not kill E2E ${label} process group ${pid}.`, {
            cause: error,
          })
        );
      }
    }
  }
  const groupsRunning = (reportErrors = false): number[] =>
    pids.filter((pid) => {
      const { state, cause } = inspectProcess(pid, { group: true });
      // Inspection can be temporarily unknown while a killed group is reaped.
      // Keep it pending within the cleanup bound; never treat it as gone.
      if (state === 'unknown' && reportErrors) {
        onError(new Error(`Could not inspect E2E ${label} process group ${pid}.`, { cause }));
      }
      return state !== 'absent';
    });
  const deadline = Date.now() + 1_000;
  while (Date.now() < deadline && groupsRunning().length > 0) {
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  return groupsRunning(true);
}

/** What one `kill(pid, 0)` probe can prove about a process. */
export type ProcessState = 'running' | 'absent' | 'unknown';

type SignalProcess = (target: number, signal: 0) => unknown;

/** A probe's state, with the error that left it `unknown` so diagnostics keep their cause. */
interface ProcessInspection {
  state: ProcessState;
  cause?: unknown;
}

/**
 * Probe a process, or with `group` its process group. Only ESRCH proves absence:
 * EPERM, EIO, any other error and a pid that cannot name a process leave the state
 * unknown, which callers must never read as exited.
 */
function inspectProcess(
  pid: number,
  options: { group?: boolean; kill?: SignalProcess } = {}
): ProcessInspection {
  if (!Number.isSafeInteger(pid) || pid <= 0) return { state: 'unknown' };
  const kill = options.kill ?? ((target, signal) => process.kill(target, signal));
  try {
    kill(options.group ? -pid : pid, 0);
    return { state: 'running' };
  } catch (error) {
    return error instanceof Error && 'code' in error && error.code === 'ESRCH'
      ? { state: 'absent' }
      : { state: 'unknown', cause: error };
  }
}

export function processState(
  pid: number,
  options: { group?: boolean; kill?: SignalProcess } = {}
): ProcessState {
  return inspectProcess(pid, options).state;
}

export function requestObserverPids(observers: string): number[] {
  const owned: number[] = [];
  for (const name of fs.readdirSync(observers)) {
    if (!/^req_[0-9a-f-]+\.log$/.test(name)) continue;
    const match = fs
      .readFileSync(path.join(observers, name), 'utf8')
      .match(/^observer_pid=(\d+)$/m);
    if (!match) continue;
    const pid = Number(match[1]);
    const request = name.slice(0, -4);
    try {
      const args = execFileSync('ps', ['-p', String(pid), '-o', 'args='], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
      });
      if (args.trim().endsWith(`__request-observer ${request}`)) owned.push(pid);
    } catch {
      // Already exited. Never signal a recycled PID with different argv.
    }
  }
  return owned;
}

/** Stop only fixture-owned reply workers with matching recorded argv. */
export function replyWorkerPids(directory: string): number[] {
  const owned: number[] = [];
  for (const name of fs.readdirSync(directory)) {
    if (!/^[0-9a-f-]{36}\.log$/.test(name)) continue;
    let log: string;
    try {
      log = fs.readFileSync(path.join(directory, name), 'utf8');
    } catch (error) {
      // Clean workers remove their own logs; a concurrent exit is harmless.
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') continue;
      throw error;
    }
    const pid = Number(log.match(/^reply_worker_pid=(\d+)$/m)?.[1]);
    const batch = log.match(/^reply_batch_id=([0-9a-f-]{36})$/m)?.[1];
    if (!Number.isSafeInteger(pid) || pid <= 1 || !batch) continue;
    try {
      const args = execFileSync('ps', ['-p', String(pid), '-o', 'args='], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
      });
      if (args.trim().endsWith(`__reply-notice-worker ${batch} ${name.slice(0, -4)}`))
        owned.push(pid);
    } catch (error) {
      if (!(error instanceof Error && 'status' in error && error.status === 1)) throw error;
      // Already exited; never signal a recycled PID.
    }
  }
  return owned;
}

export async function stopAttachedClients(
  attachedClients: readonly ChildProcess[],
  onError: (error: Error) => void
): Promise<void> {
  await Promise.all(
    attachedClients.map(async (client) => {
      if (client.exitCode === null && client.signalCode === null) {
        try {
          client.kill('SIGKILL');
        } catch (error) {
          onError(
            new Error('Could not stop an attached E2E tmux client.', {
              cause: error,
            })
          );
        }
      }
      if (client.exitCode !== null || client.signalCode !== null) return;
      const closed = await new Promise<boolean>((resolve) => {
        const timer = setTimeout(() => {
          client.removeListener('close', onClose);
          resolve(false);
        }, 1_000);
        const onClose = (): void => {
          clearTimeout(timer);
          resolve(true);
        };
        client.once('close', onClose);
      });
      if (!closed) {
        onError(new Error(`Attached E2E tmux client ${client.pid ?? 'unknown'} survived cleanup.`));
      }
    })
  );
}

export function serverIsRunning(
  socketPath: string,
  tmuxPath: string,
  isServerProcessRunning: () => boolean
): boolean {
  if (!socketPath) return false;
  try {
    execFileSync(tmuxPath, ['-S', socketPath, 'list-sessions'], {
      stdio: 'ignore',
    });
    return true;
  } catch {
    return isServerProcessRunning();
  }
}

export async function waitForCliResults(results: Iterable<Promise<unknown>>): Promise<void> {
  await Promise.race([
    Promise.allSettled(results),
    new Promise<void>((resolve) => setTimeout(resolve, 1_000)),
  ]);
}

export function activeReplyPids(events: readonly MockEvent[]): number[] {
  const activeChildren = new Set<number>();
  for (const event of events) {
    if (event.childPid === undefined) continue;
    if (event.event === 'child-start') activeChildren.add(event.childPid);
    if (event.event === 'child-close') activeChildren.delete(event.childPid);
  }
  return [...activeChildren];
}
