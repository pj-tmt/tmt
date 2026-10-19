import fs from 'node:fs';
import path from 'node:path';
import type { ProcessState } from './cleanup.js';
import type { MockEvent, CliProcess, CliResult } from './types.js';

/** Time for the CLI to reach a metadata barrier on a loaded host; only a hung CLI exceeds it. */
const METADATA_BARRIER_TIMEOUT_MS = 10_000;

export async function waitForEvent(
  readEvents: () => MockEvent[],
  predicate: (event: MockEvent) => boolean,
  timeoutMs: number
): Promise<MockEvent> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const event = readEvents().find(predicate);
    if (event) return event;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(
    `Timed out waiting for mock-agent event. Events: ${JSON.stringify(readEvents())}`
  );
}

export async function waitFor(
  predicate: () => boolean,
  timeoutMs: number,
  description: string
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(`Timed out waiting for ${description}.`);
}

/** How a bounded wait for processes to exit ended; only `exited` proves every one is gone. */
export type ProcessExit =
  | { outcome: 'exited' }
  | { outcome: 'timed-out'; running: number[]; unknown: number[] }
  | { outcome: 'unknown'; unknown: number[] };

/**
 * Wait up to `timeoutMs` for every pid to be proven absent. A pid whose state stays
 * unknown is reported as such, not as exited; one that is still running at the
 * deadline is a timeout the caller must report, not a reason to wait longer.
 */
export async function waitForProcessExit(
  pids: readonly number[],
  probe: (pid: number) => ProcessState,
  timeoutMs = 2_000
): Promise<ProcessExit> {
  const survey = (): { running: number[]; unknown: number[] } => {
    const states = pids.map((pid) => [pid, probe(pid)] as const);
    return {
      running: states.filter(([, state]) => state === 'running').map(([pid]) => pid),
      unknown: states.filter(([, state]) => state === 'unknown').map(([pid]) => pid),
    };
  };
  const deadline = Date.now() + timeoutMs;
  let { running, unknown } = survey();
  while (Date.now() < deadline && running.length + unknown.length > 0) {
    await new Promise((resolve) => setTimeout(resolve, 25));
    ({ running, unknown } = survey());
  }
  if (running.length > 0) return { outcome: 'timed-out', running, unknown };
  if (unknown.length > 0) return { outcome: 'unknown', unknown };
  return { outcome: 'exited' };
}

export async function waitForCapture(
  readCapture: () => string,
  predicate: (output: string) => boolean
): Promise<string> {
  const deadline = Date.now() + 2_000;
  while (Date.now() < deadline) {
    const output = readCapture();
    if (predicate(output)) return output;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`Timed out waiting for mock-agent pane output.\n${readCapture()}`);
}

/** Appended events are committed only by their newline; reread the tail on the next poll. */
export function readCompleteJsonLines<T = unknown>(logPath: string): T[] {
  if (!fs.existsSync(logPath)) return [];
  return fs
    .readFileSync(logPath, 'utf8')
    .split('\n')
    .slice(0, -1)
    .filter(Boolean)
    .map((line) => {
      try {
        return JSON.parse(line) as T;
      } catch (cause) {
        throw new SyntaxError(`Invalid complete JSON log line in ${logPath}: ${line}`, { cause });
      }
    });
}

export function readMockEvents(logPath: string): MockEvent[] {
  return readCompleteJsonLines<MockEvent>(logPath);
}

/**
 * Wait for the CLI to reach the barrier. This is a readiness wait: reaching it
 * takes as long as the CLI needs on a loaded host, so the bound is generous and
 * exists only to end a hung CLI. Pass the CLI process to fail at once, with its
 * exit status and output, when it exits before reaching the barrier.
 */
export async function waitForMetadataBarrier(
  directory: string,
  signal: 'entered' | 'applied',
  options: { child?: CliProcess<unknown>; timeoutMs?: number }
): Promise<void> {
  const barrier = path.join(directory, signal);
  let exited = undefined as CliResult<unknown> | undefined;
  void options.child?.result.then((result) => {
    exited = result;
  });
  await waitFor(
    () => fs.existsSync(barrier) || exited !== undefined,
    options.timeoutMs ?? METADATA_BARRIER_TIMEOUT_MS,
    `metadata barrier '${signal}'`
  );
  if (!fs.existsSync(barrier) && exited) {
    throw new Error(
      `The CLI exited with code ${exited.code} before metadata barrier '${signal}'.\n` +
        `stdout: ${exited.stdout.trim()}\nstderr: ${exited.stderr.trim()}`
    );
  }
}

export function readPaneTarget(tmux: (args: string[]) => string, pane: string): string {
  return tmux([
    'display-message',
    '-p',
    '-t',
    pane,
    '#{session_name}:#{window_index}.#{pane_index}',
  ]).trim();
}
