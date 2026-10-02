import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { resolveCliExecutables, type CliExecutable } from './cli-executable.mjs';

// The container's test-owned Secret Service is reached through its explicit session bus.
const runtimeConnectionEnvironmentKeys = ['DBUS_SESSION_BUS_ADDRESS'] as const;

const lifecycleKey = Symbol('sandbox process lifetime');
interface ActiveRun {
  result: Promise<CliResult>;
  cleanup: Promise<void>;
  stop: () => void;
}

export interface Sandbox {
  readonly [lifecycleKey]: { closing: boolean; runs: Set<ActiveRun> };
  readonly cli: CliExecutable;
  readonly root: string;
  readonly cwd: string;
  readonly home: string;
  readonly xdgConfigHome: string;
  readonly globalDir: string;
  readonly globalConfig: string;
  readonly database: string;
  readonly localConfig: string;
  readonly env: NodeJS.ProcessEnv;
}

export interface CliResult {
  readonly status: number | null;
  readonly signal: NodeJS.Signals | null;
  readonly stdout: string;
  readonly stderr: string;
}

export interface CliRunOptions {
  /** Bytes to write to stdin before closing it; omitted means stdin is ignored. */
  readonly stdin?: string | Uint8Array;
  /** Keep stdin open after writing, for deadline and cleanup scenarios. */
  readonly closeStdin?: boolean;
  /** Combined stdout/stderr bound. Reply JSON can exceed the legacy 1 MiB bound. */
  readonly outputLimitBytes?: number;
  /** Execution deadline; termination has a separate bounded cleanup phase. */
  readonly deadlineMs?: number;
}

export type JsonDocument = Record<string, unknown>;

export function createSandbox(executableEnv: NodeJS.ProcessEnv = process.env): Sandbox {
  const { cli } = resolveCliExecutables(executableEnv);
  const root = mkdtempSync(path.join(os.tmpdir(), 'tmux-team-cli-contract-'));
  try {
    const cwd = path.join(root, 'cwd');
    const home = path.join(root, 'home');
    const xdgConfigHome = path.join(root, 'xdg');
    const tmuxTmpdir = path.join(root, 'tmux');
    const tmpdir = path.join(root, 'tmp');
    mkdirSync(cwd);
    mkdirSync(home);
    mkdirSync(tmuxTmpdir);
    mkdirSync(tmpdir);

    const globalDir = path.join(xdgConfigHome, 'tmux-team');
    const env: NodeJS.ProcessEnv = {
      HOME: home,
      XDG_CONFIG_HOME: xdgConfigHome,
      XDG_DATA_HOME: path.join(root, 'xdg-data'),
      XDG_STATE_HOME: path.join(root, 'xdg-state'),
      XDG_CACHE_HOME: path.join(root, 'xdg-cache'),
      CODEX_HOME: path.join(home, '.codex'),
      TMPDIR: tmpdir,
      // Ancestor discovery must not reach the host's default tmux server.
      // Native tests never start one; Docker owns real tmux scenarios.
      TMUX_TMPDIR: tmuxTmpdir,
      PATH: [path.dirname(process.execPath), '/usr/bin', '/bin', '/usr/sbin', '/sbin'].join(
        path.delimiter
      ),
      LANG: 'en_US.UTF-8',
      LC_ALL: 'en_US.UTF-8',
    };
    for (const key of runtimeConnectionEnvironmentKeys) {
      const value = process.env[key];
      if (value !== undefined) env[key] = value;
    }
    return {
      [lifecycleKey]: { closing: false, runs: new Set<ActiveRun>() },
      cli,
      root,
      cwd,
      home,
      xdgConfigHome,
      globalDir,
      globalConfig: path.join(globalDir, 'config.json'),
      database: path.join(globalDir, 'tmux-team.db'),
      localConfig: path.join(cwd, 'tmux-team.json'),
      env,
    };
  } catch (error) {
    rmSync(root, { recursive: true, force: true });
    throw error;
  }
}

