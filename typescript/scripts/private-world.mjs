// Runs one command in a private application world, so a test that reaches HOME,
// XDG or temporary state cannot touch the host's, then reports what it left behind.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, rmSync } from 'node:fs';
import { constants, homedir, tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SKELETON = ['home', 'xdg', 'xdg/config', 'xdg/data', 'xdg/state', 'xdg/cache', 'tmp'];
/** Toolchain and Python bytecode caches are not leaks. */
const ALLOWED = [
  /^home\/\.cargo(\/|$)/,
  /^home\/\.rustup(\/|$)/,
  /^home\/Library\/Caches\/com\.apple\.python(\/|$)/,
  /(^|\/)__pycache__(\/|$)/,
  /\.pyc$/,
];
/** Variables that would point a test back at the host's agents, tmux server or Core home. */
const CLEARED = [
  'TMUX',
  'TMUX_PANE',
  'TMT_HOME',
  'TMT_TEST_ROOT',
  'CODEX_HOME',
  'CLAUDE_CONFIG_DIR',
];
const LISTED = 40;

/** The longest world root that leaves room for a socket path under it (sun_path is 104 bytes on macOS). */
export const ROOT_LIMIT = 32;

/** The temporary directory, unless a world under it would be too long: macOS gives a long /var/folders path. */
export function defaultParent(preferred = tmpdir()) {
  const real = realpathSync(preferred);
  if (path.join(real, 'tmt-pw-XXXXXX').length <= ROOT_LIMIT) return real;
  return existsSync('/tmp') ? realpathSync('/tmp') : real;
}

export function createWorld(parent = defaultParent()) {
  // A short name keeps socket paths under the platform limit.
  const root = mkdtempSync(path.join(realpathSync(parent), 'tmt-pw-'));
  for (const directory of SKELETON) mkdirSync(path.join(root, directory), { recursive: true });
  const xdg = (name) => path.join(root, 'xdg', name);
  return {
    root,
    home: path.join(root, 'home'),
    tmp: path.join(root, 'tmp'),
    config: xdg('config'),
    data: xdg('data'),
    state: xdg('state'),
    cache: xdg('cache'),
  };
}

/** The toolchain keeps its real homes; everything an application resolves moves into the world. */
export function worldEnvironment(world, base = process.env) {
  const env = { ...base };
  for (const key of CLEARED) delete env[key];
  const host = base.HOME || homedir();
  return Object.assign(env, {
    HOME: world.home,
    XDG_CONFIG_HOME: world.config,
    XDG_DATA_HOME: world.data,
    XDG_STATE_HOME: world.state,
    XDG_CACHE_HOME: world.cache,
    TMPDIR: world.tmp,
    TMUX_TMPDIR: world.tmp,
    CARGO_HOME: base.CARGO_HOME || path.join(host, '.cargo'),
    RUSTUP_HOME: base.RUSTUP_HOME || path.join(host, '.rustup'),
    TMT_AGENT: '1',
    BROWSER: '/usr/bin/true',
  });
}

/** Entries the command left in HOME/XDG (`state`) and in TMPDIR (`temp`), relative to the world. */
export function findLeaks(world) {
  const leaks = { state: [], temp: [] };
  const walk = (relative, bucket) => {
    let entries;
    try {
      entries = readdirSync(path.join(world.root, relative), { withFileTypes: true });
    } catch {
      bucket.push(`${relative} (unreadable)`);
      return;
    }
    if (!entries.length && !SKELETON.includes(relative)) bucket.push(`${relative}/`);
    for (const entry of entries) {
      const child = path.posix.join(relative, entry.name);
      if (ALLOWED.some((pattern) => pattern.test(child))) continue;
      if (entry.isDirectory()) walk(child, bucket);
      else bucket.push(child);
    }
  };
  walk('home', leaks.state);
  walk('xdg', leaks.state);
  walk('tmp', leaks.temp);
  return leaks;
}

function listing(entries) {
  const shown = entries.slice(0, LISTED).map((entry) => `  ${entry}`);
  if (entries.length > LISTED) shown.push(`  ... and ${entries.length - LISTED} more`);
  return shown;
}

/**
 * Runs `argv` in a fresh world. The command's own failure status wins; a leak in
 * HOME/XDG fails an otherwise green run only with `enforce`. Entries left in the
 * private TMPDIR are listed but never fail the run: they are untidy, not a host leak.
 */
export function runInPrivateWorld({
  argv,
  enforce = false,
  keep = false,
  base = process.env,
  parent,
  spawn = spawnSync,
  log = (line) => process.stderr.write(`${line}\n`),
}) {
  const world = createWorld(parent);
  try {
    const result = spawn(argv[0], argv.slice(1), {
      env: worldEnvironment(world, base),
      stdio: 'inherit',
    });
    let status = result.status;
    if (result.error) {
      log(`private-world: ${argv[0]} did not start: ${result.error.message}`);
      status = 127;
    } else if (status === null) {
      status = 128 + (constants.signals[result.signal] ?? 0);
    }
    const leaks = findLeaks(world);
    if (leaks.state.length) {
      log(
        [
          `private-world: ${leaks.state.length} unexpected ${leaks.state.length === 1 ? 'entry' : 'entries'} left in the private HOME/XDG:`,
          ...listing(leaks.state),
        ].join('\n')
      );
      if (base.GITHUB_ACTIONS === 'true')
        log(
          `::warning title=private-world::${leaks.state.length} test(s) left state in the private HOME/XDG`
        );
    }
    if (leaks.temp.length) {
      log(
        [
          `private-world: ${leaks.temp.length} entries left in the private TMPDIR:`,
          ...listing(leaks.temp),
        ].join('\n')
      );
    }
    if (keep) log(`private-world: kept ${world.root}`);
    return { status: status || (enforce && leaks.state.length ? 1 : 0), leaks, world };
  } finally {
    if (!keep) rmSync(world.root, { recursive: true, force: true });
  }
}

function usage() {
  process.stderr.write(
    'usage: private-world.mjs [--enforce] [--keep] -- <command> [argument...]\n'
  );
  return 2;
}

// A symlinked path must still run: skipping silently would let a gated suite pass unrun.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const separator = process.argv.indexOf('--');
  const flags = process.argv.slice(2, separator < 0 ? undefined : separator);
  const argv = separator < 0 ? [] : process.argv.slice(separator + 1);
  if (!argv.length || flags.some((flag) => flag !== '--enforce' && flag !== '--keep')) {
    process.exitCode = usage();
  } else {
    process.exitCode = runInPrivateWorld({
      argv,
      enforce: flags.includes('--enforce'),
      keep: flags.includes('--keep'),
    }).status;
  }
}
