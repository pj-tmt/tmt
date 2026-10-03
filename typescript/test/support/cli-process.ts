import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  realpathSync,
  rmSync,
  statSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import type { Duplex, Readable } from 'node:stream';
import { fileURLToPath } from 'node:url';
import { resolveCliExecutables, type CliExecutable } from './cli-executable.mjs';

// The container's test-owned Secret Service is reached through its explicit session bus.
const runtimeConnectionEnvironmentKeys = ['DBUS_SESSION_BUS_ADDRESS'] as const;
const neutralParent = fileURLToPath(new URL('./neutral-parent.mjs', import.meta.url));

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
      child = spawn(
        process.execPath,
        [neutralParent, 'detach', sandbox.cli.executable, ...sandbox.cli.args, ...args],
        {
          cwd: sandbox.cwd,
          env: sandbox.env,
          detached: true,
          stdio: [hasStdin ? 'pipe' : 'ignore', 'pipe', 'pipe', 'pipe', 'pipe'],
          windowsHide: true,
        }
      );
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
    let control = '';
    let controlBytes = 0;
    let completion: { status: number | null; signal: NodeJS.Signals | null } | undefined;
    let cliGroup: number | undefined;
    const groups = new Set(child.pid === undefined ? [] : [child.pid]);
    const acknowledgement = child.stdio[4]! as Duplex;
    acknowledgement.on('error', () => undefined);
    const controlStream = child.stdio[3]! as Readable;
    controlStream.setEncoding('utf8');
    controlStream.on('data', (chunk: string) => {
      control += chunk;
      controlBytes += Buffer.byteLength(chunk);
      if (controlBytes > 16384) {
        failure ??= new Error('Neutral-parent control exceeded its bound.');
        beginCleanup();
      }
      let newline: number;
      while ((newline = control.indexOf('\n')) !== -1) {
        const line = control.slice(0, newline);
        control = control.slice(newline + 1);
        try {
          const report: unknown = JSON.parse(line);
          if (typeof report === 'object' && report !== null && 'group' in report) {
            if (
              cliGroup !== undefined ||
              typeof report.group !== 'number' ||
              !Number.isSafeInteger(report.group) ||
              report.group <= 1 ||
              report.group === child.pid
            )
              throw new Error('Invalid CLI process-group ownership.');
            cliGroup = report.group;
            groups.add(cliGroup);
            if (finishing) stopGroup(cliGroup);
            else acknowledgement.end('ready\n');
          } else if (typeof report === 'object' && report !== null && 'error' in report) {
            const error = report.error;
            if (
              typeof error !== 'object' ||
              error === null ||
              !('message' in error) ||
              typeof error.message !== 'string' ||
              !('code' in error) ||
              typeof error.code !== 'string'
            )
              throw new Error('Invalid launcher error.');
            failure ??= Object.assign(new Error(error.message), { code: error.code });
            beginCleanup();
          } else {
            if (completion !== undefined) throw new Error('Duplicate launcher completion.');
            if (
              cliGroup === undefined ||
              typeof report !== 'object' ||
              report === null ||
              !('status' in report) ||
              !('signal' in report)
            )
              throw new Error('Missing selected CLI completion.');
            const { status, signal } = report;
            if (
              !(
                status === null ||
                (typeof status === 'number' &&
                  Number.isInteger(status) &&
                  status >= 0 &&
                  status <= 255)
              ) ||
              !(
                signal === null ||
                (typeof signal === 'string' && signal in os.constants.signals)
              ) ||
              (status === null) !== (signal !== null)
            )
              throw new Error('Invalid selected CLI completion.');
            completion = { status, signal: signal as NodeJS.Signals | null };
            beginCleanup();
          }
        } catch (error) {
          failure ??= new Error('Invalid neutral-parent control.', { cause: error });
          beginCleanup();
        }
      }
    });
    let cleanupError: Error | undefined;
    let cleanupPermissionDenied = false;
    let cleanupOtherFailure = false;
    let inspectionError: unknown;
    let closed: { status: number | null; signal: NodeJS.Signals | null } | undefined;
    let finishing = false;
    let settled = false;
    let cleanupDeadline = 0;
    const groupExists = (group: number): boolean => {
      if (!groups.has(group)) return false;
      try {
        process.kill(-group, 0);
        return true;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ESRCH') {
          groups.delete(group);
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
        controlStream.destroy();
        acknowledgement.destroy();
        reject(
          failure
            ? new AggregateError([failure, error], `${failure.message} ${error.message}`)
            : error
        );
      } else {
        cleaned();
        if (failure) reject(failure);
        else resolve({ ...completion!, stdout, stderr });
      }
    };
    const pollCleanup = (): void => {
      if (settled) return;
      try {
        for (const group of groups) groupExists(group);
      } catch (error) {
        // A transient probe failure is not proof of exit. Keep polling until
        // absence is confirmed or the cleanup deadline expires.
        inspectionError = error;
      }
      if (closed && groups.size === 0) {
        // A denied signal or initial probe can race with group exit. Require
        // direct-child close and a subsequent ESRCH probe before excusing it.
        finish(cleanupPermissionDenied && !cleanupOtherFailure ? undefined : cleanupError);
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
    const stopGroup = (group: number): void => {
      try {
        // Signal once while still owned; never signal a PID after observing absence.
        if (groupExists(group)) {
          process.kill(-group, 'SIGKILL');
        }
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ESRCH') groups.delete(group);
        else {
          if ((error as NodeJS.ErrnoException).code === 'EPERM') cleanupPermissionDenied = true;
          else cleanupOtherFailure = true;
          cleanupError = new Error('Could not stop CLI process group.', { cause: error });
        }
      }
    };
    const beginCleanup = (): void => {
      if (finishing || settled) return;
      finishing = true;
      clearTimeout(timer);
      cleanupDeadline = performance.now() + 1000;
      acknowledgement.destroy();
      for (const group of groups) stopGroup(group);
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
    // Successful setup exits to orphan the supervisor. Its completion record,
    // not this setup exit, starts cleanup of the selected CLI and descendants.
    child.once('exit', (status, signal) => {
      if (status !== 0 || signal !== null) {
        failure ??= new Error('Neutral-parent setup did not exit successfully.');
        beginCleanup();
      }
    });
    child.on('close', (status, signal) => {
      if (!failure) {
        if (control !== '' || completion === undefined)
          failure = new Error('Could not establish neutral-parent CLI completion.');
      }
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

function processCwds(pid?: number): { pid: number; cwd: string }[] {
  const pids =
    pid === undefined ? readdirSync('/proc').filter((name) => /^\d+$/.test(name)) : [String(pid)];
  return pids.flatMap((name) => {
    try {
      if (statSync(`/proc/${name}`).uid !== process.getuid!()) return [];
      return [
        {
          pid: Number(name),
          cwd: readlinkSync(`/proc/${name}/cwd`).replace(/ \(deleted\)$/, ''),
        },
      ];
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code === 'ENOENT' || code === 'ESRCH') return [];
      // Same-user system processes can deny cwd access. Only discovery may skip them;
      // an already verified resident must remain a cleanup failure if inspection fails.
      if (pid === undefined && (code === 'EACCES' || code === 'EPERM')) return [];
      throw error;
    }
  });
}

function sandboxProcesses(root: string, pid?: number): number[] {
  return processCwds(pid)
    .filter(({ cwd }) => cwd === root || cwd.startsWith(`${root}${path.sep}`))
    .map(({ pid }) => pid);
}

export async function withSandbox<T>(callback: (sandbox: Sandbox) => T | Promise<T>): Promise<T> {
  const sandbox = createSandbox();
  const processRoot = realpathSync(sandbox.root);
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
  let leak: Error | undefined;
  try {
    const leaked = process.platform === 'linux' ? sandboxProcesses(processRoot) : [];
    if (leaked.length) {
      leak = new Error(`Sandbox callback left live processes: ${leaked.join(', ')}.`);
      for (const pid of leaked) {
        // Recheck cwd immediately before signalling; a recycled or moved PID is not ours.
        if (sandboxProcesses(processRoot, pid).length === 0) continue;
        if (pid === process.pid) throw new Error('The test runner is still inside the sandbox.');
        try {
          process.kill(pid, 'SIGKILL');
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error;
        }
      }
      const deadline = performance.now() + 1000;
      while (
        sandboxProcesses(processRoot).length ||
        leaked.some((pid) => {
          try {
            process.kill(pid, 0);
            return true;
          } catch (error) {
            if ((error as NodeJS.ErrnoException).code === 'ESRCH') return false;
            throw error;
          }
        })
      ) {
        if (performance.now() >= deadline)
          throw new Error('Sandbox process cleanup did not confirm exit within 1000ms.');
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
    }
  } catch (error) {
    errors.push(error);
  }
  if (errors.length) {
    throw new AggregateError(
      [...(failure ? [failure.error] : []), ...(leak ? [leak] : []), ...errors],
      `CLI sandbox cleanup failed; retained fixture at ${sandbox.root}.`
    );
  }
  rmSync(sandbox.root, { recursive: true, force: true });
  if (leak) throw new AggregateError(failure ? [failure.error, leak] : [leak], leak.message);
  if (failure) throw failure.error;
  return value as T;
}
