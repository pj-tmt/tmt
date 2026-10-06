import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
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
import { createServer, type Socket } from 'node:net';
import type { Readable } from 'node:stream';
import { fileURLToPath } from 'node:url';
import { resolveCliExecutables, type CliExecutable } from './cli-executable.mjs';

// The container's test-owned Secret Service is reached through its explicit session bus.
const runtimeConnectionEnvironmentKeys = ['DBUS_SESSION_BUS_ADDRESS'] as const;
const neutralParent = fileURLToPath(
  new URL('../../../rust/target/debug/examples/runtime-caller-fixture', import.meta.url)
);

// Diagnostic state is bounded and never supplies process ownership or changes cleanup.
type DiagnosticArgument = { bytes: number; sha256: string; literal?: string };
type CliDiagnostic = {
  callId: number;
  sandboxRoot: string;
  executable: string;
  executableSha256: string;
  fixtureArgv: DiagnosticArgument[];
  fixtureArgc: number;
  argv: DiagnosticArgument[];
  argc: number;
  deadlineMs: number;
  events: {
    phase: string;
    atMs: number;
    pid?: number;
    status?: number | null;
    signal?: string | null;
  }[];
  omittedEvents: number;
  cleanup?: { closeObserved: boolean; remainingGroups: number[]; failed: boolean };
  partialOutput?: { stdout: DiagnosticArgument; stderr: DiagnosticArgument; observedBytes: number };
};
const diagnosticTokens = new Set([
  '--version',
  'upgrade',
  'update',
  '--json',
  '--channel',
  'stable',
  'alpha',
  '__native-install',
  '--archive',
  '--manifest',
  '--prefix',
  '--pin',
  '--unpin',
]);
function diagnosticArgument(value: string): DiagnosticArgument {
  return {
    bytes: Buffer.byteLength(value),
    sha256: createHash('sha256').update(value).digest('hex'),
    ...(diagnosticTokens.has(value) ? { literal: value } : {}),
  };
}
function diagnosticPath(value: string): string {
  return Buffer.byteLength(value) <= 256
    ? value
    : `[path exceeds 256 bytes; sha256=${createHash('sha256').update(value).digest('hex')}]`;
}
function reportDiagnostic(scope: 'call' | 'sandbox', diagnostic: object): void {
  try {
    const text = JSON.stringify(diagnostic);
    // Never emit raw output, stdin, environment, /proc cmdline or unrelated process facts.
    console.error(
      `CLI ${scope} diagnostics:`,
      Buffer.byteLength(text) <= 16_384
        ? text
        : JSON.stringify({
            omitted: 'diagnostic representation exceeds 16384 bytes',
            bytes: Buffer.byteLength(text),
            sha256: createHash('sha256').update(text).digest('hex'),
          })
    );
  } catch {
    // Observation/reporting failure cannot replace an original error or interrupt disposal.
  }
}
const lifecycleKey = Symbol('sandbox process lifetime');
interface ActiveRun {
  result: Promise<CliResult>;
  cleanup: Promise<void>;
  stop: () => void;
}

