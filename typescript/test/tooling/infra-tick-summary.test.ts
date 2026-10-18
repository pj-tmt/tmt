import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { describe, expect, it } from 'vite-plus/test';
import {
  collectInfraTick,
  createTickClient,
  failingTests,
  renderInfraTick,
  QUEUE_QUERY,
  type TickOptions,
} from '../../scripts/infra-tick-summary.mjs';

const SHA = 'a'.repeat(40);
const options: TickOptions = {
  repository: 'pj-tmt/tmt',
  since: '2026-10-18T00:00:00Z',
  until: '2026-10-18T01:00:00Z',
  reviewMap: {
    rules: [{ prefix: '.github/', leads: ['tmt-lead'], source: '#606 shared paths, lines17-19' }],
  },
  local: {
    guides: { 'ARCHITECTURE.md': 'line\n', 'DEVELOPMENT.md': 'line\n' },
    budgets: {
      lines: { 'ARCHITECTURE.md': 640, 'DEVELOPMENT.md': 339 },
      bytes: { 'ARCHITECTURE.md': 48100, 'DEVELOPMENT.md': 19187 },
    },
    freeBytes: 100 * 1024 ** 3,
    load1: 2,
  },
};
const run = (id: number, overrides = {}) => ({
  id,
  head_sha: SHA,
  run_attempt: 1,
  created_at: '2026-10-18T00:30:00Z',
  name: 'CI',
  event: 'pull_request',
  status: 'completed',
  conclusion: 'success',
  pull_requests: [{ number: id }],
  html_url: `https://github.com/pj-tmt/tmt/actions/runs/${id}`,
  ...overrides,
});
const worker = (id: number, name = 'Unit tests') => ({
  id,
  name,
  status: 'completed',
  conclusion: 'failure',
  completed_at: options.until,
  html_url: `https://github.com/pj-tmt/tmt/actions/runs/1/jobs/${id}`,
  steps: [{ name: 'Run tests', conclusion: 'failure' }],
});
function fixture({
  runs = [] as unknown[],
  queued = [] as unknown[],
  active = [] as unknown[],
  tracked = [] as ReturnType<typeof run>[],
  jobs = [] as unknown[],
  log = 'no test token',
  releases = [] as unknown[],
  queue = { totalCount: 0, pageInfo: { hasNextPage: false }, nodes: [] as unknown[] },
  open = [] as unknown[],
  closed = [] as unknown[],
  files = [] as { filename: string }[],
  comments = [] as unknown[],
  changedHead = false,
} = {}) {
  const calls: string[] = [];
  return {
    calls,
    get(endpoint: string) {
      calls.push(endpoint);
      if (endpoint.includes('/actions/runs?')) {
        const query = new URL(`https://api.github.com/${endpoint}`).searchParams;
        const status = query.get('status');
        let rows =
          status === 'queued' && queued.length
            ? queued
            : status === 'in_progress' && active.length
              ? active
              : (runs as ReturnType<typeof run>[]).filter(
                  (row) => !status || row.status === status || row.conclusion === status
                );
        const created = query.get('created');
        if (created) {
          const [from, to] = created.split('..').map(Date.parse);
          rows = (rows as ReturnType<typeof run>[]).filter(
            (row) => Date.parse(row.created_at) >= from && Date.parse(row.created_at) <= to
          );
        }
        const size = Number(query.get('per_page')),
          page = Number(query.get('page'));
        return {
          total_count: rows.length,
          workflow_runs: rows.slice((page - 1) * size, page * size),
        };
      }
      if (/\/actions\/runs\/\d+$/.test(endpoint))
        return tracked.find((record) => record.id === Number(endpoint.split('/').at(-1)));
      if (endpoint.includes('/jobs?')) {
        const id = Number(/actions\/runs\/(\d+)\//.exec(endpoint)?.[1]);
        const record = [...(runs as ReturnType<typeof run>[]), ...tracked].find(
          (item) => item.id === id
        );
        return {
          total_count: jobs.length,
          jobs: (jobs as ReturnType<typeof worker>[]).map((job) => ({
            ...job,
            id: job.id + id * 100,
            run_id: id,
            run_attempt: record?.run_attempt,
            head_sha: record?.head_sha,
            html_url: `https://github.com/pj-tmt/tmt/actions/runs/${id}/job/${job.id + id * 100}`,
          })),
        };
      }
      if (endpoint.includes('/logs')) return log;
      if (endpoint.includes('/pulls?state=open'))
        return open.map((row: any) => ({ updated_at: options.until, head: { sha: SHA }, ...row }));
      if (endpoint.includes('/pulls?state=closed'))
        return closed.map((row: any) => ({
          updated_at: options.until,
          head: { sha: SHA },
          ...row,
        }));
      if (endpoint.includes('/files?')) return files;
      if (endpoint.includes('/comments?')) return comments;
      if (/\/pulls\/\d+$/.test(endpoint))
        return {
          number: 1,
          head: { sha: changedHead ? 'b'.repeat(40) : SHA },
          changed_files: files.length,
        };
      throw new Error(`Unexpected endpoint ${endpoint}`);
    },
    releases: () => releases,
    queue: () => queue,
    evidence: () => ({ requests: calls.length }),
  };
}

describe('prepared Infra tick evidence', () => {
  it('prints an explicit empty anomaly list, coverage, host and complete evidence', () => {
    const report = collectInfraTick(fixture({ runs: [run(1)] }), options);
    expect(report.complete).toBe(true);
    expect(report.anomalies).toEqual([]);
    expect(report.health.createdSinceTick).toBe(1);
    expect(renderInfraTick(report)).toContain('No anomalies observed within the stated coverage.');
    expect(renderInfraTick(report)).toContain(
      'starts and finishes between ticks is outside coverage'
    );
    expect(report.host).toEqual({ freeBytes: options.local.freeBytes, load1: options.local.load1 });
  });
  it('discovers older active reruns without a created filter and joins overlapping discovery explicitly', () => {
    const old = run(2, {
      created_at: '2026-09-01T00:00:00Z',
      status: 'in_progress',
      conclusion: null,
      run_attempt: 2,
    });
    const current = run(1, { status: 'queued', conclusion: null });
    const api = fixture({ runs: [current], queued: [current], active: [old] });
    const report = collectInfraTick(api, options);
    expect(report.health.createdSinceTick).toBe(1);
    expect(report.olderActive).toEqual([{ run: 2, attempt: 2, status: 'in_progress' }]);
    expect(
      api.calls
        .filter((call) => /status=(queued|in_progress)(?:&|$)/.test(call))
        .every((call) => !call.includes('created='))
    ).toBe(true);
  });
  it('refuses duplicate pages and source/attempt changes between discovery sets', () => {
    expect(() =>
      collectInfraTick(
        fixture({ runs: [run(1, { conclusion: 'failure' }), run(1, { conclusion: 'failure' })] }),
        options
      )
    ).toThrow('Duplicate');
    expect(() =>
      collectInfraTick(
        fixture({
          runs: [run(1, { conclusion: 'failure' })],
          queued: [run(1, { status: 'queued', conclusion: null, run_attempt: 2 })],
        }),
        options
      )
    ).toThrow('Changing run');
  });
  it('attributes observed workers, excludes propagated gates and groups same tests across PR/MG contexts', () => {
    const api = fixture({
      runs: [
        run(1, { conclusion: 'failure' }),
        run(2, { event: 'merge_group', conclusion: 'failure' }),
      ],
      jobs: [worker(11), worker(12, 'Native Rust contracts')],
      log: 'FAIL [ 1.234s] tmt-core cron_clock\ntest different ... FAILED\n',
    });
    const report = collectInfraTick(api, options);
    expect(report.failures).toHaveLength(2);
    expect(report.failures.every((failure) => failure.worker === 'Unit tests')).toBe(true);
    expect(report.repeated.map((item) => item.test)).toEqual(['different', 'tmt-core::cron_clock']);
    expect(report.repeated.every((item) => item.occurrences.length === 2)).toBe(true);
    expect(report.failures[0].logHash).toMatch(/^[a-f0-9]{64}$/);
  });
  it('does not call failures on repeated runs of the same PR a cross-PR recurrence', () => {
    const report = collectInfraTick(
      fixture({
        runs: [
          run(1, { conclusion: 'failure' }),
          run(2, { conclusion: 'failure', pull_requests: [{ number: 1 }] }),
        ],
        jobs: [worker(11)],
        log: 'test cron_clock ... FAILED\n',
      }),
      options
    );
    expect(report.repeated).toEqual([]);
  });
  it('detects newly failed attempts created before the previous tick and follows tracked older completions', () => {
    const previous = {
      schema: 1,
      until: options.since,
      failedDraftIds: [],
      failedRunAttempts: [],
      activeRunIds: [2],
    };
    const old = run(2, {
      created_at: '2026-09-01T00:00:00Z',
      conclusion: 'failure',
      run_attempt: 2,
    });
    const inCohort = run(1, { created_at: '2026-10-17T23:30:00Z', conclusion: 'failure' });
    const report = collectInfraTick(
      fixture({ runs: [inCohort], tracked: [old], jobs: [worker(11)] }),
      { ...options, previous }
    );
    expect(report.health.createdSinceTick).toBe(0);
    expect(
      report.anomalies.filter((item) => item.kind === 'newly observed failed run')
    ).toHaveLength(2);
    expect(report.failedRunAttempts).toEqual(['1:1', '2:2']);
    expect(report.trackedCompletions).toEqual([
      { run: 2, attempt: 2, status: 'completed', conclusion: 'failure' },
    ]);
    expect(report.activeRunIds).toEqual([]);
    const repeated = collectInfraTick(fixture({ runs: [inCohort], jobs: [worker(11)] }), {
      ...options,
      previous: { ...previous, failedRunAttempts: ['1:1'], activeRunIds: [] },
    });
    expect(repeated.anomalies.some((item) => item.kind === 'newly observed failed run')).toBe(
      false
    );
  });
  it('keeps missing logs explicitly unavailable instead of claiming no anomalies', () => {
    const api = fixture({ runs: [run(1, { conclusion: 'failure' })], jobs: [worker(11)] });
    const original = api.get;
    api.get = (endpoint) => {
      if (endpoint.includes('/logs')) throw Error('Unavailable');
      return original(endpoint);
    };
    const report = collectInfraTick(api, options);
    expect(report.complete).toBe(false);
    expect(report.anomalies.some((item) => item.kind === 'test log unavailable')).toBe(true);
  });
  it('reports exact queue state/position/age and refuses partial queue coverage', () => {
    const node = {
      position: 2,
      state: 'AWAITING_CHECKS',
      enqueuedAt: '2026-10-18T00:00:00Z',
      pullRequest: { number: 1, headRefOid: SHA, url: 'https://github.com/pj-tmt/tmt/pull/1' },
    };
    const report = collectInfraTick(
      fixture({ queue: { totalCount: 1, pageInfo: { hasNextPage: false }, nodes: [node] } }),
      options
    );
    expect(report.queue[0]).toMatchObject({
      position: 2,
      state: 'AWAITING_CHECKS',
      ageMinutes: 60,
    });
    expect(report.anomalies[0].kind).toBe('merge queue entry older than 30 minutes');
    const unavailable = collectInfraTick(
      fixture({ queue: { totalCount: 2, pageInfo: { hasNextPage: true }, nodes: [node] } }),
      options
    );
    expect(unavailable.complete).toBe(false);
    expect(unavailable.queue).toEqual([]);
    expect(unavailable.anomalies.some((item) => item.kind === 'queue coverage unavailable')).toBe(
      true
    );
  });
  it('reports only newly observed failed drafts after the initial baseline and flags release failures', () => {
    const releases = [
      {
        id: 1,
        tag_name: 'ops/v1.0.0-alpha.1',
        draft: true,
        assets: [{ name: 'verification-failed.json' }],
      },
    ];
    const api = fixture({
      releases,
      runs: [
        run(1, {
          conclusion: 'failure',
          path: '.github/workflows/native-release-smoke.yml',
          name: 'Native release smoke',
        }),
      ],
      jobs: [worker(11)],
    });
    const report = collectInfraTick(api, {
      ...options,
      previous: {
        schema: 1,
        until: options.since,
        failedDraftIds: [1],
        failedRunAttempts: [],
        activeRunIds: [],
      },
    });
    expect(report.failedDrafts).toHaveLength(1);
    expect(report.anomalies.some((item) => item.kind.includes('failed draft'))).toBe(false);
    expect(
      report.anomalies.some((item) => item.kind === 'failed release or post-publication check')
    ).toBe(true);
  });
  it('preserves exact comment/head/stated lead without deciding acceptance', () => {
    const body = `NO-GO at ${SHA}\nAgent: tmt-lead codex-sol-high`;
    const report = collectInfraTick(
      fixture({
        open: [{ id: 1, number: 1 }],
        files: [{ filename: '.github/workflows/ci.yml' }],
        comments: [
          { id: 1, body, html_url: 'https://github.com/pj-tmt/tmt/pull/1#issuecomment-1' },
        ],
      }),
      options
    );
    expect(report.reviewEvidence[0].statements).toEqual([
      {
        lead: 'tmt-lead',
        head: SHA,
        comment: body,
        url: 'https://github.com/pj-tmt/tmt/pull/1#issuecomment-1',
      },
    ]);
    expect(report.reviewEvidence[0].disposition).toContain('not interpreted as approval');
    expect(report.reviewEvidence[0]).not.toHaveProperty('accepted');
  });
  it('identifies failed post-publication checks by exact job tag and published REST evidence', () => {
    const tag = 'ops/v5.0.0-alpha.122';
    const runs = [run(1, { conclusion: 'failure', path: '.github/workflows/native-release.yml' })];
    const jobs = [
      worker(
        11,
        `Public smoke / Install ${tag} from the public release (x86_64-unknown-linux-musl)`
      ),
      worker(12, `Public smoke / Install ${tag} from the public release (aarch64-apple-darwin)`),
    ];
    const release = { id: 71, tag_name: tag, draft: false, assets: [] };
    const report = collectInfraTick(fixture({ runs, jobs, releases: [release] }), options);
    expect(report.postPublicationFailures).toHaveLength(2);
    expect(renderInfraTick(report)).toContain('1 published releases with failed public checks');
    expect(report.postPublicationFailures[0]).toMatchObject({
      tag,
      releaseId: 71,
      run: 1,
      attempt: 1,
    });
    expect(
      report.anomalies.some(
        (item) => item.kind === 'published release post-publication check failed'
      )
    ).toBe(true);
    for (const releases of [[], [{ ...release, draft: true }], [release, { ...release, id: 72 }]]) {
      const refused = collectInfraTick(fixture({ runs, jobs, releases }), options);
      expect(refused.postPublicationFailures).toEqual([]);
      expect(refused.complete).toBe(false);
      expect(
        refused.anomalies.some(
          (item) => item.kind === 'public-install release evidence unavailable'
        )
      ).toBe(true);
    }
    const renamed = collectInfraTick(
      fixture({
        runs,
        jobs: [
          {
            ...jobs[0],
            name: 'Public smoke / Renamed install worker',
            steps: [{ name: 'Install from the public release', conclusion: 'failure' }],
          },
        ],
        releases: [release],
      }),
      options
    );
    expect(renamed.postPublicationFailures).toEqual([]);
    expect(renamed.complete).toBe(false);
    expect(
      renamed.anomalies.some((item) => item.kind === 'public-install release evidence unavailable')
    ).toBe(true);
  });
  it('flags unmapped shared paths, stale-head statements and changing PR heads', () => {
    const api = () =>
      fixture({
        open: [{ id: 1, number: 1 }],
        files: [{ filename: 'contracts/api.md' }, { filename: '.github/workflows/ci.yml' }],
        comments: [{ id: 1, body: `accepted ${'b'.repeat(40)}\nAgent: tmt-lead codex-sol-high` }],
      });
    const report = collectInfraTick(api(), options);
    expect(report.anomalies.some((item) => item.kind === 'unmapped shared path')).toBe(true);
    expect(report.anomalies.some((item) => item.kind.startsWith('missing head-named'))).toBe(true);
    expect(() =>
      collectInfraTick(
        fixture({
          open: [{ id: 1, number: 1 }],
          files: [{ filename: '.github/workflows/ci.yml' }],
          changedHead: true,
        }),
        options
      )
    ).toThrow('PR head changed');
  });
  it('uses the existing guide budget owner, warns at ten lines and refuses missing evidence', () => {
    const local = {
      ...options.local,
      guides: { ...options.local.guides, 'ARCHITECTURE.md': 'line\n'.repeat(630) },
    };
    expect(
      collectInfraTick(fixture(), { ...options, local }).anomalies.some(
        (item) => item.kind === 'guide near or over budget'
      )
    ).toBe(true);
    expect(() =>
      collectInfraTick(fixture(), { ...options, since: '2026-10-15T00:00:00Z' })
    ).toThrow('24-hour');
    expect(() => collectInfraTick(fixture(), { ...options, reviewMap: { rules: [] } })).toThrow(
      'path-to-lead'
    );
  });
  it('counts a 1,375-run cohort without enumerating it, and refuses capped status slices before page two', () => {
    const api = fixture();
    const original = api.get;
    api.get = (endpoint) =>
      endpoint.includes('/actions/runs?created=')
        ? { total_count: 1375, workflow_runs: [run(1)] }
        : original(endpoint);
    const report = collectInfraTick(api, options);
    expect(report.health).toEqual({ created24h: 1375, createdSinceTick: 1375 });
    for (const total of [1000, 1375]) {
      const capped = fixture();
      const calls: string[] = [];
      const read = capped.get;
      capped.get = (endpoint) => {
        calls.push(endpoint);
        if (endpoint.includes('status=failure'))
          return { total_count: total, workflow_runs: [run(1, { conclusion: 'failure' })] };
        return read(endpoint);
      };
      expect(() => collectInfraTick(capped, options)).toThrow('1,000-result limit');
      expect(calls.filter((call) => call.includes('status=failure'))).toHaveLength(1);
      expect(calls.some((call) => call.includes('page=11'))).toBe(false);
    }
  });
  it('skips prior failed-attempt detail reads and retains their cross-PR test evidence', () => {
    const api = fixture({
      runs: [run(1, { conclusion: 'failure' }), run(2, { conclusion: 'failure' })],
      jobs: [worker(11)],
      log: 'test same ... FAILED\n',
    });
    const report = collectInfraTick(api, {
      ...options,
      previous: {
        schema: 1,
        until: options.since,
        failedDraftIds: [],
        activeRunIds: [],
        failedRunAttempts: ['1:1'],
        failureEvidence: [
          {
            run: 1,
            attempt: 1,
            created: '2026-10-18T00:30:00Z',
            context: 'pr:1',
            worker: 'Unit tests',
            tests: ['same'],
            logHash: 'a'.repeat(64),
          },
        ],
      },
    });
    expect(api.calls.some((call) => call.includes('/runs/1/attempts/'))).toBe(false);
    expect(api.calls.some((call) => call.includes('/jobs/111/'))).toBe(false);
    expect(report.repeated).toHaveLength(1);
    expect(report.repeated[0].occurrences).toHaveLength(2);
  });
  it('reads only recently updated open PRs plus PRs merged in the interval, stopping at the first old row', () => {
    const old = new Date(Date.parse(options.since) - 1000).toISOString();
    const api = fixture({
      open: [
        { id: 1, number: 1, updated_at: new Date(Date.parse(options.until) + 1000).toISOString() },
        ...Array.from({ length: 99 }, (_, i) => ({ id: i + 2, number: i + 2, updated_at: old })),
      ],
      closed: [
        { id: 101, number: 101, updated_at: options.until, merged_at: options.since },
        { id: 102, number: 102, updated_at: options.until, merged_at: old },
      ],
      files: [{ filename: '.github/workflows/ci.yml' }],
    });
    const read = api.get;
    api.get = (endpoint) => {
      if (endpoint.includes('state=open') && endpoint.includes('page=2'))
        throw Error('Must stop at old row');
      if (/\/pulls\/101$/.test(endpoint))
        return { number: 101, head: { sha: SHA }, changed_files: 1 };
      return read(endpoint);
    };
    const report = collectInfraTick(api, options);
    expect(report.reviewEvidence.map((row) => row.pr)).toEqual([1, 101]);
    expect(api.calls.some((call) => /\/pulls\/(?:2|102)(?:\/|$)/.test(call))).toBe(false);
  });
  it('extracts exact known log forms without inventing a failing test from arbitrary text', () => {
    expect(failingTests('some error about cron_clock\nPASS [ 1s] suite test\n')).toEqual([]);
    expect(failingTests('test a::b ... FAILED\ntest a::b ... FAILED\n')).toEqual(['a::b']);
  });
});

describe('bounded read-only tick transport', () => {
  function client(
    execute: Parameters<typeof createTickClient>[0]['execute'],
    clock?: () => number
  ) {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-'));
    return { cache, api: createTickClient({ repository: 'pj-tmt/tmt', cache, execute, clock }) };
  }
  it('permits one fixed query, refuses mutation/query substitution and shares the request budget', () => {
    const calls: string[][] = [];
    const { cache, api } = client((_program, args) => {
      calls.push(args);
      return JSON.stringify(
        args[1] === 'graphql'
          ? {
              data: {
                repository: {
                  mergeQueue: {
                    entries: { totalCount: 0, pageInfo: { hasNextPage: false }, nodes: [] },
                  },
                },
              },
            }
          : {}
      );
    });
    try {
      expect(api.queue()).toMatchObject({ totalCount: 0 });
      expect(() => api.queue()).toThrow('one queue query');
      expect(calls[0]).toContain(`query=${QUEUE_QUERY}`);
      expect(QUEUE_QUERY).not.toMatch(/\bmutation\b/);
      for (let i = 0; i < 79; i++) api.get(`repos/pj-tmt/tmt/probe/${i}`);
      expect(api.evidence().requests).toBe(80);
      expect(() => api.get('repos/pj-tmt/tmt/over-budget')).toThrow('budget exhausted');
      expect(() => api.get('repos/foreign/repo/actions/runs')).toThrow('Only repos/pj-tmt/tmt');
      expect(() => api.get('graphql?query=mutation { unsafe }')).toThrow('Only repos/pj-tmt/tmt');
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('resumes a 54-failure baseline in two bounded invocations using qualified immutable evidence', () => {
    const evidence = fixture({
      runs: Array.from({ length: 54 }, (_, i) => run(i + 1, { conclusion: 'failure' })),
      jobs: [worker(11)],
      open: Array.from({ length: 39 }, (_, i) => ({
        id: i + 1,
        number: i + 1,
        updated_at: new Date(Date.parse(options.since) - 1000).toISOString(),
      })),
    });
    const cache = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-volume-'));
    const execute: Parameters<typeof createTickClient>[0]['execute'] = (_program, args) => {
      if (args[1] === 'graphql')
        return JSON.stringify({
          data: { repository: { mergeQueue: { entries: evidence.queue() } } },
        });
      const endpoint = args.find((arg) => arg.startsWith('repos/'))!;
      if (endpoint === 'repos/pj-tmt/tmt') return '{"permissions":{"push":true}}';
      if (endpoint.includes('/releases?')) return 'HTTP/2 200 OK\r\n\r\n[]';
      const result = evidence.get(endpoint);
      return typeof result === 'string' ? result : JSON.stringify(result);
    };
    try {
      const cold = createTickClient({ repository: 'pj-tmt/tmt', cache, execute });
      expect(() => collectInfraTick(cold, options)).toThrow(/budget/i);
      expect(cold.evidence().requests).toBe(80);
      const resumed = createTickClient({ repository: 'pj-tmt/tmt', cache, execute });
      const report = collectInfraTick(resumed, options);
      expect(report.complete).toBe(true);
      expect(report.failures).toHaveLength(54);
      expect(resumed.evidence().requests).toBeLessThanOrEqual(80);
      expect(evidence.calls.some((call) => /\/pulls\/\d+/.test(call))).toBe(false);
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('bounds the shared deadline without changing endpoint timeouts or leaking credentials', () => {
    let now = 0;
    const calls: string[][] = [];
    const { cache, api } = client(
      (_program, args) => {
        calls.push(args);
        return '{}';
      },
      () => now
    );
    try {
      api.get('repos/pj-tmt/tmt/probe');
      now = 120000;
      expect(() => api.get('repos/pj-tmt/tmt/expired')).toThrow('budget exhausted');
      expect(calls.flat().some((arg) => arg.includes('TOKEN'))).toBe(false);
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('captures a failing test beyond the packed default 2 MiB without unbounding the log read', () => {
    const directory = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-large-log-'));
    const executable = path.join(directory, 'gh');
    writeExecutable(
      executable,
      `#!${process.execPath}\nconst {writeSync}=require('node:fs');writeSync(1,'x'.repeat(2*1024*1024+1)+'\\ntest late_failure ... FAILED\\n');\n`,
      0o700
    );
    try {
      const result = spawnSync(
        process.execPath,
        [
          '--input-type=module',
          '--eval',
          `
        import {createTickClient,failingTests} from ${JSON.stringify(new URL('../../scripts/infra-tick-summary.mjs', import.meta.url).href)};
        const api=createTickClient({repository:'pj-tmt/tmt',cache:${JSON.stringify(path.join(directory, 'cache'))}});
        const log=api.get('repos/pj-tmt/tmt/actions/jobs/1/logs',true,true);
        console.log(JSON.stringify({bytes:Buffer.byteLength(log),tests:failingTests(log),requests:api.evidence().requests}));
      `,
        ],
        {
          encoding: 'utf8',
          timeout: 10000,
          env: { ...process.env, PATH: `${directory}${path.delimiter}${process.env.PATH}` },
        }
      );
      expect(result.status, result.stdout + result.stderr).toBe(0);
      const captured = JSON.parse(result.stdout);
      expect(captured.bytes).toBeGreaterThan(2 * 1024 * 1024);
      expect(captured.tests).toEqual(['late_failure']);
      expect(captured.requests).toBe(1);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
  it('identifies a subprocess timeout at the shared deadline as the 120-second limit', () => {
    let now = 0;
    const { cache, api } = client(
      () => {
        now = 120000;
        throw Error('spawnSync gh ETIMEDOUT');
      },
      () => now
    );
    try {
      expect(() => api.get('repos/pj-tmt/tmt/probe')).toThrow(
        'time budget exhausted (120 seconds; 1/80 calls)'
      );
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('refuses invisible drafts before interpreting an empty catalogue', () => {
    const calls: string[][] = [];
    const { cache, api } = client((_program, args) => {
      calls.push(args);
      return '{"permissions":{"push":false}}';
    });
    try {
      expect(() => api.releases()).toThrow('Draft visibility unavailable');
      expect(calls).toHaveLength(1);
      expect(calls[0]).toContain('repos/pj-tmt/tmt');
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('reuses only fully qualified terminal attempt job pages', () => {
    let requests = 0;
    const { cache, api } = client(() => {
      requests++;
      return JSON.stringify({ total_count: 1, jobs: [worker(11)] });
    });
    const endpoint = 'repos/pj-tmt/tmt/actions/runs/1/attempts/1/jobs?per_page=100&page=1';
    try {
      api.get(endpoint, true);
      api.get(endpoint, true);
      expect(requests).toBe(2); // A terminal-looking page alone is not qualified.
      api.qualifyTerminalJobs?.([endpoint]);
      api.get(endpoint, true);
      expect(requests).toBe(2);
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
});

describe('Infra summary CLI artifacts and cursor', () => {
  it('writes a complete report, then preserves its cursor and redacts a refused acquisition', () => {
    const directory = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-cli-'));
    const secret = 'fixture-github-credential-never-print';
    try {
      const map = path.join(directory, 'review-map.json');
      writeFileSync(map, JSON.stringify(options.reviewMap));
      writeExecutable(
        path.join(directory, 'gh'),
        `#!${process.execPath}
const args=process.argv.slice(2);
const endpoint=args.find(arg=>arg.startsWith('repos/'));
if(process.env.FAIL_READ==='true'){process.stderr.write(process.env.GH_TOKEN);process.exit(1);}
if(args[1]==='graphql')console.log(JSON.stringify({data:{repository:{mergeQueue:{entries:{totalCount:0,pageInfo:{hasNextPage:false},nodes:[]}}}}}));
else if(endpoint==='repos/pj-tmt/tmt')console.log('{"permissions":{"push":true}}');
else if(endpoint.includes('/releases?'))process.stdout.write('HTTP/2 200 OK\\r\\n\\r\\n[]');
else if(endpoint.includes('/actions/runs?'))console.log('{"total_count":0,"workflow_runs":[]}');
else if(endpoint.includes('/pulls?'))console.log('[]');
else throw Error('Unexpected endpoint');
`,
        0o700
      );
      const script = fileURLToPath(
        new URL('../../scripts/infra-tick-summary.mjs', import.meta.url)
      );
      const execute = (fail: boolean, extra: string[] = []) =>
        spawnSync(
          process.execPath,
          [script, '--output', directory, '--review-map', map, ...extra],
          {
            encoding: 'utf8',
            timeout: 10_000,
            env: {
              ...process.env,
              PATH: `${directory}${path.delimiter}${process.env.PATH}`,
              GH_TOKEN: secret,
              GITHUB_TOKEN: secret,
              FAIL_READ: String(fail),
              TMT_AGENT: '1',
              BROWSER: 'true',
            },
          }
        );
      const first = execute(false);
      expect(first.status, first.stdout + first.stderr).toBe(0);
      const bootstrap = JSON.parse(readFileSync(path.join(directory, 'summary.json'), 'utf8'));
      const initialCursor = JSON.parse(readFileSync(path.join(directory, 'cursor.json'), 'utf8'));
      expect(bootstrap).toMatchObject({
        baselineEstablished: true,
        complete: false,
        evidence: { requests: 0 },
      });
      expect(initialCursor.baselineUntil).toBe(bootstrap.until);
      expect(first.stdout).toContain('details from the next tick');
      expect(first.stdout).not.toContain('No anomalies');
      expect(readFileSync(path.join(directory, 'calls.jsonl'), 'utf8')).not.toContain('"request":');
      const detail = execute(false);
      expect(detail.status, detail.stdout + detail.stderr).toBe(0);
      const report = JSON.parse(readFileSync(path.join(directory, 'summary.json'), 'utf8'));
      const cursor = readFileSync(path.join(directory, 'cursor.json'), 'utf8');
      expect(report.complete).toBe(true);
      expect(JSON.parse(cursor).baselineUntil).toBe(initialCursor.baselineUntil);
      expect(report.coverage).toContain(initialCursor.baselineUntil);
      expect(report.evidence.requests).toBe(18);
      expect(JSON.parse(cursor).until).toBe(report.until);
      expect(readFileSync(path.join(directory, 'summary.md'), 'utf8')).toBe(detail.stdout);
      const refused = execute(true);
      expect(refused.status).toBe(1);
      expect(readFileSync(path.join(directory, 'cursor.json'), 'utf8')).toBe(cursor);
      const refusedJson = readFileSync(path.join(directory, 'summary.json'), 'utf8');
      expect(JSON.parse(refusedJson).complete).toBe(false);
      expect(refused.stdout + refused.stderr + refusedJson).not.toContain(secret);
      expect(refusedJson).toContain('[REDACTED]');
      expect(refused.stdout).not.toContain('No anomalies');
      expect(readFileSync(path.join(directory, 'calls.jsonl'), 'utf8')).not.toContain(secret);
      const overwritten = execute(false, ['--bootstrap']);
      expect(overwritten.status).toBe(1);
      expect(overwritten.stdout).toContain('Cannot bootstrap an existing cursor');
      expect(readFileSync(path.join(directory, 'cursor.json'), 'utf8')).toBe(cursor);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

it('cannot converge by retrying mutable review reads, even with no failed-run details', () => {
  const cache = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-no-convergence-'));
  const evidence = fixture({
    open: Array.from({ length: 25 }, (_, i) => ({ id: i + 1, number: i + 1 })),
    files: [{ filename: '.github/workflows/ci.yml' }],
  });
  const trace: string[][] = [];
  try {
    for (let attempt = 0; attempt < 3; attempt++) {
      const calls: string[] = [];
      trace.push(calls);
      const api = createTickClient({
        repository: 'pj-tmt/tmt',
        cache,
        trace: (row) => calls.push(row.endpoint!),
        execute: (_program, args) => {
          if (args[1] === 'graphql')
            return JSON.stringify({
              data: { repository: { mergeQueue: { entries: evidence.queue() } } },
            });
          const endpoint = args.find((arg) => arg.startsWith('repos/'))!;
          if (endpoint === 'repos/pj-tmt/tmt') return '{"permissions":{"push":true}}';
          if (endpoint.includes('/releases?')) return 'HTTP/2 200 OK\r\n\r\n[]';
          const detail = /\/pulls\/(\d+)$/.exec(endpoint);
          if (detail)
            return JSON.stringify({
              number: Number(detail[1]),
              head: { sha: SHA },
              changed_files: 1,
            });
          return JSON.stringify(evidence.get(endpoint));
        },
      });
      expect(() => collectInfraTick(api, options)).toThrow('call budget exhausted');
      expect(calls).toHaveLength(80);
    }
    expect(trace[1]).toEqual(trace[0]);
    expect(trace[2]).toEqual(trace[0]);
    expect(trace[0].filter((endpoint) => endpoint.includes('/pulls/')).length).toBeGreaterThan(40);
  } finally {
    rmSync(cache, { recursive: true, force: true });
  }
});

it('post-bootstrap skips998 historical failures but analyses a newly failed old-created run', () => {
  const before = new Date(Date.parse(options.since) - 1000).toISOString();
  const created = new Date(Date.parse(options.since) - 60_000).toISOString();
  const evidence = fixture({
    runs: [
      ...Array.from({ length: 998 }, (_, i) =>
        run(i + 1, { created_at: created, updated_at: before, conclusion: 'failure' })
      ),
      run(999, { created_at: created, updated_at: options.until, conclusion: 'failure' }),
    ],
    jobs: [worker(11)],
    open: Array.from({ length: 25 }, (_, i) => ({ id: i + 1, number: i + 1, updated_at: before })),
  });
  const report = collectInfraTick(evidence, {
    ...options,
    previous: {
      schema: 1,
      until: options.since,
      baselineUntil: options.since,
      failedDraftIds: [],
      failedRunAttempts: [],
      failureEvidence: [],
      activeRunIds: [],
    },
  });
  expect(report.complete).toBe(true);
  expect(report.health.created24h).toBe(999);
  expect(report.failures).toHaveLength(1);
  expect(report.failures[0].run).toBe(999);
  expect(report.failedRunAttempts).toEqual(['999:1']);
  expect(evidence.calls.filter((endpoint) => endpoint.includes('/jobs?'))).toHaveLength(1);
  expect(evidence.calls.filter((endpoint) => endpoint.includes('/logs'))).toHaveLength(1);
  expect(evidence.calls.some((endpoint) => /\/pulls\/\d+/.test(endpoint))).toBe(false);
  expect(report.coverage).toContain('earlier history was intentionally not analysed');
  expect(report.coverage).toContain('startup_failure');
});

it('traces only actual network calls, including errors, without response bodies', () => {
  const cache = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-trace-'));
  const trace: unknown[] = [];
  const api = createTickClient({
    repository: 'pj-tmt/tmt',
    cache,
    trace: (row) => trace.push(row),
    execute: (_program, args) => {
      if (args.some((arg) => arg.endsWith('/error'))) throw Error('private response detail');
      return args.includes('--allow-escape-sequences')
        ? 'raw private log'
        : '{"private":"response"}';
    },
  });
  try {
    api.get('repos/pj-tmt/tmt/mutable');
    api.get('repos/pj-tmt/tmt/mutable');
    api.get('repos/pj-tmt/tmt/actions/jobs/1/logs', true, true);
    api.get('repos/pj-tmt/tmt/actions/jobs/1/logs', true, true);
    expect(() => api.get('repos/pj-tmt/tmt/error')).toThrow('private response detail');
    expect(trace).toHaveLength(4);
    expect(api.evidence().requests).toBe(4);
    expect(trace[3]).toMatchObject({
      request: 4,
      method: 'GET',
      endpoint: 'repos/pj-tmt/tmt/error',
      outcome: 'error',
    });
    expect(JSON.stringify(trace)).not.toContain('private');
  } finally {
    rmSync(cache, { recursive: true, force: true });
  }
});
