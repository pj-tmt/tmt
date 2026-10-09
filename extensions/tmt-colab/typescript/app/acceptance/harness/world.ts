import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { alive, OwnedProcess, until } from './process.js';

const harness = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(harness, '../../../../../..');
/** True when a Unix socket path still accepts a connection. */
function listening(socket: string): Promise<boolean> {
  return new Promise((resolve) => {
    const client = net.connect(socket);
    client.once('connect', () => {
      client.destroy();
      resolve(true);
    });
    client.once('error', () => resolve(false));
  });
}
/** `work`, or a rejection when it has not finished within `ms`; the work itself is not cancelled. */
function bounded<T>(work: Promise<T> | T, ms: number, where = () => ''): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`did not finish within ${ms} ms${where()}`)),
      ms,
    );
    Promise.resolve(work).then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}
/** Cleanup that may name the stage it is in, so an overrun says where it stopped. */
export type Closer = (stage: (name: string) => void) => Promise<void>;
const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

/** The longest one registered closer (a browser, a page) may take during dispose(). */
const CLOSER_BOUND_MS = 10_000;

export interface Binaries {
  tmt: string;
  remote: string;
  colab: string;
}

/** Built binaries only; a missing one fails with the build command, never a skip. */
export function resolveBinaries(env = process.env): Binaries {
  const directory = env.TMT_ACCEPTANCE_BIN_DIR ?? path.join(repository, 'rust/target/debug');
  const binaries = {
    tmt: path.join(directory, 'tmt'),
    remote: path.join(directory, 'tmt-remote'),
    colab: path.join(directory, 'tmt-colab'),
  };
  for (const file of Object.values(binaries)) {
    if (!path.isAbsolute(file) || !fs.existsSync(file))
      throw new Error(
        `Missing ${file}. Build with: cd rust && cargo build --locked -p tmt-cli -p tmt-remote -p tmt-colab --bins (or set TMT_ACCEPTANCE_BIN_DIR to an absolute directory).`,
      );
  }
  return binaries;
}

export interface CliResult {
  code: number | null;
  stdout: string;
  stderr: string;
  json?: Record<string, unknown>;
}
export interface Agent {
  name: string;
  id: string;
  pane: string;
  /** Rows of the recipient's durable received-operation counter. */
  rows(): Array<Record<string, unknown>>;
  received(requestId?: string): Array<Record<string, unknown>>;
  gate: string;
}

/**
 * One isolated Colab acceptance world: a short root under /tmp (Unix socket paths are
 * limited to about 100 bytes), private HOME/XDG roots, a private tmux server
 * reached only through a `-L` wrapper, and the real tmt, tmt-remote and
 * tmt-colab binaries. Every child lives in its own process group and dispose()
 * proves each of them, the tmux server and the sockets are gone.
 */
export class AcceptanceWorld {
  readonly root = fs.mkdtempSync(path.join(fs.realpathSync('/tmp'), 'tmt-acc-'));
  readonly home = path.join(this.root, 'home');
  readonly dataRoot = path.join(this.root, 'state');
  readonly workspace = path.join(this.root, 'workspace');
  readonly barrierDirectory = path.join(this.root, 'barrier');
  readonly tmuxName = `acc-${path.basename(this.root).slice(-6)}`;
  readonly binaries: Binaries;
  private readonly wrapperDirectory = path.join(this.root, 'wrap');
  private readonly processes = new Set<OwnedProcess>();
  private readonly closers: Closer[] = [];
  private tmuxPath = '';
  private socketPath = '';
  private serverPid = 0;
  private sessionId = '';
  private hostPane = '';
  private disposed = false;
  private readonly closerBoundMs: number;

  constructor(binaries = resolveBinaries(), options: { closerBoundMs?: number } = {}) {
    this.binaries = binaries;
    this.closerBoundMs = options.closerBoundMs ?? CLOSER_BOUND_MS;
    for (const directory of [
      this.home,
      this.dataRoot,
      this.workspace,
      this.barrierDirectory,
      this.wrapperDirectory,
    ])
      fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
    for (const name of ['config', 'data', 'state', 'cache'])
      fs.mkdirSync(path.join(this.home, name), { mode: 0o700 });
  }

