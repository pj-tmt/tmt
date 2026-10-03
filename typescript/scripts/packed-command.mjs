import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import path from 'node:path';

/** Bounded process contract shared by packed CLI and storage verification. */
export function runPackedCommand(
  executable,
  args,
  { cwd, env, expectedStatus = 0, timeoutMs = 10_000, isolateProcessGroup = true }
) {
  const result = spawnSync(executable, args, {
    cwd,
    env,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    timeout: timeoutMs,
    killSignal: 'SIGKILL',
    detached: isolateProcessGroup,
    maxBuffer: 2 * 1024 * 1024,
  });
  // The verifier owns this process group, including wrapper/probe children.
  // Kill remaining descendants before its temporary home can be removed.
  if (result.pid && isolateProcessGroup) {
    try {
      process.kill(-result.pid, 'SIGKILL');
    } catch (error) {
      if (error.code !== 'ESRCH') throw error;
    }
  }
  if (result.error) throw result.error;
  const command = bounded([executable, ...args].join(' '), 300);
  assert.equal(result.signal, null, `Packed command terminated: ${result.signal}: ${command}`);
  // The first line carries the cause, because the release hold reason keeps only that line. A CLI
  // run with `--json` reports its error on stdout, so the failure shows both streams.
  const failure = (what, streams) => {
    const error = new assert.AssertionError({
      message:
        `Packed command ${what}: ${shortName(executable, args)}${firstDetail(result.stdout, result.stderr)}\n` +
        `command: ${command}\n${streams}`,
    });
    // Structured streams let callers classify a failure without parsing truncated log text.
    error.cause = { status: result.status, stdout: result.stdout, stderr: result.stderr };
    throw error;
  };
  if (result.status !== expectedStatus) {
    failure(
      `failed (exited ${result.status}, expected ${expectedStatus})`,
      `stdout: ${bounded(result.stdout)}\nstderr: ${bounded(result.stderr)}`
    );
  }
  if (result.stderr !== '') {
    failure('emitted unexpected diagnostics', `stderr: ${bounded(result.stderr)}`);
  }
  return result.stdout;
}

/** `text` cut to `limit` characters, so a failure message stays readable. */
function bounded(text, limit = 2000) {
  return text.length > limit ? `${text.slice(0, limit)}... (${text.length} characters)` : text;
}

/** The executable's name and its leading subcommand words (at most three): `tmt extension install`. */
function shortName(executable, args) {
  const words = [];
  for (const arg of args) {
    if (arg.startsWith('-') || words.length === 3) break;
    words.push(arg);
  }
  return [path.basename(executable), ...words].join(' ');
}

/** `: ` and the first line the command said about the failure (its `--json` error first), or ''. */
function firstDetail(stdout, stderr) {
  let jsonError = '';
  try {
    const message = JSON.parse(stdout)?.error?.message;
    if (typeof message === 'string') jsonError = message;
  } catch {
    // Not JSON: the streams below are the detail.
  }
  const line = [jsonError, stderr, stdout]
    .flatMap((text) => text.split('\n'))
    .map((text) => text.trim())
    .find((text) => text !== '');
  return line === undefined ? '' : `: ${bounded(line, 160)}`;
}
