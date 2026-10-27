import { randomUUID } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import os from 'node:os';
import { assertPrivateSocket } from '../../support/private-tmux.js';
import { processState } from './cleanup.js';
import fs from 'node:fs';
import path from 'node:path';

/** Durable ownership record for one fixture in this Vitest run. */
export interface OwnedFixture {
  root: string;
  socketRoot: string;
  socketPath: string;
  tmuxPath: string;
  serverPid: number;
  paneGroups: OwnedGroup[];
  cliGroups: OwnedGroup[];
}

const registryEnv = 'TMT_E2E_RUN_REGISTRY';
const pause = new Int32Array(new SharedArrayBuffer(4));

/** Serialize registration and teardown across Vitest's worker threads. */
function locked<T>(directory: string, operation: () => T): T {
  const lock = path.join(directory, '.lock');
  const deadline = Date.now() + 7_000;
  for (;;) {
    try {
      fs.mkdirSync(lock, { mode: 0o700 });
      break;
    } catch (error) {
      if (!(error instanceof Error && 'code' in error && error.code === 'EEXIST')) throw error;
      if (Date.now() >= deadline) throw new Error(`E2E registry lock did not clear: ${lock}`);
      Atomics.wait(pause, 0, 0, 20);
    }
  }
  try {
    return operation();
  } finally {
    fs.rmdirSync(lock);
  }
}

export class FixtureRegistration {
  private readonly file: string;

  constructor(
    private readonly directory: string,
    readonly fixture: OwnedFixture
  ) {
    this.file = path.join(directory, `${randomUUID()}.json`);
    locked(directory, () => {
      if (fs.existsSync(path.join(directory, '.closing'))) {
        throw new Error(`E2E run is closing: ${directory}`);
      }
      this.write();
    });
  }

  /** Hold the lock across allocation and publication so signal cleanup cannot miss it. */
  allocate<T>(allocate: () => T, remember: (value: T) => void): T {
    return locked(this.directory, () => {
      if (fs.existsSync(path.join(this.directory, '.closing'))) {
        throw new Error(`E2E run is closing: ${this.directory}`);
      }
      const value = allocate();
      try {
        remember(value);
        return value;
      } finally {
        this.write();
      }
    });
  }

  update(): void {
    locked(this.directory, () => this.write());
  }

  remove(): void {
    locked(this.directory, () => fs.rmSync(this.file, { force: true }));
  }

  finish(removeRoots: () => void): void {
    locked(this.directory, () => {
      removeRoots();
      fs.rmSync(this.file, { force: true });
    });
  }

  private write(): void {
    const stage = `${this.file}.${randomUUID()}.tmp`;
    try {
      fs.writeFileSync(stage, JSON.stringify(this.fixture), { flag: 'wx', mode: 0o600 });
      fs.renameSync(stage, this.file);
    } finally {
      fs.rmSync(stage, { force: true });
    }
  }
}

export function registerFixture(fixture: OwnedFixture): FixtureRegistration {
  const directory = process.env[registryEnv];
  if (!directory) throw new Error('E2E run registry is unavailable.');
  return new FixtureRegistration(directory, fixture);
}

export interface OwnedGroup {
  pid: number;
  started: string;
}

