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
  REVIEW_QUERY,
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
      if (endpoint.includes('/pulls?state=open') || endpoint.includes('/issues?state=closed')) {
        const query = new URL(`https://api.github.com/${endpoint}`).searchParams;
        const page = Number(query.get('page'));
        const isOpen = endpoint.includes('/pulls?');
        const rows = (isOpen ? open : closed).map((row: any) => ({
          updated_at: options.until,
          head: { sha: SHA },
          node_id: `PR_${row.id}`,
          state: isOpen ? 'open' : 'closed',
          ...(isOpen
            ? {}
            : {
                pull_request: {
                  url: `https://api.github.com/repos/pj-tmt/tmt/pulls/${row.number}`,
                },
              }),
          ...row,
        }));
        const since = query.get('since');
        return rows
          .filter((row: any) => !since || Date.parse(row.updated_at) > Date.parse(since))
          .slice((page - 1) * 100, page * 100);
      }
      if (endpoint.includes('/files?') || endpoint.includes('/comments?')) {
        const rows = endpoint.includes('/files?') ? files : comments;
        const page = Number(new URL(`https://api.github.com/${endpoint}`).searchParams.get('page'));
        return rows.slice((page - 1) * 100, page * 100);
      }
      if (/\/pulls\/\d+$/.test(endpoint)) {
        const number = Number(endpoint.split('/').at(-1));
        const pr: any = [...open, ...closed].find((row: any) => row.number === number);
        return {
          updated_at: options.until,
          state: closed.includes(pr) ? 'closed' : 'open',
          merged_at: null,
          ...pr,
          node_id: pr?.node_id ?? `PR_${pr?.id}`,
          number,
          head: { sha: changedHead ? 'b'.repeat(40) : (pr?.head?.sha ?? SHA) },
          changed_files: files.length,
        };
      }
      throw new Error(`Unexpected endpoint ${endpoint}`);
    },
    releases: () => releases,
    queue: () => queue,
    reviews: (ids: string[]) => {
      calls.push('graphql:InfraTickReviews');
      return ids.map((id) => {
        const pr: any = [...open, ...closed].find(
          (row: any) => (row.node_id ?? `PR_${row.id}`) === id
        );
        return {
          __typename: 'PullRequest',
          id,
          number: pr.number,
          repository: { nameWithOwner: options.repository },
          headRefOid: changedHead ? 'b'.repeat(40) : (pr.head?.sha ?? SHA),
          changedFiles: files.length,
          files: {
            totalCount: files.length,
            pageInfo: { hasNextPage: files.length > 100 },
            nodes: files.slice(0, 100).map((file) => ({ path: file.filename })),
          },
          comments: {
            totalCount: comments.length,
            pageInfo: { hasPreviousPage: comments.length > 100, hasNextPage: false },
            nodes: comments.slice(-100).map((comment: any) => ({
              id: `IC_${comment.id}`,
              body: comment.body,
              url:
                comment.html_url ??
                `https://github.com/pj-tmt/tmt/pull/${pr.number}#issuecomment-${comment.id}`,
            })),
          },
        };
      });
    },
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
  it('exhausts open history and hydrates closed candidates before selecting interval merges', () => {
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
    const report = collectInfraTick(api, options);
    expect(report.reviewEvidence.map((row) => row.pr)).toEqual([1, 101]);
    expect(api.calls.filter((call) => /\/pulls\/(?:101|102)$/.test(call))).toHaveLength(2);
    expect(api.calls.some((call) => call.includes('state=open') && call.endsWith('page=2'))).toBe(
      true
    );
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
        'time budget exhausted (120 seconds; 1/80 calls; stage=acquisition; PRs=0)'
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
else if(endpoint.includes('/pulls?')||endpoint.includes('/issues?state=closed'))console.log('[]');
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
      expect(report.evidence.requests).toBe(20);
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

it.each([0, 3])(
  'batches 30 PR reviews with %i closed candidates and accounts for hydration reads',
  (hydrated) => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-batched-'));
    const evidence = fixture({
      open: Array.from({ length: 30 - hydrated }, (_, i) => ({ id: i + 1, number: i + 1 })),
      closed: Array.from({ length: hydrated }, (_, i) => ({
        id: 31 - hydrated + i,
        number: 31 - hydrated + i,
        merged_at: options.until,
      })),
      files: [{ filename: '.github/workflows/ci.yml' }],
    });
    const calls: string[] = [];
    try {
      const api = createTickClient({
        repository: 'pj-tmt/tmt',
        cache,
        trace: (row) => calls.push(row.endpoint!),
        execute: (_program, args) => {
          if (args[1] === 'graphql') {
            if (args.includes(`query=${REVIEW_QUERY}`))
              return JSON.stringify({
                data: {
                  rateLimit: { cost: 1 },
                  nodes: evidence.reviews(
                    args.filter((arg) => arg.startsWith('ids[]=')).map((arg) => arg.slice(6))
                  ),
                },
              });
            return JSON.stringify({
              data: { repository: { mergeQueue: { entries: evidence.queue() } } },
            });
          }
          const endpoint = args.find((arg) => arg.startsWith('repos/'))!;
          if (endpoint === 'repos/pj-tmt/tmt') return '{"permissions":{"push":true}}';
          if (endpoint.includes('/releases?')) return 'HTTP/2 200 OK\r\n\r\n[]';
          return JSON.stringify(evidence.get(endpoint));
        },
      });
      const report = collectInfraTick(api, options);
      expect(report.complete).toBe(true);
      expect(report.reviewEvidence).toHaveLength(30);
      expect(api.evidence()).toMatchObject({
        requests: 21 + hydrated,
        queueQueries: 1,
        reviewQueries: 1,
        reviewPoints: 1,
      });
      expect(calls.filter((endpoint) => /\/(pulls|issues)\/\d+/.test(endpoint))).toEqual(
        Array.from({ length: hydrated }, (_, i) => `repos/pj-tmt/tmt/pulls/${31 - hydrated + i}`)
      );
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  }
);

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

describe('one-pass hourly PR volume through the real CLI', () => {
  function volume(count: number, mode = 'normal') {
    const directory = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-hourly-'));
    const cursorFile = path.join(directory, 'cursor.json');
    const cursor =
      mode === 'corrupt'
        ? '{broken cursor'
        : JSON.stringify({
            schema: 1,
            until: options.since,
            failedDraftIds: [],
            failedRunAttempts: [],
            activeRunIds: Array.from({ length: 32 }, (_, i) => i + 1),
          });
    writeFileSync(cursorFile, cursor);
    const map = path.join(directory, 'review-map.json');
    writeFileSync(map, JSON.stringify(options.reviewMap));
    writeExecutable(
      path.join(directory, 'gh'),
      `#!${process.execPath}
const args=process.argv.slice(2), mode=process.env.VOLUME_MODE, count=Number(process.env.VOLUME_COUNT);
const sha=${JSON.stringify(SHA)}, until=${JSON.stringify(options.until)};
const endpoint=args.find(arg=>arg.startsWith('repos/'));
const run=id=>({id,head_sha:sha,run_attempt:1,created_at:'2026-10-18T00:30:00Z',name:'CI',event:'pull_request',status:'completed',conclusion:'success',pull_requests:[{number:id}]});
const files=pr=>Array.from({length:mode==='overflow'||(mode==='pagination'&&pr===1)?101:1},(_,i)=>({filename:'.github/file-'+i+'.yml'}));
const comment=pr=>({id:pr,body:'Observed '+sha+'\\nAgent: tmt-lead codex-sol-high',html_url:'https://github.com/pj-tmt/tmt/pull/'+pr+'#issuecomment-'+pr});
if(args[1]==='graphql'){
 const query=args[args.indexOf('-f')+1];
 if(query.includes('InfraTickQueue'))console.log(JSON.stringify({data:{repository:{mergeQueue:{entries:{totalCount:0,pageInfo:{hasNextPage:false},nodes:[]}}}}}));
 else {
  if(!query.includes('InfraTickReviews'))throw Error('Unknown query');
  const ids=args.filter(arg=>arg.startsWith('ids[]=')).map(arg=>arg.slice(6));
  const nodes=ids.map(id=>{const number=Number(id.slice(3)), paths=files(number);return {__typename:'PullRequest',id,number,repository:{nameWithOwner:'pj-tmt/tmt'},headRefOid:mode==='changed-head'&&number===1?'b'.repeat(40):sha,changedFiles:paths.length,files:{totalCount:paths.length,pageInfo:{hasNextPage:paths.length>100},nodes:mode==='truncated'&&number===1?[]:paths.slice(0,100).map(file=>({path:file.filename}))},comments:{totalCount:1,pageInfo:{hasPreviousPage:false,hasNextPage:false},nodes:[{id:'IC_'+number,body:comment(number).body,url:comment(number).html_url}]}};});
  console.log(JSON.stringify({data:{rateLimit:{cost:mode==='cost'?21:1},nodes},...(mode==='unavailable'?{errors:[{message:'fixture unavailable'}]}:{})}));
 }
} else if(endpoint==='repos/pj-tmt/tmt')console.log('{"permissions":{"push":true}}');
else if(endpoint.includes('/releases?'))process.stdout.write('HTTP/2 200 OK\\r\\n\\r\\n[]');
else if(endpoint.includes('/actions/runs?')){
 const status=new URL('https://api.github.com/'+endpoint).searchParams.get('status');
 console.log(JSON.stringify(status?{total_count:0,workflow_runs:[]}:{total_count:32,workflow_runs:[run(1)]}));
} else if(endpoint.startsWith('repos/pj-tmt/tmt/actions/runs/'))console.log(JSON.stringify(run(Number(endpoint.split('/').at(-1)))));
else if(endpoint.includes('/issues?state=closed'))console.log(JSON.stringify(mode==='hydration-budget'?Array.from({length:30},(_,i)=>({id:1000+i,number:1000+i,node_id:'PR_'+(1000+i),updated_at:until,pull_request:{url:'https://api.github.com/repos/pj-tmt/tmt/pulls/'+(1000+i)}})):[]));
else if(endpoint.includes('/pulls?')){
 const q=new URL('https://api.github.com/'+endpoint).searchParams, page=Number(q.get('page'));
 const rows=q.get('state')==='open'?Array.from({length:count},(_,i)=>({id:i+1,number:i+1,node_id:'PR_'+(i+1),updated_at:until,head:{sha}})):[];
 console.log(JSON.stringify(rows.slice((page-1)*100,page*100)));
} else if(mode==='pagination'||mode==='overflow'){
 const number=Number(endpoint.split('/')[4]);
 const page=Number(new URL('https://api.github.com/'+endpoint).searchParams.get('page'));
 if(endpoint.includes('/files?'))console.log(JSON.stringify(files(number).slice((page-1)*100,page*100)));
 else if(endpoint.includes('/comments?'))console.log(JSON.stringify([comment(number)]));
 else console.log(JSON.stringify({number,head:{sha},changed_files:files(number).length}));
} else throw Error('Unexpected REST review read: '+endpoint);
`,
      0o700
    );
    return {
      directory,
      cursor,
      invoke: () =>
        spawnSync(
          process.execPath,
          [
            fileURLToPath(new URL('../../scripts/infra-tick-summary.mjs', import.meta.url)),
            '--output',
            directory,
            '--review-map',
            map,
            '--until',
            options.until,
          ],
          {
            encoding: 'utf8',
            timeout: 30_000,
            env: {
              ...process.env,
              PATH: `${directory}${path.delimiter}${process.env.PATH}`,
              VOLUME_COUNT: String(count),
              VOLUME_MODE: mode,
              TMT_AGENT: '1',
              BROWSER: 'true',
            },
          }
        ),
      report: () => JSON.parse(readFileSync(path.join(directory, 'summary.json'), 'utf8')),
      currentCursor: () => readFileSync(cursorFile, 'utf8'),
      calls: () =>
        readFileSync(path.join(directory, 'calls.jsonl'), 'utf8')
          .trim()
          .split('\n')
          .map((line) => JSON.parse(line)),
      cleanup: () => rmSync(directory, { recursive: true, force: true }),
    };
  }
  it.each([20, 30])('completes a %i-PR, 52-call non-review window in one invocation', (count) => {
    const test = volume(count);
    try {
      const result = test.invoke();
      expect(result.status, result.stdout + result.stderr).toBe(0);
      expect(test.report()).toMatchObject({
        complete: true,
        until: options.until,
        evidence: { requests: 53, reviewQueries: 1, reviewPoints: 1 },
      });
      expect(
        test.report().reviewEvidence.map((row: any) => ({
          pr: row.pr,
          head: row.head,
          shared: row.shared,
          statements: row.statements.map((statement: any) => ({
            lead: statement.lead,
            head: statement.head,
            url: statement.url,
          })),
        }))
      ).toEqual(
        Array.from({ length: count }, (_, i) => ({
          pr: i + 1,
          head: SHA,
          shared: ['.github/file-0.yml'],
          statements: [
            {
              lead: 'tmt-lead',
              head: SHA,
              url: `https://github.com/pj-tmt/tmt/pull/${i + 1}#issuecomment-${i + 1}`,
            },
          ],
        }))
      );
      expect(JSON.parse(test.currentCursor()).until).toBe(options.until);
      expect(
        test.calls().filter((row) => row.endpoint === 'graphql:InfraTickReviews')
      ).toMatchObject([{ graphqlCost: 1, outcome: 'success' }]);
      expect(test.calls().some((row) => /\/(?:pulls|issues)\/\d+/.test(row.endpoint ?? ''))).toBe(
        false
      );
    } finally {
      test.cleanup();
    }
  });
  it('falls back only for the PR whose files need another page', () => {
    const test = volume(20, 'pagination');
    try {
      const result = test.invoke();
      expect(result.status, result.stdout + result.stderr).toBe(0);
      expect(test.report()).toMatchObject({ complete: true, evidence: { requests: 57 } });
      const reads = test
        .calls()
        .filter((row) => /\/(?:pulls|issues)\/\d+/.test(row.endpoint ?? ''));
      expect(reads).toHaveLength(4);
      expect(reads.every((row) => /\/(?:pulls|issues)\/1(?:\/|$)/.test(row.endpoint))).toBe(true);
    } finally {
      test.cleanup();
    }
  });
  it.each([
    'overflow',
    'changed-head',
    'truncated',
    'cost',
    'unavailable',
    'corrupt',
    'hydration-budget',
  ])('refuses %s without advancing the cursor or claiming health', (mode) => {
    const test = volume(20, mode);
    try {
      const result = test.invoke();
      expect(result.status, result.stdout + result.stderr).toBe(1);
      expect(test.currentCursor()).toBe(test.cursor);
      expect(test.report().complete).toBe(false);
      expect(result.stdout).not.toContain('No anomalies');
      if (mode !== 'corrupt')
        expect(test.report().anomalies[0]).toMatchObject(
          mode === 'hydration-budget'
            ? { stage: 'PR hydration', prCount: 50 }
            : { stage: 'PR review', prCount: 20 }
        );
      if (mode === 'hydration-budget') {
        expect(test.report().evidence.requests).toBe(52);
        expect(test.calls().some((row) => /\/pulls\/\d+$/.test(row.endpoint ?? ''))).toBe(false);
        expect(test.calls().some((row) => row.endpoint === 'graphql:InfraTickReviews')).toBe(false);
      }
      if (mode === 'overflow') expect(test.report().evidence.requests).toBe(80);
      if (mode === 'cost')
        expect(
          test.calls().find((row) => row.endpoint === 'graphql:InfraTickReviews')
        ).toMatchObject({ graphqlCost: 21, outcome: 'error' });
    } finally {
      test.cleanup();
    }
  });
});

it('fetches the full REST comments only for a paginated PR, including an older head statement', () => {
  const api = fixture({
    open: [
      { id: 1, number: 1 },
      { id: 2, number: 2 },
    ],
    files: [{ filename: '.github/workflows/ci.yml' }],
    comments: Array.from({ length: 101 }, (_, i) => ({
      id: i + 1,
      body: i === 0 ? `Observed ${SHA}\nAgent: tmt-lead codex-sol-high` : `Note ${i}`,
      html_url: `https://github.com/pj-tmt/tmt/pull/1#issuecomment-${i + 1}`,
    })),
  });
  const reviews = api.reviews;
  api.reviews = (ids) =>
    reviews(ids).map((row, i) =>
      i === 0
        ? row
        : {
            ...row,
            comments: {
              totalCount: 0,
              pageInfo: { hasPreviousPage: false, hasNextPage: false },
              nodes: [],
            },
          }
    );
  const report = collectInfraTick(api, options);
  expect(report.complete).toBe(true);
  expect(report.reviewEvidence[0].statements[0]).toMatchObject({
    head: SHA,
    lead: 'tmt-lead',
    url: 'https://github.com/pj-tmt/tmt/pull/1#issuecomment-1',
  });
  const rest = api.calls.filter((endpoint) => /\/(?:pulls|issues)\/\d+/.test(endpoint));
  expect(rest).toHaveLength(4);
  expect(rest.every((endpoint) => /\/(?:pulls|issues)\/1(?:\/|$)/.test(endpoint))).toBe(true);
});

it.each(['comments', 'files', 'count', 'repository', 'duplicate'])(
  'refuses malformed batched %s evidence instead of treating it as pagination',
  (mode) => {
    const api = fixture({
      open: [{ id: 1, number: 1 }],
      files: [{ filename: '.github/workflows/ci.yml' }],
      comments: [{ id: 1, body: 'Note' }],
    });
    const reviews = api.reviews;
    api.reviews = (ids) =>
      reviews(ids).map((row) => {
        if (mode === 'comments') row.comments.nodes = [];
        if (mode === 'files') row.files.nodes = [];
        if (mode === 'count') row.changedFiles++;
        if (mode === 'repository') row.repository.nameWithOwner = 'foreign/repo';
        if (mode === 'duplicate') {
          row.files.nodes.push(row.files.nodes[0]);
          row.files.totalCount++;
          row.changedFiles++;
        }
        return row;
      });
    expect(() => collectInfraTick(api, options)).toThrow(/Truncated|file count|identity|Duplicate/);
    expect(api.calls.some((endpoint) => /\/(?:pulls|issues)\/\d+/.test(endpoint))).toBe(false);
  }
);

it('admits exactly the two fixed read-only queries, counts review cost and rejects variable injection', () => {
  const cache = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-query-guard-'));
  const calls: string[][] = [],
    trace: unknown[] = [];
  const api = createTickClient({
    repository: options.repository,
    cache,
    trace: (row) => trace.push(row),
    execute: (_program, args) => {
      calls.push(args);
      return JSON.stringify(
        args.includes(`query=${REVIEW_QUERY}`)
          ? { data: { rateLimit: { cost: 2 }, nodes: [] } }
          : { data: { repository: { mergeQueue: { entries: { totalCount: 0 } } } } }
      );
    },
  });
  try {
    for (const ids of [
      ['@/private/credential'],
      ['PR_1) { mutation { unsafe } }'],
      ['PR_1', 'PR_1'],
      Array.from({ length: 101 }, (_, i) => `PR_${i}`),
    ])
      expect(() => api.reviews(ids)).toThrow(/node|variables/);
    expect(calls).toHaveLength(0);
    api.reviews(['PR_1']);
    api.queue();
    expect(api.evidence()).toMatchObject({
      requests: 2,
      queueQueries: 1,
      reviewQueries: 1,
      reviewPoints: 2,
    });
    expect(trace[0]).toMatchObject({ endpoint: 'graphql:InfraTickReviews', graphqlCost: 2 });
    expect(calls[0]).toEqual([
      'api',
      'graphql',
      '--method',
      'POST',
      '-f',
      `query=${REVIEW_QUERY}`,
      '-F',
      'ids[]=PR_1',
    ]);
    expect(REVIEW_QUERY).not.toMatch(/\bmutation\b/);
    expect(() => api.reviews(['PR_2'])).toThrow('one review query');
    expect(() => api.queue()).toThrow('one queue query');
    expect(calls).toHaveLength(2);
  } finally {
    rmSync(cache, { recursive: true, force: true });
  }
});

it.each([undefined, 0, -1, 1.5, '1'])(
  'refuses invalid GraphQL cost %s before accepting review evidence',
  (cost) => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'infra-tick-query-cost-'));
    const api = createTickClient({
      repository: options.repository,
      cache,
      execute: () => JSON.stringify({ data: { rateLimit: { cost }, nodes: [] } }),
    });
    try {
      expect(() => api.reviews(['PR_1'])).toThrow(/cost missing, invalid/);
      expect(api.evidence().requests).toBe(1);
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  }
);

