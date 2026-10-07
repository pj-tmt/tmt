import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';

const script = fileURLToPath(new URL('../../../scripts/install-ci-rust.sh', import.meta.url));
const host = 'x86_64-unknown-linux-gnu';
let root: string;
let bin: string;

interface State {
  installed: string[];
  selected?: string;
  installArgs?: string[];
  calls: string[][];
  lists: number;
}

beforeAll(() => {
  root = mkdtempSync(path.join(os.tmpdir(), 'install-ci-rust-'));
  bin = path.join(root, 'bin');
  mkdirSync(bin);
  // This executable models only rustup's setup operations; it never starts Rust.
  writeExecutable(
    path.join(bin, 'rustup'),
    `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
const file = process.env.FAKE_STATE;
const state = JSON.parse(fs.readFileSync(file, 'utf8'));
state.calls.push(args);
let stage;
if (args[0] === 'toolchain' && args[1] === 'install') stage = 'install';
else if (args[0] === 'default') stage = args.length === 1 ? 'resolve' : 'default';
else if (args[0] === 'show') stage = 'active';
else if (args[0] === 'run') stage = 'version';
else if (args[1] === 'uninstall') stage = 'uninstall';
else if (args[1] === 'list') stage = state.lists++ === 0 ? 'before-list' : 'after-list';
else throw new Error('Unexpected fake operation: ' + args);
fs.writeFileSync(file, JSON.stringify(state));
if (process.env.FAKE_FAIL === stage) process.exit(41);
const canonical = (version) => version + '-' + process.env.FAKE_HOST;
if (stage === 'install') {
  const name = canonical(args[2]);
  if (!state.installed.includes(name)) state.installed.push(name);
  state.installArgs = args.slice(2);
} else if (stage === 'default') {
  state.selected = canonical(args[1]);
} else if (stage === 'resolve') {
  console.log((process.env.FAKE_DEFAULT || state.selected) + ' (default)');
} else if (stage === 'active') {
  console.log((process.env.FAKE_ACTIVE || (state.lists > 1 && process.env.FAKE_FINAL_ACTIVE) || state.selected) + ' (default)');
} else if (stage === 'uninstall') {
  if (!process.env.FAKE_KEEP_EXTRA) state.installed = state.installed.filter((x) => x !== args[2]);
} else if (stage === 'before-list' || stage === 'after-list') {
  let names = [...state.installed];
  if (stage === 'after-list' && process.env.FAKE_EMPTY_FINAL) names = [];
  if (stage === 'after-list' && process.env.FAKE_DUPLICATE_FINAL) names.push(state.selected);
  for (const name of names) console.log(name + (name === state.selected ? ' (active, default)' : ''));
} else if (stage === 'version') {
  console.log('release: ' + args[1] + '\\nhost: ' + process.env.FAKE_HOST + '\\ncommit-hash: inert');
}
fs.writeFileSync(file, JSON.stringify(state));
`
  );
});

afterAll(() => rmSync(root, { recursive: true, force: true }));

function run(
  args: string[] = ['1.97.0', '--profile', 'minimal'],
  environment: Record<string, string | undefined> = {},
  installed = [`stable-${host}`, `nightly-${host}`]
) {
  const dir = mkdtempSync(path.join(root, 'case-'));
  const stateFile = path.join(dir, 'state.json');
  writeFileSync(stateFile, JSON.stringify({ installed, calls: [], lists: 0 }));
  const result = spawnSync(
    '/bin/sh',
    ['-c', '/bin/sh "$@" && printf admitted > "$EFFECT_FILE"', 'fixture', script, ...args],
    {
      env: {
        PATH: `${bin}:/usr/bin:/bin`,
        HOME: dir,
        GITHUB_ACTIONS: 'true',
        FAKE_HOST: host,
        FAKE_STATE: stateFile,
        EFFECT_FILE: path.join(dir, 'cache-admitted'),
        ...environment,
      },
      encoding: 'utf8',
      timeout: 10_000,
    }
  );
  const state = JSON.parse(readFileSync(stateFile, 'utf8')) as State;
  const effects = readdirSync(dir);
  return { ...result, state, admitted: effects.includes('cache-admitted') };
}

describe('CI Rust toolchain normalization', () => {
  it.each(['', 'false'])('refuses non-Actions execution (%s) before rustup effects', (value) => {
    const result = run(undefined, { GITHUB_ACTIONS: value });
    expect(result.status).toBe(1);
    expect(result.state.calls).toEqual([]);
    expect(result.admitted).toBe(false);
  });

  it('retains the requested compiler and install flags while removing annotated extras', () => {
    const args = [
      '1.97.0',
      '--profile',
      'minimal',
      '--component',
      'rustfmt,clippy',
      '--target',
      'aarch64-unknown-linux-musl',
    ];
    const result = run(args);
    expect(result.status).toBe(0);
    expect(result.admitted).toBe(true);
    expect(result.state.installed).toEqual([`1.97.0-${host}`]);
    expect(result.state.installArgs).toEqual(args);
    expect(result.state.calls.filter((x) => x[1] === 'uninstall')).toEqual([
      ['toolchain', 'uninstall', `stable-${host}`],
      ['toolchain', 'uninstall', `nightly-${host}`],
    ]);
    expect(result.state.calls.at(-1)).toEqual(['run', '1.97.0', 'rustc', '-vV']);
    expect(result.stdout).toContain(`Installed toolchains after normalization:\n1.97.0-${host}`);
  });

  it('does not uninstall a singleton or change the dynamic MSRV pin', () => {
    const result = run(['1.95', '--profile', 'minimal'], {}, [`1.95-${host}`]);
    expect(result.status).toBe(0);
    expect(result.admitted).toBe(true);
    expect(result.state.installed).toEqual([`1.95-${host}`]);
    expect(result.state.installArgs).toEqual(['1.95', '--profile', 'minimal']);
    expect(result.state.calls.some((x) => x[1] === 'uninstall')).toBe(false);
  });

  it.each(['aarch64-unknown-linux-gnu', 'aarch64-apple-darwin'])(
    'retains its canonical host identity (%s)',
    (value) => {
      const result = run(undefined, { FAKE_HOST: value });
      expect(result.status).toBe(0);
      expect(result.state.installed).toEqual([`1.97.0-${value}`]);
      expect(result.admitted).toBe(true);
    }
  );

  it.each([
    'install',
    'default',
    'resolve',
    'active',
    'before-list',
    'uninstall',
    'after-list',
    'version',
  ])('refuses %s failure before the next operation', (stage) => {
    const result = run(undefined, { FAKE_FAIL: stage });
    expect(result.status).toBe(41);
    expect(result.admitted).toBe(false);
  });

  it.each([
    { FAKE_DEFAULT: `1.99.0-${host}` },
    { FAKE_ACTIVE: `stable-${host}` },
    { FAKE_FINAL_ACTIVE: `stable-${host}` },
    { FAKE_KEEP_EXTRA: '1' },
    { FAKE_EMPTY_FINAL: '1' },
    { FAKE_DUPLICATE_FINAL: '1' },
  ])('refuses invalid selected or final inventory evidence (%j)', (environment) => {
    const result = run(undefined, environment);
    expect(result.status).toBe(1);
    expect(result.admitted).toBe(false);
  });
});
