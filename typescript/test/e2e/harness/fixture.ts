import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { TMUX_COMMAND_INSPECTION } from '../tmux-command-inspection.js';
import { resolveCliExecutables, type CliExecutables } from '../../support/cli-executable.mjs';

import type {
  CliResult,
  MockEvent,
  MockPane,
  CliRunOptions,
  CliProcess,
  MetadataBarrierOptions,
  E2EFixtureOptions,
} from './types.js';
import {
  waitFor,
  waitForEvent,
  waitForProcessExit,
  waitForCapture,
  readMockEvents,
  readPaneTarget,
  waitForMetadataBarrier,
} from './readiness.js';
import {
  killAndWait,
  processGroupIsRunning,
  requestObserverPids,
  replyWorkerPids,
  stopAttachedClients,
  processIsRunning,
  serverIsRunning,
  waitForCliResults,
  activeReplyPids,
} from './cleanup.js';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const mockAgentPath = path.join(repoRoot, 'test', 'e2e', 'mock-agent.mjs');
// Generous for one tmux client call on loaded CI; synchronous calls must leave
// the event loop available for the scenario timeout instead of hanging forever.
const TMUX_COMMAND_TIMEOUT_MS = 5_000;

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

export class E2EFixture {
  readonly executables: CliExecutables;
  readonly root: string;
  readonly socketRoot: string;
  readonly workspace: string;
  readonly globalDir: string;
  readonly logPath: string;
  readonly transportTracePath: string;
  readonly forbiddenTmuxLogPath: string;
  readonly metadataBarrierDirectory: string;
  readonly replyGateDirectory: string;
  readonly socket = `tmt-e2e-${process.pid}-${Math.random().toString(16).slice(2)}`;
  readonly wrapperDir: string;
  readonly tmuxPath: string;
  pane = '';
  panePid = 0;
  serverPid = 0;
  socketPath = '';
  private started = false;
  private serverStarted = false;
  private env: NodeJS.ProcessEnv = {};
  private panePids: number[] = [];
  private attachedClients: ReturnType<typeof spawn>[] = [];
  private cliProcessPids = new Set<number>();
  private cliProcessResults = new Map<number, Promise<CliResult<unknown>>>();