// Frozen n/u projection from #3857's 2026-10-19 live listing (20 rows).
// Original SHA-256: b6cef107a8b16ef3bc7858678ed195614f2d5c673975e53e9fd404f23b95e04d.
// IDs, node IDs and heads are synthetic fixture enrichment; order/timestamps are unchanged.
const nonMonotonicOpenSnapshot = [
  {
    n: 3856,
    u: '2026-10-19T03:00:08Z',
  },
  {
    n: 3849,
    u: '2026-10-19T02:57:30Z',
  },
  {
    n: 3827,
    u: '2026-10-19T02:41:53Z',
  },
  {
    n: 3855,
    u: '2026-10-19T02:52:07Z',
  },
  {
    n: 3843,
    u: '2026-10-19T01:40:35Z',
  },
  {
    n: 3693,
    u: '2026-10-19T01:36:47Z',
  },
  {
    n: 3356,
    u: '2026-10-19T01:08:28Z',
  },
  {
    n: 3780,
    u: '2026-10-18T14:31:11Z',
  },
  {
    n: 3096,
    u: '2026-10-18T13:39:28Z',
  },
  {
    n: 3692,
    u: '2026-10-18T13:39:27Z',
  },
  {
    n: 3653,
    u: '2026-10-18T13:39:26Z',
  },
  {
    n: 3673,
    u: '2026-10-18T13:39:25Z',
  },
  {
    n: 3674,
    u: '2026-10-18T04:08:55Z',
  },
  {
    n: 3675,
    u: '2026-10-18T03:55:10Z',
  },
  {
    n: 3702,
    u: '2026-10-18T03:43:27Z',
  },
  {
    n: 3610,
    u: '2026-10-18T03:42:24Z',
  },
  {
    n: 3694,
    u: '2026-10-18T03:31:14Z',
  },
  {
    n: 3633,
    u: '2026-10-18T02:02:01Z',
  },
  {
    n: 3102,
    u: '2026-10-17T20:54:37Z',
  },
  {
    n: 3655,
    u: '2026-10-17T17:36:05Z',
  },
];
it('includes the frozen non-monotonic PR listing without dropping a later eligible row', () => {
  const collect = (rows: typeof nonMonotonicOpenSnapshot, since: string) =>
    collectInfraTick(
      fixture({
        open: rows.map(({ n, u }) => ({ id: n, number: n, updated_at: u })),
        files: [{ filename: '.github/workflows/ci.yml' }],
      }),
      { ...options, since, until: '2026-10-19T03:05:45.491Z' }
    );
  const sorted = [...nonMonotonicOpenSnapshot].sort((a, b) => Date.parse(b.u) - Date.parse(a.u));
  expect(collect(sorted, '2026-10-19T01:38:29.018Z').complete).toBe(true);
  const report = collect(nonMonotonicOpenSnapshot, '2026-10-19T01:38:29.018Z');
  expect(report.complete).toBe(true);
  expect(report.reviewEvidence.map(({ pr }) => pr)).toContain(3855);
  // #3827 is older than this boundary; #3855, later on the same page, is eligible.
  expect(
    collect(nonMonotonicOpenSnapshot, '2026-10-19T02:45:00Z').reviewEvidence.map(({ pr }) => pr)
  ).toContain(3855);
});

