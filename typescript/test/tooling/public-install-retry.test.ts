import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  planSmokeRetry,
  readRetryHosts,
  reportSmokeRetry,
  waitForSmokeReset,
  parseRetryRequest,
  validateRetrySource,
  ghSmokeRetryApi,
  TARGETS as RETRY_TARGETS,
  type RetryHost,
  type RetrySource,
} from '../../scripts/public-install-retry.mjs';
import { parseRateLimitDiagnostic } from '../../scripts/verify-public-install.mjs';
import { ghPublishApi } from '../../scripts/release-publish.mjs';
import { writeExecutable } from '../support/executable-fixture.mjs';

const NOW = 1791000000000;
const TAG = 'v5.0.0-alpha.43';
const TARGETS = [
  'aarch64-apple-darwin',
  'x86_64-apple-darwin',
  'aarch64-unknown-linux-musl',
  'x86_64-unknown-linux-musl',
];
const cause = (epoch = 1791000292) =>
  `GitHub API rate limit: reset/earliest retry time 2026-10-03 4:04:52.0 +00:00:00 (UTC epoch ${epoch}); the required wait exceeds the remaining deadline. Retry later or optionally set GITHUB_TOKEN.`;
const limit = (epoch = 1791000292) => ({
  check: 'tmt upgrade',
  reason: 'Public install infrastructure: wait bound exceeded (300 seconds)',
  infrastructure: 'github-api-rate-limit' as const,
  rateLimit: { diagnostic: cause(epoch), resetAtMs: epoch * 1000 },
});
const hosts = (): RetryHost[] =>
  TARGETS.map((target, i) => ({
    product: 'cli',
    tag: TAG,
    target,
    runAttempt: 1,
    failed: i === 0 ? [limit()] : [],
  }));
const plan = (input = hosts()) => planSmokeRetry(input, { now: NOW });
const ORIGINAL = 'https://github.com/pj-tmt/tmt/actions/runs/37091818291';
const RETRY = 'https://github.com/pj-tmt/tmt/actions/runs/2';
function reporter() {
  const calls: { kind: string; number?: number; title?: string; body?: string }[] = [];
  return {
    calls,
    api: {
      openIssue: (title: string) => {
        calls.push({ kind: 'find', title });
        return 1272;
      },
      createIssue: (title: string, body: string) => {
        calls.push({ kind: 'create', title, body });
        return 1272;
      },
      commentIssue: (number: number, body: string) => {
        calls.push({ kind: 'comment', number, body });
      },
      closeIssue: (number: number) => {
        calls.push({ kind: 'close', number });
      },
      latestFailureRun: () => ORIGINAL,
    },
  };
}