  constructor(options: Pick<E2EFixtureOptions, 'globalDir' | 'executableEnv'> = {}) {
    this.executables = resolveCliExecutables(options.executableEnv);
    this.root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'tmux-team-e2e-')));
    try {
      this.socketRoot = fs.realpathSync(
        fs.mkdtempSync(
          path.join(process.platform === 'darwin' ? '/private/tmp' : os.tmpdir(), 'te2e-')
        )
      );
    } catch (error) {
      fs.rmSync(this.root, { recursive: true, force: true });
      throw error;
    }
    this.workspace = path.join(this.root, 'workspace');
    this.logPath = path.join(this.root, 'mock-agent.jsonl');
    this.transportTracePath = path.join(this.root, 'transport-trace.log');
    this.forbiddenTmuxLogPath = path.join(this.root, 'forbidden-tmux.log');
    this.metadataBarrierDirectory = path.join(this.root, 'metadata-barrier');
    this.replyGateDirectory = path.join(this.root, 'reply-gate');
    this.wrapperDir = path.join(this.root, 'bin');
    this.globalDir = options.globalDir ?? path.join(this.root, 'global');
    try {
      this.tmuxPath = execFileSync('/bin/sh', ['-lc', 'command -v tmux'], {
        encoding: 'utf8',
      }).trim();
      if (!this.tmuxPath) throw new Error('tmux was not found on PATH.');
    } catch (error) {
      fs.rmSync(this.root, { recursive: true, force: true });
      fs.rmSync(this.socketRoot, { recursive: true, force: true });
      throw new Error(
        `E2E fixture requires tmux: ${error instanceof Error ? error.message : String(error)}`,
        { cause: error }
      );
    }
  }

  async start(options: E2EFixtureOptions = {}): Promise<void> {
    try {
      fs.mkdirSync(this.workspace, { recursive: true });
      fs.mkdirSync(this.globalDir, { recursive: true });
      fs.mkdirSync(this.wrapperDir, { recursive: true });
      if (options.metadataBarrier) fs.mkdirSync(this.metadataBarrierDirectory);
      fs.writeFileSync(
        path.join(this.wrapperDir, 'tmux'),
        `#!/bin/sh
if [ "${'$'}{TMT_E2E_FORBID_TMUX:-}" = "1" ]; then
  echo "unexpected tmux invocation" >> "${'$'}TMT_E2E_FORBIDDEN_TMUX_LOG"
  exit 97
fi
if [ -n "${'$'}{TMT_E2E_PROGRESS_FILE:-}" ]; then
  : > "${'$'}TMT_E2E_PROGRESS_FILE"
fi
metadata_write=0
metadata_clear=0
# Keep explicit-socket native calls visible to the same barriers and fault injection.
# Inspect the command without changing argv passed to the real tmux binary.
${TMUX_COMMAND_INSPECTION}
if [ "${'$'}tmux_command" = "capture-pane" ]; then
  case "${'$'}{TMT_E2E_CAPTURE_FAULT:-}" in
    exit) exit 97 ;;
    overflow) exec head -c 4194305 /dev/zero ;;
    timeout) exec sleep 5 ;;
  esac
fi
if [ "${'$'}tmux_command" = "set-option" ]; then
  unset_metadata=0
  pane_metadata=0
  for arg in "${'$'}@"; do
    if [ "${'$'}arg" = "-u" ]; then unset_metadata=1; fi
    if [ "${'$'}arg" = "-p" ]; then pane_metadata=1; fi
    if [ "${'$'}arg" = "@tmux-team.agent" ]; then metadata_write=1; fi
  done
  if [ "${'$'}unset_metadata" = "1" ] && [ "${'$'}pane_metadata" = "1" ] && [ "${'$'}metadata_write" = "1" ]; then
    metadata_clear=1
  fi
  if [ "${'$'}unset_metadata" = "1" ] || [ "${'$'}pane_metadata" = "0" ]; then
    metadata_write=0
  fi
fi
if [ -z "${'$'}{TMT_E2E_PROGRESS_FILE:-}" ] && [ -z "${'$'}{TMT_E2E_METADATA_BARRIER_DIR:-}" ] && [ -z "${'$'}{TMT_E2E_TRANSPORT_TRACE_FILE:-}" ]; then
  exec ${shellQuote(this.tmuxPath)} -f /dev/null -L "${'$'}TMT_E2E_SOCKET" "${'$'}@"
fi
metadata_target=0
if [ "${'$'}{TMT_E2E_METADATA_BARRIER_OPERATION:-publish}" = "clear" ]; then
  metadata_target=${'$'}metadata_clear
else
  metadata_target=${'$'}metadata_write
fi
if [ "${'$'}metadata_target" = "1" ] && [ -n "${'$'}{TMT_E2E_METADATA_BARRIER_DIR:-}" ]; then
  : > "${'$'}TMT_E2E_METADATA_BARRIER_DIR/entered"
  if [ "${'$'}{TMT_E2E_METADATA_BARRIER_PHASE:-}" = "before" ]; then
    barrier_wait=0
    while [ ! -e "${'$'}TMT_E2E_METADATA_BARRIER_DIR/release" ] && [ "${'$'}barrier_wait" -lt 200 ]; do
      sleep 0.01
      barrier_wait=${'$'}((barrier_wait + 1))
    done
    [ -e "${'$'}TMT_E2E_METADATA_BARRIER_DIR/release" ] || exit 124
  fi
fi
trace_transport() {
  if [ -z "${'$'}{TMT_E2E_TRANSPORT_TRACE_FILE:-}" ]; then return; fi
  trace_stage="${'$'}1"
  shift
  trace_argv=""
  for trace_arg in "${'$'}@"; do
    trace_arg=$(printf '%s' "${'$'}trace_arg" | tr '\n' ' ')
    trace_argv="${'$'}trace_argv [${'$'}trace_arg]"
  done
  printf '%s|%s\n' "${'$'}trace_stage" "${'$'}trace_argv" >> "${'$'}TMT_E2E_TRANSPORT_TRACE_FILE"
}
transport_stage=""
case "${'$'}tmux_command" in
  set-buffer) transport_stage="set-buffer" ;;
  paste-buffer) transport_stage="paste-buffer" ;;
  send-keys)
    if [ "${'$'}tmux_command_arg_count" -eq 3 ] && [ "${'$'}tmux_last_arg" = "Enter" ]; then
      transport_stage="submit"
    else
      transport_stage="literal-input"
    fi
    ;;
esac
if [ "${'$'}transport_stage" = "set-buffer" ] && [ "${'$'}{TMT_E2E_TRANSPORT_FAULT_STAGE:-}" = "set-buffer" ]; then
  trace_transport "set-buffer.fault-before" "${'$'}@"
  exit 97
fi
if [ -n "${'$'}transport_stage" ]; then
  trace_transport "${'$'}transport_stage.before" "${'$'}@"
fi
${shellQuote(this.tmuxPath)} -f /dev/null -L "${'$'}TMT_E2E_SOCKET" "${'$'}@"
status=${'$'}?
if [ -n "${'$'}transport_stage" ]; then
  trace_transport "${'$'}transport_stage.after.${'$'}status" "${'$'}@"
fi
fault_after=0
if [ "${'$'}status" -eq 0 ]; then
  if [ "${'$'}transport_stage" = "paste-buffer" ] && [ "${'$'}{TMT_E2E_TRANSPORT_FAULT_STAGE:-}" = "paste" ]; then
    fault_after=1
  fi
  if [ "${'$'}transport_stage" = "submit" ] && [ "${'$'}{TMT_E2E_TRANSPORT_FAULT_STAGE:-}" = "submit" ]; then
    fault_after=1
  fi
fi
if [ "${'$'}fault_after" -eq 1 ]; then
  trace_transport "${'$'}transport_stage.fault-after" "${'$'}@"
  exit 98
fi
if [ "${'$'}metadata_target" = "1" ] && [ -n "${'$'}{TMT_E2E_METADATA_BARRIER_DIR:-}" ] && [ "${'$'}{TMT_E2E_METADATA_BARRIER_PHASE:-}" = "after" ]; then
  : > "${'$'}TMT_E2E_METADATA_BARRIER_DIR/applied"
  barrier_wait=0
  while [ ! -e "${'$'}TMT_E2E_METADATA_BARRIER_DIR/release" ] && [ "${'$'}barrier_wait" -lt 200 ]; do
    sleep 0.01
    barrier_wait=${'$'}((barrier_wait + 1))
  done
  [ -e "${'$'}TMT_E2E_METADATA_BARRIER_DIR/release" ] || exit 124
fi
exit ${'$'}status
`
      );
      fs.chmodSync(path.join(this.wrapperDir, 'tmux'), 0o755);

      this.env = {
        ...process.env,
        PATH: `${this.wrapperDir}${path.delimiter}${process.env.PATH ?? ''}`,
        TMT_E2E_SOCKET: this.socket,
        TMUX_TMPDIR: this.socketRoot,
        TMUX_TEAM_HOME: this.globalDir,
        TMT_MOCK_MODE: options.mode ?? 'respond',
        TMT_MOCK_DELAY_MS: String(options.delayMs ?? 0),
        TMT_MOCK_LOG: this.logPath,
        TMT_TEST_CLI: JSON.stringify(this.executables.cli),
        TMT_TEST_PEER_CLI: JSON.stringify(this.executables.peer),
      };
      if (options.responseBodyBase64 !== undefined)
        this.env.TMT_MOCK_RESPONSE_BODY_BASE64 = options.responseBodyBase64;
      if (options.responseBytes !== undefined)
        this.env.TMT_MOCK_RESPONSE_BYTES = String(options.responseBytes);
      if (options.responseMultibyte) this.env.TMT_MOCK_RESPONSE_MULTIBYTE = '1';
      if (options.replyInput !== undefined) this.env.TMT_MOCK_REPLY_INPUT = options.replyInput;
      if (options.replyDelayMs !== undefined)
        this.env.TMT_MOCK_REPLY_DELAY_MS = String(options.replyDelayMs);
      if (options.replyGate) {
        fs.mkdirSync(this.replyGateDirectory);
        this.env.TMT_MOCK_REPLY_GATE = this.replyGateDirectory;
      }
      if (options.replyFailure) this.env.TMT_MOCK_REPLY_FAILURE = '1';
      if (options.holdReplyEof) this.env.TMT_MOCK_HOLD_REPLY_EOF = '1';
      if (options.replyRetry) this.env.TMT_MOCK_REPLY_RETRY = '1';
      if (options.replyConflict) this.env.TMT_MOCK_REPLY_CONFLICT = '1';
      if (options.replyAckLoss) this.env.TMT_MOCK_REPLY_ACK_LOSS = '1';
      if (options.summaryFailure) this.env.TMT_MOCK_SUMMARY_FAILURE = '1';
      if (options.metadataBarrier) {
        this.enableMetadataBarrier(options.metadataBarrier);
      }
      delete this.env.TMUX;
      delete this.env.TMUX_PANE;

      this.tmux(['-V']);
      await this.launchPrivateServer();
      this.started = true;
    } catch (error) {
      await this.stop();
      throw new Error(
        `E2E fixture failed to start its private tmux server: ${error instanceof Error ? error.message : String(error)}`,
        { cause: error }
      );
    }
  }

  runCli<T = Record<string, unknown>>(
    args: string[],
    options: CliRunOptions = {}
  ): Promise<CliResult<T>> {
    return this.runCliProcess<T>(args, options).result;
  }

  runCliProcess<T = Record<string, unknown>>(
    args: string[],
    options: CliRunOptions = {}
  ): CliProcess<T> {
    if (!this.started) throw new Error('E2E fixture must be started before invoking the CLI.');
    const env: NodeJS.ProcessEnv = { ...this.env };
    if (!options.outsideTmux && !options.withoutTmux) {
      const callerPane = options.pane ?? this.pane;
      env.TMUX = `${this.socketPath},${this.serverPid},${this.paneSessionId(callerPane)}`;
      env.TMUX_PANE = callerPane;
      if (options.caller) {
        if (options.caller.tmux === null) delete env.TMUX;
        else if (options.caller.tmux !== undefined) env.TMUX = options.caller.tmux;
        if (options.caller.pane === null) delete env.TMUX_PANE;
        else if (options.caller.pane !== undefined) env.TMUX_PANE = options.caller.pane;
      }
    } else {
      delete env.TMUX;
      delete env.TMUX_PANE;
    }
    if (options.withoutTmux) {
      delete env.TMUX;
      delete env.TMUX_PANE;
      env.TMT_E2E_FORBID_TMUX = '1';
      env.TMT_E2E_FORBIDDEN_TMUX_LOG = this.forbiddenTmuxLogPath;
    }
    if (options.progressFile) env.TMT_E2E_PROGRESS_FILE = options.progressFile;
    if (options.captureFault) env.TMT_E2E_CAPTURE_FAULT = options.captureFault;
    if (options.transportFault) {
      env.TMT_E2E_TRANSPORT_FAULT_STAGE = options.transportFault.stage;
    }
    if (options.transportTrace || options.transportFault) {
      env.TMT_E2E_TRANSPORT_TRACE_FILE = this.transportTracePath;
    }
    const child = spawn(this.executables.cli.executable, [...this.executables.cli.args, ...args], {
      cwd: options.cwd ?? this.workspace,
      env,
      detached: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    if (child.pid) this.cliProcessPids.add(child.pid);
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk: string) => (stdout += chunk));
    child.stderr.on('data', (chunk: string) => (stderr += chunk));
    const result = new Promise<CliResult<T>>((resolve) => {
      let settled = false;
      child.once('error', (error) => {
        if (settled) return;
        settled = true;
        stderr += `${error.message}\n`;
        resolve({ code: 1, stdout, stderr });
      });
      child.on('close', (code) => {
        if (settled) return;
        settled = true;
        let json: T | undefined;
        try {
          json = JSON.parse(stdout) as T;
        } catch {
          // The caller receives stdout/stderr for a useful assertion failure.
        }
        resolve({ code: code ?? 1, stdout, stderr, json });
      });
    });
    if (child.pid) this.cliProcessResults.set(child.pid, result as Promise<CliResult<unknown>>);
    void result.then(
      () => {
        if (child.pid) {
          try {
            if (!processGroupIsRunning(child.pid)) this.cliProcessPids.delete(child.pid);
          } catch {
            // Leave the group tracked so fixture cleanup reports the failure.
          }
          this.cliProcessResults.delete(child.pid);
        }
      },
      () => {
        if (child.pid) {
          try {
            if (!processGroupIsRunning(child.pid)) this.cliProcessPids.delete(child.pid);
          } catch {
            // Leave the group tracked so fixture cleanup reports the failure.
          }
          this.cliProcessResults.delete(child.pid);
        }
      }
    );
    return {
      pid: child.pid ?? 0,
      result,
      kill(signal = 'SIGTERM') {
        if (!child.pid) return;
        try {
          process.kill(-child.pid, signal);
        } catch (error) {
          if (!(error instanceof Error) || !('code' in error) || error.code !== 'ESRCH') {
            throw error;
          }
        }
      },
    };
  }

  runJsonCli<T = Record<string, unknown>>(
    args: string[],
    options: CliRunOptions = {}
  ): Promise<CliResult<T>> {
    return this.runCli<T>(['--json', ...args], options);
  }

  createWorkspace(name: string): string {
    const workspace = path.join(this.root, name);
    fs.mkdirSync(workspace, { recursive: true });
    return workspace;
  }

  async createMockPane(name: string, workspace = this.workspace): Promise<MockPane> {
    const created = this.createPane(
      name,
      `${shellQuote(process.execPath)} ${shellQuote(mockAgentPath)}`,
      workspace
    );
    await this.waitForEvent((event) => event.event === 'ready' && event.pid === created.pid);
    return created;
  }

  /** A real interactive shell for foreground job-control scenarios. */
  createShellPane(name: string, workspace = this.workspace): MockPane {
    return this.createPane(name, '/bin/bash --noprofile --norc -i', workspace);
  }

  private createPane(name: string, command: string, workspace: string): MockPane {
    if (!this.started) throw new Error('E2E fixture must be started before creating panes.');
    fs.mkdirSync(workspace, { recursive: true });
    const pane = this.tmux([
      'new-window',
      '-d',
      '-P',
      '-F',
      '#{pane_id}',
      '-t',
      'e2e',
      '-n',
      name,
      '-c',
      workspace,
      command,
    ]).trim();
    const pid = Number(this.tmux(['display-message', '-p', '-t', pane, '#{pane_pid}']).trim());
    if (!pane || !Number.isInteger(pid) || pid <= 0) {
      throw new Error(`E2E fixture could not create pane '${name}'.`);
    }
    this.panePids.push(pid);
    return { pane, pid, workspace };
  }

  /** Attach a real control-mode tmux client to the disposable fixture server. */
  async attachSessionClient(session: string): Promise<void> {
    if (!this.started) throw new Error('E2E fixture must be started before attaching a client.');
    const client = spawn(
      this.tmuxPath,
      ['-f', '/dev/null', '-L', this.socket, '-C', 'attach-session', '-t', session],
      { env: this.env, stdio: ['pipe', 'pipe', 'pipe'] }
    );
    let spawnError: Error | undefined;
    client.once('error', (error) => {
      spawnError = error;
    });
    client.stdout?.resume();
    client.stderr?.resume();
    this.attachedClients.push(client);
    await this.waitFor(
      () => {
        if (spawnError) {
          throw new Error(`Could not start tmux client for '${session}'.`, { cause: spawnError });
        }
        return (
          client.exitCode === null &&
          client.signalCode === null &&
          client.pid !== undefined &&
          this.tmux(['list-clients', '-t', session, '-F', '#{client_pid}'])
            .trim()
            .split('\n')
            .includes(String(client.pid))
        );
      },
      2_000,
      `tmux client attached to '${session}'`
    );
  }

  /**
   * Restart the fixture's private tmux server while retaining the fixture
   * environment and global directory. Pane user-options belong to a server,
   * so a fresh session is the authoritative persistence boundary for global
   * identities.
   */
  async restartServer(): Promise<MockPane> {
    if (!this.started) throw new Error('E2E fixture must be started before restarting its server.');

    const previousServerPid = this.serverPid;
    try {
      this.tmux(['kill-server']);
    } catch {
      // The server may have exited between the test operation and restart.
    }
    this.serverStarted = false;

    await this.waitFor(
      () => !processIsRunning(previousServerPid),
      2_000,
      'private tmux server to exit before restart'
    );

    return this.launchPrivateServer();
  }

  private async launchPrivateServer(): Promise<MockPane> {
    this.tmux([
      'new-session',
      '-d',
      '-s',
      'e2e',
      '-x',
      '160',
      '-y',
      '50',
      '-c',
      this.workspace,
      `${shellQuote(process.execPath)} ${shellQuote(mockAgentPath)}`,
    ]);
    this.serverStarted = true;
    this.socketPath = this.tmux(['display-message', '-p', '#{socket_path}']).trim();
    this.serverPid = Number(this.tmux(['display-message', '-p', '#{pid}']).trim());
    this.pane = this.tmux(['display-message', '-p', '-t', 'e2e:0.0', '#{pane_id}']).trim();
    this.panePid = Number(
      this.tmux(['display-message', '-p', '-t', this.pane, '#{pane_pid}']).trim()
    );
    if (!this.socketPath || !this.pane) {
      throw new Error('E2E fixture could not determine private server socket and pane ID.');
    }
    if (!Number.isInteger(this.panePid) || this.panePid <= 0) {
      throw new Error('E2E fixture could not determine mock-agent pane process ID.');
    }
    if (!Number.isInteger(this.serverPid) || this.serverPid <= 0) {
      throw new Error('E2E fixture could not determine private tmux server process ID.');
    }
    await this.waitForEvent((event) => event.event === 'ready' && event.pid === this.panePid);
    this.panePids.push(this.panePid);
    return { pane: this.pane, pid: this.panePid, workspace: this.workspace };
  }

  tmux(args: string[]): string {
    return execFileSync(path.join(this.wrapperDir, 'tmux'), args, {
      env: this.env,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      timeout: TMUX_COMMAND_TIMEOUT_MS,
      killSignal: 'SIGKILL',
    });
  }

  paneTarget(pane = this.pane): string {
    return readPaneTarget((args) => this.tmux(args), pane);
  }

  paneSessionId(pane = this.pane): string {
    return this.tmux(['display-message', '-p', '-t', pane, '#{session_id}'])
      .trim()
      .replace(/^\$/, '');
  }

  paneMetadata(pane = this.pane): string {
    try {
      return this.tmux(['show-options', '-p', '-t', pane, '-v', '@tmux-team.agent']).trim();
    } catch {
      return '';
    }
  }

  paneTitle(pane = this.pane): string {
    return this.tmux(['display-message', '-p', '-t', pane, '#{pane_title}']).trim();
  }

  transportTrace(): string[] {
    if (!fs.existsSync(this.transportTracePath)) return [];
    return fs.readFileSync(this.transportTracePath, 'utf8').split('\n').filter(Boolean);
  }

  events(): MockEvent[] {
    return readMockEvents(this.logPath);
  }

  async waitForEvent(
    predicate: (event: MockEvent) => boolean,
    timeoutMs = 2_000
  ): Promise<MockEvent> {
    return waitForEvent(() => this.events(), predicate, timeoutMs);
  }

  async waitFor(
    predicate: () => boolean,
    timeoutMs = 2_000,
    description = 'condition'
  ): Promise<void> {
    return waitFor(predicate, timeoutMs, description);
  }

  async waitForMetadataBarrier(
    signal: 'entered' | 'applied' = 'entered',
    options: { child?: CliProcess<unknown>; timeoutMs?: number } = {}
  ): Promise<void> {
    return waitForMetadataBarrier(this.metadataBarrierDirectory, signal, options);
  }

  releaseMetadataBarrier(): void {
    if (!fs.existsSync(this.metadataBarrierDirectory)) {
      throw new Error('Metadata barrier is not enabled for this fixture.');
    }
    fs.writeFileSync(path.join(this.metadataBarrierDirectory, 'release'), 'release');
  }

  releaseReplyGate(requestId?: string): void {
    if (!fs.existsSync(this.replyGateDirectory)) {
      throw new Error('Reply gate is not enabled for this fixture.');
    }
    if (requestId !== undefined && !/^req_[0-9a-f-]+$/.test(requestId)) {
      throw new Error(`Invalid request ID for reply gate: ${requestId}`);
    }
    const filename = requestId ? `${requestId}.release` : 'release';
    fs.writeFileSync(path.join(this.replyGateDirectory, filename), 'release');
  }

  enableMetadataBarrier(options: MetadataBarrierOptions): void {
    fs.mkdirSync(this.metadataBarrierDirectory, { recursive: true });
    for (const signal of ['entered', 'applied', 'release']) {
      fs.rmSync(path.join(this.metadataBarrierDirectory, signal), { force: true });
    }
    this.env.TMT_E2E_METADATA_BARRIER_DIR = this.metadataBarrierDirectory;
    this.env.TMT_E2E_METADATA_BARRIER_PHASE = options.phase;
    this.env.TMT_E2E_METADATA_BARRIER_OPERATION = options.operation ?? 'publish';
  }

  async stop(): Promise<void> {
    const cliPids = [...this.cliProcessPids];
    let cleanupError: Error | undefined;
    const recordCleanupError = (error: Error): void => {
      cleanupError ??= error;
    };
    const survivors = await killAndWait(cliPids, 'CLI', recordCleanupError);
    if (survivors.length > 0) {
      cleanupError = new Error(`E2E CLI process groups survived cleanup: ${survivors.join(', ')}`);
    }
    this.cliProcessPids.clear();
    await waitForCliResults(this.cliProcessResults.values());
    if (this.cliProcessResults.size > 0) {
      cleanupError = new Error(
        `E2E CLI processes did not report termination: ${[...this.cliProcessResults.keys()].join(', ')}`
      );
    }
    this.cliProcessResults.clear();
    // Detached request observers intentionally outlive talk. Only this fixture's
    // recorded request IDs and matching live argv may be stopped here.
    const observers = path.join(this.globalDir, 'request-observers');
    if (fs.existsSync(observers)) {
      const owned = requestObserverPids(observers);
      const survivors = await killAndWait(owned, 'request observer', recordCleanupError);
      if (survivors.length > 0)
        cleanupError ??= new Error(`Request observers survived cleanup: ${survivors.join(', ')}`);
    }
    const replyWorkers = path.join(this.globalDir, 'reply-notice-workers');
    if (fs.existsSync(replyWorkers)) {
      const owned = replyWorkerPids(replyWorkers);
      const survivors = await killAndWait(owned, 'reply notice worker', recordCleanupError);
      if (survivors.length > 0)
        cleanupError ??= new Error(
          `Reply notice workers survived cleanup: ${survivors.join(', ')}`
        );
    }
    const attachedClients = this.attachedClients.splice(0);
    await stopAttachedClients(attachedClients, recordCleanupError);
    if (this.serverStarted) {
      try {
        this.tmux(['kill-server']);
      } catch {
        // The server may already have exited; cleanup remains best effort.
      }
    }
    this.serverStarted = false;
    this.started = false;
    await waitForProcessExit(this.panePids, (pid) => this.mockProcessIsRunning(pid));

    const replyPids = activeReplyPids(this.events());
    const replySurvivors = await killAndWait(replyPids, 'reply', recordCleanupError);
    if (replySurvivors.length > 0) {
      cleanupError ??= new Error(
        `E2E reply process groups survived cleanup: ${replySurvivors.join(', ')}`
      );
    }
    fs.rmSync(this.root, { recursive: true, force: true });
    fs.rmSync(this.socketRoot, { recursive: true, force: true });
    if (cleanupError) throw cleanupError;
  }

  serverIsRunning(): boolean {
    return serverIsRunning(this.socketPath, this.tmuxPath, () => this.serverProcessIsRunning());
  }

  serverProcessIsRunning(): boolean {
    return processIsRunning(this.serverPid);
  }

  mockProcessIsRunning(pid = this.panePid): boolean {
    return processIsRunning(pid);
  }

  sendMockInput(lines: string[], pane = this.pane): void {
    for (const line of lines) {
      execFileSync(
        path.join(this.wrapperDir, 'tmux'),
        ['send-keys', '-t', pane, '--', line, 'Enter'],
        {
          env: this.env,
          stdio: 'ignore',
        }
      );
    }
  }

  capture(lines = 100, pane = this.pane): string {
    return this.tmux(['capture-pane', '-p', '-t', pane, '-S', `-${lines}`]);
  }

  async waitForCapture(predicate: (output: string) => boolean, pane = this.pane): Promise<string> {
    return waitForCapture(() => this.capture(100, pane), predicate);
  }
}

export async function withE2EFixture<T>(
  callback: (fixture: E2EFixture) => Promise<T> | T,
  options: E2EFixtureOptions = {}
): Promise<T> {
  const fixture = new E2EFixture(options);
  try {
    await fixture.start(options);
    return await callback(fixture);
  } finally {
    await fixture.stop();
  }
}
