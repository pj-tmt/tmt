import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';

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
    writeExecutable(target, `#!/bin/sh\n${body}\n`, 0o755);
  };
  // All privileged and host-package operations are isolated fakes.
  tool('grep', 'printf "%s\\n" /etc/apt/sources.list.d/google-chrome.list');
  tool('sudo', 'printf "%s\\n" "$*" >> "$LOG_DIR/sudo"; exec "$@"');
  tool('rm', 'printf "%s\\n" "$*" >> "$LOG_DIR/removals"');
  tool(
    'sed',
    `[ "$1" = -i ] && [ "$2" = 1d ] && [ "$3" = "$APT_MIRRORS" ] || exit 97
printf '%s\\n' "$*" >> "$LOG_DIR/mirror-edits"
[ "$SED_FAILURE" = 0 ] || exit "$SED_FAILURE"
/usr/bin/sed '1d' "$3" > "$LOG_DIR/edited-list"
/bin/mv "$LOG_DIR/edited-list" "$3"`
  );
  tool('awk', '[ "$AWK_FAILURE" = 0 ] || exit "$AWK_FAILURE"; exec /usr/bin/awk "$@"');
  tool('sleep', 'printf "%s\\n" "$*" >> "$LOG_DIR/sleeps"');
  // Simulate timeout exit 124; GNU timeout itself is provided by the Linux runner.
  tool(
    'timeout',
    `printf '%s\\n' "$*" >> "$LOG_DIR/timeouts"
case " $* " in *' update '*) phase=update; bound=60s;; *) phase=install; bound=120s;; esac
[ "$1" = --verbose ] && [ "$2" = --kill-after=10s ] && [ "$3" = "$bound" ] || exit 98
shift 3
printf '%s\\n' "$phase" >> "$LOG_DIR/$phase"
count=$(wc -l < "$LOG_DIR/$phase")
count=$((count))
if [ "$phase" = install ] && [ -f "$APT_MIRRORS" ]; then /bin/cp "$APT_MIRRORS" "$LOG_DIR/mirrors-$count"; fi
if { [ "$phase" = "$FAIL_PHASE" ] || [ "$FAIL_PHASE" = both ]; } && [ "$count" -le "$FAIL_COUNT" ]; then
  if [ "$count" -eq 1 ]; then exit "$FIRST_FAIL_STATUS"; fi
  exit "$FAIL_STATUS"
fi
exec "$@"`
  );
  tool('apt-get', 'printf "%s\\n" "$*" >> "$LOG_DIR/apt"');
});
afterAll(() => rmSync(root, { recursive: true, force: true }));

// Literal endpoint/priority bytes from runner-images ubuntu24/20261004.327
// configure-apt-sources.sh; these expectations are independent of the action.
const azure = 'http://azure.archive.ubuntu.com/ubuntu/\tpriority:1\n';
const remainingMirrors =
  'https://archive.ubuntu.com/ubuntu/\tpriority:2\n' +
  'https://security.ubuntu.com/ubuntu/\tpriority:3\n';
const mirrors = azure + remainingMirrors;
const sourcesFor = (list: string) =>
  `Types: deb\nURIs: mirror+file:${list}\nSuites: noble noble-updates noble-backports\nComponents: main restricted universe multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\n` +
  `Types: deb\nURIs: mirror+file:${list}\nSuites: noble-security\nComponents: main restricted universe multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n`;