describe('deferred anonymous public-install retry', () => {
  it('parses the incident reset epoch and selects only the affected target on its matching host', () => {
    expect(parseRateLimitDiagnostic(cause())?.resetAtMs).toBe(1791000292000);
    const selected = plan();
    expect(selected.matrix.include).toEqual([
      { product: 'cli', tag: TAG, target: TARGETS[0], runner: 'macos-15' },
    ]);
    expect(selected.waitUntilMs).toBe(1791000293000);
    expect(selected.groups[0].hosts).toHaveLength(4);
  });

  it('waits once until the last selected reset, then allows the single target re-proofs', async () => {
    const input = hosts();
    input[2].failed = [limit(1791000600)];
    const selected = plan(input);
    expect(selected.matrix.include.map(({ target }) => target)).toEqual([TARGETS[0], TARGETS[2]]);
    let time = NOW;
    const waits: number[] = [];
    await waitForSmokeReset(selected, {
      now: () => time,
      wait: async (ms) => {
        waits.push(ms);
        time += ms;
      },
    });
    expect(waits).toEqual([601000]);
    expect(time).toBe(selected.waitUntilMs);
  });

  it('skips a reset beyond 60 minutes and refuses an excessive or premature timer', async () => {
    const input = hosts();
    input[0].failed = [limit(NOW / 1000 + 3600)];
    expect(plan(input).matrix.include).toEqual([]);
    expect(plan(input).skipped[0].reason).toBe('reset wait exceeds 60 minutes');
    input[0].failed = [limit(NOW / 1000 + 3599)];
    const atBound = plan(input);
    expect(atBound.matrix.include).toHaveLength(1);
    await expect(
      waitForSmokeReset(atBound, { now: () => NOW - 1, wait: async () => {} })
    ).rejects.toThrow('60 minutes');
    await expect(
      waitForSmokeReset(plan(), { now: () => NOW, wait: async () => {} })
    ).rejects.toThrow('has not elapsed');
  });

  it('does not wait again when the reset elapsed while the originating run finished', async () => {
    let waited = false;
    const selected = planSmokeRetry(hosts(), { now: NOW + 600000 });
    await waitForSmokeReset(selected, {
      now: () => NOW + 600000,
      wait: async () => {
        waited = true;
      },
    });
    expect(waited).toBe(false);
    expect(selected.matrix.include).toHaveLength(1);
  });

  it.each([
    { check: 'tmt upgrade', reason: 'HTTP 429' },
    { ...limit(), infrastructure: undefined },
    { ...limit(), check: 'install' },
    { ...limit(), rateLimit: undefined },
    { ...limit(), rateLimit: { diagnostic: 'HTTP 403: API rate limit exceeded', resetAtMs: NOW } },
  ])('never retries an unclassified result or an unrelated failing step: %j', (failure) => {
    const input = hosts();
    input[0].failed = [failure];
    expect(plan(input).matrix.include).toEqual([]);
  });

  it('retains missing and inconsistent reset timing rather than guessing a retry time', () => {
    for (const rateLimit of [
      {
        diagnostic: cause().replace(/2026[^;]+/, 'unavailable (missing or invalid timing header)'),
        resetAtMs: null,
      },
      { diagnostic: cause(), resetAtMs: NOW },
    ]) {
      const input = hosts();
      input[0].failed = [{ ...limit(), rateLimit }];
      expect(plan(input).matrix.include).toEqual([]);
      expect(plan(input).skipped[0].reason).toContain('unavailable or inconsistent');
    }
  });

  it('admits all four distinct known hosts before planning, including successful hosts', () => {
    expect(() => plan(hosts().slice(0, 3))).toThrow('Incomplete');
    expect(() => plan([...hosts(), hosts()[0]])).toThrow('Duplicate');
    expect(() => plan([{ ...hosts()[0], target: 'unknown' }])).toThrow('target');
    expect(() => plan([{ ...hosts()[0], tag: '../../untrusted' }])).toThrow();
    expect(plan([]).matrix.include).toEqual([]);
    expect(plan(hosts().map((host) => ({ ...host, failed: [] }))).matrix.include).toEqual([]);
  });

  it('keeps products and tags distinct while mapping an extension install to its runner', () => {
    const extension = hosts().map((host) => ({
      ...host,
      product: 'squad',
      tag: 'tmt-squad-v0.1.0-alpha.4',
      failed: host.failed.map((failure) => ({ ...failure, check: 'squad install' })),
    }));
    expect(plan([...hosts(), ...extension]).groups).toHaveLength(2);
    expect(plan(extension).matrix.include[0].product).toBe('squad');
  });

  it('keeps every target runner aligned with the original public smoke matrix', () => {
    const original = readFileSync(
      new URL('../../../.github/workflows/native-release-smoke.yml', import.meta.url),
      'utf8'
    );
    const expected = [...original.matchAll(/- target: (\S+)\n\s+runner: (\S+)/g)].map(
      ([, target, runner]) => ({ target, runner })
    );
    const selected = plan(hosts().map((host) => ({ ...host, failed: [limit()] })));
    expect(selected.matrix.include.map(({ target, runner }) => ({ target, runner }))).toEqual(
      expected
    );
  });

  it('pins the source smoke matrix to TARGETS, including every target and runner', () => {
    const original = readFileSync(
      new URL('../../../.github/workflows/native-release-smoke.yml', import.meta.url),
      'utf8'
    );
    const rows = [...original.matchAll(/- target: (\S+)\n\s+runner: (\S+)/g)].map(
      ([, target, runner]) => [target, runner]
    );
    expect(rows).toHaveLength(Object.keys(RETRY_TARGETS).length);
    expect(Object.fromEntries(rows)).toEqual(RETRY_TARGETS);
  });

  it('comments with both runs before closing the recovered infrastructure issue', () => {
    const { api, calls } = reporter();
    expect(
      reportSmokeRetry({
        api,
        plan: plan(),
        retried: [{ ...hosts()[0], failed: [] }],
        originalRunUrl: ORIGINAL,
        retryRunUrl: RETRY,
      })
    ).toEqual([{ tag: TAG, ok: true }]);
    expect(calls.map(({ kind }) => kind)).toEqual(['find', 'comment', 'close']);
    expect(calls[0].title).toContain('blocked by GitHub API rate limit');
    expect(calls[1].body).toContain(ORIGINAL);
    expect(calls[1].body).toContain(RETRY);
    expect(calls[1].body).toContain('original failed jobs remain failed');
  });

  it.each(['classified', 'real', 'missing'])(
    'reports a %s retry failure without closing the issue',
    (kind) => {
      const { api, calls } = reporter();
      const retried =
        kind === 'missing'
          ? []
          : [
              {
                ...hosts()[0],
                failed:
                  kind === 'classified'
                    ? [limit()]
                    : [{ check: 'install', reason: 'archive corrupt' }],
              },
            ];
      expect(
        reportSmokeRetry({
          api,
          plan: plan(),
          retried,
          originalRunUrl: ORIGINAL,
          retryRunUrl: RETRY,
        })
      ).toEqual([{ tag: TAG, ok: false }]);
      expect(calls.some(({ kind }) => kind === 'close')).toBe(false);
      expect(calls[0].title).toContain(
        kind === 'classified'
          ? 'blocked by GitHub API rate limit'
          : 'failed its post-publication checks'
      );
      expect(calls[1].body).toContain(ORIGINAL);
      expect(calls[1].body).toContain(RETRY);
    }
  );

  it('preserves an unclassified failure on another host even when the selected retry passes', () => {
    const input = hosts();
    input[1].failed = [{ check: 'managed skills', reason: 'different bytes' }];
    const { api, calls } = reporter();
    expect(
      reportSmokeRetry({
        api,
        plan: plan(input),
        retried: [{ ...input[0], failed: [] }],
        originalRunUrl: ORIGINAL,
        retryRunUrl: RETRY,
      })[0].ok
    ).toBe(false);
    expect(calls[0].title).toContain('failed its post-publication checks');
    expect(calls[1].body).toContain('different bytes');
    expect(calls.some(({ kind }) => kind === 'close')).toBe(false);
  });

  it('cannot close after an unexpected result, duplicate result or failed recovery comment', () => {
    const { api, calls } = reporter();
    const input = { api, plan: plan(), originalRunUrl: ORIGINAL, retryRunUrl: RETRY };
    expect(() => reportSmokeRetry({ ...input, retried: [hosts()[1]] })).toThrow('Unexpected');
    expect(() => reportSmokeRetry({ ...input, retried: [hosts()[0], hosts()[0]] })).toThrow(
      'duplicate'
    );
    expect(() =>
      reportSmokeRetry({
        ...input,
        retried: [{ ...hosts()[0], failed: [] }],
        api: {
          ...api,
          commentIssue: () => {
            throw new Error('HTTP 500');
          },
        },
      })
    ).toThrow('HTTP 500');
    expect(calls.some(({ kind }) => kind === 'close')).toBe(false);
  });

  it.each([null, 'https://github.com/pj-tmt/tmt/actions/runs/3'])(
    'leaves a newer or unidentified reported failure open: %s',
    (latestFailureRun) => {
      const { api, calls } = reporter();
      reportSmokeRetry({
        api: { ...api, latestFailureRun: () => latestFailureRun },
        plan: plan(),
        retried: [{ ...hosts()[0], failed: [] }],
        originalRunUrl: ORIGINAL,
        retryRunUrl: RETRY,
      });
      expect(calls.map(({ kind }) => kind)).toEqual(['find', 'comment']);
      expect(calls[1].body).toContain('issue remains open');
      expect(calls[1].body).toContain(ORIGINAL);
    }
  );

  it('reads bounded JSON with the product/tag/target identity bound to its artifact name', () => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'smoke-retry-'));
    const directory = path.join(root, `smoke-failures-cli-${TAG}-${TARGETS[0]}`);
    mkdirSync(directory);
    const file = path.join(directory, 'smoke-result.json');
    try {
      writeFileSync(file, JSON.stringify(hosts()[0]));
      expect(readRetryHosts(root)).toEqual([hosts()[0]]);
      expect(readRetryHosts(root, 'smoke-failures-', 2)).toEqual([]);
      writeFileSync(file, JSON.stringify({ ...hosts()[0], target: TARGETS[1] }));
      expect(() => readRetryHosts(root)).toThrow('identity mismatch');
      writeFileSync(file, 'x'.repeat(64 * 1024 + 1));
      expect(() => readRetryHosts(root)).toThrow('64 KiB');
      writeFileSync(file, 'malformed');
      expect(() => readRetryHosts(root)).toThrow();
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it('uses REST for issue discovery, comment and verified closure', () => {
    const calls: string[][] = [];
    const api = ghPublishApi({
      repository: 'pj-tmt/tmt',
      spawn: (_command, args) => {
        calls.push([...args]);
        const response = args[1].includes('?state=open')
          ? [
              {
                number: 1272,
                title: `Release ${TAG} public install blocked by GitHub API rate limit`,
              },
            ]
          : args[1].includes('comments?')
            ? []
            : args.includes('PATCH')
              ? { number: 1272, state: 'closed' }
              : { body: `Run: ${ORIGINAL}\nOpened by \`typescript/scripts/release-publish.mjs\`` };
        return { status: 0, stdout: JSON.stringify(response), stderr: '' };
      },
    });
    reportSmokeRetry({
      api,
      plan: plan(),
      retried: [{ ...hosts()[0], failed: [] }],
      originalRunUrl: ORIGINAL,
      retryRunUrl: RETRY,
    });
    expect(
      calls.every((args) => args[0] === 'api' && args[1].startsWith('repos/pj-tmt/tmt/'))
    ).toBe(true);
    expect(calls[3]).toContain(`repos/pj-tmt/tmt/issues/1272/comments`);
    expect(calls[4]).toContain('state=closed');
    expect(calls[4]).toContain('PATCH');
  });

  it('reads reporter-owned comments before closing and confirms the REST closure state', () => {
    const marker = 'Opened by `typescript/scripts/release-publish.mjs`';
    const api = ghPublishApi({
      repository: 'pj-tmt/tmt',
      spawn: (_command, args) => {
        const response = args[1].includes('comments?')
          ? [
              { body: `Run: ${RETRY}\n${marker}` },
              { body: 'A human comment without a failure report' },
            ]
          : { body: `Run: ${ORIGINAL}\n${marker}`, number: 1272, state: 'open' };
        return { status: 0, stdout: JSON.stringify(response), stderr: '' };
      },
    });
    expect(api.latestFailureRun(1272)).toBe(RETRY);
    expect(() => api.closeIssue(1272)).toThrow('closure was not confirmed');
  });
});

