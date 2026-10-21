import { spawnSync, type SpawnSyncReturns } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

/**
 * The only way a TypeScript test starts, drives or stops a tmux server it owns.
 *
 * A process inherits TMUX and TMUX_PANE from the pane it runs in, and `$TMUX` outranks
 * `TMUX_TMPDIR`, so a bare `tmux` reaches the host's server. Every call here selects one
 * socket with an explicit `-S`, drops TMUX and TMUX_PANE, and first proves the socket is
 * private: inside the test's own root, not a symlink, and not inside a directory the host's
 * tmux uses. The e2e wrapper (`harness/fixture.ts`) and `scripts/private-tmux.sh` hold the
 * same rule; the tmt-dev skill's focused-checks reference states it once.
 */

export class PrivateTmuxRefused extends Error {
  constructor(message: string) {
    super(`private tmux refused: ${message}`);
    this.name = 'PrivateTmuxRefused';
  }
}

/** The path tmux binds is limited to 104 bytes on macOS, NUL included. */
const SOCKET_PATH_LIMIT = 103;

/** The physical form of `target`, even when its tail does not exist yet. */
function resolveLoosely(target: string): string {
  let existing = path.resolve(target);
  const tail: string[] = [];
  while (!fs.existsSync(existing)) {
    tail.unshift(path.basename(existing));
    const parent = path.dirname(existing);
    if (parent === existing) break;
    existing = parent;
  }
  return path.join(fs.realpathSync(existing), ...tail);
}

const inside = (child: string, parent: string) =>
  child === parent || child.startsWith(parent.endsWith(path.sep) ? parent : parent + path.sep);

/** Directories a host tmux server keeps its sockets in, from the ambient environment. */
export function hostSocketDirectories(
  ambient: NodeJS.ProcessEnv = process.env,
  uid: number = process.getuid!()
): string[] {
  const directories = [`/tmp/tmux-${uid}`, `/private/tmp/tmux-${uid}`];
  if (ambient.TMUX_TMPDIR) directories.push(path.join(ambient.TMUX_TMPDIR, `tmux-${uid}`));
  const server = ambient.TMUX?.split(',')[0];
  if (server) directories.push(path.dirname(server));
  return directories.filter((directory) => path.isAbsolute(directory)).map(resolveLoosely);
}

/**
 * Returns the socket path as tmux will bind it, or throws before any tmux command runs. `root` is the
 * directory the test owns; the socket must be a new or existing file inside it.
 */
export function assertPrivateSocket(
  socket: string,
  root: string,
  ambient: NodeJS.ProcessEnv = process.env
): string {
  if (!path.isAbsolute(socket)) throw new PrivateTmuxRefused(`${socket} is not absolute`);
  if (!path.isAbsolute(root) || !fs.existsSync(root))
    throw new PrivateTmuxRefused(`the test root ${root} is not an existing absolute directory`);
  // lstat, not exists: a dangling link is still a link.
  if (fs.lstatSync(socket, { throwIfNoEntry: false })?.isSymbolicLink())
    throw new PrivateTmuxRefused(`${socket} is a symlink`);
  const given = path.resolve(socket);
  const physical = resolveLoosely(socket);
  // The kernel measures the string tmux binds, not its resolved form.
  if (given.length > SOCKET_PATH_LIMIT)
    throw new PrivateTmuxRefused(`${given} is too long for a Unix socket path`);
  if (!inside(physical, fs.realpathSync(root)) || physical === fs.realpathSync(root))
    throw new PrivateTmuxRefused(`${physical} is outside the test root ${root}`);
  for (const host of hostSocketDirectories(ambient))
    if (inside(physical, host))
      throw new PrivateTmuxRefused(
        `${physical} lies inside the host tmux socket directory ${host}`
      );
  return given;
}

/** The tmux on PATH, or a clear failure for a check that needs a real server. */
export function locateTmux(): string {
  const found = spawnSync('/usr/bin/which', ['tmux'], { encoding: 'utf8' });
  if (found.status !== 0) throw new Error('Install tmux for the private tmux checks');
  return found.stdout.trim();
}

export interface PrivateTmux {
  readonly executable: string;
  readonly socket: string;
  /** One tmux command on the private socket; the caller reads status and output. */
  run(args: string[], options?: { timeout?: number }): SpawnSyncReturns<string>;
  /** Trimmed stdout of one tmux command; a nonzero exit throws with tmux's stderr. */
  tmux(args: string[]): string;
  /** Ends the server; absence is not an error. */
  stop(): void;
}

export interface PrivateTmuxOptions {
  /** The directory the test owns; the socket lives in it as `<name>.sock`. */
  root: string;
  name: string;
  /** Environment for the command; read on every call, TMUX and TMUX_PANE removed. */
  env: NodeJS.ProcessEnv;
  executable?: string;
  /** A tmux configuration file; none (/dev/null) by default. */
  config?: string;
  /** The ambient environment the host's server is found from; the process's own. */
  ambient?: NodeJS.ProcessEnv;
}

export function privateTmux(options: PrivateTmuxOptions): PrivateTmux {
  const ambient = options.ambient ?? process.env;
  const socket = assertPrivateSocket(
    path.join(options.root, `${options.name}.sock`),
    options.root,
    ambient
  );
  const executable = options.executable ?? locateTmux();
  const config = options.config ?? '/dev/null';
  const run = (args: string[], settings: { timeout?: number } = {}) => {
    // Only -u and -2 may precede the command: any other global option chooses a server.
    for (const arg of args) {
      if (!arg.startsWith('-')) break;
      if (arg !== '-u' && arg !== '-2')
        throw new PrivateTmuxRefused(`tmux option ${arg} is not allowed before the command`);
    }
    assertPrivateSocket(socket, options.root, ambient);
    const env = { ...options.env };
    delete env.TMUX;
    delete env.TMUX_PANE;
    return spawnSync(executable, ['-f', config, '-S', socket, ...args], {
      env,
      encoding: 'utf8',
      timeout: settings.timeout ?? 5000,
    });
  };
  return {
    executable,
    socket,
    run,
    tmux(args) {
      const result = run(args);
      if (result.status !== 0) throw new Error(`Private tmux failed: ${result.stderr}`);
      return result.stdout.trimEnd();
    },
    stop() {
      run(['kill-server']);
    },
  };
}