export interface Sandbox {
  readonly [lifecycleKey]: {
    closing: boolean;
    runs: Set<ActiveRun>;
    nextCallId: number;
    diagnostics: CliDiagnostic[];
  };
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
      [lifecycleKey]: {
        closing: false,
        runs: new Set<ActiveRun>(),
        nextCallId: 0,
        diagnostics: [],
      },
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
  const lifetime = sandbox[lifecycleKey];
  const diagnostic: CliDiagnostic = {
    callId: ++lifetime.nextCallId,
    sandboxRoot: diagnosticPath(sandbox.root),
    executable: diagnosticPath(sandbox.cli.executable),
    executableSha256: createHash('sha256').update(sandbox.cli.executable).digest('hex'),
    fixtureArgv: sandbox.cli.args.slice(0, 8).map(diagnosticArgument),
    fixtureArgc: sandbox.cli.args.length,
    argv: args.slice(0, 8).map(diagnosticArgument),
    argc: args.length,
    deadlineMs,
    events: [],
    omittedEvents: 0,
  };
  // Clones share one sequence. Two recent calls provide context without retaining output bodies.
  lifetime.diagnostics.push(diagnostic);
  if (lifetime.diagnostics.length > 2) lifetime.diagnostics.shift();
  const record = (
    phase: string,
    detail: { pid?: number; status?: number | null; signal?: string | null } = {}
  ): void => {
    if (diagnostic.events.length === 32) {
      diagnostic.events.splice(16, 1);
      diagnostic.omittedEvents++;
    }
    diagnostic.events.push({ phase, atMs: performance.now(), ...detail });
  };
  record('call-start');
  let cleaned: () => void = () => {};
  let cleanupFailed: (error: Error) => void = () => {};
  const cleanup = new Promise<void>((resolve, reject) => {
    cleaned = resolve;
    cleanupFailed = reject;
  });
  let stop = () => {};
  const result = new Promise<CliResult>((resolve, reject) => {
    let socketRoot: string;
    try {
      // A nested sandbox's TMPDIR can exceed macOS's Unix socket path bound.
      // This private per-run directory has the same explicit cleanup owner.
      socketRoot = mkdtempSync(`/tmp/tmt-cli-parent-${process.pid}-`);
    } catch (error) {
      cleaned();
      record('control-root-failed');
      reportDiagnostic('call', diagnostic);
      reject(error);
      return;
    }
    const socketPath = path.join(socketRoot, 'control');
    let child: ReturnType<typeof spawn> | undefined;
    // Decode at the stream boundary so a multibyte UTF-8 character split
    // across OS chunks cannot be corrupted by per-buffer toString() calls.
    // Both output descriptors are configured as pipes when setup starts.
    let stdoutStream: Readable | undefined;
    let stderrStream: Readable | undefined;
    let stdout = '';
    let stderr = '';
    let outputBytes = 0;
    let failure: Error | undefined;
    let controlBytes = 0;
    let completion: { status: number | null; signal: NodeJS.Signals | null } | undefined;
    let cliGroup: number | undefined;
    const groups = new Set<number>();
    const connections = new Set<Socket>();
    let input: Socket | undefined;
    const server = createServer((socket) => {
      if (settled) {
        socket.destroy();
        return;
      }
      connections.add(socket);
      record('control-connected');
      socket.setEncoding('utf8');
      socket.on('error', () => undefined);
      let control = '';
      socket.on('data', (chunk: string) => {
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
                report.group === child?.pid
              )
                throw new Error('Invalid CLI process-group ownership.');
              cliGroup = report.group;
              groups.add(cliGroup);
              record('cli-group-adopted', { pid: cliGroup });
              input = socket;
              if (finishing) {
                stopGroup(cliGroup);
                socket.destroy();
              } else {
                // The bootstrap consumes exactly this acknowledgement, then maps
                // its socket to stdin. Input outlives the setup child's exit.
                socket.write('ready\n');
                record('ready-acknowledged');
                if (options.stdin !== undefined) {
                  if (options.closeStdin === false) socket.write(options.stdin);
                  else socket.end(options.stdin);
                } else socket.end();
              }
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
              record('launcher-error');
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
              record('cli-completion', completion);
              beginCleanup();
            }
          } catch (error) {
            failure ??= new Error('Invalid neutral-parent control.', { cause: error });
            beginCleanup();
          }
        }
      });
      socket.once('close', () => {
        connections.delete(socket);
        record('control-closed');
        if (control !== '') {
          failure ??= new Error('Incomplete neutral-parent control.');
          beginCleanup();
        }
        checkCompletion();
      });
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
        child?.unref();
        stdoutStream?.destroy();
        stderrStream?.destroy();
      }
      for (const socket of connections) socket.destroy();
      const complete = (transportError?: Error): void => {
        try {
          rmSync(socketRoot, { recursive: true, force: true });
        } catch (cause) {
          transportError ??= new Error('Could not remove CLI control socket.', { cause });
        }
        const cleanupFailure = error ?? transportError;
        record('cleanup-settled');
        diagnostic.cleanup = {
          closeObserved: closed !== undefined,
          remainingGroups: [...groups],
          failed: cleanupFailure !== undefined,
        };
        diagnostic.partialOutput = {
          stdout: {
            bytes: Buffer.byteLength(stdout),
            sha256: createHash('sha256').update(stdout).digest('hex'),
          },
          stderr: {
            bytes: Buffer.byteLength(stderr),
            sha256: createHash('sha256').update(stderr).digest('hex'),
          },
          observedBytes: outputBytes,
        };
        if (failure || cleanupFailure) reportDiagnostic('call', diagnostic);
        if (cleanupFailure) {
          cleanupFailed(cleanupFailure);
          reject(
            failure
              ? new AggregateError(
                  [failure, cleanupFailure],
                  `${failure.message} ${cleanupFailure.message}`
                )
              : cleanupFailure
          );
        } else {
          cleaned();
          if (failure) reject(failure);
          else resolve({ ...completion!, stdout, stderr });
        }
      };
      if (server.listening) server.close(complete);
      else complete();
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
      record('cleanup-start');
      input?.destroy();
      if (!child) closed = { status: null, signal: null };
      for (const group of groups) stopGroup(group);
      pollCleanup();
    };
    const timer = setTimeout(() => {
      failure ??= new Error(`CLI subprocess exceeded the ${deadlineMs} millisecond test bound.`);
      record('deadline');
      beginCleanup();
    }, deadlineMs);
    record('deadline-armed');
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
    const checkCompletion = (): void => {
      if (!closed || finishing || settled) return;
      // Socket records and stdio close are independent event streams. Wait for
      // both to drain before treating a missing completion as failure.
      if (completion === undefined && connections.size !== 0) return;
      if (completion === undefined)
        failure ??= new Error('Could not establish neutral-parent CLI completion.');
      beginCleanup();
    };
    server.once('error', (error) => {
      failure ??= error;
      beginCleanup();
    });
    server.once('listening', () => {
      record('socket-listening');
      if (finishing || settled) return;
      try {
        record('setup-spawn-requested');
        child = spawn(
          neutralParent,
          [
            'neutral-setup',
            socketPath,
            hasStdin ? 'input' : 'ignore',
            sandbox.cli.executable,
            ...sandbox.cli.args,
            ...args,
          ],
          {
            cwd: sandbox.cwd,
            env: sandbox.env,
            detached: true,
            stdio: ['ignore', 'pipe', 'pipe'],
            windowsHide: true,
          }
        );
      } catch (error) {
        failure = error as Error;
        beginCleanup();
        return;
      }
      if (child.pid !== undefined) groups.add(child.pid);
      record('setup-spawn-returned', { pid: child.pid });
      stdoutStream = child.stdout!;
      stderrStream = child.stderr!;
      stdoutStream.setEncoding('utf8');
      stderrStream.setEncoding('utf8');
      stdoutStream.on('data', readOutput('stdout'));
      stderrStream.on('data', readOutput('stderr'));
      stdoutStream.once('close', () => record('stdout-closed'));
      stderrStream.once('close', () => record('stderr-closed'));
      child.once('spawn', () => record('setup-spawn-observed', { pid: child?.pid }));
      child.on('error', (error) => {
        failure ??= error;
        beginCleanup();
      });
      // Successful setup exits to orphan the supervisor. Its completion record,
      // not setup exit, starts cleanup of the selected CLI and descendants.
      child.once('exit', (status, signal) => {
        record('setup-exit', { status, signal });
        if (status !== 0 || signal !== null) {
          failure ??= new Error('Neutral-parent setup did not exit successfully.');
          beginCleanup();
        }
      });
      child.once('close', (status, signal) => {
        closed = { status, signal };
        record('setup-pipes-closed', { status, signal });
        checkCompletion();
      });
    });
    record('socket-listen-requested');
    server.listen(socketPath);
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

