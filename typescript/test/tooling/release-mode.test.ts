import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { releaseMode } from '../../scripts/release-mode.mjs';

const script = fileURLToPath(new URL('../../scripts/release-mode.mjs', import.meta.url));

describe('release run mode', () => {
  it('keeps a push a dry run until both App secrets exist, then makes it live', () => {
    expect(releaseMode({ event: 'push', hasSecrets: false }).live).toBe(false);
    expect(releaseMode({ event: 'push', hasSecrets: true }).live).toBe(true);
  });

  it('lets a dispatch choose, and never falls back to the other mode', () => {
    for (const hasSecrets of [true, false]) {
      expect(releaseMode({ event: 'workflow_dispatch', dryRun: 'true', hasSecrets }).live).toBe(
        false
      );
    }
    expect(
      releaseMode({ event: 'workflow_dispatch', dryRun: 'false', hasSecrets: true }).live
    ).toBe(true);
    expect(() =>
      releaseMode({ event: 'workflow_dispatch', dryRun: 'false', hasSecrets: false })
    ).toThrow('RELEASE_APP_ID and RELEASE_APP_PRIVATE_KEY');
  });

  it('refuses a dispatch without a boolean dry_run and any other event', () => {
    for (const dryRun of [undefined, '', 'yes', 'True']) {
      expect(
        () => releaseMode({ event: 'workflow_dispatch', dryRun, hasSecrets: true }),
        String(dryRun)
      ).toThrow('needs dry_run to be true or false');
    }
    for (const event of ['pull_request', 'schedule', '']) {
      expect(() => releaseMode({ event, hasSecrets: true }), event).toThrow('does not start on');
    }
  });
});

describe('release-mode.mjs', () => {
  function run(environment: Record<string, string>) {
    const directory = mkdtempSync(path.join(os.tmpdir(), 'release-mode-'));
    try {
      const output = path.join(directory, 'output');
      const summary = path.join(directory, 'summary');
      writeFileSync(output, '');
      writeFileSync(summary, '');
      const result = spawnSync('node', [script], {
        encoding: 'utf8',
        env: {
          PATH: process.env.PATH,
          GITHUB_OUTPUT: output,
          GITHUB_STEP_SUMMARY: summary,
          ...environment,
        },
        timeout: 10_000,
      });
      return {
        status: result.status,
        stderr: result.stderr,
        output: readFileSync(output, 'utf8'),
        summary: readFileSync(summary, 'utf8'),
      };
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }

  it('treats an empty secret as missing and never prints a secret', () => {
    const empty = run({ EVENT: 'push', APP_ID: '', APP_KEY: 'key-material' });
    expect(empty.output).toBe('live=false\n');
    const live = run({ EVENT: 'push', APP_ID: '12345', APP_KEY: 'key-material' });
    expect(live.output).toBe('live=true\n');
    for (const result of [empty, live]) {
      expect(result.stderr + result.summary).not.toContain('key-material');
      expect(result.stderr + result.summary).not.toContain('12345');
    }
  });

  it('reports the mode in the run summary and fails a live dispatch without secrets', () => {
    const dry = run({ EVENT: 'workflow_dispatch', DRY_RUN: 'true' });
    expect(dry.status).toBe(0);
    expect(dry.summary).toBe('Dry release run: a dry run was requested.\n');
    const refused = run({ EVENT: 'workflow_dispatch', DRY_RUN: 'false', APP_ID: '1' });
    expect(refused.status).toBe(1);
    expect(refused.output).toBe('');
    expect(refused.stderr).toContain('RELEASE_APP_ID and RELEASE_APP_PRIVATE_KEY');
  });
});
