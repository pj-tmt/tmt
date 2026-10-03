import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { pushCadence, releaseMode } from '../../scripts/release-mode.mjs';

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

const NOW = Date.parse('2026-10-04T12:00:00Z');
const cadenceInputs = { repository: 'pj-tmt/tmt', token: 'test-token', runId: '999', now: NOW };
function workflowRun(id = 1, ageMinutes = 10, event = 'schedule') {
  return {
    id,
    event,
    head_branch: 'main',
    status: 'completed',
    conclusion: 'success',
    run_attempt: 1,
    run_started_at: new Date(NOW - ageMinutes * 60_000).toISOString(),
    display_title: 'Release cut (live dispatch)',
  };
}
function pushJobs(admitted: boolean) {
  return {
    total_count: 1,
    jobs: [
      {
        name: 'Cut releases from main',
        steps: [{ name: 'Admit live release cut', conclusion: admitted ? 'success' : 'skipped' }],
      },
    ],
  };
}

describe('push cut cadence admission', () => {
  function gate(runs: ReturnType<typeof workflowRun>[], jobs = pushJobs(true)) {
    const calls: string[] = [];
    const mode = pushCadence(cadenceInputs, (executable, args, options) => {
      expect(executable).toBe('gh');
      expect(args.slice(-2)).toEqual(['--method', 'GET']);
      expect(options.env.GH_TOKEN).toBe('test-token');
      calls.push(args[1]);
      return JSON.stringify(
        args[1].includes('/jobs?') ? jobs : { total_count: runs.length, workflow_runs: runs }
      );
    });
    return { mode, calls };
  }

  it.each(['schedule', 'workflow_dispatch', 'push'])(
    'skips after a recent live %s cut',
    (event) => {
      expect(gate([workflowRun(1, 54, event)]).mode).toEqual({ live: false, reason: 'cadence' });
    }
  );
  it.each([55, 56, 120])('admits a push when the last live start is %s minutes old', (age) => {
    expect(gate([workflowRun(1, age)]).mode.live).toBe(true);
  });
  it('admits the first cut and excludes the current run from its own history', () => {
    expect(gate([]).mode.live).toBe(true);
    expect(gate([workflowRun(999, 0, 'push')]).mode.live).toBe(true);
  });
  it('does not count dry dispatches or pushes skipped by cadence', () => {
    const dry = {
      ...workflowRun(2, 1, 'workflow_dispatch'),
      display_title: 'Release cut (dry dispatch)',
    };
    expect(gate([dry, workflowRun(3, 60, 'push')]).mode.live).toBe(true);
    const skipped = gate([workflowRun(4, 1, 'push')], pushJobs(false));
    expect(skipped.mode.live).toBe(true);
    expect(skipped.calls[1]).toContain('actions/runs/4/attempts/1/jobs?');
    // A skipped push cannot hide an earlier live cut still inside the interval.
    expect(gate([workflowRun(4, 1, 'push'), workflowRun(5, 30)], pushJobs(false)).mode.live).toBe(
      false
    );
  });
  it('counts failed admitted attempts, but not a pending run that has not started', () => {
    expect(gate([{ ...workflowRun(), conclusion: 'failure' }]).mode.live).toBe(false);
    expect(gate([{ ...workflowRun(), status: 'queued' }]).mode.live).toBe(true);
  });
  it('fails closed on unknown recent dispatch mode, but not expired legacy history', () => {
    const legacy = { ...workflowRun(1, 1, 'workflow_dispatch'), display_title: 'Release' };
    expect(() => gate([legacy])).toThrow('no dry/live mode evidence');
    expect(gate([{ ...legacy, run_started_at: workflowRun(1, 60).run_started_at }]).mode.live).toBe(
      true
    );
  });
  it('does not depend on API ordering to find a recent live cut', () => {
    expect(gate([workflowRun(1, 70), workflowRun(2, 5)]).mode.live).toBe(false);
  });
  it('fails closed on REST errors, including admission job reads', () => {
    expect(() =>
      pushCadence(cadenceInputs, () => {
        throw new Error('API unavailable');
      })
    ).toThrow('Release cadence history unavailable; refusing live push cut: API unavailable');
    expect(() =>
      pushCadence(cadenceInputs, (_exe, args) => {
        if (args[1].includes('/jobs?')) throw new Error('job API unavailable');
        return JSON.stringify({ total_count: 1, workflow_runs: [workflowRun(1, 1, 'push')] });
      })
    ).toThrow('refusing live push cut: job API unavailable');
  });
  it.each([
    {},
    { total_count: 1, workflow_runs: [] },
    { total_count: 0, workflow_runs: [workflowRun()] },
    { total_count: 2, workflow_runs: [workflowRun(1, 60), workflowRun(1, 60)] },
    { total_count: 1, workflow_runs: [{ ...workflowRun(), run_started_at: null }] },
  ])('refuses malformed or incomplete history: %j', (response) => {
    expect(() => pushCadence(cadenceInputs, () => JSON.stringify(response))).toThrow(
      'refusing live push cut'
    );
  });
  it('fails closed when a recent push has no admission job evidence', () => {
    expect(() => gate([workflowRun(1, 1, 'push')], { total_count: 0, jobs: [] })).toThrow(
      'no cut admission evidence'
    );
  });
  it('follows run pagination, including an old-created run started recently on a later page', () => {
    const calls: string[] = [];
    const old = Array.from({ length: 100 }, (_, i) => workflowRun(i + 1, 60));
    const mode = pushCadence(cadenceInputs, (_exe, args) => {
      calls.push(args[1]);
      return JSON.stringify({
        total_count: 101,
        workflow_runs: args[1].endsWith('page=1') ? old : [workflowRun(101, 5)],
      });
    });
    expect(mode.live).toBe(false);
    expect(calls).toHaveLength(2);
    expect(calls[1]).toContain('page=2');
    expect(calls.join(' ')).not.toContain('created=');
  });
});

