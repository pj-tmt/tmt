import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

interface Stats {
  n: number;
  median: number | null;
  total: number;
  max: number | null;
}
interface Summary {
  groupRuns: number;
  groupAttempts: number;
  tipPrs: number;
  mergedPrs: number;
  mergesPerHour: number;
  inFlight: number;
  groupDuration: Stats;
  latency: Stats & { prs: { pr: number; minutes: number | null }[] };
  cancelledGroups: number;
  removals: { reason: string }[];
  flakes: { failed: string; passed: string }[];
  jobs: {
    job: string;
    executed: number;
    skipped: number;
    failures: number;
    soleFailures: number;
    duration: Stats;
    wait: Stats;
    eventAttempts: number;
  }[];
}
interface Api {
  get: (endpoint: string, immutable?: boolean) => unknown;
  evidence?: () => unknown;
  qualifyTerminalJobs?: (paths: string[]) => void;
  testedCheckout?: (worker: ReturnType<typeof job>) => unknown;
}
const { summarizeMetrics, collectMetrics, restClient, renderMetrics } = (await import(
  pathToFileURL(path.resolve('scripts/merge-queue-metrics.mjs')).href
)) as unknown as {
  summarizeMetrics: (snapshot: object, since?: string, until?: string) => Summary;
  collectMetrics: (
    api: Api,
    options: object
  ) => { runs: { id: number; inclusion: Record<number, string> }[] };
  renderMetrics: (snapshot: object, boundary?: string) => string;
  restClient: (options: {
    cache: string;
    repo?: string;
    offline?: boolean;
    maxRequests?: number;
    execute?: (command: string, args: string[], options: object) => string;
  }) => Api;
};
const since = '2026-10-02T00:00:00Z';
const boundary = '2026-10-02T01:00:00Z';
const until = '2026-10-02T02:00:00Z';
function checkout(id: number, tree = 'a'.repeat(40), sha = 'b'.repeat(40)) {
  return {
    sha,
    tree,
    logPath: `repos/pj-tmt/tmt/actions/jobs/${id}/logs`,
    logSha256: 'c'.repeat(64),
    reason: null as string | null,
  };
}
function job(id = 1, conclusion = 'success') {
  return {
    id,
    name: 'Worker',
    status: 'completed',
    conclusion,
    testedCheckout: checkout(id),
    runner_id: 42,
    labels: ['macos-14'],
    created_at: '2026-10-02T00:00:00Z',
    started_at: '2026-10-02T00:12:00Z',
    completed_at: '2026-10-02T00:32:00Z',
    html_url: `https://example.test/job/${id}`,
    steps: [],
  };
}
function run(id = 1, attempt = 1) {
  return {
    id,
    attempt,
    event: 'merge_group',
    createdAt: since,
    status: 'completed',
    conclusion: 'success',
    duration: 32 as number | null,
    tipPr: 10,
    tree: 'tree-a',
    inclusion: {},
    jobs: [job()],
  };
}
function snapshot() {
  return {
    since,
    until,
    workflow: 'ci.yml',
    tags: [],
    runs: [run()],
    timelines: [
      {
        number: 10,
        mergedAt: boundary,
        events: [
          { event: 'added_to_merge_queue', created_at: '2026-10-01T23:40:00Z' },
          { event: 'removed_from_merge_queue', created_at: '2026-10-01T23:45:00Z' },
          { event: 'added_to_merge_queue', created_at: '2026-10-02T00:30:00Z' },
        ],
      },
    ],
    evidence: { requests: 0, files: [] },
  };
}
describe('merge queue REST metrics', () => {
  it('uses half-open run cohorts and merge-time throughput, with last recorded enqueue latency', () => {
    const source = snapshot();
    source.runs.push({ ...run(2), createdAt: until });
    const before = summarizeMetrics(source, since, boundary);
    const after = summarizeMetrics(source, boundary, until);
    expect(before.groupRuns).toBe(1);
    expect(before.mergedPrs).toBe(0);
    expect(after.groupRuns).toBe(0);
    expect(after.mergedPrs).toBe(1);
    expect(after.mergesPerHour).toBe(1);
    expect(after.latency.prs).toEqual([{ pr: 10, minutes: 30 }]);
  });
  it('distinguishes rerun attempts, run IDs, runner cost, and runner wait', () => {
    const source = snapshot();
    source.runs.push(run(1, 2));
    const result = summarizeMetrics(source);
    expect(result.groupRuns).toBe(1);
    expect(result.groupAttempts).toBe(2);
    expect(result.tipPrs).toBe(1);
    expect(result.jobs[0]?.duration.total).toBe(40);
    expect(result.jobs[0]?.wait.median).toBe(12);
    expect(result.jobs[0]?.eventAttempts).toBe(2);
  });
  it('excludes skipped runner cost, missing timestamps, and in-flight group durations', () => {
    const source = snapshot();
    source.runs[0] = {
      ...run(),
      status: 'in_progress',
      duration: null,
      jobs: [
        { ...job(), name: 'Skipped', conclusion: 'skipped' },
        { ...job(2), name: 'Missing', created_at: '', completed_at: '' },
      ],
    };
    const result = summarizeMetrics(source);
    expect(result.inFlight).toBe(1);
    expect(result.groupDuration.n).toBe(0);
    expect(result.jobs[0]?.skipped).toBe(1);
    expect(result.jobs[0]?.duration.total).toBe(0);
    expect(result.jobs[1]?.wait.n).toBe(0);
    expect(result.jobs[1]?.duration.n).toBe(0);
  });
  it('retains enqueue latency when the queue bot emits removal as part of merging', () => {
    const source = snapshot();
    source.timelines[0]!.events.push({ event: 'removed_from_merge_queue', created_at: boundary });
    source.timelines[0]!.events.push({ event: 'merged', created_at: boundary });
    expect(summarizeMetrics(source).latency.prs).toEqual([{ pr: 10, minutes: 30 }]);
  });
  it('counts the sole worker failure without double-counting propagated aggregate failures', () => {
    const source = snapshot();
    source.runs[0].jobs = [job(1, 'failure'), { ...job(2, 'failure'), name: 'Docker E2E' }];
    const result = summarizeMetrics(source);
    expect(result.jobs[0]?.soleFailures).toBe(1);
    expect(result.jobs[1]?.failures).toBe(1);
    expect(result.jobs[1]?.soleFailures).toBe(0);
    source.runs[0].jobs.push({ ...job(3, 'failure'), name: 'Other worker' });
    expect(summarizeMetrics(source).jobs[0]?.soleFailures).toBe(0);
  });
  it('requires matching trees, event and runner plus temporal order for fail-to-pass candidates', () => {
    const source = snapshot();
    source.runs[0].jobs = [job(1, 'failure')];
    source.runs.push({ ...run(1, 2), jobs: [{ ...job(2), started_at: boundary }] });
    expect(summarizeMetrics(source).flakes).toEqual([
      {
        job: 'Worker',
        tree: 'a'.repeat(40),
        failed: 'https://example.test/job/1',
        passed: 'https://example.test/job/2',
      },
    ]);
    source.runs[1]!.jobs[0]!.testedCheckout.tree = 'd'.repeat(40);
    expect(summarizeMetrics(source).flakes).toEqual([]);
    source.runs[1]!.jobs[0]!.testedCheckout.tree = 'a'.repeat(40);
    source.runs[1]!.jobs[0]!.labels = ['ubuntu-24.04'];
    expect(summarizeMetrics(source).flakes).toEqual([]);
  });
  it('never treats an API source tree or an unproven checkout as tested-tree authority', () => {
    const source = snapshot();
    source.runs[0].jobs = [job(1, 'failure')];
    source.runs.push({ ...run(1, 2), jobs: [{ ...job(2), started_at: boundary }] });
    // Different checkout commits with equal trees still qualify; source tree can differ.
    source.runs[1]!.tree = 'different-api-source-tree';
    source.runs[1]!.jobs[0]!.testedCheckout.sha = 'd'.repeat(40);
    expect(summarizeMetrics(source).flakes).toHaveLength(1);
    source.runs[1]!.jobs[0]!.testedCheckout.reason = 'Checkout unknown';
    expect(summarizeMetrics(source).flakes).toEqual([]);
    source.runs[1]!.jobs[0]!.testedCheckout = undefined as never;
    expect(summarizeMetrics(source).flakes).toEqual([]);
    source.runs[0].jobs[0]!.testedCheckout = undefined as never;
    source.runs[1]!.tree = source.runs[0].tree;
    expect(summarizeMetrics(source).flakes).toEqual([]);
  });
  it('reports cancellations separately and does not invent invalidation causes or enqueue times', () => {
    const source = snapshot();
    source.runs[0].conclusion = 'cancelled';
    source.timelines[0]!.events.push({
      event: 'removed_from_merge_queue',
      created_at: '2026-10-02T00:45:00Z',
    });
    const result = summarizeMetrics(source);
    expect(result.cancelledGroups).toBe(1);
    expect(result.removals[0]?.reason).toContain('unknown');
    expect(result.latency.prs).toEqual([{ pr: 10, minutes: 30 }]);
    source.timelines[0]!.events = [];
    expect(summarizeMetrics(source).latency.n).toBe(0);
  });
  it('rejects invalid windows and boundaries', () => {
    expect(() => summarizeMetrics(snapshot(), until, since)).toThrow('until');
    expect(() => renderMetrics(snapshot(), until)).toThrow('boundary');
    expect(() => summarizeMetrics(snapshot(), '2026-10-02T00:00:00+00:00')).toThrow('UTC');
  });
  it('makes only bounded gh REST GET calls, reuses immutable evidence, and fails closed offline', () => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'tmt-queue-metrics-'));
    const calls: string[][] = [];
    const endpoint = 'repos/pj-tmt/tmt/actions/runs/1';
    try {
      const api = restClient({
        cache,
        maxRequests: 1,
        execute: (command, args) => {
          expect(command).toBe('gh');
          calls.push(args);
          return '{"id":1}';
        },
      });
      expect(api.get(endpoint, true)).toEqual({ id: 1 });
      expect(api.get(endpoint, true)).toEqual({ id: 1 });
      expect(calls).toEqual([['api', '--method', 'GET', endpoint]]);
      expect(() => api.get(endpoint)).toThrow('budget');
      expect(() => api.get('graphql')).toThrow('REST paths');
      const offline = restClient({ cache, offline: true });
      expect(offline.get(endpoint)).toEqual({ id: 1 });
      expect(() => offline.get(`${endpoint}/jobs`)).toThrow('Offline evidence missing');
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('promotes mutable jobs only after a fresh complete terminal acquisition, then reuses online/offline', () => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'tmt-queue-terminal-'));
    let completed = false;
    let jobReads = 0;
    try {
      const api = restClient({
        cache,
        execute: (_command, args) => {
          const endpoint = args.at(-1)!;
          if (endpoint.includes('/actions/workflows/'))
            return JSON.stringify({
              workflow_runs: endpoint.includes('event=merge_group')
                ? [
                    {
                      id: 1,
                      event: 'merge_group',
                      created_at: since,
                      run_started_at: since,
                      run_attempt: 1,
                      status: completed ? 'completed' : 'in_progress',
                      conclusion: completed ? 'success' : null,
                      head_sha: 'source',
                    },
                  ]
                : [],
            });
          if (endpoint.includes('/pulls?')) return '[]';
          if (endpoint.includes('/jobs?')) {
            jobReads++;
            return JSON.stringify({
              total_count: 1,
              jobs: [
                {
                  ...job(),
                  testedCheckout: undefined,
                  status: completed ? 'completed' : 'in_progress',
                  conclusion: completed ? 'success' : null,
                  completed_at: completed ? job().completed_at : null,
                },
              ],
            });
          }
          throw new Error('Unexpected ' + endpoint);
        },
      });
      const first = collectMetrics(api, { since, until, tagPrs: [] });
      expect(summarizeMetrics(first).inFlight).toBe(1);
      completed = true;
      expect(summarizeMetrics(collectMetrics(api, { since, until, tagPrs: [] })).inFlight).toBe(0);
      expect(jobReads).toBe(2);
      collectMetrics(api, { since, until, tagPrs: [] });
      expect(jobReads).toBe(2);
      const offline = restClient({
        cache,
        offline: true,
        execute: () => {
          throw new Error('must not execute');
        },
      });
      expect(
        summarizeMetrics(collectMetrics(offline, { since, until, tagPrs: [] })).groupDuration.n
      ).toBe(1);
      expect(jobReads).toBe(2);
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('refuses legacy or partially acquired job pages as offline terminal evidence', () => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'tmt-queue-unqualified-'));
    const endpoint = 'repos/pj-tmt/tmt/actions/runs/1/attempts/1/jobs?per_page=30&page=1';
    const file = path.join(cache, createHash('sha256').update(endpoint).digest('hex') + '.json');
    try {
      const legacy = { path: endpoint, data: { jobs: [job()], total_count: 1 } };
      writeFileSync(file, JSON.stringify(legacy));
      const offline = restClient({ cache, offline: true });
      expect(() => offline.get(endpoint, true)).toThrow('unqualified');
      expect(offline.get(endpoint)).toEqual(legacy.data);
      let reads = 0;
      const api = restClient({
        cache,
        execute: () => {
          reads++;
          return JSON.stringify(legacy.data);
        },
      });
      expect(api.get(endpoint, true)).toEqual(legacy.data);
      expect(reads).toBe(1);
      expect(() => offline.get(endpoint, true)).toThrow('unqualified');
      const collectApi: Api = {
        get: (route) =>
          route.includes('/actions/workflows/')
            ? {
                workflow_runs: route.includes('event=merge_group')
                  ? [
                      {
                        id: 1,
                        event: 'merge_group',
                        created_at: since,
                        run_attempt: 1,
                        status: 'completed',
                      },
                    ]
                  : [],
              }
            : route.includes('/pulls?')
              ? []
              : route.includes('/jobs?')
                ? { jobs: [job()], total_count: 2 }
                : {},
        qualifyTerminalJobs: () => {
          throw new Error('must not qualify incomplete acquisition');
        },
      };
      expect(() => collectMetrics(collectApi, { since, until, tagPrs: [] })).toThrow(
        'Incomplete REST page'
      );
      expect(JSON.parse(readFileSync(file, 'utf8'))).not.toHaveProperty('terminalJobs');
      collectApi.get = (route) =>
        route.includes('/actions/workflows/')
          ? {
              workflow_runs: route.includes('event=merge_group')
                ? [
                    {
                      id: 1,
                      event: 'merge_group',
                      created_at: since,
                      run_attempt: 1,
                      status: 'completed',
                    },
                  ]
                : [],
            }
          : route.includes('/pulls?')
            ? []
            : { jobs: [{ ...job(), status: 'in_progress', conclusion: null }], total_count: 1 };
      expect(() => collectMetrics(collectApi, { since, until, tagPrs: [] })).toThrow(
        'Nonterminal jobs'
      );
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('never qualifies earlier pages when the last terminal page is still in progress', () => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'tmt-queue-pages-'));
    const endpoint = 'repos/pj-tmt/tmt/actions/runs/1/attempts/1/jobs?per_page=30&page=1';
    let complete = false;
    const reads: number[] = [];
    try {
      const api = restClient({
        cache,
        execute: (_command, args) => {
          const route = args.at(-1)!;
          if (route.includes('/actions/workflows/'))
            return JSON.stringify({
              workflow_runs: route.includes('event=merge_group')
                ? [
                    {
                      id: 1,
                      event: 'merge_group',
                      created_at: since,
                      run_attempt: 1,
                      status: 'completed',
                    },
                  ]
                : [],
            });
          if (route.includes('/pulls?')) return '[]';
          const page = Number(route.split('page=').at(-1));
          reads.push(page);
          return JSON.stringify({
            total_count: 31,
            jobs:
              page === 1
                ? Array.from({ length: 30 }, (_, index) => job(index + 1))
                : [
                    {
                      ...job(31),
                      status: complete ? 'completed' : 'in_progress',
                      conclusion: complete ? 'success' : null,
                    },
                  ],
          });
        },
      });
      expect(() => collectMetrics(api, { since, until, tagPrs: [] })).toThrow('Nonterminal jobs');
      expect(() => restClient({ cache, offline: true }).get(endpoint, true)).toThrow('unqualified');
      complete = true;
      collectMetrics(api, { since, until, tagPrs: [] });
      expect(reads).toEqual([1, 2, 1, 2]);
      collectMetrics(api, { since, until, tagPrs: [] });
      expect(reads).toEqual([1, 2, 1, 2]);
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it.each(['count-change', 'duplicate-id'])(
    'refuses %s across terminal job pages before qualifying them',
    (variant) => {
      const api: Api = {
        get: (route) => {
          if (route.includes('/actions/workflows/'))
            return {
              workflow_runs: route.includes('event=merge_group')
                ? [
                    {
                      id: 1,
                      event: 'merge_group',
                      created_at: since,
                      run_attempt: 1,
                      status: 'completed',
                    },
                  ]
                : [],
            };
          if (route.includes('/pulls?')) return [];
          const first = route.endsWith('page=1');
          return {
            total_count: !first && variant === 'count-change' ? 30 : 31,
            jobs: first
              ? Array.from({ length: 30 }, (_, index) => job(index + 1))
              : [job(variant === 'duplicate-id' ? 1 : 31)],
          };
        },
        qualifyTerminalJobs: () => {
          throw new Error('must not qualify inconsistent acquisition');
        },
      };
      expect(() => collectMetrics(api, { since, until, tagPrs: [] })).toThrow(
        variant === 'count-change'
          ? 'Inconsistent terminal job count'
          : 'Incomplete or duplicate terminal jobs'
      );
    }
  );
  // Original PR1942 equality control from issue1439/comment6032936565:
  // source64256e5d != checked77e4b1dd, but both immutable trees418a6955 are equal.
  it('retains equal tree authority despite a different synthetic checkout commit', () => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'tmt-queue-equal-tree-'));
    const source = '64256e5d1404b3cb4b312a7f68f404888bdc3c4a';
    const sha = '77e4b1dd9566b04d9b4df78eb4b27407ed4da64e';
    const tree = '418a6955740ad06fda0090e8b7a5383398c0ff8f';
    const log = `##[group]Checking out the ref\n##[endgroup]\n[command]/usr/bin/git log -1 --format=%H\n2026-10-07T04:56:09Z ${sha}\n`;
    try {
      const api = restClient({
        cache,
        execute: (_command, args) =>
          args.at(-1)!.endsWith('/logs') ? log : JSON.stringify({ sha, tree: { sha: tree } }),
      });
      expect(api.testedCheckout!(job())).toMatchObject({ sha, tree, reason: null });
      expect(sha).not.toBe(source);
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  // Original PR1910 checkout proof: issue1439/comment6032936565, original job112415581089.
  // Its synthetic checkout tree differs from Actions sourceTree; no historical false-flake claim.
  it('resolves the logged synthetic worker commit instead of the API source head', () => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'tmt-queue-checkout-'));
    const sha = '518e4c159b40cb05e66ddcd6511907cd8b235600';
    const tree = 'bc81a53c45e817692beda9f583852ff61d26b615';
    const log = `2026-10-06T17:47:36.5835632Z ##[group]Checking out the ref\n2026-10-06T17:47:36.8000911Z ##[endgroup]\n2026-10-06T17:47:36.8034614Z [command]/usr/bin/git log -1 --format=%H\n2026-10-06T17:47:36.8056879Z ${sha}\n`;
    const calls: string[][] = [];
    try {
      const api = restClient({
        cache,
        execute: (_command, args) => {
          calls.push(args);
          return args.at(-1)!.endsWith('/logs')
            ? log
            : JSON.stringify({ sha, tree: { sha: tree } });
        },
      });
      const worker = { ...job(112415581089), testedCheckout: undefined as never };
      expect(api.testedCheckout!(worker)).toEqual({
        sha,
        tree,
        logPath: 'repos/pj-tmt/tmt/actions/jobs/112415581089/logs',
        logSha256: createHash('sha256').update(log).digest('hex'),
        reason: null,
      });
      expect(tree).not.toBe('0eb714c7b7730042ba788d8ad828e34cdabd1f24');
      expect(calls).toEqual([
        [
          'api',
          '--method',
          'GET',
          '--allow-escape-sequences',
          'repos/pj-tmt/tmt/actions/jobs/112415581089/logs',
        ],
        ['api', '--method', 'GET', `repos/pj-tmt/tmt/git/commits/${sha}`],
      ]);
      expect(restClient({ cache, offline: true }).testedCheckout!(worker)).toEqual(
        api.testedCheckout!(worker)
      );
      expect(calls).toHaveLength(2);
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it.each(['missing', 'ambiguous', 'unbound', 'oversized', 'wrong-commit'])(
    'keeps %s worker checkout evidence unknown without source fallback',
    (variant) => {
      const cache = mkdtempSync(path.join(os.tmpdir(), 'tmt-queue-unknown-'));
      const sha = 'a'.repeat(40);
      let commitReads = 0;
      const log = `##[group]Checking out the ref\n##[endgroup]\n[command]/usr/bin/git log -1 --format=%H\n2026-10-02T00:00:00Z ${sha}\n`;
      try {
        const api = restClient({
          cache,
          execute: (_command, args) => {
            if (variant === 'oversized') throw new Error('spawnSync gh ENOBUFS');
            if (args.at(-1)!.endsWith('/logs'))
              return variant === 'missing'
                ? 'No checkout'
                : variant === 'ambiguous'
                  ? log + log
                  : variant === 'unbound'
                    ? log.replace(
                        '##[endgroup]\n[command]',
                        '##[endgroup]\nA later step\n[command]'
                      )
                    : log;
            commitReads++;
            return JSON.stringify({
              sha: variant === 'wrong-commit' ? 'b'.repeat(40) : sha,
              tree: { sha: 'c'.repeat(40) },
            });
          },
        });
        expect(api.testedCheckout!(job())).toMatchObject({
          sha: null,
          tree: null,
          reason: expect.any(String),
        });
        expect(commitReads).toBe(variant === 'wrong-commit' ? 1 : 0);
        expect(restClient({ cache, offline: true }).testedCheckout!(job())).toMatchObject({
          sha: null,
          tree: null,
        });
      } finally {
        rmSync(cache, { recursive: true, force: true });
      }
    }
  );
  it('keeps the REST budget a hard bound even while acquiring checkout proof', () => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'tmt-queue-checkout-budget-'));
    const log = `##[group]Checking out the ref\n##[endgroup]\n[command]/usr/bin/git log -1 --format=%H\n2026-10-02T00:00:00Z ${'a'.repeat(40)}\n`;
    let requests = 0;
    try {
      const api = restClient({
        cache,
        maxRequests: 1,
        execute: () => {
          requests++;
          return log;
        },
      });
      expect(() => api.testedCheckout!(job())).toThrow('REST budget 1 exhausted');
      expect(requests).toBe(1);
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('collects attempts and tags queued squash-head ancestry without using PR head SHA', () => {
    const calls: string[] = [];
    const api: Api = {
      get: (endpoint) => {
        calls.push(endpoint);
        if (endpoint.includes('/actions/workflows/'))
          return {
            workflow_runs: endpoint.includes('event=merge_group')
              ? [
                  {
                    id: 1,
                    event: 'merge_group',
                    created_at: since,
                    run_attempt: 2,
                    status: 'completed',
                    head_sha: 'queue-squash',
                    head_branch: 'gh-readonly-queue/main/pr-963-abcdef',
                    head_commit: { tree_id: 'tree-a' },
                  },
                  {
                    id: 2,
                    event: 'merge_group',
                    created_at: boundary,
                    run_attempt: 1,
                    status: 'completed',
                    head_sha: 'later-group',
                    head_branch: 'gh-readonly-queue/main/pr-964-abcdef',
                    head_commit: { tree_id: 'tree-b' },
                  },
                ]
              : [],
          };
        if (endpoint.includes('/pulls?')) return [];
        if (/\/pulls\/\d+$/.test(endpoint))
          return {
            number: Number(endpoint.split('/').at(-1)),
            merged_at: null,
            head: { sha: 'original-pr-sha' },
          };
        if (endpoint.includes('/timeline')) return [];
        if (endpoint.includes('/compare/')) return { status: 'ahead' };
        if (endpoint.includes('/jobs?')) return { jobs: [job()] };
        if (endpoint.includes('/attempts/')) return { status: 'completed', run_started_at: since };
        throw new Error(`Unexpected fixture endpoint ${endpoint}`);
      },
    };
    const result = collectMetrics(api, { since, until, tagPrs: [963] });
    expect(result.runs).toHaveLength(3);
    expect(result.runs.map((r) => r.inclusion[963])).toEqual(['included', 'included', 'included']);
    expect(calls.some((endpoint) => endpoint.includes('original-pr-sha'))).toBe(false);
    expect(calls.some((endpoint) => endpoint.includes('/compare/queue-squash...later-group'))).toBe(
      true
    );
  });
  it('collects checkout proof only for failed workers and temporally subsequent matching successes', () => {
    const selected: number[] = [];
    const api: Api = {
      get: (endpoint) => {
        if (endpoint.includes('/actions/workflows/'))
          return {
            workflow_runs: endpoint.includes('event=pull_request')
              ? [1, 2].map((id) => ({
                  id,
                  event: 'pull_request',
                  created_at: id === 1 ? since : boundary,
                  run_attempt: 1,
                  status: 'completed',
                  run_started_at: since,
                  head_sha: 'e'.repeat(40),
                  head_commit: { tree_id: 'f'.repeat(40) },
                }))
              : [],
          };
        if (endpoint.includes('/pulls?')) return [];
        if (endpoint.includes('/jobs?'))
          return {
            jobs: endpoint.includes('/runs/1/')
              ? [job(1, 'failure'), { ...job(3), name: 'Unrelated' }]
              : [
                  { ...job(2), started_at: boundary },
                  { ...job(4), name: 'Unrelated' },
                ],
          };
        throw new Error('Unexpected ' + endpoint);
      },
      testedCheckout: (worker) => {
        selected.push(worker.id);
        return checkout(
          worker.id,
          'a'.repeat(40),
          worker.id === 1 ? 'b'.repeat(40) : 'd'.repeat(40)
        );
      },
    };
    const result = collectMetrics(api, { since, until, tagPrs: [] });
    expect(selected).toEqual([1, 2]);
    expect(result.runs[0]).toMatchObject({ sourceSha: 'e'.repeat(40), sourceTree: 'f'.repeat(40) });
    expect(result.runs[0]).not.toHaveProperty('tree');
    expect(summarizeMetrics(result).flakes).toEqual([
      {
        job: 'Worker',
        tree: 'a'.repeat(40),
        failed: 'https://example.test/job/1',
        passed: 'https://example.test/job/2',
      },
    ]);
    api.testedCheckout = (worker) =>
      checkout(worker.id, worker.id === 1 ? 'a'.repeat(40) : 'c'.repeat(40));
    expect(summarizeMetrics(collectMetrics(api, { since, until, tagPrs: [] })).flakes).toEqual([]);
  });
  it('reads every bounded page instead of silently dropping runs after the first page', () => {
    const calls: string[] = [];
    const api: Api = {
      get: (endpoint) => {
        calls.push(endpoint);
        if (endpoint.includes('/actions/workflows/')) {
          const ids = !endpoint.includes('event=merge_group')
            ? []
            : endpoint.endsWith('page=1')
              ? Array.from({ length: 30 }, (_, index) => index + 1)
              : [31];
          return {
            workflow_runs: ids.map((id) => ({
              id,
              event: 'merge_group',
              created_at: since,
              run_attempt: 1,
              status: 'completed',
              head_sha: `sha-${id}`,
            })),
          };
        }
        if (endpoint.includes('/jobs?')) return { jobs: [] };
        if (endpoint.includes('/pulls?')) return [];
        throw new Error(`Unexpected fixture endpoint ${endpoint}`);
      },
    };
    expect(collectMetrics(api, { since, until, tagPrs: [] }).runs).toHaveLength(31);
    expect(
      calls.some(
        (endpoint) =>
          endpoint.includes('event=merge_group') && endpoint.endsWith('per_page=30&page=2')
      )
    ).toBe(true);
  });
  it('refuses a short page whose advertised count proves evidence is incomplete', () => {
    expect(() =>
      collectMetrics({ get: () => ({ workflow_runs: [], total_count: 1 }) }, { since, until })
    ).toThrow('Incomplete REST page');
  });
  it('terminates with an error when pagination never yields a final page', () => {
    let requests = 0;
    expect(() =>
      collectMetrics(
        {
          get: () => {
            requests += 1;
            return { workflow_runs: Array.from({ length: 30 }, () => ({})) };
          },
        },
        { since, until }
      )
    ).toThrow('Pagination bound exceeded');
    expect(requests).toBe(100);
  });
  it('restricts requests to the explicitly selected repository', () => {
    const cache = mkdtempSync(path.join(os.tmpdir(), 'tmt-queue-repo-'));
    try {
      const calls: string[][] = [];
      const api = restClient({
        cache,
        repo: 'example/moved',
        execute: (_command, args) => {
          calls.push(args);
          return '{}';
        },
      });
      api.get('repos/example/moved/pulls');
      expect(calls).toEqual([['api', '--method', 'GET', 'repos/example/moved/pulls']]);
      expect(() => api.get('repos/pj-tmt/tmt/pulls')).toThrow('REST paths');
      expect(() => restClient({ cache, repo: '../graphql' })).toThrow('OWNER/REPO');
    } finally {
      rmSync(cache, { recursive: true, force: true });
    }
  });
  it('fails rather than reporting a truncated run-search result', () => {
    expect(() =>
      collectMetrics({ get: () => ({ workflow_runs: [], total_count: 1001 }) }, { since, until })
    ).toThrow('capped at 1000');
  });
});