function run(
  failPhase = '',
  failCount = '0',
  input: {
    mirrors?: string | null;
    sources?: (list: string) => string;
    failStatus?: string;
    sedFailure?: string;
    firstFailStatus?: string;
    awkFailure?: string;
    missingSources?: boolean;
  } = {}
) {
  const dir = mkdtempSync(path.join(root, 'case-'));
  const list = path.join(dir, 'apt-mirrors.txt');
  const sources = path.join(dir, 'ubuntu.sources');
  const scratch = path.join(dir, 'scratch');
  mkdirSync(scratch);
  const originalSources = (input.sources ?? sourcesFor)(list);
  const originalMirrors = input.mirrors === undefined ? mirrors : input.mirrors;
  if (!input.missingSources) writeFileSync(sources, originalSources);
  if (originalMirrors !== null) writeFileSync(list, originalMirrors);
  const result = spawnSync('/bin/bash', ['-e', '-o', 'pipefail', '-c', script], {
    env: {
      PATH: `${bin}:/usr/bin:/bin`,
      LOG_DIR: dir,
      GITHUB_WORKSPACE: repository,
      PACKAGES: 'zsh binutils musl-tools',
      FAIL_PHASE: failPhase,
      FAIL_COUNT: failCount,
      FAIL_STATUS: input.failStatus ?? '124',
      FIRST_FAIL_STATUS: input.firstFailStatus ?? input.failStatus ?? '124',
      AWK_FAILURE: input.awkFailure ?? '0',
      SED_FAILURE: input.sedFailure ?? '0',
      APT_SOURCES: sources,
      APT_MIRRORS: list,
      TMPDIR: scratch,
    },
    encoding: 'utf8',
    timeout: 10_000,
  });
  const lines = (name: string) =>
    existsSync(path.join(dir, name))
      ? readFileSync(path.join(dir, name), 'utf8').trim().split('\n')
      : [];
  expect(result.error).toBeUndefined();
  expect(result.signal).toBeNull();
  expect(readdirSync(scratch)).toEqual([]);
  if (input.missingSources) expect(existsSync(sources)).toBe(false);
  else expect(readFileSync(sources, 'utf8')).toBe(originalSources);
  const mirrorBytes = () => (existsSync(list) ? readFileSync(list, 'utf8') : null);
  const beforeInstall = (attempt: number) =>
    readFileSync(path.join(dir, `mirrors-${attempt}`), 'utf8');
  return { result, lines, mirrorBytes, beforeInstall, list, originalMirrors };
}

const options = '-o Acquire::Retries=2 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30';
const update = `${options} update`;
const install = `${options} install --no-install-recommends -y zsh binutils musl-tools`;

