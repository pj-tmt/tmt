import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const action = readFileSync(
  path.join(repository, '.github/actions/apt-install/action.yml'),
  'utf8'
);
const script = action.split('      run: |\n')[1].replace(/^        /gm, '');
let root: string;
let bin: string;

beforeAll(() => {
  root = mkdtempSync(path.join(os.tmpdir(), 'apt-install-'));
  bin = path.join(root, 'bin');
  mkdirSync(bin);
  const tool = (name: string, body: string) => {
    const target = path.join(bin, name);
    writeFileSync(target, `#!/bin/sh\n${body}\n`);
    chmodSync(target, 0o755);
  };
  // All privileged and host-package operations are isolated fakes.
  tool('grep', 'printf "%s\\n" /etc/apt/sources.list.d/google-chrome.list');
  tool('sudo', 'printf "%s\\n" "$*" >> "$LOG_DIR/sudo"; exec "$@"');
  tool('rm', 'printf "%s\\n" "$*" >> "$LOG_DIR/removals"');
  tool('sleep', 'printf "%s\\n" "$*" >> "$LOG_DIR/sleeps"');
  // Simulate timeout exit 124; GNU timeout itself is provided by the Linux runner.
  tool(
    'timeout',
    `printf '%s\\n' "$*" >> "$LOG_DIR/timeouts"
[ "$1" = --verbose ] && [ "$2" = --kill-after=10s ] && [ "$3" = 120s ] || exit 98
shift 3
case " $* " in *' update '*) phase=update;; *) phase=install;; esac
printf '%s\\n' "$phase" >> "$LOG_DIR/$phase"
count=$(wc -l < "$LOG_DIR/$phase")
if [ "$phase" = "$FAIL_PHASE" ] && [ "$count" -le "$FAIL_COUNT" ]; then exit 124; fi
exec "$@"`
  );
  tool('apt-get', 'printf "%s\\n" "$*" >> "$LOG_DIR/apt"');
});
afterAll(() => rmSync(root, { recursive: true, force: true }));

function run(failPhase = '', failCount = '0') {
  const dir = mkdtempSync(path.join(root, 'case-'));
  const result = spawnSync('/bin/bash', ['-e', '-o', 'pipefail', '-c', script], {
    env: {
      PATH: `${bin}:/usr/bin:/bin`,
      LOG_DIR: dir,
      GITHUB_WORKSPACE: repository,
      PACKAGES: 'zsh binutils musl-tools',
      FAIL_PHASE: failPhase,
      FAIL_COUNT: failCount,
    },
    encoding: 'utf8',
    timeout: 10_000,
  });
  const lines = (name: string) =>
    existsSync(path.join(dir, name))
      ? readFileSync(path.join(dir, name), 'utf8').trim().split('\n')
      : [];
  return { result, lines };
}

const options = '-o Acquire::Retries=2 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30';
const update = `${options} update`;
const install = `${options} install --no-install-recommends -y zsh binutils musl-tools`;

describe('apt-install composite action', () => {
  it('removes Chrome sources and preserves package arguments with bounded update and install', () => {
    const { result, lines } = run();
    expect(result.status).toBe(0);
    expect(lines('removals')).toEqual(['-f /etc/apt/sources.list.d/google-chrome.list']);
    expect(lines('apt')).toEqual([update, install]);
    expect(lines('sudo')).toEqual([
      'rm -f /etc/apt/sources.list.d/google-chrome.list',
      ...[update, install].map((args) => `timeout --verbose --kill-after=10s 120s apt-get ${args}`),
    ]);
    expect(lines('sleeps')).toEqual([]);
  });

  it.each(['update', 'install'])('retries each timed-out %s attempt and recovers', (phase) => {
    const { result, lines } = run(phase, '2');
    expect(result.status).toBe(0);
    expect(lines(phase)).toHaveLength(3);
    expect(lines('timeouts')).toHaveLength(4);
    expect(lines('apt')).toEqual([update, install]);
    expect(lines('sleeps')).toEqual(['5', '10']);
    expect(result.stderr).toContain('attempt 2 of 3 with status 124');
  });

  it.each(['update', 'install'])(
    'stops after three timed-out %s attempts with clear failure',
    (phase) => {
      const { result, lines } = run(phase, '99');
      expect(result.status).toBe(124);
      expect(lines(phase)).toHaveLength(3);
      expect(lines('sleeps')).toEqual(['5', '10']);
      expect(result.stderr).toContain('attempt 3 of 3 with status 124');
      expect(lines('apt')).toEqual(phase === 'update' ? [] : [update]);
      if (phase === 'update') expect(lines('install')).toEqual([]);
    }
  );
});
