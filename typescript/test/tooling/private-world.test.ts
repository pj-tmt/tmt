import { spawnSync } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vite-plus/test';
import {
  ROOT_LIMIT,
  createWorld,
  defaultParent,
  findLeaks,
  runInPrivateWorld,
  worldEnvironment,
} from '../../scripts/private-world.mjs';

const SCRIPT = new URL('../../scripts/private-world.mjs', import.meta.url).pathname;
let sandbox: string;
let host: string;

beforeEach(() => {
  sandbox = mkdtempSync(join(tmpdir(), 'tmt-pw-test-'));
  // A fake host: the sentinel proves no run reads or writes the surrounding HOME.
  host = join(sandbox, 'host');
  mkdirSync(join(host, '.config'), { recursive: true });
  writeFileSync(join(host, '.config', 'sentinel'), 'host bytes');
});
afterEach(() => rmSync(sandbox, { recursive: true, force: true }));

const base = () => ({ PATH: process.env.PATH, HOME: host, TMT_HOME: join(host, '.config', 'tmt') });
const node = (script: string) => [process.execPath, '-e', script];
const run = (script: string, options: { enforce?: boolean; keep?: boolean } = {}) => {
  const lines: string[] = [];
  const result = runInPrivateWorld({
    argv: node(script),
    base: base(),
    parent: sandbox,
    log: (line: string) => lines.push(line),
    ...options,
  });
  return { ...result, lines };
};
const hostIsUntouched = () => {
  expect(readFileSync(join(host, '.config', 'sentinel'), 'utf8')).toBe('host bytes');
  expect(existsSync(join(host, '.cache'))).toBe(false);
};