function residentIdentity(pid: number, root: string): object {
  try {
    const identity = (): string[] => {
      const text = readFileSync(`/proc/${pid}/stat`, 'utf8');
      if (!text.startsWith(`${pid} (`)) throw new Error('stat PID differs');
      if (Buffer.byteLength(text) > 8192) throw new Error('stat exceeds diagnostic bound');
      const end = text.lastIndexOf(')');
      const fields = text
        .slice(end + 2)
        .trim()
        .split(/\s+/);
      if (
        end < 0 ||
        fields.length < 20 ||
        !/^[A-Za-z]$/.test(fields[0]) ||
        ![1, 2, 3, 19].every((index) => /^\d{1,20}$/.test(fields[index]))
      )
        throw new Error('invalid stat');
      return fields;
    };
    const before = identity();
    const cwd = readlinkSync(`/proc/${pid}/cwd`).replace(/ \(deleted\)$/, '');
    if (cwd !== root && !cwd.startsWith(`${root}${path.sep}`))
      return { pid, observation: 'no longer a sandbox resident' };
    const executable = readlinkSync(`/proc/${pid}/exe`);
    const after = identity();
    if (before[19] !== after[19]) return { pid, observation: 'identity changed' };
    return {
      pid,
      observation: 'endpoint identity observed',
      startTicks: after[19],
      state: after[0],
      ppid: after[1],
      pgid: after[2],
      session: after[3],
      cwd: diagnosticPath(cwd),
      executable: diagnosticPath(executable),
    };
  } catch {
    return { pid, observation: 'identity unavailable' };
  }
}