/** Match a recorded group leader by both process group and start time. */
export function groupLeader(pid: number): OwnedGroup | undefined {
  try {
    const text = execFileSync('/bin/ps', ['-p', String(pid), '-o', 'pgid=', '-o', 'lstart='], {
      encoding: 'utf8',
      timeout: 1_000,
      env: { ...process.env, LC_ALL: 'C' },
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
    const match = /^(\d+)\s+(.+)$/.exec(text);
    if (!match || Number(match[1]) !== pid) {
      throw new Error(`E2E process ${pid} is not its own group leader: ${text}`);
    }
    return { pid, started: match[2] };
  } catch (error) {
    if (error instanceof Error && 'status' in error && error.status === 1) return undefined;
    throw error;
  }
}

/** Capture both the pane leader and its terminal's foreground group. */
export function paneOwnedGroups(panePid: number): OwnedGroup[] {
  const pane = groupLeader(panePid);
  if (!pane) return [];
  const groups = [pane];
  try {
    const text = execFileSync('/bin/ps', ['-p', String(panePid), '-o', 'tpgid='], {
      encoding: 'utf8',
      timeout: 1_000,
      env: { ...process.env, LC_ALL: 'C' },
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
    if (!/^-?\d+$/.test(text)) throw new Error(`Invalid E2E terminal foreground group: ${text}`);
    const pid = Number(text);
    if (pid > 1 && pid !== panePid) groups.push(groupLeader(pid) ?? { pid, started: '' });
  } catch (error) {
    if (!(error instanceof Error && 'status' in error && error.status === 1)) throw error;
  }
  return groups;
}

function groupExists(pid: number): boolean {
  try {
    process.kill(-pid, 0);
    return true;
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ESRCH') return false;
    throw error;
  }
}

/** Signal only a group whose leader still has the recorded start time. */
export function stopOwnedGroups(groups: readonly OwnedGroup[]): void {
  const failures: Error[] = [];
  for (const group of groups) {
    try {
      if (!groupExists(group.pid)) continue;
      const leader = groupLeader(group.pid);
      if (!leader || leader.started !== group.started) continue;
      process.kill(-group.pid, 'SIGTERM');
    } catch (error) {
      if (
        !(
          error instanceof Error &&
          'code' in error &&
          (error.code === 'ESRCH' || error.code === 'EPERM')
        )
      ) {
        failures.push(new Error(`Could not stop E2E group ${group.pid}: ${String(error)}`));
      }
      // A raced exit or permission denial is acceptable only if the group disappears.
    }
  }
  const deadline = Date.now() + 2_000;
  for (;;) {
    const pending = groups.filter((group) => {
      try {
        return groupExists(group.pid);
      } catch (error) {
        if (error instanceof Error && 'code' in error && error.code === 'EPERM') return true;
        failures.push(new Error(`Could not inspect E2E group ${group.pid}: ${String(error)}`));
        return true;
      }
    });
    if (pending.length === 0) {
      if (failures.length)
        throw new AggregateError(failures, failures.map((failure) => failure.message).join(' '));
      return;
    }
    if (Date.now() >= deadline) {
      throw new AggregateError(
        failures,
        `E2E process groups survived or cannot be inspected: ${pending.map((group) => group.pid).join(', ')}`
      );
    }
    Atomics.wait(pause, 0, 0, 20);
  }
}

/** Clean one registered fixture using only its asserted private socket and groups. */
function cleanupFixture(fixture: OwnedFixture): void {
  const tempParent = fs.realpathSync(os.tmpdir());
  const socketParent = process.platform === 'darwin' ? fs.realpathSync('/private/tmp') : tempParent;
  const validateRoot = (root: string, parent: string, prefix: string): void => {
    if (
      !path.isAbsolute(root) ||
      path.dirname(root) !== parent ||
      !path.basename(root).startsWith(prefix)
    ) {
      throw new Error(`Refusing non-owned E2E root: ${root}`);
    }
    const stat = fs.lstatSync(root);
    if (!stat.isDirectory() || stat.isSymbolicLink() || fs.realpathSync(root) !== root) {
      throw new Error(`Refusing changed E2E root: ${root}`);
    }
  };
  validateRoot(fixture.root, tempParent, 'tmt-e2e-');
  validateRoot(fixture.socketRoot, socketParent, 'te2e-');
  if (!path.isAbsolute(fixture.tmuxPath) || path.basename(fixture.tmuxPath) !== 'tmux') {
    throw new Error(`Refusing non-tmux executable: ${fixture.tmuxPath}`);
  }
  assertPrivateSocket(fixture.socketPath, fixture.socketRoot);
  const env = { ...process.env };
  delete env.TMUX;
  delete env.TMUX_PANE;
  const groups = [...fixture.paneGroups, ...fixture.cliGroups];
  const failures: Error[] = [];
  let serverPid = fixture.serverPid;
  if (serverPid > 0 && processState(serverPid) === 'absent' && fs.existsSync(fixture.socketPath)) {
    if (!fs.lstatSync(fixture.socketPath).isSocket())
      throw new Error(`E2E socket changed type: ${fixture.socketPath}`);
    fs.unlinkSync(fixture.socketPath);
  }
  if (fs.existsSync(fixture.socketPath)) {
    if (serverPid === 0) {
      const queried = spawnSync(
        fixture.tmuxPath,
        ['-S', fixture.socketPath, 'display-message', '-p', '#{pid}'],
        {
          env,
          encoding: 'utf8',
          timeout: 2_000,
        }
      );
      const pid = Number(queried.stdout?.trim());
      if (queried.error || queried.status !== 0 || !Number.isSafeInteger(pid) || pid <= 1) {
        failures.push(new Error(`Could not identify E2E server at ${fixture.socketPath}.`));
      } else {
        serverPid = pid;
      }
    }
    const panes = spawnSync(
      fixture.tmuxPath,
      ['-S', fixture.socketPath, 'list-panes', '-a', '-F', '#{pane_pid}'],
      {
        env,
        encoding: 'utf8',
        timeout: 2_000,
      }
    );
    if (panes.error || panes.status !== 0) {
      failures.push(
        new Error(
          `Could not enumerate E2E panes at ${fixture.socketPath}: ${panes.error ?? panes.stderr}`
        )
      );
    } else {
      for (const line of panes.stdout.split('\n').filter(Boolean)) {
        try {
          const pid = Number(line);
          if (!Number.isSafeInteger(pid) || pid <= 1)
            throw new Error(`Invalid E2E pane PID: ${line}`);
          for (const group of paneOwnedGroups(pid)) {
            if (!groups.some((existing) => existing.pid === group.pid)) groups.push(group);
          }
        } catch (error) {
          failures.push(new Error(`Could not identify E2E pane group: ${String(error)}`));
        }
      }
    }
    const stopped = spawnSync(fixture.tmuxPath, ['-S', fixture.socketPath, 'kill-server'], {
      env,
      encoding: 'utf8',
      timeout: 2_000,
    });
    if (stopped.error || stopped.status !== 0) {
      failures.push(
        new Error(
          `Could not stop E2E server ${fixture.socketPath}: ${stopped.error ?? stopped.stderr}`
        )
      );
    }
  }
  try {
    stopOwnedGroups(groups);
  } catch (error) {
    failures.push(new Error(`E2E process group cleanup failed: ${String(error)}`));
  }
  if (serverPid > 0) {
    const deadline = Date.now() + 2_000;
    while (processState(serverPid) !== 'absent' && Date.now() < deadline) {
      Atomics.wait(pause, 0, 0, 20);
    }
    if (processState(serverPid) !== 'absent') {
      failures.push(new Error(`E2E server ${serverPid} survived or cannot be inspected.`));
    } else if (fs.existsSync(fixture.socketPath)) {
      if (!fs.lstatSync(fixture.socketPath).isSocket()) {
        failures.push(new Error(`E2E socket changed type: ${fixture.socketPath}`));
      } else {
        fs.unlinkSync(fixture.socketPath);
      }
    }
  }
  if (fs.existsSync(fixture.socketPath))
    failures.push(new Error(`E2E socket survived: ${fixture.socketPath}`));
  if (failures.length)
    throw new AggregateError(failures, failures.map((failure) => failure.message).join(' '));

  fs.rmSync(fixture.root, { recursive: true, force: true });
  fs.rmSync(fixture.socketRoot, { recursive: true, force: true });
}

/** Main-process owner. Vitest 5.0.1's reporter schedules exit 1 ms after a
 * signal, so this prepended handler must finish synchronous cleanup first. */
export function setup(): () => void {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-e2e-run-'));
  process.env[registryEnv] = directory;
  let done = false;
  const cleanup = (suiteEnd: boolean): void => {
    if (done) return;
    done = true;
    const failures: string[] = [];
    locked(directory, () => {
      fs.writeFileSync(path.join(directory, '.closing'), '');
      for (const name of fs.readdirSync(directory).filter((item) => item.endsWith('.json'))) {
        const file = path.join(directory, name);
        let fixture: OwnedFixture | undefined;
        try {
          fixture = JSON.parse(fs.readFileSync(file, 'utf8')) as OwnedFixture;
          if (suiteEnd)
            failures.push(`E2E fixture survived suite end: ${fixture.root} ${fixture.socketRoot}`);
          cleanupFixture(fixture);
          fs.rmSync(file);
        } catch (error) {
          failures.push(
            `E2E fixture cleanup failed; retained ${fixture?.root ?? file} and ${fixture?.socketRoot ?? file}: ${String(error)}`
          );
        }
      }
    });
    delete process.env[registryEnv];
    if (!fs.readdirSync(directory).some((item) => item.endsWith('.json')))
      fs.rmSync(directory, { recursive: true, force: true });
    if (failures.length) throw new Error(failures.join('\n'));
  };
  const interrupt = (): void => {
    try {
      cleanup(false);
    } catch (error) {
      console.error(error);
    }
    process.exit(130);
  };
  const terminate = (): void => {
    try {
      cleanup(false);
    } catch (error) {
      console.error(error);
    }
    process.exit(143);
  };
  process.prependOnceListener('SIGINT', interrupt);
  process.prependOnceListener('SIGTERM', terminate);
  return () => {
    process.off('SIGINT', interrupt);
    process.off('SIGTERM', terminate);
    cleanup(true);
  };
}