describe('private world', () => {
  it('keeps the world root short enough for socket paths, whatever the temporary directory is', () => {
    // The real default must always fit; a long temporary directory (macOS) falls back to /tmp.
    const fitted = createWorld();
    try {
      expect(fitted.root.length).toBeLessThanOrEqual(ROOT_LIMIT);
    } finally {
      rmSync(fitted.root, { recursive: true, force: true });
    }
    const long = join(sandbox, 'a-deliberately-long-temporary-directory-name', 'nested');
    mkdirSync(long, { recursive: true });
    expect(defaultParent(long)).not.toBe(realpathSync(long));
    expect(join(defaultParent(long), 'tmt-pw-XXXXXX').length).toBeLessThanOrEqual(ROOT_LIMIT);
    // A short one is kept, so TMPDIR still chooses where worlds live when it can.
    const short = mkdtempSync(join(tmpdir(), 'w-'));
    try {
      if (join(realpathSync(short), 'tmt-pw-XXXXXX').length <= ROOT_LIMIT) {
        expect(defaultParent(short)).toBe(realpathSync(short));
      }
    } finally {
      rmSync(short, { recursive: true, force: true });
    }
  });

  it('moves every application root into the world and keeps only the toolchain homes', () => {
    const world = createWorld(sandbox);
    const env = worldEnvironment(world, {
      ...base(),
      TMUX: '/tmp/tmux-1/default,1,0',
      TMUX_PANE: '%1',
      TMT_TEST_ROOT: host,
      CODEX_HOME: join(host, '.codex'),
      CLAUDE_CONFIG_DIR: join(host, '.claude'),
    });
    expect(env.HOME).toBe(world.home);
    expect([
      env.XDG_CONFIG_HOME,
      env.XDG_DATA_HOME,
      env.XDG_STATE_HOME,
      env.XDG_CACHE_HOME,
    ]).toEqual([world.config, world.data, world.state, world.cache]);
    expect([env.TMPDIR, env.TMUX_TMPDIR]).toEqual([world.tmp, world.tmp]);
    expect([env.TMT_AGENT, env.BROWSER]).toEqual(['1', '/usr/bin/true']);
    for (const key of [
      'TMUX',
      'TMUX_PANE',
      'TMT_HOME',
      'TMT_TEST_ROOT',
      'CODEX_HOME',
      'CLAUDE_CONFIG_DIR',
    ])
      expect(env[key], key).toBeUndefined();
    expect(env.PATH).toBe(process.env.PATH);
    expect([env.CARGO_HOME, env.RUSTUP_HOME]).toEqual([
      join(host, '.cargo'),
      join(host, '.rustup'),
    ]);
    const explicit = worldEnvironment(world, { ...base(), CARGO_HOME: '/c', RUSTUP_HOME: '/r' });
    expect([explicit.CARGO_HOME, explicit.RUSTUP_HOME]).toEqual(['/c', '/r']);
    expect(findLeaks(world)).toEqual({ state: [], temp: [] });
  });

  it('runs a clean command with the world as its HOME and removes the world afterwards', () => {
    const marker = join(sandbox, 'seen.json');
    const result = run(
      `require('node:fs').writeFileSync(${JSON.stringify(marker)}, JSON.stringify({home: process.env.HOME, agent: process.env.TMT_AGENT, tmt: process.env.TMT_HOME ?? null}))`
    );
    expect(result.status).toBe(0);
    expect(result.leaks).toEqual({ state: [], temp: [] });
    const seen = JSON.parse(readFileSync(marker, 'utf8'));
    expect(seen).toEqual({ home: result.world.home, agent: '1', tmt: null });
    expect(seen.home.startsWith(host)).toBe(false);
    expect(existsSync(result.world.root)).toBe(false);
    expect(result.lines).toEqual([]);
    hostIsUntouched();
  });

  it('reports state left in HOME and XDG without failing, and fails only when enforced', () => {
    const script = `
      const fs = require('node:fs'), path = require('node:path');
      fs.mkdirSync(path.join(process.env.HOME, '.config', 'tmt'), { recursive: true });
      fs.writeFileSync(path.join(process.env.HOME, '.config', 'tmt', 'tmux-team.db'), 'x');
      fs.mkdirSync(path.join(process.env.XDG_CACHE_HOME, 'tmt-ops', 'staleness'), { recursive: true });
      fs.writeFileSync(path.join(process.env.XDG_CACHE_HOME, 'tmt-ops', 'staleness', 'a.json'), '{}');
      fs.mkdirSync(path.join(process.env.XDG_STATE_HOME, 'empty'));
    `;
    const reported = run(script);
    expect(reported.status).toBe(0);
    expect(reported.leaks.state.sort()).toEqual([
      'home/.config/tmt/tmux-team.db',
      'xdg/cache/tmt-ops/staleness/a.json',
      'xdg/state/empty/',
    ]);
    expect(reported.lines.join('\n')).toContain(
      '3 unexpected entries left in the private HOME/XDG'
    );
    expect(reported.lines.join('\n')).toContain('  home/.config/tmt/tmux-team.db');
    const enforced = run(script, { enforce: true });
    expect(enforced.status).toBe(1);
    expect(existsSync(enforced.world.root)).toBe(false);
    hostIsUntouched();
  });

  it('treats toolchain and Python bytecode caches as clean', () => {
    const result = run(
      `
      const fs = require('node:fs'), path = require('node:path');
      const put = (...parts) => {
        const file = path.join(process.env.HOME, ...parts);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, 'x');
      };
      put('.cargo', 'registry', 'index');
      put('.rustup', 'settings.toml');
      put('Library', 'Caches', 'com.apple.python', 'Applications', 'site.cpython-39.pyc');
      put('project', '__pycache__', 'module.cpython-312.pyc');
    `,
      { enforce: true }
    );
    expect(result.status).toBe(0);
    expect(result.leaks).toEqual({ state: [], temp: [] });
  });

  it('lists leftovers in the private TMPDIR but never fails on them', () => {
    const result = run(
      `require('node:fs').mkdirSync(require('node:path').join(process.env.TMPDIR, 'tmt-ops-links-1'))`,
      { enforce: true }
    );
    expect(result.status).toBe(0);
    expect(result.leaks).toEqual({ state: [], temp: ['tmp/tmt-ops-links-1/'] });
    expect(result.lines.join('\n')).toContain('1 entries left in the private TMPDIR');
  });

  it('keeps the command failure status over any leak finding and names a command that cannot start', () => {
    const failing = run(
      `require('node:fs').writeFileSync(process.env.HOME + '/x', ''); process.exit(7)`,
      {
        enforce: true,
      }
    );
    expect(failing.status).toBe(7);
    expect(failing.leaks.state).toEqual(['home/x']);
    const lines: string[] = [];
    const missing = runInPrivateWorld({
      argv: [join(sandbox, 'no-such-command')],
      base: base(),
      parent: sandbox,
      log: (line: string) => lines.push(line),
    });
    expect(missing.status).toBe(127);
    expect(lines.join('\n')).toContain('did not start');
    const killed = run(`process.kill(process.pid, 'SIGTERM')`);
    expect(killed.status).toBe(128 + 15);
  });

  it('keeps the world on request', () => {
    const result = run(`0`, { keep: true });
    expect(existsSync(result.world.root)).toBe(true);
    expect(result.lines.join('\n')).toContain(`kept ${result.world.root}`);
  });

  it('runs from the command line with the same contract', () => {
    const cli = (...args: string[]) =>
      spawnSync(process.execPath, [SCRIPT, ...args], {
        env: base(),
        encoding: 'utf8',
        cwd: sandbox,
      });
    const usage = cli('--enforce');
    expect(usage.status).toBe(2);
    expect(usage.stderr).toContain('usage: private-world.mjs');
    expect(cli('--bogus', '--', 'true').status).toBe(2);
    const clean = cli('--', process.execPath, '-e', 'process.exit(0)');
    expect([clean.status, clean.stderr]).toEqual([0, '']);
    const leaking = cli(
      '--enforce',
      '--',
      process.execPath,
      '-e',
      `require('node:fs').writeFileSync(process.env.HOME + '/stray', '')`
    );
    expect(leaking.status).toBe(1);
    expect(leaking.stderr).toContain('  home/stray');
    hostIsUntouched();
  });

  it('runs the command when invoked through a symlink instead of skipping it', () => {
    const link = join(sandbox, 'linked-runner.mjs');
    symlinkSync(SCRIPT, link);
    const result = spawnSync(
      process.execPath,
      [link, '--', process.execPath, '-e', 'process.exit(9)'],
      {
        env: base(),
        encoding: 'utf8',
      }
    );
    expect(result.status).toBe(9);
  });
});
