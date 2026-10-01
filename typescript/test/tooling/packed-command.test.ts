import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const { runPackedCommand } = (await import(
  pathToFileURL(path.join(repositoryRoot, 'scripts', 'packed-command.mjs')).href
)) as unknown as {
  runPackedCommand: (
    executable: string,
    args: readonly string[],
    options: {
      cwd: string;
      env?: NodeJS.ProcessEnv;
      expectedStatus?: number;
      timeoutMs?: number;
    }
  ) => string;
};

function createFixture(): string {
  return mkdtempSync(path.join(os.tmpdir(), 'tmt-packed-command-'));
}

function removeFixture(root: string): void {
  rmSync(root, { recursive: true, force: true });
}

function runNode(
  root: string,
  script: string,
  options: { expectedStatus?: number; timeoutMs?: number } = {}
): string {
  return runPackedCommand(process.execPath, ['--eval', script], {
    cwd: root,
    env: process.env,
    ...options,
  });
}

function processIsAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ESRCH') return false;
    throw error;
  }
}

async function waitForProcessExit(pid: number, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (processIsAlive(pid)) {
    if (Date.now() >= deadline)
      throw new Error(`Process ${pid} remained alive after ${timeoutMs}ms.`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

describe('packed command verifier', () => {
  it(
    'returns exact UTF-8 stdout, including complete JSON and empty output',
    { timeout: 10_000 },
    () => {
      const root = createFixture();
      try {
        const value = {
          empty: '',
          unicode: '東京🙂',
          newline: 'first\r\nsecond',
          nul: '\0',
          bom: '\uFEFF',
        };
        const output = JSON.stringify(value);
        expect(runNode(root, `process.stdout.write(${JSON.stringify(output)});`)).toBe(output);
        expect(runNode(root, '')).toBe('');
      } finally {
        removeFixture(root);
      }
    }
  );

  it(
    'accepts an expected exit-one failure but rejects it as unexpected by default',
    { timeout: 10_000 },
    () => {
      const root = createFixture();
      try {
        const failure = 'process.exitCode = 1;';
        expect(runNode(root, failure, { expectedStatus: 1 })).toBe('');
        expect(() => runNode(root, failure)).toThrow('Packed command failed');
      } finally {
        removeFixture(root);
      }
    }
  );

  it(
    'names the command, the status and both output streams, which a --json CLI fills on stdout',
    { timeout: 10_000 },
    () => {
      const root = createFixture();
      try {
        const script =
          'process.stdout.write(\'{"error":{"message":"unrecognized subcommand squad"}}\'); process.exitCode = 2;';
        let message = '';
        try {
          runNode(root, script);
        } catch (error) {
          message = (error as Error).message;
        }
        expect(message).toContain('Packed command failed:');
        expect(message).toContain(`${process.execPath} --eval`);
        expect(message).toContain('exited 2, expected 0');
        expect(message).toContain('stdout: {"error":{"message":"unrecognized subcommand squad"}}');
        expect(message).toContain('stderr: ');
        expect(() => runNode(root, "process.stderr.write('a diagnostic\\n');")).toThrow(
          /unexpected diagnostics: .*\na diagnostic/
        );
        // A long stream is cut, so the message stays readable.
        const long = runNode(
          root,
          "process.stdout.write('x'.repeat(5000)); process.exitCode = 1;",
          {
            expectedStatus: 1,
          }
        );
        expect(long).toHaveLength(5000);
        expect(() =>
          runNode(root, "process.stdout.write('y'.repeat(5000)); process.exitCode = 1;")
        ).toThrow(/y{2000}\.\.\. \(5000 characters\)/);
      } finally {
        removeFixture(root);
      }
    }
  );

  it('rejects a signal termination and nonempty stderr', { timeout: 10_000 }, () => {
    const root = createFixture();
    try {
      expect(() => runNode(root, "process.kill(process.pid, 'SIGTERM');")).toThrow(
        'Packed command terminated: SIGTERM'
      );
      expect(() => runNode(root, "process.stderr.write('diagnostic\\n');")).toThrow(
        'Packed command emitted unexpected diagnostics'
      );
    } finally {
      removeFixture(root);
    }
  });

  it(
    'kills a timed-out child and leaves no running process behind',
    { timeout: 15_000 },
    async () => {
      const root = createFixture();
      const pidFile = path.join(root, 'child.pid');
      const descendantPidFile = path.join(root, 'descendant.pid');
      try {
        const env = {
          ...process.env,
          TMT_PACKED_TEST_PID_FILE: pidFile,
          TMT_PACKED_TEST_DESCENDANT_PID_FILE: descendantPidFile,
        };
        const descendantScript =
          "require('node:fs').writeFileSync(process.env.TMT_PACKED_TEST_DESCENDANT_PID_FILE, String(process.pid)); setInterval(() => {}, 1000);";
        const parentScript = [
          "const { spawn } = require('node:child_process');",
          "require('node:fs').writeFileSync(process.env.TMT_PACKED_TEST_PID_FILE, String(process.pid));",
          `spawn(process.execPath, ['--eval', ${JSON.stringify(descendantScript)}], { env: process.env, stdio: 'ignore' });`,
          'setInterval(() => {}, 1000);',
        ].join(' ');
        expect(() =>
          runPackedCommand(process.execPath, ['--eval', parentScript], {
            cwd: root,
            env,
            timeoutMs: 1_000,
          })
        ).toThrow(/ETIMEDOUT|timed out/i);

        expect(existsSync(pidFile)).toBe(true);
        expect(existsSync(descendantPidFile)).toBe(true);
        const pid = Number.parseInt(readFileSync(pidFile, 'utf8'), 10);
        const descendantPid = Number.parseInt(readFileSync(descendantPidFile, 'utf8'), 10);
        expect(Number.isSafeInteger(pid)).toBe(true);
        expect(Number.isSafeInteger(descendantPid)).toBe(true);
        await waitForProcessExit(pid, 3_000);
        await waitForProcessExit(descendantPid, 3_000);
        expect(processIsAlive(pid)).toBe(false);
        expect(processIsAlive(descendantPid)).toBe(false);
      } finally {
        removeFixture(root);
      }
    }
  );

  it('reports a missing executable as a spawn failure', { timeout: 10_000 }, () => {
    const root = createFixture();
    try {
      expect(() =>
        runPackedCommand(path.join(root, 'does-not-exist'), [], {
          cwd: root,
          env: process.env,
        })
      ).toThrow(/ENOENT|spawn/);
    } finally {
      removeFixture(root);
    }
  });
});
