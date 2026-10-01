import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';

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
  // A CLI run with `--json` reports its error on stdout, so the failure shows both streams.
  assert.equal(
    result.status,
    expectedStatus,
    `Packed command failed: ${command} exited ${result.status}, expected ${expectedStatus}\n` +
      `stdout: ${bounded(result.stdout)}\nstderr: ${bounded(result.stderr)}`
  );
  assert.equal(
    result.stderr,
    '',
    `Packed command emitted unexpected diagnostics: ${command}\n${bounded(result.stderr)}`
  );
  return result.stdout;
}

/** `text` cut to `limit` characters, so a failure message stays readable. */
function bounded(text, limit = 2000) {
  return text.length > limit ? `${text.slice(0, limit)}... (${text.length} characters)` : text;
}
