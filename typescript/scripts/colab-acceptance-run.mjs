import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

/** Absolute cutoff includes setup; the remaining job budget belongs to grace, proof and evidence. */
export function acceptanceDeadline(startedAtSeconds, shard) {
  if (!Number.isSafeInteger(startedAtSeconds) || startedAtSeconds <= 0 || ![1, 2].includes(shard))
    throw new Error('Acceptance requires a positive start timestamp and shard 1 or 2.');
  return startedAtSeconds * 1000 + (shard === 1 ? 1560 : 1740) * 1000;
}

/** Own only this child's new process group. Harness-detached groups remain the hosted job's concern. */
export async function runAcceptance({ command, args, deadlineMs, graceMs, verdictPath }) {
  const write = (verdict) => {
    fs.mkdirSync(path.dirname(verdictPath), { recursive: true });
    fs.writeFileSync(verdictPath, JSON.stringify(verdict) + '\n');
  };
  if (!Number.isSafeInteger(deadlineMs) || !Number.isSafeInteger(graceMs) || graceMs < 0)
    throw new Error('Acceptance deadline and grace must be bounded integer milliseconds.');
  if (deadlineMs <= Date.now()) {
    write({ state: 'deadline', forced: false, launched: false });
    return 124;
  }
  write({ state: 'running' });
  return await new Promise((resolve, reject) => {
    let child;
    let deadline;
    let grace;
    let reason;
    let settled = false;
    const groupSignal = (signal) => {
      if (!child?.pid || child.pid === process.pid) return false;
      try {
        process.kill(-child.pid, signal);
        return true;
      } catch (error) {
        if (error.code === 'ESRCH') return false;
        throw error;
      }
    };
    const finish = (verdict, code) => {
      if (settled) return;
      settled = true;
      clearTimeout(deadline);
      clearTimeout(grace);
      process.off('SIGINT', onInterrupt);
      process.off('SIGTERM', onTerminate);
      try {
        write(verdict);
        resolve(code);
      } catch (error) {
        reject(error);
      }
    };
    const interrupt = (state) => {
      if (reason || settled) return;
      reason = state;
      try {
        if (!groupSignal('SIGINT')) {
          finish({ state, forced: false, launched: true }, state === 'deadline' ? 124 : 130);
          return;
        }
        grace = setTimeout(() => {
          try {
            const forced = groupSignal('SIGKILL');
            finish({ state, forced, launched: true }, state === 'deadline' ? 124 : 130);
          } catch {
            finish({ state: 'runner-failed' }, 1);
          }
        }, graceMs);
      } catch {
        finish({ state: 'runner-failed' }, 1);
      }
    };
    const onInterrupt = () => interrupt('interrupted');
    const onTerminate = () => interrupt('interrupted');
    process.once('SIGINT', onInterrupt);
    process.once('SIGTERM', onTerminate);
    child = spawn(command, args, { detached: true, stdio: 'inherit' });
    child.once('error', () => finish({ state: 'startup-failed' }, 1));
    child.once('exit', (code) => {
      if (!reason) {
        finish({ state: 'completed', exitCode: code ?? 1 }, code ?? 1);
        return;
      }
      // The leader may exit before a same-group descendant; keep the bounded grace in that case.
      try {
        if (!groupSignal(0))
          finish(
            { state: reason, forced: false, launched: true },
            reason === 'deadline' ? 124 : 130
          );
      } catch {
        finish({ state: 'runner-failed' }, 1);
      }
    });
    deadline = setTimeout(() => interrupt('deadline'), Math.max(0, deadlineMs - Date.now()));
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [shard, command, ...args] = process.argv.slice(2);
  const verdictPath = path.join(process.env.RUNNER_TEMP, 'colab-acceptance-private/execution.json');
  try {
    const startedAt = Number(process.env.TMT_ACCEPTANCE_STARTED_AT);
    if (!command || startedAt * 1000 > Date.now())
      throw new Error('Invalid acceptance invocation.');
    process.exitCode = await runAcceptance({
      command,
      args,
      deadlineMs: acceptanceDeadline(startedAt, Number(shard)),
      graceMs: 30_000,
      verdictPath,
    });
  } catch {
    fs.mkdirSync(path.dirname(verdictPath), { recursive: true });
    fs.writeFileSync(verdictPath, JSON.stringify({ state: 'startup-failed' }) + '\n');
    console.error('Acceptance runner failed; private output is not published.');
    process.exitCode = 1;
  }
}