export async function withSandbox<T>(
  callback: (sandbox: Sandbox) => T | Promise<T>,
  executableEnv: NodeJS.ProcessEnv = process.env
): Promise<T> {
  const sandbox = createSandbox(executableEnv);
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
  let residents: object[] = [];
  let residentCount = 0;
  let residentCleanup = 'not required';
  try {
    const leaked = process.platform === 'linux' ? sandboxProcesses(processRoot) : [];
    if (leaked.length) {
      leak = new Error(`Sandbox callback left live processes: ${leaked.join(', ')}.`);
      residentCount = leaked.length;
      residents = leaked.slice(0, 8).map((pid) => residentIdentity(pid, processRoot));
      residentCleanup = 'not confirmed';
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
      residentCleanup = 'absence confirmed by existing cleanup checks';
    }
  } catch (error) {
    errors.push(error);
  }
  if (errors.length || leak)
    reportDiagnostic('sandbox', {
      sandboxRoot: diagnosticPath(processRoot),
      residents,
      residentCount,
      residentCleanup,
      cleanupFailed: errors.length !== 0,
      recentCalls: lifetime.diagnostics.map(
        ({
          callId,
          executable,
          executableSha256,
          fixtureArgv,
          fixtureArgc,
          argv,
          argc,
          deadlineMs,
          cleanup,
          events,
          omittedEvents,
        }) => ({
          callId,
          executable,
          executableSha256,
          fixtureArgv,
          fixtureArgc,
          argv,
          argc,
          deadlineMs,
          cleanup,
          omittedEvents,
          events: events.filter(({ phase }) =>
            [
              'socket-listening',
              'setup-spawn-observed',
              'cli-group-adopted',
              'ready-acknowledged',
              'setup-exit',
              'cli-completion',
              'setup-pipes-closed',
              'deadline',
              'cleanup-start',
              'cleanup-settled',
            ].includes(phase)
          ),
        })
      ),
    });
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