export function runCli(
  sandbox: Sandbox,
  args: readonly string[],
  options: CliRunOptions = {}
): Promise<CliResult> {
  const lifetime = sandbox[lifecycleKey];
  if (lifetime.closing) return Promise.reject(new Error('CLI sandbox is closing.'));
  const run = startRun(sandbox, args, options);
  lifetime.runs.add(run);
  // Keep failed cleanup registered so sandbox disposal cannot delete its files.
  void run.cleanup.then(
    () => lifetime.runs.delete(run),
    () => {}
  );
  // A callback can fail before it awaits the run; disposal still owns cleanup.
  void run.result.catch(() => {});
  return run.result;
}

function startRun(sandbox: Sandbox, args: readonly string[], options: CliRunOptions): ActiveRun {
  const outputLimitBytes = options.outputLimitBytes ?? 1024 * 1024;
  const deadlineMs = options.deadlineMs ?? 5_000;
  const hasStdin = options.stdin !== undefined;
  let cleaned: () => void = () => {};
  let cleanupFailed: (error: Error) => void = () => {};
  const cleanup = new Promise<void>((resolve, reject) => {
    cleaned = resolve;
    cleanupFailed = reject;
  });
  let stop = () => {};
  const result = new Promise<CliResult>((resolve, reject) => {
    let child: ReturnType<typeof spawn>;
    try {
      child = spawn(sandbox.cli.executable, [...sandbox.cli.args, ...args], {
        cwd: sandbox.cwd,
        env: sandbox.env,
        detached: true,
        stdio: [hasStdin ? 'pipe' : 'ignore', 'pipe', 'pipe'],
        windowsHide: true,
      });
    } catch (error) {
      // Synchronous argument rejection creates no process to dispose.
      cleaned();
      reject(error);
      return;
    }
    // Decode at the stream boundary so a multibyte UTF-8 character split
    // across OS chunks cannot be corrupted by per-buffer toString() calls.
    // Both descriptors are unconditionally configured as pipes above.
    const stdoutStream = child.stdout!;
    const stderrStream = child.stderr!;
    stdoutStream.setEncoding('utf8');
    stderrStream.setEncoding('utf8');
    let stdout = '';
    let stderr = '';
    let outputBytes = 0;
    let failure: Error | undefined;
    let cleanupError: Error | undefined;
    let cleanupPermissionDenied = false;
    let inspectionError: unknown;
    let closed: { status: number | null; signal: NodeJS.Signals | null } | undefined;
    let groupGone = child.pid === undefined;
    let finishing = false;
    let settled = false;
    let cleanupDeadline = 0;
    const groupExists = (): boolean => {
      if (groupGone || child.pid === undefined) return false;
      try {
        process.kill(-child.pid, 0);
        return true;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ESRCH') {
          groupGone = true;
          return false;
        }
        throw error;
      }
    };
    const finish = (error?: Error): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) {
        cleanupFailed(error);
        child.unref();
        child.stdin?.destroy();
        stdoutStream.destroy();
        stderrStream.destroy();
        reject(
          failure
            ? new AggregateError([failure, error], `${failure.message} ${error.message}`)
            : error
        );
      } else {
        cleaned();
        if (failure) reject(failure);
        else resolve({ ...closed!, stdout, stderr });
      }
    };
    const pollCleanup = (): void => {
      if (settled) return;
      try {
        groupExists();
      } catch (error) {
        // A transient probe failure is not proof of exit. Keep polling until
        // absence is confirmed or the cleanup deadline expires.
        inspectionError = error;
      }
      if (closed && groupGone) {
        // A denied signal or initial probe can race with group exit. Require
        // direct-child close and a subsequent ESRCH probe before excusing it.
        finish(cleanupPermissionDenied ? undefined : cleanupError);
        return;
      }
      if (performance.now() >= cleanupDeadline) {
        finish(
          new Error('CLI process cleanup did not confirm close and group exit within 1000ms.', {
            cause: cleanupError ?? inspectionError,
          })
        );
        return;
      }
      setTimeout(pollCleanup, 10);
    };
    const beginCleanup = (): void => {
      if (finishing || settled) return;
      finishing = true;
      clearTimeout(timer);
      cleanupDeadline = performance.now() + 1000;
      try {
        // Signal once while still owned; never signal a PID after observing absence.
        if (groupExists()) {
          process.kill(-child.pid!, 'SIGKILL');
        }
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ESRCH') groupGone = true;
        else {
          cleanupPermissionDenied = (error as NodeJS.ErrnoException).code === 'EPERM';
          cleanupError = new Error('Could not stop CLI process group.', { cause: error });
        }
      }
      pollCleanup();
    };
    const timer = setTimeout(() => {
      failure ??= new Error(`CLI subprocess exceeded the ${deadlineMs} millisecond test bound.`);
      beginCleanup();
    }, deadlineMs);
    stop = () => {
      failure ??= new Error('CLI run cancelled during sandbox disposal.');
      beginCleanup();
    };
    const readOutput =
      (stream: 'stdout' | 'stderr') =>
      (chunk: Buffer | string): void => {
        const text = chunk.toString();
        outputBytes += Buffer.byteLength(text);
        if (outputBytes > outputLimitBytes) {
          failure ??= new Error(
            `CLI subprocess exceeded the ${outputLimitBytes}-byte output bound.`
          );
          beginCleanup();
          return;
        }
        if (stream === 'stdout') stdout += text;
        else stderr += text;
      };
    stdoutStream.on('data', readOutput('stdout'));
    stderrStream.on('data', readOutput('stderr'));
    if (child.stdin) child.stdin.on('error', () => undefined);
    child.on('error', (error) => {
      failure ??= error;
      beginCleanup();
    });
    try {
      if (hasStdin && child.stdin) {
        if (options.closeStdin === false) child.stdin.write(options.stdin);
        else child.stdin.end(options.stdin);
      }
    } catch (error) {
      failure = new Error('Could not write CLI stdin.', { cause: error });
      beginCleanup();
    }
    // Descendants can keep inherited pipes open after the direct child exits.
    child.once('exit', beginCleanup);
    child.on('close', (status, signal) => {
      closed = { status, signal };
      beginCleanup();
    });
  });
  return { result, cleanup, stop: () => stop() };
}

