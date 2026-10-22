import { describe, expect, it } from 'vite-plus/test';
import {
  collectDemand,
  demandReader,
  renderDemand,
  traceWorkflows,
  type WorkflowSource,
} from '../../scripts/ci-macos-demand.mjs';
const since = '2026-10-21T00:00:00Z';
const until = '2026-10-22T00:00:00Z';
const sources: WorkflowSource[] = [
  {
    path: '.github/workflows/ci.yml',
    text: 'jobs:\n  build:\n    uses: ./.github/workflows/prepare.yml\n',
  },
  {
    path: '.github/workflows/prepare.yml',
    text: 'jobs:\n  build:\n    runs-on: ${{ matrix.runner }}\n    strategy:\n      matrix:\n        include:\n          - runner: macos-15\n',
  },
  { path: '.github/workflows/linux.yml', text: 'jobs:\n  check:\n    runs-on: ubuntu-latest\n' },
];
const workflow = { id: 11, path: sources[0]!.path, name: 'CI' };
const run = {
  id: 21,
  workflow_id: 11,
  event: 'pull_request',
  created_at: since,
  run_attempt: 1,
  status: 'completed',
};
const job = {
  id: 31,
  run_id: 21,
  name: 'Build macOS',
  labels: ['macos-15'],
  status: 'completed',
  conclusion: 'failure',
  created_at: since,
  started_at: '2026-10-21T00:01:00Z',
  completed_at: '2026-10-21T00:03:00Z',
  steps: [{ name: 'build', status: 'completed' }],
};
function fixture(
  options: { runs?: object[]; jobs?: object[]; override?: (path: string) => unknown } = {}
) {
  const calls: string[] = [];
  const read = Object.assign(
    (path: string): unknown => {
      calls.push(path);
      const replacement = options.override?.(path);
      if (replacement !== undefined) return replacement;
      if (path.startsWith('actions/workflows?'))
        return {
          total_count: 2,
          workflows: [workflow, { id: 12, path: sources[2]!.path, name: 'Linux' }],
        };
      if (path.includes('/runs?'))
        return { total_count: options.runs?.length ?? 1, workflow_runs: options.runs ?? [run] };
      if (path.includes('/jobs?'))
        return { total_count: options.jobs?.length ?? 1, jobs: options.jobs ?? [job] };
      throw new Error(`Unexpected GET ${path}`);
    },
    { requests: () => calls.length, remaining: () => 399 - calls.length }
  );
  return { read, calls };
}
function collect(options: Parameters<typeof fixture>[0] = {}) {
  const f = fixture(options);
  return { result: collectDemand(f.read, { since, until, sources }), calls: f.calls };
}
it('traces reusable macOS jobs and lists safe exclusions', () => {
  expect(traceWorkflows(sources)).toEqual([
    { path: sources[0]!.path, macos: true, evidence: [sources[1]!.path] },
    { path: sources[1]!.path, macos: true, evidence: [sources[1]!.path] },
    { path: sources[2]!.path, macos: false, evidence: [] },
  ]);
  for (const text of [
    'jobs:\n  a:\n    uses: ./.github/workflows/missing.yml',
    'jobs:\n  a:\n    uses: org/repo/.github/workflows/x.yml@main',
    'jobs:\n  a:\n    runs-on: ${{ inputs.runner }}',
  ])
    expect(() => traceWorkflows([{ path: 'x.yml', text }])).toThrow();
  expect(() =>
    traceWorkflows([
      { path: sources[0]!.path, text: 'jobs:\n  a:\n    uses: ./.github/workflows/ci.yml' },
    ])
  ).toThrow('cycle');
});
it('hydrates only macOS-capable workflows and measures failed execution occupancy/queue', () => {
  const { result, calls } = collect();
  expect(calls.some((path) => path.includes('workflows/12/runs'))).toBe(false);
  expect(result).toMatchObject({ runs: 1, actualExecutions: 1, retainedCopies: 0 });
  expect(result.rows[0]).toMatchObject({
    workflow: 'CI',
    event: 'pull_request',
    jobs: 1,
    completed: 1,
    runnerMinutes: 2,
    queueSeconds: { n: 1, median: 60, p90: 60, max: 60 },
  });
  expect(result.workflows.find((entry) => entry.id === 12)?.macos).toBe(false);
  expect(renderDemand(result)).toContain('Measured');
});
it('keeps skips, pending, unfinished and invalid timings explicit rather than zero-filled', () => {
  const jobs = [
    job,
    { ...job, id: 32, conclusion: 'skipped' },
    { ...job, id: 33, status: 'queued', conclusion: null, started_at: null, completed_at: null },
    { ...job, id: 34, status: 'in_progress', conclusion: null, completed_at: null },
    { ...job, id: 35, created_at: until },
  ];
  expect(collect({ jobs }).result.rows[0]).toMatchObject({
    jobs: 5,
    skipped: 1,
    pending: 1,
    unfinished: 1,
    invalidTimings: 1,
    completed: 2,
    runnerMinutes: 4,
    queueSeconds: { n: 2, median: 60 },
  });
});
it('separates all events while excluding Linux jobs', () => {
  const events = [
    'pull_request',
    'merge_group',
    'push',
    'schedule',
    'workflow_dispatch',
    'release',
  ];
  const { result } = collect({
    runs: events.map((event, i) => ({ ...run, id: 21 + i, event })),
    override: (path) =>
      path.includes('/jobs?')
        ? {
            total_count: 2,
            jobs: [
              { ...job, run_id: Number(/runs\/(\d+)/.exec(path)![1]) },
              {
                ...job,
                id: 32,
                labels: ['ubuntu-latest'],
                run_id: Number(/runs\/(\d+)/.exec(path)![1]),
              },
            ],
          }
        : undefined,
  });
  expect(result.rows.map((row) => row.event).sort()).toEqual([...events].sort());
  expect(result.actualExecutions).toBe(6);
});
it('refuses before hydration when the minimum census cost cannot fit', () => {
  const f = fixture();
  f.read.remaining = () => 0;
  expect(() => collectDemand(f.read, { since, until, sources })).toThrow(/preflight.*1.*0/);
  expect(f.calls.some((path) => path.includes('/jobs?'))).toBe(false);
});
it('splits 1000-result searches at disjoint second boundaries', () => {
  const ranges: string[] = [];
  const { result } = collect({
    override: (path) => {
      if (!path.includes('/runs?')) return undefined;
      ranges.push(decodeURIComponent(path));
      return { total_count: ranges.length === 1 ? 1000 : 0, workflow_runs: [] };
    },
  });
  expect(result.runs).toBe(0);
  expect(ranges).toHaveLength(3);
  expect(ranges[1]).toContain('2026-10-21T00:00:00.000Z..2026-10-21T11:59:59.000Z');
  expect(ranges[2]).toContain('2026-10-21T12:00:00.000Z..2026-10-21T23:59:59.000Z');
});
it('refuses truncated, duplicate, foreign and out-of-window evidence', () => {
  expect(() =>
    collect({
      override: (path) =>
        path.includes('/runs?') ? { total_count: 2, workflow_runs: [run] } : undefined,
    })
  ).toThrow();
  expect(() => collect({ runs: [run, run] })).toThrow();
  expect(() => collect({ runs: [{ ...run, created_at: until }] })).toThrow();
  expect(() => collect({ jobs: [job, job] })).toThrow();
  expect(() => collect({ jobs: [{ ...job, run_id: 99 }] })).toThrow();
});
it('does not count synthetic retained successes with new IDs as fresh executions', () => {
  const successful = { ...job, conclusion: 'success' };
  const override = (changed: boolean) => (path: string) => {
    const m = /attempts\/(\d+)(\/jobs)?/.exec(path);
    if (!m) return undefined;
    const attempt = Number(m[1]);
    if (!m[2])
      return { id: 21, run_attempt: attempt, run_started_at: `2026-10-21T0${attempt - 1}:00:00Z` };
    return {
      total_count: 1,
      jobs: [
        {
          ...successful,
          id: 30 + attempt,
          run_attempt: attempt,
          ...(changed ? { steps: [{ name: m[1] }] } : {}),
        },
      ],
    };
  };
  const { result } = collect({ runs: [{ ...run, run_attempt: 3 }], override: override(false) });
  expect(result).toMatchObject({ actualExecutions: 1, retainedCopies: 2 });
  expect(result.rows[0]?.runnerMinutes).toBe(2);
  expect(() => collect({ runs: [{ ...run, run_attempt: 2 }], override: override(true) })).toThrow(
    'retained'
  );
});
describe('REST ledger', () => {
  it('counts rate snapshots and failed GET retries, reserving the final core snapshot inside the cap', () => {
    const calls: string[][] = [];
    let failed = false;
    const read = demandReader({
      repo: 'pj-tmt/tmt',
      maxRequests: 5,
      execute: (_cmd, args) => {
        calls.push(args);
        if (args.at(-1) === '/rate_limit')
          return JSON.stringify({
            resources: {
              core: { remaining: calls.length === 1 ? 1000 : 995, limit: 5000, reset: 1 },
            },
          });
        if (!failed) {
          failed = true;
          throw new Error('HTTP 502');
        }
        return '{}';
      },
    });
    read('actions/workflows?per_page=100&page=1');
    read('actions/workflows?per_page=100&page=2');
    expect(() => read('actions/workflows?per_page=100&page=3')).toThrow('budget');
    expect(read.finish()).toMatchObject({
      calls: 5,
      coreBefore: { remaining: 1000 },
      coreAfter: { remaining: 995 },
    });
    expect(calls.every((args) => args[1] === '--method' && args[2] === 'GET')).toBe(true);
    expect(() => read.finish()).toThrow();
  });
  it('rejects unsafe endpoints/repositories and caps above 400', () => {
    expect(() => demandReader({ repo: '../repo' })).toThrow();
    expect(() => demandReader({ maxRequests: 401 })).toThrow();
    const read = demandReader({
      execute: () => '{"resources":{"core":{"remaining":5,"limit":5000,"reset":1}}}',
    });
    expect(() => read('graphql')).toThrow('path');
  });
});