describe('release-mode.mjs', () => {
  function run(environment: Record<string, string>, response?: object | 'error') {
    const directory = mkdtempSync(path.join(os.tmpdir(), 'release-mode-'));
    try {
      const output = path.join(directory, 'output');
      const summary = path.join(directory, 'summary');
      writeFileSync(output, '');
      writeFileSync(summary, '');
      if (response)
        writeExecutable(
          path.join(directory, 'gh'),
          `#!${process.execPath}\n${response === 'error' ? "process.stderr.write('API unavailable\\n'); process.exit(1);" : `console.log(${JSON.stringify(JSON.stringify(response))});`}\n`,
          0o700
        );

      const result = spawnSync('node', [script], {
        encoding: 'utf8',
        env: {
          PATH: `${directory}${path.delimiter}${process.env.PATH ?? ''}`,
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

  it.each(['schedule'])('reports automatic %s as live', (event) => {
    const result = run({ EVENT: event, REF: MAIN });
    expect(result.status).toBe(0);
    expect(result.output).toBe('live=true\nreason=hourly cuts and recovery\n');
  });

  it('exits successfully as cadence with no live admission output', () => {
    const result = run(
      {
        EVENT: 'push',
        REF: MAIN,
        GITHUB_REPOSITORY: 'pj-tmt/tmt',
        GH_TOKEN: 'secret',
        GITHUB_RUN_ID: '999',
      },
      {
        total_count: 1,
        workflow_runs: [{ ...workflowRun(), run_started_at: new Date().toISOString() }],
      }
    );
    expect(result.status).toBe(0);
    expect(result.output).toBe('live=false\nreason=cadence\n');
    expect(result.summary).toContain('Release cut skipped: cadence');
  });
  it('fails closed without any admission output when the REST command fails', () => {
    const result = run(
      {
        EVENT: 'push',
        REF: MAIN,
        GITHUB_REPOSITORY: 'pj-tmt/tmt',
        GH_TOKEN: 'secret',
        GITHUB_RUN_ID: '999',
      },
      'error'
    );
    expect(result.status).toBe(1);
    expect(result.output).toBe('');
    expect(result.stderr).toContain('refusing live push cut');
    expect(result.stderr).not.toContain('secret');
  });
  it('leaves schedule and manual dispatch independent of cadence API availability', () => {
    for (const environment of [
      { EVENT: 'schedule', REF: MAIN, DRY_RUN: '' },
      { EVENT: 'workflow_dispatch', REF: MAIN, DRY_RUN: 'false' },
      { EVENT: 'workflow_dispatch', REF: MAIN, DRY_RUN: 'true' },
    ]) {
      expect(run(environment, 'error').status).toBe(0);
    }
  });

  it('never reads or prints the App credentials, even when they reach its environment', () => {
    const result = run({
      EVENT: 'schedule',
      REF: MAIN,
      APP_ID: '12345',
      APP_KEY: 'key-material',
      RELEASE_APP_ID: '12345',
      RELEASE_APP_PRIVATE_KEY: 'key-material',
    });
    expect(result.output).toBe('live=true\nreason=hourly cuts and recovery\n');
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
