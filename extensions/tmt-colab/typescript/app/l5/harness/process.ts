import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import fs from 'node:fs';
import { createInterface } from 'node:readline';

export function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    // EPERM means the process exists but is not ours; only ESRCH is absence.
    return (error as NodeJS.ErrnoException).code !== 'ESRCH';
  }
}

export async function until(
  predicate: () => boolean | Promise<boolean>,
  description: string,
  timeoutMs = 10_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!(await predicate())) {
    if (Date.now() >= deadline) throw new Error(`Timed out: ${description}`);
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}

export interface Exit {
  code: number | null;
  signal: NodeJS.Signals | null;
}

/**
 * One owned child in its own process group. Stdout is read as JSON lines;
 * stderr goes to a file in the world root so a passing run never prints it.
 * stop() proves the whole group is gone, not only the direct child.
 */
export class OwnedProcess {
  readonly child: ChildProcessWithoutNullStreams;
  readonly pid: number;
  readonly lines: string[] = [];
  readonly exited: Promise<Exit>;
  private waiters = new Set<() => void>();

  constructor(
    readonly label: string,
    executable: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; stderrPath: string },
  ) {
    this.child = spawn(executable, args, {
      cwd: options.cwd,
      env: options.env,
      detached: true,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    this.pid = this.child.pid ?? 0;
    this.child.stdin.on('error', () => {});
    createInterface({ input: this.child.stdout }).on('line', (line) => {
      this.lines.push(line);
      for (const wake of this.waiters) wake();
    });
    this.child.stderr.on('data', (chunk: Buffer) => fs.appendFileSync(options.stderrPath, chunk));
    this.exited = new Promise((resolve) => {
      this.child.once('error', () => resolve({ code: 1, signal: null }));
      this.child.once('close', (code, signal) => {
        for (const wake of this.waiters) wake();
        resolve({ code, signal });
      });
    });
  }

  /** The first stdout JSON line matching `predicate`, within a bound. */
  event(
    predicate: (value: Record<string, unknown>) => boolean,
    timeoutMs = 20_000,
  ): Promise<Record<string, unknown>> {
    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout>;
      const check = () => {
        for (const line of this.lines) {
          let value: Record<string, unknown>;
          try {
            value = JSON.parse(line) as Record<string, unknown>;
          } catch {
            continue;
          }
          if (predicate(value)) return done(() => resolve(value));
        }
        if (this.child.exitCode !== null || this.child.signalCode !== null)
          done(() => reject(new Error(`${this.label} exited before the expected event`)));
      };
      const done = (finish: () => void) => {
        clearTimeout(timer);
        this.waiters.delete(check);
        finish();
      };
      timer = setTimeout(
        () => done(() => reject(new Error(`Timed out waiting for ${this.label} event`))),
        timeoutMs,
      );
      this.waiters.add(check);
      check();
    });
  }

  signal(signal: NodeJS.Signals): void {
    if (!this.pid) return;
    try {
      process.kill(-this.pid, signal);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error;
    }
  }

  /** Graceful stop first; a survivor is killed and reported as a failure. */
  async stop(): Promise<void> {
    this.signal('SIGTERM');
    const timeout = new Promise<'timeout'>((resolve) =>
      setTimeout(() => resolve('timeout'), 5_000),
    );
    if ((await Promise.race([this.exited, timeout])) === 'timeout') {
      this.signal('SIGKILL');
      await this.exited;
      throw new Error(`${this.label} ignored SIGTERM`);
    }
    await this.confirmGone();
  }

  /** SIGKILL for crash/restart barriers; the group must still disappear. */
  async kill(): Promise<void> {
    this.signal('SIGKILL');
    await this.exited;
    await this.confirmGone();
  }

  private async confirmGone(): Promise<void> {
    try {
      await until(() => !alive(-this.pid), `${this.label} process group absence`, 3_000);
    } catch (error) {
      this.signal('SIGKILL');
      throw error;
    }
  }
}
