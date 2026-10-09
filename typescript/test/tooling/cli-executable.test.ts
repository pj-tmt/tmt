import { writeExecutable as publishExecutable } from '../support/executable-fixture.mjs';
import fs from 'node:fs';
import { execFileSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { createCliProbe } from '../support/cli-probe.js';
import { createSandbox, parseWholeStdout, runCli } from '../support/cli-process.js';

const temporaryRoots: string[] = [];

function temporaryRoot(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-cli-executable-'));
  temporaryRoots.push(root);
  return root;
}

function writeExecutable(root: string, name: string, source: string, mode = 0o755): string {
  const executable = path.join(root, name);
  publishExecutable(executable, source, mode);
  return executable;
}

function descriptor(executable: string, args: readonly string[] = []): string {
  return JSON.stringify({ executable, args });
}

function sandboxWithEnv(env: NodeJS.ProcessEnv) {
  const sandbox = createSandbox(env);
  temporaryRoots.push(sandbox.root);
  return sandbox;
}

function processIsAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ESRCH') return false;
    throw error;
  }
}

async function waitForProcessExit(pid: number, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (processIsAlive(pid)) {
    if (Date.now() >= deadline) throw new Error(`Process ${pid} remained alive.`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

async function waitForFile(file: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!fs.existsSync(file)) {
    if (Date.now() >= deadline) {
      const directory = path.dirname(file);
      let entries: string;
      try {
        entries = JSON.stringify(fs.readdirSync(directory).sort());
      } catch (error) {
        entries = `unavailable (${error instanceof Error ? error.message : String(error)})`;
      }
      throw new Error(
        `File ${file} did not appear. Started marker present: ${fs.existsSync(path.join(directory, 'started'))}; directory entries: ${entries}.`
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

// Test-local ordering control: capture only runCli's synchronously armed execution timer.
// Cleanup/observation timers stay real; this is not elapsed startup-time evidence.
function controlledExecutionExpiry(start: () => Promise<unknown>) {
  const schedule = globalThis.setTimeout;
  let expire: (() => void) | undefined;
  let requestedDelay: number | undefined;
  const timer = vi
    .spyOn(globalThis, 'setTimeout')
    .mockImplementation((callback, delay, ...args) => {
      if (delay === 1_000 && expire === undefined) {
        requestedDelay = delay;
        let expired = false;
        expire = () => {
          if (expired) return;
          expired = true;
          callback(...args);
        };
        return schedule(() => {}, delay);
      }
      return schedule(callback, delay, ...args);
    });
  try {
    const pending = start();
    if (!expire) throw new Error('The 1000ms execution timer was not armed synchronously.');
    return { pending, expire, requestedDelay };
  } finally {
    timer.mockRestore();
  }
}

async function settleControlledExpiry(
  run: ReturnType<typeof controlledExecutionExpiry>,
  ownedRoots: string[]
) {
  run.expire();
  const failure = await run.pending.catch((error: unknown) => error);
  if (
    !(failure instanceof Error) ||
    failure instanceof AggregateError ||
    failure.message !== 'CLI subprocess exceeded the 1000 millisecond test bound.'
  ) {
    for (const root of ownedRoots) {
      const index = temporaryRoots.indexOf(root);
      if (index !== -1) temporaryRoots.splice(index, 1);
    }
    console.error('Controlled CLI cleanup unconfirmed; retained fixtures:', ownedRoots);
    throw failure;
  }
}

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe('CLI executable descriptors', () => {
  it('uses the repository native executable by default for both invocation roles', () => {
    const executables = resolveCliExecutables({});
    const expected = fileURLToPath(new URL('../../../rust/target/debug/tmt', import.meta.url));

    expect(path.isAbsolute(executables.cli.executable)).toBe(true);
    expect(executables.cli.executable).toBe(expected);
    expect(fs.statSync(executables.cli.executable).isFile()).toBe(true);
    expect(executables.cli.args).toEqual([]);
    expect(executables.peer).toBe(executables.cli);
  });

  it('fails clearly when the repository native executable is unavailable', () => {
    const expected = fileURLToPath(new URL('../../../rust/target/debug/tmt', import.meta.url));
    const stat = vi.spyOn(fs, 'statSync').mockImplementation(() => {
      throw new Error('native executable intentionally unavailable');
    });
    try {
      expect(() => resolveCliExecutables({})).toThrow(
        `TMT_TEST_CLI executable is unavailable or not executable: ${expected}`
      );
      expect(() => resolveCliExecutables({})).toThrow(
        'cargo build --manifest-path rust/Cargo.toml --locked'
      );
    } finally {
      stat.mockRestore();
    }
  });

  it.each([
    ['malformed JSON', '{'],
    ['missing executable', JSON.stringify({ args: [] })],
    ['relative executable', JSON.stringify({ executable: 'bin/tmt', args: [] })],
    ['NUL in executable', JSON.stringify({ executable: '/tmp/tmt\0cli', args: [] })],
    ['non-array prefix', JSON.stringify({ executable: process.execPath, args: '--flag' })],
    ['non-string prefix item', JSON.stringify({ executable: process.execPath, args: [1] })],
    ['NUL in prefix item', JSON.stringify({ executable: process.execPath, args: ['a\0b'] })],
    [
      'unknown descriptor key',
      JSON.stringify({ executable: process.execPath, args: [], extra: true }),
    ],
  ])('rejects %s before any executable lookup', (_label, serialized) => {
    expect(() => resolveCliExecutables({ TMT_TEST_CLI: serialized })).toThrow(/TMT_TEST_CLI/);
  });

  it('rejects a missing executable path', () => {
    const root = temporaryRoot();
    const executable = path.join(root, 'missing');

    expect(() => resolveCliExecutables({ TMT_TEST_CLI: descriptor(executable) })).toThrow(
      /TMT_TEST_CLI executable is unavailable or not executable/
    );
  });

  it('rejects a directory descriptor', () => {
    const root = temporaryRoot();
    const executable = path.join(root, 'directory');
    fs.mkdirSync(executable);

    expect(() => resolveCliExecutables({ TMT_TEST_CLI: descriptor(executable) })).toThrow(
      /TMT_TEST_CLI executable is unavailable or not executable/
    );
  });

  it('rejects a non-executable file descriptor', () => {
    const root = temporaryRoot();
    const executable = writeExecutable(root, 'not-executable', '#!/bin/sh\n', 0o644);

    expect(() => resolveCliExecutables({ TMT_TEST_CLI: descriptor(executable) })).toThrow(
      /TMT_TEST_CLI executable is unavailable or not executable/
    );
  });

  it('validates an explicitly selected peer and never falls back to the main executable', () => {
    const root = temporaryRoot();
    const main = writeExecutable(root, 'main', '#!/bin/sh\nexit 0\n');
    const missingPeer = path.join(root, 'missing-peer');

    expect(() =>
      resolveCliExecutables({
        TMT_TEST_CLI: descriptor(main),
        TMT_TEST_PEER_CLI: descriptor(missingPeer),
      })
    ).toThrow(/TMT_TEST_PEER_CLI executable is unavailable/);
  });

  it('uses the selected main executable as the peer when no peer is supplied', () => {
    const root = temporaryRoot();
    const main = writeExecutable(root, 'main', '#!/bin/sh\nexit 0\n');
    const defaults = resolveCliExecutables({});
    const selected = resolveCliExecutables({ TMT_TEST_CLI: descriptor(main, ['--main-prefix']) });

    expect(selected.cli).toEqual({ executable: main, args: ['--main-prefix'] });
    expect(selected.peer).toBe(selected.cli);
    expect(defaults.peer).not.toEqual(selected.cli);
  });

  it('returns frozen descriptor and prefix copies', () => {
    const root = temporaryRoot();
    const executable = writeExecutable(root, 'main', '#!/bin/sh\nexit 0\n');
    const selected = resolveCliExecutables({
      TMT_TEST_CLI: descriptor(executable, ['prefix', 'with spaces']),
      TMT_TEST_PEER_CLI: descriptor(executable, ['peer']),
    });

    expect(Object.isFrozen(selected)).toBe(true);
    expect(Object.isFrozen(selected.cli)).toBe(true);
    expect(Object.isFrozen(selected.cli.args)).toBe(true);
    expect(() => (selected.cli.args as string[]).push('mutated')).toThrow(TypeError);
    expect(selected.cli.args).toEqual(['prefix', 'with spaces']);
  });

  it('resolves selection before sandbox allocation and snapshots it', () => {
    const root = temporaryRoot();
    const main = writeExecutable(root, 'main', '#!/bin/sh\nexit 0\n');
    const executableEnv = { TMT_TEST_CLI: descriptor(main, ['stable']) };
    const sandbox = sandboxWithEnv(executableEnv);
    executableEnv.TMT_TEST_CLI = descriptor(path.join(root, 'missing'));

    expect(sandbox.cli).toEqual({ executable: main, args: ['stable'] });
    expect(fs.existsSync(sandbox.root)).toBe(true);
  });

  it('fails an invalid selector before creating a sandbox directory', () => {
    const mkdtemp = vi.spyOn(fs, 'mkdtempSync');
    try {
      expect(() =>
        createSandbox({ TMT_TEST_CLI: JSON.stringify({ executable: 'relative' }) })
      ).toThrow(/TMT_TEST_CLI/);
      expect(mkdtemp).not.toHaveBeenCalled();
    } finally {
      mkdtemp.mockRestore();
    }
  });

  it.each(['default', 'explicit'])(
    'runs the %s native executable through the real contract helper',
    async (selection) => {
      const sandbox = sandboxWithEnv(
        selection === 'default'
          ? {}
          : { TMT_TEST_CLI: JSON.stringify(resolveCliExecutables({}).cli) }
      );
      const created = await runCli(sandbox, ['identity', 'create', 'Selected', '--json']);
      const shown = await runCli(sandbox, ['identity', 'show', 'selected', '--json']);

      expect(created.status).toBe(0);
      expect(shown).toMatchObject({ status: 0, signal: null });
      expect(parseWholeStdout(shown)).toMatchObject({
        identity: parseWholeStdout(created).identity,
      });
      expect(parseWholeStdout(shown)).toMatchObject({
        identity: { name: 'Selected', canonicalName: 'selected' },
      });
    }
  );

  it('runs an explicit non-Node probe and preserves its argv prefix and arguments', async () => {
    const root = temporaryRoot();
    const defaults = resolveCliExecutables({});
    const probe = createCliProbe(root, defaults.cli);
    const sandbox = sandboxWithEnv({ TMT_TEST_CLI: JSON.stringify(probe.descriptor) });
    const identityName = "Reader's identity";
    const created = await runCli(sandbox, ['identity', 'create', identityName, '--json']);
    const shown = await runCli(sandbox, ['identity', 'show', identityName, '--json']);
    const body = 'body with spaces; "$HOME" and \'quotes\'';
    const args = ['role', 'set', body, '--identity', identityName, '--json'];
    const role = await runCli(sandbox, args);
    const persistedRole = await runCli(sandbox, [
      'role',
      'show',
      '--identity',
      identityName,
      '--json',
    ]);

    expect(created.status).toBe(0);
    expect(shown.status).toBe(0);
    expect(parseWholeStdout(shown)).toMatchObject({ identity: { name: identityName } });
    expect(role.status).toBe(0);
    expect(persistedRole.status).toBe(0);
    expect(parseWholeStdout(persistedRole)).toMatchObject({ role: { content: body } });
    expect(probe.invocations()).toContainEqual([
      defaults.cli.executable,
      ...defaults.cli.args,
      ...args,
    ]);
  });

  it('propagates a selected executable nonzero exit without falling back', async () => {
    const root = temporaryRoot();
    const executable = writeExecutable(root, 'nonzero', '#!/bin/sh\nexit 23\n');
    const sandbox = sandboxWithEnv({ TMT_TEST_CLI: descriptor(executable) });
    const result = await runCli(sandbox, ['--version']);

    expect(result).toEqual({ status: 23, signal: null, stdout: '', stderr: '' });
  });

  it('controlled expiry cleans up an already-ready selected descendant process group', async () => {
    const root = temporaryRoot();
    const childPidPath = path.join(root, 'child.pid');
    const executable = writeExecutable(
      root,
      'long-running',
      '#!/bin/sh\necho started > "${1%/*}/started"\n(sleep 30) &\nprintf "%s %s\\n" "$!" "$$" > "$1.pending"\nmv "$1.pending" "$1"\nwhile :; do sleep 1; done\n'
    );
    const sandbox = sandboxWithEnv({ TMT_TEST_CLI: descriptor(executable, [childPidPath]) });
    const run = controlledExecutionExpiry(() => runCli(sandbox, [], { deadlineMs: 1_000 }));
    expect(run.requestedDelay).toBe(1_000);
    try {
      await waitForFile(childPidPath, 1_000);
      const [childPid, group] = fs
        .readFileSync(childPidPath, 'utf8')
        .trim()
        .split(/\s+/)
        .map(Number);
      expect(Number.isSafeInteger(childPid) && childPid > 0).toBe(true);
      expect(Number.isSafeInteger(group) && group > 0 && group !== process.pid).toBe(true);
      expect(
        Number(
          execFileSync('/bin/ps', ['-o', 'pgid=', '-p', String(childPid)], {
            encoding: 'utf8',
          }).trim()
        )
      ).toBe(group);
      expect(processIsAlive(childPid)).toBe(true);
      run.expire();
      await expect(run.pending).rejects.toThrow(
        'CLI subprocess exceeded the 1000 millisecond test bound.'
      );
      await waitForProcessExit(childPid, 2_000);
      expect(processIsAlive(childPid)).toBe(false);
      expect(processIsAlive(-group)).toBe(false);
    } finally {
      // Also expire after failed readiness/identity assertions; never leave this controlled run live.
      await settleControlledExpiry(run, [root, sandbox.root]);
    }
  });

  it('controlled expiry before selected readiness cannot claim descendant cleanup', async () => {
    const root = temporaryRoot();
    const childPidPath = path.join(root, 'child.pid');
    const executable = writeExecutable(root, 'not-ready', '#!/bin/sh\nwhile :; do sleep 1; done\n');
    const sandbox = sandboxWithEnv({ TMT_TEST_CLI: descriptor(executable, [childPidPath]) });
    const run = controlledExecutionExpiry(() => runCli(sandbox, [], { deadlineMs: 1_000 }));
    expect(run.requestedDelay).toBe(1_000);
    try {
      run.expire();
      await expect(run.pending).rejects.toThrow(
        'CLI subprocess exceeded the 1000 millisecond test bound.'
      );
      expect(fs.existsSync(childPidPath)).toBe(false);
    } finally {
      await settleControlledExpiry(run, [root, sandbox.root]);
    }
  });
});

it('inert controlled expiry captures the unchanged timer once and restores scheduling immediately', async () => {
  const failure = new Error('original deadline');
  let fired = 0;
  const originalSchedule = globalThis.setTimeout;
  const run = controlledExecutionExpiry(
    () =>
      new Promise((_, reject) => {
        const timer = setTimeout(() => {
          clearTimeout(timer);
          fired++;
          reject(failure);
        }, 1_000);
      })
  );
  void run.pending.catch(() => {});
  expect(globalThis.setTimeout).toBe(originalSchedule);
  expect(run.requestedDelay).toBe(1_000);
  expect(fired).toBe(0);
  run.expire();
  run.expire();
  await expect(run.pending).rejects.toBe(failure);
  expect(fired).toBe(1);
});
