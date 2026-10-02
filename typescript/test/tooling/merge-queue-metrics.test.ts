import { mkdtempSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

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
function job(id = 1, conclusion = 'success') {
  return {
    id,
    name: 'Worker',
    conclusion,
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
        tree: 'tree-a',
        failed: 'https://example.test/job/1',
        passed: 'https://example.test/job/2',
      },
    ]);
    source.runs[1]!.tree = 'tree-b';
    expect(summarizeMetrics(source).flakes).toEqual([]);
    source.runs[1]!.tree = 'tree-a';
    source.runs[1]!.jobs[0]!.labels = ['ubuntu-24.04'];
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
