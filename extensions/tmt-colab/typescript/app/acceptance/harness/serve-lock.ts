import { execFileSync } from 'node:child_process';
import fs from 'node:fs';

/** Current holders of one world's exact private lock, never executable-name matches. */
export function serveLockOwners(lock: string): Set<number> {
  const found = new Set<number>();
  if (!fs.existsSync(lock)) return found;
  if (process.platform === 'linux') {
    for (const entry of fs.readdirSync('/proc')) {
      if (!/^\d+$/.test(entry)) continue;
      try {
        for (const fd of fs.readdirSync(`/proc/${entry}/fd`)) {
          try {
            if (fs.readlinkSync(`/proc/${entry}/fd/${fd}`) === lock) found.add(Number(entry));
          } catch {
            // A concurrently closed fd is not ownership evidence.
          }
        }
      } catch {
        // Exited or inaccessible processes cannot attest this lock.
      }
    }
  } else {
    let rows: string;
    try {
      rows = execFileSync('/usr/sbin/lsof', ['-Fpn', lock], { encoding: 'utf8' });
    } catch (error) {
      const failure = error as { status?: number; stderr?: string | Buffer };
      // The lock file survives a clean serve exit; lsof returns 1 for no holders.
      if (failure.status === 1 && !String(failure.stderr ?? '').trim()) return found;
      throw error;
    }
    let pid = 0;
    for (const row of rows.split('\n')) {
      if (row.startsWith('p')) pid = Number(row.slice(1));
      if (row === `n${lock}` && Number.isSafeInteger(pid) && pid > 0) found.add(pid);
    }
  }
  return found;
}

/** A recorded PID is historical evidence; only current lock ownership permits a signal. */
export function signalServeLockOwner(
  lock: string,
  pid: number,
  signal: NodeJS.Signals,
  probe = serveLockOwners,
): boolean {
  if (!probe(lock).has(pid)) return false;
  try {
    process.kill(pid, signal);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ESRCH') return false;
    throw error;
  }
}