describe('PR discovery membership and hydration boundaries', () => {
  const old = new Date(Date.parse(options.since) - 1000).toISOString();
  const pr = (id: number, updated_at = options.until) => ({
    id,
    number: id,
    node_id: `PR_${id}`,
    head: { sha: SHA },
    state: 'open',
    updated_at,
  });
  it('accepts the same members reordered across pages and keeps a newer PR after a full old page', () => {
    const rows = Array.from({ length: 102 }, (_, i) => pr(i + 1, i < 100 ? old : options.until));
    const api = fixture({ open: rows, files: [{ filename: '.github/ci.yml' }] });
    const read = api.get;
    let pass = 0;
    api.get = (endpoint) => {
      const result = read(endpoint);
      if (!endpoint.includes('/pulls?state=open')) return result;
      const page = Number(new URL(`https://api.github.com/${endpoint}`).searchParams.get('page'));
      if (page === 1) pass++;
      const order = pass === 1 ? rows : [...rows].reverse();
      return order.slice((page - 1) * 100, page * 100);
    };
    const report = collectInfraTick(api, options);
    expect(report.complete).toBe(true);
    expect(report.reviewEvidence.map(({ pr }) => pr)).toEqual([101, 102]);
    expect(api.calls.filter((url) => url.includes('state=open'))).toHaveLength(4);
  });
  it.each(['added', 'dropped', 'shrunken page'])('refuses a %s member observation', (kind) => {
    const first = Array.from({ length: kind === 'shrunken page' ? 100 : 2 }, (_, i) => pr(i + 1));
    const second = kind === 'added' ? [...first, pr(999)] : first.slice(0, -1);
    const api = fixture({ open: first });
    const read = api.get;
    let pass = 0;
    api.get = (endpoint) => {
      const result = read(endpoint);
      if (!endpoint.includes('/pulls?state=open')) return result;
      const page = Number(new URL(`https://api.github.com/${endpoint}`).searchParams.get('page'));
      if (page === 1) pass++;
      return (pass === 1 ? first : second).slice((page - 1) * 100, page * 100);
    };
    expect(() => collectInfraTick(api, options)).toThrow('Changing recent PR listing.');
    expect(api.calls).not.toContain('graphql:InfraTickReviews');
  });
  it('refuses a repeated identity on a later page rather than deduplicating it', () => {
    const rows = Array.from({ length: 101 }, (_, i) => pr(i + 1));
    const api = fixture({ open: rows });
    const read = api.get;
    api.get = (endpoint) => {
      const result = read(endpoint);
      return endpoint.includes('/pulls?state=open') && endpoint.endsWith('page=2')
        ? [rows[0]]
        : result;
    };
    expect(() => collectInfraTick(api, options)).toThrow('Duplicate REST identity.');
    expect(api.calls).not.toContain('graphql:InfraTickReviews');
  });
  it('uses inclusive since, discards overlap locally, ignores issues and selects actual merge times', () => {
    const overlap = new Date(Date.parse(options.since) - 500).toISOString();
    const closed = [
      { ...pr(1, options.since), merged_at: options.since },
      { ...pr(2, overlap), merged_at: overlap },
      { ...pr(3), created_at: '2025-01-01T00:00:00Z', merged_at: old },
      { ...pr(4), pull_request: undefined, node_id: 'I_4' },
      { ...pr(5), created_at: '2025-01-01T00:00:00Z', merged_at: options.until },
      { ...pr(6), merged_at: null },
    ].map((row) => ({ ...row, state: 'closed' }));
    const api = fixture({ closed, files: [{ filename: '.github/ci.yml' }] });
    const read = api.get;
    let pass = 0;
    api.get = (endpoint) => {
      const result = read(endpoint);
      if (!endpoint.includes('/issues?state=closed')) return result;
      const query = new URL(`https://api.github.com/${endpoint}`).searchParams;
      expect(query.get('since')).toBe(new Date(Date.parse(options.since) - 1000).toISOString());
      expect(Array.isArray(result)).toBe(true);
      if (!Array.isArray(result)) throw Error('Expected issue fixture page');
      // Issues and pulls have different REST ids; node_id and number bind hydration.
      const projected = result.map((row: any) => ({ ...row, id: row.id + 1_000_000 }));
      return ++pass === 1 ? projected : projected.reverse();
    };
    const report = collectInfraTick(api, options);
    expect(report.complete).toBe(true);
    expect(report.reviewEvidence.map(({ pr }) => pr)).toEqual([1, 5]);
    expect(
      api.calls
        .filter((url) => /\/pulls\/\d+$/.test(url))
        .map((url) => Number(url.split('/').at(-1)))
    ).toEqual([1, 3, 5, 6]);
  });
  it.each([76, 77])(
    'reserves all H reads plus the review batch with %i calls already spent',
    (used) => {
      const base = fixture({
        closed: [1, 2, 3].map((id) => ({ ...pr(id), state: 'closed', merged_at: options.until })),
      });
      const api = { ...base, evidence: () => ({ requests: used }) };
      if (used === 76) {
        expect(collectInfraTick(api, options).complete).toBe(true);
        expect(api.calls.filter((url) => /\/pulls\/\d+$/.test(url))).toHaveLength(3);
        expect(api.calls).toContain('graphql:InfraTickReviews');
      } else {
        expect(() => collectInfraTick(api, options)).toThrow(
          'stage=PR hydration; PRs=3; H=3; required=4; remaining=3'
        );
        expect(api.calls.some((url) => /\/pulls\/\d+$/.test(url))).toBe(false);
        expect(api.calls).not.toContain('graphql:InfraTickReviews');
      }
    }
  );
  it('refuses a closed candidate reopened during hydration', () => {
    const api = fixture({ closed: [{ ...pr(1), state: 'closed', merged_at: options.until }] });
    const read = api.get;
    api.get = (endpoint) =>
      /\/pulls\/1$/.test(endpoint) ? { ...read(endpoint), state: 'open' } : read(endpoint);
    expect(() => collectInfraTick(api, options)).toThrow(
      'Changing closed PR candidate identity or state.'
    );
  });
  it('retains the exact-head refusal between hydration and the review batch', () => {
    const api = fixture({ closed: [{ ...pr(1), state: 'closed', merged_at: options.until }] });
    const reviews = api.reviews;
    api.reviews = (ids) => reviews(ids).map((row) => ({ ...row, headRefOid: 'b'.repeat(40) }));
    expect(() => collectInfraTick(api, options)).toThrow(
      'PR head changed while reading review evidence.'
    );
  });
});
