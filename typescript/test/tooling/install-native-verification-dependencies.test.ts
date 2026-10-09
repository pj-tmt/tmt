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
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';

const script = fileURLToPath(
  new URL('../../../scripts/install-native-verification-dependencies.sh', import.meta.url)
);
const cliPackage = /^CLI_PACKAGE='([^']+)'$/m.exec(readFileSync(script, 'utf8'))![1];
const signature = 'Error: async hook stack has become corrupted (actual: 1269, expected: 1269)';
const note = 'pnpm install retry after Node async-hook abort (#1806)';
let root: string;
let bin: string;

beforeAll(() => {
  root = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'native-install-retry-')));
  bin = path.join(root, 'bin');
  mkdirSync(bin);
  const tool = (name: string, body: string) =>
    writeExecutable(path.join(bin, name), `#!/bin/sh\nset -eu\n${body}\n`);
  tool('git', 'printf "%s\\n" "${FAKE_GIT_ROOT:-${PWD%/typescript}}"');
  tool('sleep', 'echo "$*" >> "$LOG_DIR/sleeps"');
  tool(
    'rm',
    '[ "${FAIL_CLEANUP:-0}" = 0 ] || exit 37\n[ "${NOOP_CLEANUP:-0}" = 0 ] || exit 0\nexec /bin/rm "$@"'
  );
  tool(
    'find',
    `echo call >> "$LOG_DIR/find-calls"
count=$(wc -l < "$LOG_DIR/find-calls" | tr -d ' ')
[ "$count" != "\${FAIL_FIND_CALL:-0}" ] || exit 38
exec /usr/bin/find "$@"`
  );
  tool(
    'pnpm',
    `echo "$*" >> "$LOG_DIR/runs"
count=$(wc -l < "$LOG_DIR/runs" | tr -d ' ')
if [ "$count" = 2 ]; then
  [ ! -e node_modules/partial ] && [ ! -e "$INSTALL_ROOT/design/browser-ui/node_modules/partial" ] && [ ! -L "$INSTALL_ROOT/extensions/fake/node_modules" ] || exit 71
fi
mkdir -p node_modules "$INSTALL_ROOT/design/browser-ui/node_modules" "$INSTALL_ROOT/extensions/fake"
echo partial > node_modules/partial
echo partial > "$INSTALL_ROOT/design/browser-ui/node_modules/partial"
ln -s "$STORE" "$INSTALL_ROOT/extensions/fake/node_modules"
echo "stdout attempt $count"
if [ "$count" = 1 ]; then
  printf '%s\\n' "\${DIAGNOSTIC:-}" >&2
  [ "\${SIGNATURE_STDOUT:-0}" = 0 ] || printf '%s\\n' "$SIGNATURE"
  exit "\${FIRST_STATUS:-0}"
fi
printf '%s\\n' "\${SECOND_DIAGNOSTIC:-}" >&2
exit "\${SECOND_STATUS:-0}"`
  );
});
afterAll(() => rmSync(root, { recursive: true, force: true }));

function fixture(candidate = false) {
  const dir = mkdtempSync(path.join(root, 'case-'));
  const workspace = path.join(dir, 'checkout');
  const installRoot = candidate ? path.join(workspace, 'release-source') : workspace;
  const cwd = path.join(installRoot, 'typescript');
  const temp = path.join(dir, 'temp');
  const store = path.join(dir, 'store');
  for (const folder of [cwd, temp, store]) mkdirSync(folder, { recursive: true });
  for (const file of ['package.json', 'pnpm-lock.yaml', 'pnpm-workspace.yaml']) {
    writeFileSync(path.join(cwd, file), 'fixture');
  }
  writeFileSync(path.join(store, 'sentinel'), 'untouched store bytes');
  const summary = path.join(dir, 'summary');
  const lines = (name: string) => {
    const file = path.join(dir, name);
    return existsSync(file) ? readFileSync(file, 'utf8').trim().split('\n') : [];
  };
  const run = (env: Record<string, string> = {}, args: string[] = [], at = cwd) =>
    spawnSync('/bin/bash', [script, ...args], {
      cwd: at,
      env: {
        PATH: `${bin}:/usr/bin:/bin`,
        PWD: at,
        GITHUB_WORKSPACE: workspace,
        RUNNER_TEMP: temp,
        GITHUB_STEP_SUMMARY: summary,
        LOG_DIR: dir,
        INSTALL_ROOT: installRoot,
        STORE: store,
        SIGNATURE: signature,
        ...env,
      },
      encoding: 'utf8',
      timeout: 10_000,
    });
  return { dir, workspace, installRoot, cwd, store, summary, lines, run };
}

function expectAttempts(
  c: ReturnType<typeof fixture>,
  result: ReturnType<typeof spawnSync>,
  n: number
) {
  expect(result.error).toBeUndefined();
  expect(c.lines('runs')).toHaveLength(n);
  expect(c.lines('sleeps')).toEqual(n === 2 ? ['5'] : []);
  expect(readFileSync(c.summary, 'utf8')).toContain(`attempts=${n}; final status=${result.status}`);
}

