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
  it.each(['push', 'schedule'])('makes %s live on main with no App dependency', (event) => {
    expect(releaseMode({ event, ref: MAIN }).live).toBe(true);
  });
  it('lets a dispatch choose without silently falling back', () => {
    expect(releaseMode({ event: 'workflow_dispatch', ref: MAIN, dryRun: 'true' }).live).toBe(false);
    expect(releaseMode({ event: 'workflow_dispatch', ref: MAIN, dryRun: 'false' }).live).toBe(true);
  });
  it('is live only on main for every supported event', () => {
    for (const ref of [BRANCH, 'refs/pull/1/merge', 'refs/tags/v1', '', undefined]) {
      for (const event of ['push', 'schedule', 'workflow_dispatch']) {
        expect(() => releaseMode({ event, ref, dryRun: 'false' })).toThrow(
          'only allowed on refs/heads/main'
        );
      }
    }
    expect(releaseMode({ event: 'workflow_dispatch', ref: BRANCH, dryRun: 'true' }).live).toBe(
      false
    );
  });
  it('refuses a dispatch without a boolean dry_run and unsupported events', () => {
    for (const dryRun of [undefined, '', 'yes', 'True'])
      expect(() => releaseMode({ event: 'workflow_dispatch', ref: MAIN, dryRun })).toThrow(
        'needs dry_run to be true or false'
      );
    for (const event of ['pull_request', 'merge_group', ''])
      expect(() => releaseMode({ event, ref: MAIN })).toThrow('does not start on');
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

  it.each(['push', 'schedule'])('reports automatic %s as live', (event) => {
    const result = run({ EVENT: event, REF: MAIN });
    expect(result.status).toBe(0);
    expect(result.output).toBe('live=true\n');
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
    expect(result.output).toBe('live=true\n');
    expect(result.stderr + result.summary).not.toContain('key-material');
    expect(result.stderr + result.summary).not.toContain('12345');
  });

  it('reports the mode in the run summary and fails a live run that is not allowed', () => {
    const dry = run({ EVENT: 'workflow_dispatch', REF: BRANCH, DRY_RUN: 'true' });
    expect(dry.status).toBe(0);
    expect(dry.summary).toBe('Dry release run: a dry run was requested.\n');
    const elsewhere = run({
      EVENT: 'workflow_dispatch',
      REF: BRANCH,
      DRY_RUN: 'false',
    });
    expect(elsewhere.status).toBe(1);
    expect(elsewhere.output).toBe('');
    expect(elsewhere.stderr).toContain(`not on ${BRANCH}`);
  });
});