export function parseWholeStdout(result: CliResult): JsonDocument {
  assert.equal(result.signal, null);
  assert.equal(result.stderr, '');
  assert.notEqual(result.stdout.trim(), '');
  // Parse the complete stream. Parsing only the final line would allow human
  // output or a second JSON document to leak into JSON mode unnoticed.
  const document = JSON.parse(result.stdout) as unknown;
  assert.equal(typeof document, 'object');
  assert.notEqual(document, null);
  return document as JsonDocument;
}

export function expectError(result: CliResult, code: string, message?: string): JsonDocument {
  const document = parseWholeStdout(result);
  const error = document.error as { code?: unknown; message?: unknown } | undefined;
  assert.ok(error);
  assert.equal(error.code, code);
  assert.ok(typeof error.message === 'string');
  assert.ok(error.message.length > 0);
  if (message !== undefined) assert.equal(error.message, message);
  return document;
}

export function expectJsonSuccess(result: CliResult, value: JsonDocument): void {
  assert.equal(result.status, 0);
  assert.deepEqual(parseWholeStdout(result), value);
}

export function fileSnapshot(root: string): Record<string, string> {
  const snapshot: Record<string, string> = {};
  const visit = (current: string): void => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const entryPath = path.join(current, entry.name);
      if (entry.isDirectory()) visit(entryPath);
      else snapshot[path.relative(root, entryPath)] = readFileSync(entryPath, 'utf8');
    }
  };
  visit(root);
  return snapshot;
}

export async function withSandbox<T>(callback: (sandbox: Sandbox) => T | Promise<T>): Promise<T> {
  const sandbox = createSandbox();
  let value: T | undefined;
  let failure: { error: unknown } | undefined;
  try {
    value = await callback(sandbox);
  } catch (error) {
    failure = { error };
  }
  const lifetime = sandbox[lifecycleKey];
  lifetime.closing = true;
  const runs = [...lifetime.runs];
  for (const run of runs) run.stop();
  const results = await Promise.allSettled(runs.map((run) => run.cleanup));
  const errors = results.flatMap((result) => (result.status === 'rejected' ? [result.reason] : []));
  if (errors.length) {
    throw new AggregateError(
      failure ? [failure.error, ...errors] : errors,
      `CLI sandbox cleanup failed; retained fixture at ${sandbox.root}.`
    );
  }
  rmSync(sandbox.root, { recursive: true, force: true });
  if (failure) throw failure.error;
  return value as T;
}