describe('native verification install Node-abort retry (#1806)', () => {
  it.each([false, true])('runs the default install once in candidate=%s', (candidate) => {
    const c = fixture(candidate);
    const result = c.run();
    expect(result.status).toBe(0);
    expectAttempts(c, result, 1);
    expect(c.lines('runs')).toEqual(['install --frozen-lockfile --ignore-scripts']);
    expect(result.stderr).not.toContain('::warning');
  });

  it('cleans only attempt-created modules and keeps both outputs before a successful retry', () => {
    const c = fixture();
    const other = path.join(c.workspace, 'release-source/typescript/node_modules');
    mkdirSync(other, { recursive: true });
    writeFileSync(path.join(other, 'sentinel'), 'successful candidate bytes');
    const result = c.run({ FIRST_STATUS: '7', DIAGNOSTIC: signature });
    expect(result.status).toBe(0);
    expectAttempts(c, result, 2);
    expect(c.lines('runs')).toEqual(Array(2).fill('install --frozen-lockfile --ignore-scripts'));
    expect(result.stdout).toBe('stdout attempt 1\nstdout attempt 2\n');
    expect(result.stderr).toContain(signature);
    expect(result.stderr).toContain(`::warning::${note}`);
    const text = readFileSync(c.summary, 'utf8');
    expect(text).toContain(note);
    const logs = /logs=(.+)\n/.exec(text)![1];
    expect(readFileSync(path.join(logs, 'attempt-1.stderr'), 'utf8')).toBe(signature + '\n');
    expect(readFileSync(path.join(logs, 'attempt-2.stdout'), 'utf8')).toBe('stdout attempt 2\n');
    expect(readFileSync(path.join(other, 'sentinel'), 'utf8')).toBe('successful candidate bytes');
    expect(readFileSync(path.join(c.store, 'sentinel'), 'utf8')).toBe('untouched store bytes');
  });

  it('keeps the last status when the exact signature occurs twice', () => {
    const c = fixture(true);
    const result = c.run({
      FIRST_STATUS: '7',
      SECOND_STATUS: '9',
      DIAGNOSTIC: signature,
      SECOND_DIAGNOSTIC: signature,
    });
    expect(result.status).toBe(9);
    expectAttempts(c, result, 2);
    expect(result.stderr.split(signature)).toHaveLength(3);
  });

  it('uses the single package constant in trusted CLI mode', () => {
    const c = fixture();
    const result = c.run({ FIRST_STATUS: '7', DIAGNOSTIC: signature }, ['--trusted-cli']);
    expect(result.status).toBe(0);
    expectAttempts(c, result, 2);
    expect(c.lines('runs')).toEqual(
      Array(2).fill(
        `--filter ${cliPackage} --fail-if-no-match install --frozen-lockfile --ignore-scripts`
      )
    );
  });

  it.each([
    ['lockfile mismatch', '8', 'ERR_PNPM_OUTDATED_LOCKFILE', '0'],
    ['other tool error', '127', 'pnpm: command not found', '0'],
    ['similar error', '7', 'async hook stack has become corrupted', '0'],
    ['malformed diagnostic', '7', signature + ' unrelated', '0'],
    ['stdout-only diagnostic', '7', '', '1'],
    ['successful command with diagnostic', '0', signature, '0'],
  ])('never retries %s', (_, status, diagnostic, stdout) => {
    const c = fixture();
    const result = c.run({
      FIRST_STATUS: status,
      DIAGNOSTIC: diagnostic,
      SIGNATURE_STDOUT: stdout,
    });
    expect(result.status).toBe(Number(status));
    expectAttempts(c, result, 1);
    expect(result.stderr).not.toContain('::warning');
  });

  it.each([
    ['cleanup command failure', { FAIL_CLEANUP: '1' }],
    ['cleanup leaves partial files', { NOOP_CLEANUP: '1' }],
    ['cleanup verification unavailable', { FAIL_FIND_CALL: '3' }],
  ] as const)('does not launch attempt 2 after %s', (_, env) => {
    const c = fixture();
    const result = c.run({ FIRST_STATUS: '7', DIAGNOSTIC: signature, ...env });
    expect(result.status).toBe(7);
    expectAttempts(c, result, 1);
    expect(result.stderr).toContain('partial modules cleanup failed; no second attempt');
  });

  it.each(['package.json', 'pnpm-lock.yaml', 'pnpm-workspace.yaml'])(
    'refuses missing or symlinked %s',
    (file) => {
      for (const link of [false, true]) {
        const c = fixture();
        rmSync(path.join(c.cwd, file));
        if (link) symlinkSync(path.join(c.store, 'sentinel'), path.join(c.cwd, file));
        const result = c.run();
        expect(result.status).toBe(2);
        expect(result.stderr).toContain(`missing or symlinked ${file}`);
        expect(c.lines('runs')).toEqual([]);
      }
    }
  );

  it.each([
    'unsupported mode',
    'missing workspace',
    'unexpected cwd',
    'git root',
    'inspection failure',
    'existing modules',
    'symlinked cwd',
    'symlinked checkout',
  ])('refuses %s before pnpm', (kind) => {
    const c = fixture();
    let args: string[] = [];
    let at = c.cwd;
    let workspace = c.workspace;
    const env: Record<string, string> = {};
    if (kind === 'unsupported mode') args = ['--force'];
    if (kind === 'missing workspace') env.GITHUB_WORKSPACE = '';
    if (kind === 'unexpected cwd') at = c.workspace;
    if (kind === 'git root') env.FAKE_GIT_ROOT = c.dir;
    if (kind === 'inspection failure') env.FAIL_FIND_CALL = '1';
    if (kind === 'existing modules') mkdirSync(path.join(c.cwd, 'node_modules'));
    if (kind === 'symlinked cwd') {
      const original = path.join(c.workspace, 'real-typescript');
      mkdirSync(original);
      rmSync(c.cwd, { recursive: true });
      symlinkSync(original, c.cwd);
    }
    if (kind === 'symlinked checkout') {
      workspace = path.join(c.dir, 'alias-checkout');
      symlinkSync(c.workspace, workspace);
      at = path.join(workspace, 'typescript');
      env.GITHUB_WORKSPACE = workspace;
    }
    const result = c.run(env, args, at);
    expect(result.status).toBe(2);
    expect(c.lines('runs')).toEqual([]);
    expect(result.stderr).toMatch(/refused|unsupported/);
  });
});
