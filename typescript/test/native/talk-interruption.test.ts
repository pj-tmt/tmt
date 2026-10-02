import { existsSync, readFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import Database from 'better-sqlite3';
import path from 'node:path';
import { expect, it } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { installTmuxTripwire } from './tmux-tripwire.js';

// The sandbox runner owns this wrapper's process group, including its CLI child.
// Keep the scenario's signal control here rather than adding a product test seam.
function startTalk(sandbox: Sandbox, delay: boolean, json: boolean) {
  const ready = path.join(sandbox.root, 'talk-pid');
  const wrapper = path.join(sandbox.root, 'talk-wrapper.mjs');
  writeExecutable(
    wrapper,
    `import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
const child = spawn(${JSON.stringify(sandbox.cli.executable)},
  [...${JSON.stringify(sandbox.cli.args)}, ...process.argv.slice(2)], { stdio: 'inherit' });
child.once('spawn', () => writeFileSync(${JSON.stringify(ready)}, String(child.pid)));
child.once('error', (error) => { console.error(error); process.exit(1); });
child.once('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(code);
});
`,
    0o644
  );
  const result = runCli({ ...sandbox, cli: { executable: process.execPath, args: [wrapper] } }, [
    'talk',
    'receiver',
    'probe',
    '--inbox',
    '--identity',
    'sender',
    '--timeout',
    '30s',
    ...(delay ? ['--delay', '30s'] : []),
    ...(json ? ['--json'] : []),
  ]);
  return { ready, result };
}

async function until(condition: () => boolean) {
  const deadline = performance.now() + 2000;
  while (!condition()) {
    if (performance.now() >= deadline) throw new Error('Talk readiness timed out.');
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

async function identities(sandbox: Sandbox) {
  for (const name of ['Sender', 'Receiver']) {
    expect((await runCli(sandbox, ['identity', 'create', name, '--json'])).status).toBe(0);
  }
}

it.each([true, false])('SIGINT during --delay reports a safe retry (JSON=%s)', async (json) => {
  await withSandbox(async (sandbox) => {
    const tmuxLog = installTmuxTripwire(sandbox);
    await identities(sandbox);
    const talk = startTalk(sandbox, true, json);
    await until(() => existsSync(talk.ready));
    // There is no public delay-entry event. Allow startup to settle; the
    // structured failure and zero attempts below prove the unsent boundary.
    await new Promise((resolve) => setTimeout(resolve, 1000));
    process.kill(Number(readFileSync(talk.ready, 'utf8')), 'SIGINT');
    const result = await talk.result;
    expect(result.status).toBe(1);
    expect(result.signal).toBeNull();
    if (json) {
      expect(parseWholeStdout(result)).toEqual({
        error: {
          code: 'INTERRUPTED',
          message: 'Interrupted before sending; no message was sent.',
          suggestion: 'It is safe to run the command again when ready.',
        },
      });
    } else {
      expect(result.stdout).toBe('');
      expect(result.stderr).toBe(
        'error: Interrupted before sending; no message was sent\n' +
          'hint: It is safe to run the command again when ready\n'
      );
    }
    const database = new Database(sandbox.database, { readonly: true });
    try {
      for (const table of ['request_attempts', 'request_responses']) {
        expect(database.prepare(`SELECT COUNT(*) AS count FROM ${table}`).get()).toEqual({
          count: 0,
        });
      }
    } finally {
      database.close();
    }
    expect(existsSync(tmuxLog)).toBe(false);
  });
});

it.each([true, false])(
  'SIGINT after enqueue preserves inspection and correlation (JSON=%s)',
  async (json) => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = installTmuxTripwire(sandbox);
      await identities(sandbox);
      const database = new Database(sandbox.database, { readonly: true });
      try {
        const talk = startTalk(sandbox, false, json);
        const attempts = database.prepare(
          'SELECT request_id, status, wait_active FROM request_attempts'
        );
        await until(() => existsSync(talk.ready) && attempts.all().length === 1);
        const attempt = attempts.get() as {
          request_id: string;
          status: string;
          wait_active: number;
        };
        expect(attempt.status).toBe('queued');
        expect(attempt.wait_active).toBe(1);
        process.kill(Number(readFileSync(talk.ready, 'utf8')), 'SIGINT');
        const result = await talk.result;
        expect(result.status).toBe(1);
        expect(result.signal).toBeNull();
        const suggestion = `Inspect with 'tmt result ${attempt.request_id}'. Do not resend solely because the observer ended.`;
        if (json) {
          expect(result.stderr).toBe('');
          expect(result.stdout).toBe(
            JSON.stringify({
              error: {
                code: 'INTERRUPTED',
                message: 'Interrupted while waiting for a durable reply.',
                suggestion,
              },
              requestId: attempt.request_id,
              target: 'receiver',
              identity: { name: 'Receiver', canonicalName: 'receiver' },
            }) + '\n'
          );
        } else {
          expect(result.stdout).toBe('');
          expect(result.stderr).toBe(
            'error: Interrupted while waiting for a durable reply\n' +
              `hint: ${suggestion.slice(0, -1)}\n`
          );
        }
        expect(attempts.get()).toEqual({ ...attempt, wait_active: 0 });
        const answered = await runCli(sandbox, [
          'answer',
          'sender',
          'late final',
          '--identity',
          'receiver',
          '--request',
          attempt.request_id,
          '--json',
        ]);
        expect(answered.status).toBe(0);
        const recovered = await runCli(sandbox, ['result', attempt.request_id, '--json']);
        expect(recovered.status).toBe(0);
        expect(parseWholeStdout(recovered)).toMatchObject({
          status: 'completed',
          requestId: attempt.request_id,
          response: 'late final',
        });
      } finally {
        database.close();
      }
      expect(existsSync(tmuxLog)).toBe(false);
    });
  }
);