describe('independent smoke-retry workflow', () => {
  const workflow = readFileSync(
    new URL('../../../.github/workflows/native-release-smoke-retry.yml', import.meta.url),
    'utf8'
  );
  const retry = workflow.split('\n  retry:\n')[1].split('\n  report:\n')[0];
  const host = readFileSync(
    new URL('../../../.github/actions/public-install-smoke/action.yml', import.meta.url),
    'utf8'
  );
  it.each(['false', 'true', 'invalid'])(
    'passes the shared host arguments with retry=%s',
    (retryMode) => {
      const root = mkdtempSync(path.join(os.tmpdir(), 'public-install-host-'));
      const record = path.join(root, 'arguments');
      try {
        writeExecutable(
          path.join(root, 'node'),
          '#!/bin/sh\nprintf \'%s\\n\' "$@" > "$RECORD_FILE"\n',
          0o755
        );
        const script = host.split('      run: |\n')[1].replace(/^        /gm, '');
        const result = spawnSync('/bin/bash', ['-e', '-o', 'pipefail', '-c', script], {
          env: {
            PATH: `${root}:/usr/bin:/bin`,
            RECORD_FILE: record,
            PRODUCT: 'cli',
            RELEASE_TAG: TAG,
            TARGET: TARGETS[0],
            RETRY: retryMode,
            RUNNER_TEMP: root,
          },
          encoding: 'utf8',
          timeout: 10_000,
        });
        if (retryMode === 'invalid') {
          expect(result.status).toBe(1);
          expect(existsSync(record)).toBe(false);
        } else {
          expect(result.status).toBe(0);
          expect(readFileSync(record, 'utf8').trim().split('\n')).toEqual([
            'typescript/scripts/verify-public-install.mjs',
            '--product',
            'cli',
            '--tag',
            TAG,
            '--source',
            'release-source',
            '--target',
            TARGETS[0],
            ...(retryMode === 'true' ? ['--retry'] : []),
            '--result-file',
            path.join(root, 'smoke-result.json'),
          ]);
        }
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    }
  );
  it('uses an explicit main dispatch, independently of suppressed workflow_run events and release concurrency', () => {
    expect(workflow).toContain('workflow_dispatch:');
    expect(workflow).not.toContain('workflow_run:');
    expect(workflow).toContain("if: github.ref == 'refs/heads/main'");
    for (const input of ['source_run_id', 'source_run_attempt', 'product', 'tag', 'targets'])
      expect(workflow).toContain(`      ${input}:`);
    expect(workflow).toContain('group: public-install-retry-');
    expect(workflow).toContain('${{ inputs.product }}-${{ inputs.tag }}');
    expect(workflow).not.toMatch(
      /group: release-|workflow_call:|actions: write|contents: write|secrets\./
    );
    expect(workflow).toContain('timeout-minutes: 65');
    expect(workflow.match(/issues: write/g)).toHaveLength(1);
  });
  it('schedules only the shared infrastructure outcome with write access scoped to the dispatch job', () => {
    const smoke = readFileSync(
      new URL('../../../.github/workflows/native-release-smoke.yml', import.meta.url),
      'utf8'
    );
    const dispatch = smoke.split('\n  retry-dispatch:\n')[1];
    expect(dispatch).toContain('needs: report');
    expect(dispatch).toContain("needs.report.outputs.outcome == 'infrastructure'");
    expect(dispatch).toContain('actions: write');
    expect(dispatch).not.toMatch(/issues: write|contents: write|verify-public-install|wait --plan/);
    expect(dispatch).toContain('public-install-retry.mjs dispatch');
    expect(smoke.match(/actions: write/g)).toHaveLength(1);
    expect(workflow).toContain('GH_TOKEN: ${{ github.token }}');
    expect(workflow).toContain('SOURCE_RUN_ID: ${{ inputs.source_run_id }}');
    expect(workflow).toContain('RETRY_TARGETS: ${{ inputs.targets }}');
  });
  it('keeps acquisition token-free on matching hosts, uses tag checkout only as data and disables further retries', () => {
    expect(retry).toContain('matrix: ${{ fromJSON(needs.plan.outputs.matrix) }}');
    expect(retry).toContain('runs-on: ${{ matrix.runner }}');
    expect(retry).toContain('uses: ./.github/actions/public-install-smoke');
    expect(retry).toContain("retry: 'true'");
    expect(retry).toContain('tag: ${{ matrix.tag }}');
    expect(retry).toContain('target: ${{ matrix.target }}');
    expect(retry.match(/persist-credentials: false/g)).toHaveLength(1);
    expect(host).toContain('ref: ${{ inputs.tag }}\n        path: release-source');
    expect(host).toContain('persist-credentials: false');
    expect(host).toContain('node-version: 22.23.2');
    expect(host).toContain('true) retry_args=(--retry)');
    expect(host).toContain("default: 'false'");
    expect(host).toContain('node typescript/scripts/verify-public-install.mjs');
    expect(retry).not.toMatch(
      /GH_TOKEN|GITHUB_TOKEN|issues: write|release-source\/(scripts|typescript)/
    );
    expect(retry).not.toContain('continue-on-error');
    expect(host).not.toMatch(
      /GH_TOKEN|GITHUB_TOKEN|continue-on-error|release-source\/(scripts|typescript)/
    );
  });
  it('uses the same shared host entry for original and retry so architecture wrappers cannot drift', () => {
    const smoke = readFileSync(
      new URL('../../../.github/workflows/native-release-smoke.yml', import.meta.url),
      'utf8'
    )
      .split('\n  smoke:\n')[1]
      .split('\n  report:\n')[0];
    for (const entry of [smoke, retry]) {
      expect(entry).toContain('uses: ./.github/actions/public-install-smoke');
      expect(entry).not.toMatch(/setup-node|verify-public-install\.mjs|arch -|node /);
    }
    expect(smoke).not.toContain("retry: 'true'");
  });
});

describe('REST validation of retry dispatch source', () => {
  const repository = 'pj-tmt/tmt';
  const request = () => ({
    sourceRunId: 37100024356,
    sourceRunAttempt: 1,
    product: 'cli',
    tag: TAG,
    targets: [TARGETS[0]],
  });
  const source = (): RetrySource => ({
    run: {
      id: 37100024356,
      run_attempt: 1,
      head_branch: 'main',
      event: 'workflow_dispatch',
      path: '.github/workflows/native-release.yml',
      repository: { full_name: repository },
      head_repository: { full_name: repository },
    },
    jobs: TARGETS.map((target, i) => ({
      name: `Release ${TAG} / Install the published ${TAG} / Install ${TAG} from the public release (${target})`,
      conclusion: i === 0 ? 'failure' : 'success',
    })),
  });
  const validate = (extra: Partial<Parameters<typeof validateRetrySource>[0]> = {}) =>
    validateRetrySource({
      request: request(),
      repository,
      ...source(),
      hosts: hosts(),
      now: NOW,
      ...extra,
    });

  it('accepts only real native release or standalone smoke main evidence for the requested tag and targets', () => {
    expect(validate().matrix).toEqual(plan().matrix);
    const standalone = source();
    standalone.run.path = '.github/workflows/native-release-smoke.yml';
    standalone.jobs = standalone.jobs.map(({ name, conclusion }) => ({
      name: name.split(' / ').at(-1)!,
      conclusion,
    }));
    expect(validate(standalone).matrix).toEqual(plan().matrix);
  });

  it.each([
    { id: 9 },
    { run_attempt: 2 },
    { head_branch: 'feature' },
    { event: 'pull_request' },
    { path: '.github/workflows/native-release-smoke-retry.yml' },
    { repository: { full_name: 'other/repo' } },
    { head_repository: { full_name: 'fork/tmt' } },
  ])('rejects a fabricated, stale, recursive or non-main source: %j', (change) => {
    expect(() => validate({ run: { ...source().run, ...change } })).toThrow('requested main');
  });

  it('rejects altered product/tag/attempt evidence and unclassified source failures', () => {
    expect(() => validate({ request: { ...request(), tag: 'v5.0.0-alpha.44' } })).toThrow();
    expect(() => validate({ request: { ...request(), product: 'squad' } })).toThrow();
    expect(() => validate({ hosts: hosts().map((host) => ({ ...host, runAttempt: 2 })) })).toThrow(
      'attempt mismatch'
    );
    const mixed = hosts();
    mixed[1].failed = [{ check: 'skills', reason: 'wrong skills' }];
    const jobs = source().jobs;
    jobs[1].conclusion = 'failure';
    expect(() => validate({ hosts: mixed, jobs })).toThrow('infrastructure-only');
  });

  it('requires each target job to match a complete source artifact and its actual conclusion', () => {
    const jobs = source().jobs;
    expect(() => validate({ jobs: jobs.slice(1) })).toThrow('target job');
    expect(() => validate({ jobs: [...jobs, jobs[0]] })).toThrow('target job');
    jobs[0].conclusion = 'success';
    expect(() => validate({ jobs })).toThrow('target job');
    jobs[0].conclusion = null;
    expect(() => validate({ jobs })).toThrow('target job');
    expect(() => validate({ hosts: hosts().slice(1) })).toThrow('Incomplete');
  });

  it('does not expand requested targets or reject a queued retry when another reset enters the bound', () => {
    const input = hosts();
    input[1].failed = [limit(NOW / 1000 + 3601)];
    const jobs = source().jobs;
    jobs[1].conclusion = 'failure';
    expect(planSmokeRetry(input, { now: NOW }).matrix.include.map(({ target }) => target)).toEqual([
      TARGETS[0],
    ]);
    expect(planSmokeRetry(input, { now: NOW + 2000 }).matrix.include).toHaveLength(2);
    const selected = validate({ hosts: input, jobs, now: NOW + 2000 });
    expect(selected.matrix.include.map(({ target }) => target)).toEqual([TARGETS[0]]);
    expect(selected.waitUntilMs).toBe(1791000293000);
    const { api, calls } = reporter();
    expect(
      reportSmokeRetry({
        api,
        plan: selected,
        retried: [{ ...input[0], failed: [] }],
        originalRunUrl: ORIGINAL,
        retryRunUrl: RETRY,
      })
    ).toEqual([{ tag: TAG, ok: false }]);
    expect(calls.some(({ kind }) => kind === 'close')).toBe(false);
    expect(calls.find(({ kind }) => kind === 'comment')?.body).toContain(TARGETS[1]);
  });

  it('cannot request a healthy, additional, duplicate or unknown target, or a reset beyond the bound', () => {
    expect(() => validate({ request: { ...request(), targets: [TARGETS[1]] } })).toThrow(
      'Requested targets'
    );
    expect(() =>
      validate({ request: { ...request(), targets: [TARGETS[0], TARGETS[1]] } })
    ).toThrow('Requested targets');
    for (const targets of [[], ['unknown'], [TARGETS[0], TARGETS[0]]])
      expect(() => parseRetryRequest({ ...request(), targets })).toThrow('targets');
    expect(() => parseRetryRequest({ ...request(), sourceRunId: 'malicious/path' })).toThrow(
      'source run'
    );
    const distant = hosts();
    distant[0].failed = [limit(NOW / 1000 + 3600)];
    expect(() => validate({ hosts: distant })).toThrow('Requested targets');
  });

  it('reads the current run and attempt-specific jobs through bounded REST and dispatches only smoke retry on main', () => {
    const calls: { args: readonly string[]; options: object }[] = [];
    const api = ghSmokeRetryApi({
      repository,
      spawn: (_command, args, options) => {
        calls.push({ args, options });
        return {
          status: 0,
          stderr: '',
          stdout: args[1].includes('/jobs?')
            ? JSON.stringify({ jobs: source().jobs })
            : args.includes('POST')
              ? ''
              : JSON.stringify(source().run),
        };
      },
    });
    expect(api.readSource(request())).toEqual(source());
    api.dispatch(request());
    expect(calls.map(({ args }) => args[1])).toEqual([
      `repos/${repository}/actions/runs/37100024356`,
      `repos/${repository}/actions/runs/37100024356/attempts/1/jobs?per_page=100&page=1`,
      `repos/${repository}/actions/workflows/native-release-smoke-retry.yml/dispatches`,
    ]);
    expect(calls[2].args).toEqual([
      'api',
      `repos/${repository}/actions/workflows/native-release-smoke-retry.yml/dispatches`,
      '--method',
      'POST',
      '--input',
      '-',
    ]);
    expect(JSON.parse((calls[2].options as { input: string }).input)).toEqual({
      ref: 'main',
      inputs: {
        source_run_id: '37100024356',
        source_run_attempt: '1',
        product: 'cli',
        tag: TAG,
        targets: JSON.stringify([TARGETS[0]]),
      },
    });
  });

  it('fails closed on REST errors, malformed jobs or exhausted pagination', () => {
    const bad = ghSmokeRetryApi({
      repository,
      spawn: () => ({ status: 1, stdout: '', stderr: 'forbidden' }),
    });
    expect(() => bad.readSource(request())).toThrow('REST request failed');
    let requests = 0;
    const pages = ghSmokeRetryApi({
      repository,
      spawn: (_command, args) => {
        requests += 1;
        return {
          status: 0,
          stderr: '',
          stdout: JSON.stringify(
            args[1].includes('/jobs?')
              ? { jobs: Array.from({ length: 100 }, () => source().jobs[0]) }
              : source().run
          ),
        };
      },
    });
    expect(() => pages.readSource(request())).toThrow('ten pages');
    expect(requests).toBe(11);
    const malformed = ghSmokeRetryApi({
      repository,
      spawn: () => ({ status: 0, stdout: '{}', stderr: '' }),
    });
    expect(() => malformed.readSource(request())).toThrow('Invalid source jobs');
  });
});