  /** Environment for every real child; a pane is added only for tmux callers. */
  env(pane?: string): NodeJS.ProcessEnv {
    return {
      PATH: [this.wrapperDirectory, path.dirname(process.execPath), '/usr/bin', '/bin'].join(
        path.delimiter,
      ),
      HOME: this.home,
      XDG_CONFIG_HOME: path.join(this.home, 'config'),
      XDG_DATA_HOME: path.join(this.home, 'data'),
      XDG_STATE_HOME: path.join(this.home, 'state'),
      XDG_CACHE_HOME: path.join(this.home, 'cache'),
      TMPDIR: this.root,
      LANG: 'C.UTF-8',
      TMT_HOME: this.dataRoot,
      TMUX_TMPDIR: this.root,
      TMT_EXECUTABLE: path.join(this.root, 'core'),
      TMT_ACCEPTANCE_REAL_TMT: this.binaries.tmt,
      TMT_ACCEPTANCE_BARRIER_DIR: this.barrierDirectory,
      ...(pane === undefined
        ? {}
        : {
            TMUX: `${this.socketPath},${this.serverPid},${this.sessionId}`,
            TMUX_PANE: pane,
          }),
    };
  }

  /** Start the private tmux server and the executables every child resolves. */
  async start(): Promise<void> {
    const pathDirs = (process.env.PATH ?? '').split(path.delimiter);
    this.tmuxPath =
      pathDirs.map((dir) => path.join(dir, 'tmux')).find((file) => fs.existsSync(file)) ?? '';
    if (!this.tmuxPath) throw new Error('tmux is required for the acceptance');
    fs.writeFileSync(
      path.join(this.wrapperDirectory, 'tmux'),
      `#!/bin/sh\nexec ${quote(this.tmuxPath)} -f /dev/null -L ${quote(this.tmuxName)} "$@"\n`,
      { mode: 0o700 },
    );
    fs.writeFileSync(
      path.join(this.root, 'core'),
      `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(path.join(harness, 'core-barrier.mjs'))} "$@"\n`,
      { mode: 0o700 },
    );
    // The host pane only keeps the private server alive and anchors callers.
    this.tmux([
      'new-session',
      '-d',
      '-s',
      'acceptance',
      '-x',
      '160',
      '-y',
      '50',
      '-c',
      this.workspace,
      `${quote(process.execPath)} -e ${quote('setInterval(() => {}, 1e6)')}`,
    ]);
    this.socketPath = this.tmux(['display-message', '-p', '#{socket_path}']).trim();
    this.serverPid = Number(this.tmux(['display-message', '-p', '#{pid}']).trim());
    // TMUX carries the session number without the `$` prefix of #{session_id}.
    this.sessionId = this.tmux(['display-message', '-p', '-t', 'acceptance', '#{session_id}'])
      .trim()
      .replace(/^\$/, '');
    this.hostPane = this.tmux([
      'display-message',
      '-p',
      '-t',
      'acceptance:0.0',
      '#{pane_id}',
    ]).trim();
    if (!this.socketPath || !this.serverPid || !this.hostPane)
      throw new Error('Could not identify the private tmux server');
  }

  /**
   * Put the built `tmt-remote` and `tmt-colab` on the world's PATH, which is how the real core
   * resolves `tmt remote ...` (the public CLI Colab uses to attach to or start the door).
   */
  linkExtensions(): void {
    for (const [name, target] of [
      ['tmt-remote', this.binaries.remote],
      ['tmt-colab', this.binaries.colab],
    ] as const) {
      const link = path.join(this.wrapperDirectory, name);
      if (!fs.existsSync(link)) fs.symlinkSync(target, link);
    }
  }

  /**
   * A test that times out is abandoned, not cancelled: its scenario keeps running while the
   * world is torn down. Once dispose() has begun, nothing may add a child or a closer that no
   * one will stop, so the abandoned scenario fails on its own instead of leaking.
   */
  private live(what: string): void {
    if (this.disposed) throw new Error(`The acceptance world is disposed; cannot ${what}.`);
  }

  tmux(args: string[]): string {
    this.live('run tmux');
    return this.tmuxUnguarded(args);
  }

