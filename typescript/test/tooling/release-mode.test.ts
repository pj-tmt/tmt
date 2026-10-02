import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { releaseMode } from '../../scripts/release-mode.mjs';

const script = fileURLToPath(new URL('../../scripts/release-mode.mjs', import.meta.url));
const MAIN = 'refs/heads/main';
const BRANCH = 'refs/heads/feature';

describe('release run mode', () => {
  it('keeps a push a dry run until both App secrets exist, then makes it live', () => {
    expect(releaseMode({ event: 'push', ref: MAIN, hasSecrets: false }).live).toBe(false);
    expect(releaseMode({ event: 'push', ref: MAIN, hasSecrets: true }).live).toBe(true);
  });

  it('lets a dispatch choose, and never falls back to the other mode', () => {
    for (const hasSecrets of [true, false]) {
      expect(
        releaseMode({ event: 'workflow_dispatch', ref: MAIN, dryRun: 'true', hasSecrets }).live
      ).toBe(false);
    }
    expect(
      releaseMode({ event: 'workflow_dispatch', ref: MAIN, dryRun: 'false', hasSecrets: true }).live
    ).toBe(true);
    expect(() =>
      releaseMode({ event: 'workflow_dispatch', ref: MAIN, dryRun: 'false', hasSecrets: false })
    ).toThrow('RELEASE_APP_ID and RELEASE_APP_PRIVATE_KEY');
  });

  it('is live only on main, whatever the event and the secrets', () => {
    for (const ref of [BRANCH, 'refs/pull/1/merge', 'refs/tags/v1', '', undefined]) {
      expect(
        () => releaseMode({ event: 'workflow_dispatch', ref, dryRun: 'false', hasSecrets: true }),
        `dispatch ${ref}`
      ).toThrow('only allowed on refs/heads/main');
      expect(
        () => releaseMode({ event: 'workflow_dispatch', ref, dryRun: 'false', hasSecrets: false }),
        `dispatch without secrets ${ref}`
      ).toThrow('only allowed on refs/heads/main');
      expect(() => releaseMode({ event: 'push', ref, hasSecrets: true }), `push ${ref}`).toThrow(
        'only allowed on refs/heads/main'
      );
    }
    // A dry run changes nothing, so any ref may ask for one.
    expect(
      releaseMode({ event: 'workflow_dispatch', ref: BRANCH, dryRun: 'true', hasSecrets: true })
        .live
    ).toBe(false);
    expect(releaseMode({ event: 'push', ref: BRANCH, hasSecrets: false }).live).toBe(false);
  });

  it('refuses a dispatch without a boolean dry_run and any other event', () => {
    for (const dryRun of [undefined, '', 'yes', 'True']) {
      expect(
        () => releaseMode({ event: 'workflow_dispatch', ref: MAIN, dryRun, hasSecrets: true }),
        String(dryRun)
      ).toThrow('needs dry_run to be true or false');
    }
    for (const event of ['pull_request', 'schedule', '']) {
      expect(() => releaseMode({ event, ref: MAIN, hasSecrets: true }), event).toThrow(
        'does not start on'
      );
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

  it('reads only whether the secrets exist, and only "true" counts', () => {
    for (const present of ['', 'false', '1', 'True']) {
      const result = run({ EVENT: 'push', REF: MAIN, HAS_APP_SECRETS: present });
      expect(result.output, present).toBe('live=false\n');
    }
    expect(run({ EVENT: 'push', REF: MAIN }).output).toBe('live=false\n');
    expect(run({ EVENT: 'push', REF: MAIN, HAS_APP_SECRETS: 'true' }).output).toBe('live=true\n');
  });

  it('never reads or prints the App credentials, even when they reach its environment', () => {
    const result = run({
      EVENT: 'push',
      REF: MAIN,
      APP_ID: '12345',
      APP_KEY: 'key-material',
      RELEASE_APP_ID: '12345',
      RELEASE_APP_PRIVATE_KEY: 'key-material',
    });
    expect(result.output).toBe('live=false\n');
    expect(result.stderr + result.summary).not.toContain('key-material');
    expect(result.stderr + result.summary).not.toContain('12345');
  });

  it('reports the mode in the run summary and fails a live run that is not allowed', () => {
    const dry = run({ EVENT: 'workflow_dispatch', REF: BRANCH, DRY_RUN: 'true' });
    expect(dry.status).toBe(0);
    expect(dry.summary).toBe('Dry release run: a dry run was requested.\n');
    const withoutSecrets = run({ EVENT: 'workflow_dispatch', REF: MAIN, DRY_RUN: 'false' });
    expect(withoutSecrets.status).toBe(1);
    expect(withoutSecrets.output).toBe('');
    expect(withoutSecrets.stderr).toContain('RELEASE_APP_ID and RELEASE_APP_PRIVATE_KEY');
    const elsewhere = run({
      EVENT: 'workflow_dispatch',
      REF: BRANCH,
      DRY_RUN: 'false',
      HAS_APP_SECRETS: 'true',
    });
    expect(elsewhere.status).toBe(1);
    expect(elsewhere.output).toBe('');
    expect(elsewhere.stderr).toContain(`not on ${BRANCH}`);
  });
});
