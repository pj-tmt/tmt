import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it, vi } from 'vite-plus/test';

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

async function stopFixtureGroup(group: number): Promise<void> {
  try {
    process.kill(-group, 'SIGKILL');
  } catch (error) {
    if (!['ESRCH', 'EPERM'].includes((error as NodeJS.ErrnoException).code ?? '')) throw error;
  }
  const deadline = performance.now() + 3_000;
  for (;;) {
    try {
      process.kill(-group, 0);
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code === 'ESRCH') return;
      if (code !== 'EPERM') throw error;
    }
    if (performance.now() >= deadline) throw new Error('Fixture group absence was not confirmed.');
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

describe('packed command verifier', () => {
  it('excuses signal EPERM only after direct termination and a real ESRCH group probe', () => {
    const root = createFixture();
    const kill = process.kill.bind(process);
    const denied = Object.assign(new Error('Simulated signal exit race'), { code: 'EPERM' });
    let group = 0;
    let signals = 0;
    let absent = false;
    const signal = vi.spyOn(process, 'kill').mockImplementation((pid, operation) => {
      if (operation === 'SIGKILL') {
        group = -pid;
        signals++;
        throw denied;
      }
      try {
        return kill(pid, operation);
      } catch (error) {
        absent = pid === -group && (error as NodeJS.ErrnoException).code === 'ESRCH';
        throw error;
      }
    });
    try {
      const output = runNode(root, 'process.stdout.write(String(process.pid));');
      expect(group).toBe(Number(output));
      expect(signals).toBe(1);
      expect(absent).toBe(true);
      expect(() => kill(group, 0)).toThrow(expect.objectContaining({ code: 'ESRCH' }));
    } finally {
      signal.mockRestore();
      removeFixture(root);
    }
  });

  it.each(['live', 'EPERM', 'EIO'])(
    'preserves signal EPERM when the group probe reports %s',
    async (observation) => {
      const root = createFixture();
      const marker = path.join(root, 'ready.json');
      const kill = process.kill.bind(process);
      const denied = Object.assign(new Error('Simulated signal denial'), { code: 'EPERM' });
      const descendant = 'process.send("ready"); setInterval(() => {}, 1000);';
      const parent = [
        "const { spawn } = require('node:child_process');",
        `const child = spawn(process.execPath, ['--eval', ${JSON.stringify(descendant)}], { stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });`,
        `child.once('message', () => { require('node:fs').writeFileSync(${JSON.stringify(marker)}, JSON.stringify({group:process.pid, child:child.pid})); process.exit(0); });`,
      ].join('\n');
      let signals = 0;
      let probes = 0;
      const signal = vi.spyOn(process, 'kill').mockImplementation((pid, operation) => {
        const group = existsSync(marker) ? JSON.parse(readFileSync(marker, 'utf8')).group : 0;
        if (pid === -group && operation === 'SIGKILL') {
          signals++;
          throw denied;
        }
        if (pid === -group && operation === 0) {
          probes++;
          if (observation !== 'live')
            throw Object.assign(new Error('Simulated group inspection failure'), {
              code: observation,
            });
        }
        return kill(pid, operation);
      });
      try {
        expect(() => runNode(root, parent)).toThrow(denied);
        const { child, group } = JSON.parse(readFileSync(marker, 'utf8'));
        expect(signals).toBe(1);
        expect(probes).toBe(1);
        expect(kill(child, 0)).toBe(true);
        expect(kill(-group, 0)).toBe(true);
      } finally {
        signal.mockRestore();
        if (existsSync(marker)) {
          const { group } = JSON.parse(readFileSync(marker, 'utf8'));
          await stopFixtureGroup(group);
        }
        removeFixture(root);
      }
    }
  );

  it('does not excuse EPERM without a terminated direct-child result', () => {
    const root = createFixture();
    const spawn = childProcess.spawnSync;
    const incomplete = vi.spyOn(childProcess, 'spawnSync').mockImplementation((...args) => {
      const result = spawn(...args);
      return { ...result, status: null, signal: null };
    });
    syncBuiltinESMExports();
    const denied = Object.assign(new Error('Simulated signal denial'), { code: 'EPERM' });
    const signal = vi.spyOn(process, 'kill').mockImplementation(() => {
      throw denied;
    });
    try {
      expect(() => runNode(root, '')).toThrow(denied);
      expect(signal).toHaveBeenCalledTimes(1);
      expect(signal.mock.calls[0][1]).toBe('SIGKILL');
    } finally {
      signal.mockRestore();
      incomplete.mockRestore();
      syncBuiltinESMExports();
      removeFixture(root);
    }
  });

  it('preserves the execution timeout after excusing cleanup EPERM on an absent group', () => {
    const root = createFixture();
    const kill = process.kill.bind(process);
    const signal = vi.spyOn(process, 'kill').mockImplementation((pid, operation) => {
      if (operation === 'SIGKILL')
        throw Object.assign(new Error('Simulated signal exit race'), { code: 'EPERM' });
      return kill(pid, operation);
    });
    try {
      expect(() => runNode(root, 'setInterval(() => {}, 1000);', { timeoutMs: 250 })).toThrow(
        /ETIMEDOUT|timed out/i
      );
      expect(signal.mock.calls.map(([, operation]) => operation)).toEqual(['SIGKILL', 0]);
    } finally {
      signal.mockRestore();
      removeFixture(root);
    }
  });

  it('preserves non-permission signal failures without probing the group', () => {
    const root = createFixture();
    const failure = Object.assign(new Error('Simulated signal failure'), { code: 'EIO' });
    const signal = vi.spyOn(process, 'kill').mockImplementation(() => {
      throw failure;
    });
    try {
      expect(() => runNode(root, '')).toThrow(failure);
      expect(signal).toHaveBeenCalledTimes(1);
    } finally {
      signal.mockRestore();
      removeFixture(root);
    }
  });

  it('returns exact UTF-8 stdout, including complete JSON and empty output', () => {
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
  });

  it('accepts an expected exit-one failure but rejects it as unexpected by default', () => {
    const root = createFixture();
    try {
      const failure = 'process.exitCode = 1;';
      expect(runNode(root, failure, { expectedStatus: 1 })).toBe('');
      expect(() => runNode(root, failure)).toThrow('Packed command failed');
    } finally {
      removeFixture(root);
    }
  });

  it('names the command, the status and both output streams, which a --json CLI fills on stdout', () => {
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
      const [first, ...rest] = message.split('\n');
      // The first line alone says what ran, how it ended and what it said.
      expect(first).toBe(
        'Packed command failed (exited 2, expected 0): node: unrecognized subcommand squad'
      );
      expect(rest.join('\n')).toContain(`command: ${process.execPath} --eval`);
      expect(rest.join('\n')).toContain(
        'stdout: {"error":{"message":"unrecognized subcommand squad"}}'
      );
      expect(rest.join('\n')).toContain('stderr: ');
      expect(() => runNode(root, "process.stderr.write('a diagnostic\\n');")).toThrow(
        /^Packed command emitted unexpected diagnostics: node: a diagnostic\ncommand: .*\nstderr: a diagnostic/
      );
      // A long stream is cut, so the message stays readable.
      const long = runNode(root, "process.stdout.write('x'.repeat(5000)); process.exitCode = 1;", {
        expectedStatus: 1,
      });
      expect(long).toHaveLength(5000);
      expect(() =>
        runNode(root, "process.stdout.write('y'.repeat(5000)); process.exitCode = 1;")
      ).toThrow(/y{2000}\.\.\. \(5000 characters\)/);
    } finally {
      removeFixture(root);
    }
  });

  it('puts the executable, its first three subcommand words and the first line it said on one line', () => {
    const root = createFixture();
    try {
      // Run by Node without executable bits. It fails the way a CLI does, per its mode.
      const script = path.join(root, 'fake-tmt.mjs');
      writeExecutable(
        script,
        [
          'const mode = process.argv[2];',
          "if (mode === 'json') process.stdout.write(JSON.stringify({ error: { message: 'error: no such thing\\n\\nUsage: tmt' } }));",
          "if (mode === 'plain') process.stdout.write('\\n  first line\\nsecond line\\n');",
          "if (mode === 'long') process.stderr.write('z'.repeat(500));",
          'process.exitCode = 1;',
        ].join('\n'),
        0o644
      );
      const firstLine = (...args: string[]): string => {
        try {
          runPackedCommand(process.execPath, [script, ...args], { cwd: root, env: process.env });
        } catch (error) {
          return (error as Error).message.split('\n')[0];
        }
        throw new Error('The command was expected to fail.');
      };
      const head = `Packed command failed (exited 1, expected 0): node ${script}`;
      // A --json error message wins over the streams; words stop at an option and at three.
      expect(firstLine('json', '--yes')).toBe(`${head} json: error: no such thing`);
      expect(firstLine('plain', 'a', 'b', 'c')).toBe(`${head} plain a: first line`);
      expect(firstLine('long', '--yes')).toBe(
        `${head} long: ${'z'.repeat(160)}... (500 characters)`
      );
      expect(firstLine('silent')).toBe(`${head} silent`);
    } finally {
      removeFixture(root);
    }
  });

  it('rejects a signal termination and nonempty stderr', () => {
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

  it('reports a missing executable as a spawn failure', () => {
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