describe('apt-install composite action', () => {
  it('pins the production source paths without exposing new action inputs', () => {
    expect(action).toContain('        APT_SOURCES: /etc/apt/sources.list.d/ubuntu.sources\n');
    expect(action).toContain('        APT_MIRRORS: /etc/apt/apt-mirrors.txt\n');
    expect(action.split('runs:')[0]).not.toMatch(/APT_SOURCES|APT_MIRRORS/);
  });

  it('leaves the list unchanged on a healthy first install', () => {
    const { result, lines, mirrorBytes, beforeInstall } = run();
    expect(result.status).toBe(0);
    expect(mirrorBytes()).toBe(mirrors);
    expect(beforeInstall(1)).toBe(mirrors);
    expect(lines('mirror-edits')).toEqual([]);
    expect(result.stdout).not.toContain('apt-install:');
  });

  it.each(['1', '2'])(
    'removes only Azure before the second install after %s timeout(s)',
    (count) => {
      const { result, lines, mirrorBytes, beforeInstall, list } = run('install', count);
      expect(result.status).toBe(0);
      expect(beforeInstall(1)).toBe(mirrors);
      expect(beforeInstall(2)).toBe(remainingMirrors);
      if (count === '2') expect(beforeInstall(3)).toBe(remainingMirrors);
      expect(mirrorBytes()).toBe(remainingMirrors);
      expect(lines('mirror-edits')).toEqual([`-i 1d ${list}`]);
      expect(result.stdout).toContain(
        `removed http://azure.archive.ubuntu.com/ubuntu/ from ${list}`
      );
      expect(lines('apt')).toEqual([update, install]);
    }
  );

  it('retains fallback and returns the last timeout after all three install attempts', () => {
    const { result, lines, mirrorBytes, beforeInstall } = run('install', '99');
    expect(result.status).toBe(124);
    expect(beforeInstall(1)).toBe(mirrors);
    expect(beforeInstall(2)).toBe(remainingMirrors);
    expect(beforeInstall(3)).toBe(remainingMirrors);
    expect(mirrorBytes()).toBe(remainingMirrors);
    expect(lines('mirror-edits')).toHaveLength(1);
    expect(lines('install')).toHaveLength(3);
    expect(lines('sleeps')).toEqual(['5', '10']);
  });

  it.each(['2', '99'])('never switches for non-timeout install failures (%s failures)', (count) => {
    const { result, lines, mirrorBytes } = run('install', count, { failStatus: '100' });
    expect(result.status).toBe(count === '2' ? 0 : 100);
    expect(mirrorBytes()).toBe(mirrors);
    expect(lines('mirror-edits')).toEqual([]);
    expect(lines('install')).toHaveLength(3);
    expect(lines('sleeps')).toEqual(['5', '10']);
  });

  it('does not arm fallback for a later timeout after a first non-timeout failure', () => {
    const { result, lines, mirrorBytes } = run('install', '2', { firstFailStatus: '100' });
    expect(result.status).toBe(0);
    expect(lines('install')).toHaveLength(3);
    expect(lines('mirror-edits')).toEqual([]);
    expect(mirrorBytes()).toBe(mirrors);
    expect(result.stderr).toContain('attempt 1 of 3 with status 100');
    expect(result.stderr).toContain('attempt 2 of 3 with status 124');
  });

  it('does not arm fallback after update timeouts', () => {
    const { result, lines, mirrorBytes } = run('update', '2');
    expect(result.status).toBe(0);
    expect(mirrorBytes()).toBe(mirrors);
    expect(lines('mirror-edits')).toEqual([]);
  });

  it.each([
    ['missing sources', { missingSources: true }],
    ['source reader failure', { awkFailure: '13' }],
    ['absent list', { mirrors: null }],
    ['empty list', { mirrors: '' }],
    ['no archive candidate', { mirrors: azure }],
    ['custom priority', { mirrors: mirrors.replace('priority:2', 'priority:9') }],
    ['ports mirror', { mirrors: 'http://ports.ubuntu.com/ubuntu-ports/\tpriority:1\n' }],
    [
      'direct Azure URI',
      {
        sources: () =>
          sourcesFor('unused').replaceAll(
            'mirror+file:unused',
            'http://azure.archive.ubuntu.com/ubuntu/'
          ),
      },
    ],
    ['different referenced list', { sources: () => sourcesFor('/etc/apt/another-list.txt') }],
    [
      'unexpected URI field casing',
      { sources: (list: string) => sourcesFor(list).replace('URIs:', 'uris:') },
    ],
    [
      'mixed URIs',
      {
        sources: (list: string) =>
          sourcesFor(list).replace('Suites:', 'URIs: https://archive.ubuntu.com/ubuntu/\nSuites:'),
      },
    ],
    [
      'continuation',
      {
        sources: (list: string) =>
          sourcesFor(list).replace('Suites:', ' https://archive.ubuntu.com/ubuntu/\nSuites:'),
      },
    ],
  ] as const)(
    'reports an explicit no-op for %s and preserves bytes/status/retry policy',
    (_, input) => {
      const { result, lines, mirrorBytes, originalMirrors } = run('install', '99', input);
      expect(result.status).toBe(124);
      expect(mirrorBytes()).toBe(originalMirrors);
      expect(lines('mirror-edits')).toEqual([]);
      expect(result.stderr.match(/apt-install: mirror fallback no-op:/g)).toHaveLength(1);
      expect(lines('install')).toHaveLength(3);
      expect(lines('sleeps')).toEqual(['5', '10']);
    }
  );

  it('retains a mirror-write failure instead of running apt or reporting fallback success', () => {
    const { result, lines, mirrorBytes } = run('install', '99', { sedFailure: '13' });
    expect(result.status).toBe(13);
    expect(lines('install')).toHaveLength(1);
    expect(lines('mirror-edits')).toHaveLength(2);
    expect(mirrorBytes()).toBe(mirrors);
    expect(result.stdout).not.toContain('apt-install: removed');
    expect(result.stderr).toContain('attempt 3 of 3 with status 13');
  });

  it('removes Chrome sources and preserves package arguments with bounded update and install', () => {
    const { result, lines } = run();
    expect(result.status).toBe(0);
    expect(lines('removals')).toEqual(['-f /etc/apt/sources.list.d/google-chrome.list']);
    expect(lines('apt')).toEqual([update, install]);
    expect(lines('sudo')).toEqual([
      'rm -f /etc/apt/sources.list.d/google-chrome.list',
      `timeout --verbose --kill-after=10s 60s apt-get ${update}`,
      `timeout --verbose --kill-after=10s 120s apt-get ${install}`,
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

  it('bounds both phases independently when each recovers on its third attempt', () => {
    const { result, lines } = run('both', '2');
    expect(result.status).toBe(0);
    expect(lines('update')).toHaveLength(3);
    expect(lines('install')).toHaveLength(3);
    expect(lines('timeouts')).toEqual([
      ...Array(3).fill(`--verbose --kill-after=10s 60s apt-get ${update}`),
      ...Array(3).fill(`--verbose --kill-after=10s 120s apt-get ${install}`),
    ]);
    expect(lines('sleeps')).toEqual(['5', '10', '5', '10']);
    expect(lines('apt')).toEqual([update, install]);
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
