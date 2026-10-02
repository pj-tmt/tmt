import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const script = fileURLToPath(new URL('../../../scripts/retry-command.sh', import.meta.url));

let root: string;
let bin: string;

/**
 * The fake tools are written once, because the first run of a new executable
 * is slow on some hosts, and they take their behavior from the environment so
 * that each case only needs its own log directory.
 */
beforeAll(() => {
  root = mkdtempSync(path.join(os.tmpdir(), 'retry-command-'));
  bin = path.join(root, 'bin');
  mkdirSync(bin);
  const tool = (name: string, body: string) => {
    writeExecutable(path.join(bin, name), `#!/bin/sh\n${body}\n`, 0o755);
  };
  // Records each wait, so no case sleeps or depends on the host's `sleep`.
  tool('sleep', 'echo "$*" >> "$LOG_DIR/sleeps.log"');
  // Fails with $FAKE_STATUS on its first $FAKE_FAILURES runs, then succeeds;
  // records its arguments, one line per run.
  tool(
    'corepack',
    `echo "$*" >> "$LOG_DIR/runs.log"
count=$(wc -l < "$LOG_DIR/runs.log" | tr -d ' ')
[ "$count" -gt "\${FAKE_FAILURES:-0}" ] || { echo "network glitch $count" >&2; exit "\${FAKE_STATUS:-1}"; }`
  );
  // Writes each argument on its own line.
  tool(
    'record',
    'for argument in "$@"; do printf \'%s\\n\' "$argument" >> "$LOG_DIR/seen.log"; done'
  );
});

afterAll(() => rmSync(root, { recursive: true, force: true }));

interface Case {
  dir: string;
  lines(name: string): string[];
  retry(args: string[], environment?: Record<string, string>): ReturnType<typeof run>;
}

function run(caseDir: string, args: string[], environment: Record<string, string>) {
  const result = spawnSync('/bin/sh', [script, ...args], {
    env: {
      PATH: `${bin}${path.delimiter}/usr/bin:/bin`,
      LOG_DIR: caseDir,
      ...environment,
    },
    encoding: 'utf8',
    timeout: 10_000,
  });
  return { status: result.status, stderr: result.stderr };
}

function newCase(): Case {
  const dir = mkdtempSync(path.join(root, 'case-'));
  return {
    dir,
    lines: (name) => {
      const log = path.join(dir, name);
      return existsSync(log) ? readFileSync(log, 'utf8').split('\n').slice(0, -1) : [];
    },
    retry: (args, environment = {}) => run(dir, args, environment),
  };
}

describe('scripts/retry-command.sh', () => {
  it('runs a command that succeeds once, without waiting or reporting', () => {
    const c = newCase();
    const result = c.retry(['3', '5', 'corepack', 'prepare', 'pnpm@10.33.0', '--activate']);
    expect(result.status).toBe(0);
    expect(c.lines('runs.log')).toEqual(['prepare pnpm@10.33.0 --activate']);
    expect(c.lines('sleeps.log')).toEqual([]);
    expect(result.stderr).toBe('');
  });

  it('retries a command that fails twice, doubling the wait, and then succeeds', () => {
    const c = newCase();
    const result = c.retry(['3', '5', 'corepack', 'prepare'], { FAKE_FAILURES: '2' });
    expect(result.status).toBe(0);
    expect(c.lines('runs.log')).toHaveLength(3);
    expect(c.lines('sleeps.log')).toEqual(['5', '10']);
    expect(result.stderr).toContain('corepack failed on attempt 1 of 3 with status 1.');
    expect(result.stderr).toContain('corepack failed on attempt 2 of 3 with status 1.');
    expect(result.stderr).not.toContain('attempt 3');
  });

  it('stops after the bound and exits with the last status, reporting every attempt', () => {
    const c = newCase();
    const result = c.retry(['3', '5', 'corepack', 'prepare'], {
      FAKE_FAILURES: '99',
      FAKE_STATUS: '7',
    });
    expect(result.status).toBe(7);
    expect(c.lines('runs.log')).toHaveLength(3);
    // No wait after the last attempt.
    expect(c.lines('sleeps.log')).toEqual(['5', '10']);
    for (const attempt of [1, 2, 3]) {
      expect(result.stderr).toContain(`corepack failed on attempt ${attempt} of 3 with status 7.`);
    }
    expect(result.stderr).toContain('network glitch 3');
  });

  it('runs a single attempt without any wait when the bound is one', () => {
    const c = newCase();
    const result = c.retry(['1', '5', 'corepack'], { FAKE_FAILURES: '99', FAKE_STATUS: '3' });
    expect(result.status).toBe(3);
    expect(c.lines('runs.log')).toHaveLength(1);
    expect(c.lines('sleeps.log')).toEqual([]);
  });

  it('passes every argument through unchanged, including spaces and quotes', () => {
    const c = newCase();
    const awkward = ['two words', "it's", '"quoted"', '$HOME', '*', ''];
    const result = c.retry(['2', '0', 'record', ...awkward]);
    expect(result.status).toBe(0);
    expect(c.lines('seen.log')).toEqual(awkward);
  });

  it('rejects malformed invocations without running anything', () => {
    const c = newCase();
    for (const args of [
      [],
      ['3', '5'],
      ['0', '5', 'corepack'],
      ['x', '5', 'corepack'],
      ['3', 'x', 'corepack'],
      ['3', '-1', 'corepack'],
      ['-3', '5', 'corepack'],
      ['', '5', 'corepack'],
    ]) {
      const result = c.retry(args);
      expect(result.status, JSON.stringify(args)).toBe(2);
      expect(result.stderr).toContain('usage: retry-command.sh');
    }
    expect(c.lines('runs.log')).toEqual([]);
    expect(c.lines('sleeps.log')).toEqual([]);
  });
});