  private tmuxUnguarded(args: string[]): string {
    return execFileSync(path.join(this.wrapperDirectory, 'tmux'), args, {
      env: this.env(),
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      timeout: 10_000,
      killSignal: 'SIGKILL',
    });
  }

  /** Run the real tmt as a caller in `pane` (default: the host pane). */
  tmt(args: string[], options: { pane?: string; stdin?: string; timeoutMs?: number } = {}) {
    return new Promise<CliResult>((resolve, reject) => {
      this.live('run tmt');
      const child = spawn(this.binaries.tmt, args, {
        cwd: this.workspace,
        env: this.env(options.pane ?? this.hostPane),
        detached: true,
        stdio: ['pipe', 'pipe', 'pipe'],
      });
      let stdout = '';
      let stderr = '';
      const timer = setTimeout(() => {
        try {
          process.kill(-(child.pid ?? 0), 'SIGKILL');
        } catch {
          // Already gone.
        }
        reject(new Error(`tmt ${args.join(' ')} timed out`));
      }, options.timeoutMs ?? 30_000);
      child.stdout.on('data', (chunk: Buffer) => (stdout += chunk));
      child.stderr.on('data', (chunk: Buffer) => (stderr += chunk));
      child.stdin.on('error', () => {});
      child.stdin.end(options.stdin ?? '');
      child.once('error', (error) => {
        clearTimeout(timer);
        reject(error);
      });
      child.once('close', (code) => {
        clearTimeout(timer);
        let json: Record<string, unknown> | undefined;
        try {
          json = JSON.parse(stdout) as Record<string, unknown>;
        } catch {
          // Human output: the caller asserts on stdout/stderr.
        }
        resolve({ code, stdout, stderr, json });
      });
    });
  }

  /** A recipient agent in its own pane, bound as a saved identity. */
  async startAgent(name: string, options: { gated?: boolean } = {}): Promise<Agent> {
    const log = path.join(this.root, `${name}.received.jsonl`);
    const gate = path.join(this.root, `${name}.gate`);
    fs.mkdirSync(gate, { mode: 0o700 });
    const pane = this.tmux([
      'new-window',
      '-d',
      '-P',
      '-F',
      '#{pane_id}',
      '-t',
      'acceptance:',
      '-n',
      name,
      '-c',
      this.workspace,
      [
        quote(process.execPath),
        quote(path.join(harness, 'recipient.mjs')),
        quote(this.binaries.tmt),
        quote(log),
        ...(options.gated ? [quote(gate)] : []),
      ].join(' '),
    ]).trim();
    const rows = () =>
      fs.existsSync(log)
        ? fs
            .readFileSync(log, 'utf8')
            .split('\n')
            .filter(Boolean)
            .map((line) => JSON.parse(line) as Record<string, unknown>)
        : [];
    await until(() => rows().some((row) => row.event === 'ready'), `${name} readiness`);
    const named = await this.tmt(['name', name, '--save', '--json'], { pane });
    if (named.code !== 0 || typeof named.json?.id !== 'string')
      throw new Error(`Could not name ${name}: ${named.stdout}${named.stderr}`);
    return {
      name,
      id: named.json.id,
      pane,
      gate,
      rows,
      received: (requestId) =>
        rows().filter(
          (row) => row.event === 'received' && (!requestId || row.requestId === requestId),
        ),
    };
  }

  /** Spawn a long-lived real child (serve, pair) tracked for dispose(). */
  spawn(label: string, executable: string, args: string[], pane?: string): OwnedProcess {
    this.live(`spawn ${label}`);
    const owned = new OwnedProcess(label, executable, args, {
      cwd: this.workspace,
      env: this.env(pane),
      stderrPath: path.join(this.root, `${label}.stderr`),
    });
    this.processes.add(owned);
    return owned;
  }

  /** Register cleanup that must run before the process checks (browsers, pages). */
  onDispose(close: Closer): void {
    this.live('register a closer');
    this.closers.push(close);
  }

