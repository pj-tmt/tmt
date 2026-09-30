import fs from 'node:fs';
import path from 'node:path';
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

export async function waitForProcessExit(
  panePids: readonly number[],
  isRunning: (pid: number) => boolean,
  timeoutMs = 2_000
): Promise<void> {
  if (panePids.length === 0) return;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline && panePids.some((panePid) => isRunning(panePid))) {
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
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

export function readMockEvents(logPath: string): MockEvent[] {
  if (!fs.existsSync(logPath)) return [];
  return fs
    .readFileSync(logPath, 'utf8')
    .trim()
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line) as MockEvent);
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
  options: { child?: CliProcess<unknown>; timeoutMs?: number },
  waitForCondition: (
    predicate: () => boolean,
    timeoutMs: number,
    description: string
  ) => Promise<void>
): Promise<void> {
  const barrier = path.join(directory, signal);
  let exited = undefined as CliResult<unknown> | undefined;
  void options.child?.result.then((result) => {
    exited = result;
  });
  await waitForCondition(
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