it('does not assume a deleted workflow was Linux: trace its exact SHA or prove zero cohort runs', () => {
  const path = '.github/workflows/deleted.yml';
  const sha = 'a'.repeat(40);
  const f = fixture({
    override: (url) =>
      url.startsWith('actions/workflows?')
        ? { total_count: 1, workflows: [{ id: 11, name: 'Deleted', path }] }
        : url.includes('/runs?')
          ? { total_count: 1, workflow_runs: [{ ...run, head_sha: sha }] }
          : undefined,
  });
  expect(() => collectDemand(f.read, { since, until, sources })).toThrow('source unavailable');
  const result = collectDemand(f.read, {
    since,
    until,
    sources,
    historicalSources: (file, revision) => {
      expect([file, revision]).toEqual([path, sha]);
      return [{ path, text: 'jobs:\n  build:\n    runs-on: macos-15\n' }];
    },
  });
  expect(result.actualExecutions).toBe(1);
  expect(result.workflows[0]?.macos).toBe(true);
  const empty = fixture({
    override: (url) =>
      url.startsWith('actions/workflows?')
        ? { total_count: 1, workflows: [{ id: 11, name: 'Deleted', path }] }
        : url.includes('/runs?')
          ? { total_count: 0, workflow_runs: [] }
          : undefined,
  });
  expect(collectDemand(empty.read, { since, until, sources }).workflows[0]?.reason).toContain(
    'zero runs'
  );
});

it('reports job and disjoint PR associations without multiplying occupancy', () => {
  const { result } = collect({
    runs: [{ ...run, pull_requests: [{ number: 42 }, { number: 43 }] }],
  });
  expect(result.jobRows[0]).toMatchObject({
    name: 'Build macOS',
    runnerMinutes: 2,
    durationSamples: 1,
  });
  expect(result.prRows).toHaveLength(1);
  expect(result.prRows[0]).toMatchObject({ association: '#42, #43 (shared)', runnerMinutes: 2 });
  expect(renderDemand(result)).toContain('PR association');
  expect(collect().result.prRows[0]?.association).toBe('PR unavailable');
});
it('does not invent occupancy for a pending-only group or count self-hosted macOS', () => {
  const { result } = collect({
    jobs: [
      { ...job, status: 'queued', conclusion: null, started_at: null, completed_at: null },
      { ...job, id: 32, labels: ['self-hosted', 'macos-15'] },
      { ...job, id: 33, labels: [] },
    ],
  });
  expect(result.rows[0]).toMatchObject({ jobs: 1, runnerMinutes: null, durationSamples: 0 });
  expect(result.unclassifiedJobs).toBe(1);
  expect(result.selfHostedJobs).toBe(1);
});