  // Deterministic barrier on one dispatch.create (see core-barrier.mjs).
  /** Direct sends generate IDs on Enter, so a test may park the next dispatch instead. */
  armNextBarrier(phase: 'before' | 'after'): void {
    this.armBarrier(null, phase);
  }
  armBarrier(operationId: string | null, phase: 'before' | 'after'): void {
    for (const file of ['entered.json', 'release'])
      fs.rmSync(path.join(this.barrierDirectory, file), { force: true });
    fs.writeFileSync(
      path.join(this.barrierDirectory, 'barrier.json'),
      JSON.stringify({ operationId, phase }),
    );
  }
  async barrierEntered(): Promise<Record<string, unknown>> {
    const file = path.join(this.barrierDirectory, 'entered.json');
    await until(
      () => fs.existsSync(file) && fs.statSync(file).size > 0,
      'core barrier entry',
      20_000,
    );
    return JSON.parse(fs.readFileSync(file, 'utf8')) as Record<string, unknown>;
  }
  releaseBarrier(): void {
    fs.writeFileSync(path.join(this.barrierDirectory, 'release'), '');
    fs.rmSync(path.join(this.barrierDirectory, 'barrier.json'), { force: true });
  }
  /** Every real core launch by Remote/Colab children, in order. */
  coreCalls(): Array<{ operation: string | null; operationId: string | null }> {
    const file = path.join(this.barrierDirectory, 'calls.jsonl');
    return fs.existsSync(file)
      ? fs
          .readFileSync(file, 'utf8')
          .split('\n')
          .filter(Boolean)
          .map((line) => JSON.parse(line))
      : [];
  }

  /**
   * Stop everything, then prove the world left nothing behind. Cleanup always
   * runs fully; the leak report is thrown afterwards so an injected assertion
   * failure in the scenario still exercises the same checks.
   */
  async dispose(): Promise<string[]> {
    if (this.disposed) return [];
    this.disposed = true;
    const leaks: string[] = [];
    const attempt = async (what: string, action: () => Promise<void> | void) => {
      try {
        await action();
      } catch (error) {
        leaks.push(`${what}: ${error instanceof Error ? error.message : String(error)}`);
      }
    };
    for (const close of this.closers.reverse()) {
      let stage = '';
      await attempt('closer', () =>
        bounded(
          close((name) => (stage = name)),
          this.closerBoundMs,
          () => (stage ? ` (in ${stage})` : ''),
        ),
      );
    }
    for (const owned of this.processes) await attempt(owned.label, () => owned.stop());
    if (this.serverPid) {
      await attempt('tmux server', async () => {
        try {
          this.tmuxUnguarded(['kill-server']);
        } catch {
          // The server may already be gone; absence is checked below.
        }
        await until(() => !alive(this.serverPid), 'private tmux server exit', 5_000);
        // tmux leaves its socket file behind; what matters is that nothing listens on it.
        if (await listening(this.socketPath)) throw new Error('private tmux socket still accepts');
      });
    }
    for (const survivor of this.survivors()) leaks.push(`process remains: ${survivor}`);
    for (const socket of this.sockets()) leaks.push(`socket remains: ${socket}`);
    // TMT_ACCEPTANCE_KEEP=1 keeps the root (logs, stderr, counters) for diagnosis.
    if (process.env.TMT_ACCEPTANCE_KEEP !== '1')
      fs.rmSync(this.root, { recursive: true, force: true });
    return leaks;
  }

  /** Processes whose command line still names this world's root or tmux socket. */
  survivors(): string[] {
    const listing = execFileSync('/bin/ps', ['-axo', 'pid=,command='], { encoding: 'utf8' });
    return listing
      .split('\n')
      .filter(
        (line) =>
          (line.includes(this.root) || line.includes(` -L ${this.tmuxName}`)) &&
          !line.includes('/bin/ps -axo'),
      )
      .map((line) => line.trim());
  }

  /** Unix sockets left under the root. */
  sockets(): string[] {
    const found: string[] = [];
    const walk = (directory: string) => {
      if (!fs.existsSync(directory)) return;
      for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const file = path.join(directory, entry.name);
        if (entry.isSocket() && file !== this.socketPath) found.push(file);
        else if (entry.isDirectory()) walk(file);
      }
    };
    walk(this.root);
    return found;
  }
}
