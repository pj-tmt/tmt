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
  const groupsRunning = (): number[] =>
    pids.filter((pid) => {
      try {
        return processGroupIsRunning(pid);
      } catch (error) {
        onError(
          new Error(`Could not inspect E2E ${label} process group ${pid}.`, {
            cause: error,
          })
        );
        return false;
      }
    });
  const deadline = Date.now() + 1_000;
  while (Date.now() < deadline && groupsRunning().length > 0) {
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  return groupsRunning();
}

export function processGroupIsRunning(pid: number): boolean {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(-pid, 0);
    return true;
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ESRCH') return false;
    throw error;
  }
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

export function processIsRunning(pid: number): boolean {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
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
