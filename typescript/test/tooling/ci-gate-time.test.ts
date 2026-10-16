import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import {
  AGGREGATE_JOBS,
  REQUIRED_CONTEXTS,
  collectGateRuns,
  ghReader,
  median,
  overBudget,
  parseBudgets,
  percentile,
  renderReport,
  summarizeGate,
  type GateEvent,
  type GateJob,
  type GateRun,
} from '../../scripts/ci-gate-time.mjs';

const fixture = JSON.parse(
  readFileSync(new URL('../fixtures/ci-gate-time.json', import.meta.url), 'utf8')
) as { responses: Record<string, unknown> };
const budgetText = readFileSync(
  new URL('../../scripts/ci-gate-budgets.json', import.meta.url),
  'utf8'
);
const budgets = parseBudgets(budgetText);

/** Serve the recorded GitHub responses; any unrecorded path is a test failure. */
function recorded(log: string[] = []) {
  return (path: string) => {
    log.push(path);
    const response = fixture.responses[path];
    if (response === undefined) throw new Error(`Unrecorded REST path: ${path}`);
    return response;
  };
}

describe('percentiles', () => {
  it('uses the ordinary median and nearest-rank p90', () => {
    expect(median([5, 1, 3])).toBe(3);
    expect(median([4, 1, 3, 2])).toBe(2.5);
    const forty = Array.from({ length: 40 }, (_, index) => index + 1);
    expect(percentile(forty, 0.9)).toBe(36);
    expect(percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.9)).toBe(9);
    expect(percentile([7], 0.9)).toBe(7);
    expect(median([])).toBeNull();
    expect(percentile([], 0.9)).toBeNull();
  });
});

describe('recorded gate cohort', () => {
  const log: string[] = [];
  const runs = collectGateRuns(recorded(log), { count: 6 });

  it('takes the newest successful runs per event and pages through large job lists', () => {
    // The recording holds 8 runs per event; the two oldest of each must be left out.
    expect(runs.filter((run) => run.event === 'pull_request').map((run) => run.id)).toEqual([
      38870415776, 38871651699, 38877178423, 38884841774, 38893555274, 38897303392,
    ]);
    expect(runs.filter((run) => run.event === 'merge_group')).toHaveLength(6);
    // Run 38871651699 rehearses a release and has 121 jobs over two pages.
    expect(runs.find((run) => run.id === 38871651699)?.jobs).toHaveLength(121);
    expect(log.filter((path) => path.includes('/jobs?'))).toHaveLength(13);
    expect(log.every((path) => !path.includes('38868625716'))).toBe(true);
  });

  const summary = summarizeGate(runs);

  it('reproduces gate wall and runner-minutes computed independently with jq', () => {
    // PR gate walls (s): 1578 (attempt 2), 4406 (rehearsal), 891, 883, 1295, 887.
    const pr = summary.pull_request;
    expect(pr).toMatchObject({ runs: 6, measured: 6, retried: 1 });
    expect(pr.gate).toMatchObject({ n: 6, median: 1093, p90: 4406, max: 4406 });
    expect(pr.runnerMinutes.median).toBeCloseTo(64.3333, 3);
    expect(pr.runnerMinutes.p90).toBeCloseTo(276.9333, 3);
    // MG gate walls (s): 895, 883, 889, 880, 829, 858.
    const group = summary.merge_group;
    expect(group.retried).toBe(0);
    expect(group.gate).toMatchObject({ n: 6, median: 881.5, p90: 895, max: 895 });
    expect(group.runnerMinutes.median).toBeCloseTo(58.4167, 3);
  });

  it('measures per-job wall and queue, dropping skipped jobs and negative spans', () => {
    const wall = (event: GateEvent, name: string) =>
      summary[event].jobs.find((job) => job.name === name);
    expect(wall('merge_group', 'Native Rust workspace tests')?.wall).toMatchObject({
      n: 6,
      median: 832,
      p90: 849,
    });
    expect(wall('merge_group', 'Code quality')?.wall).toMatchObject({ n: 6, p90: 182 });
    // The retried PR's reused copy completed before its reset creation time: no queue sample.
    expect(wall('pull_request', 'Native Rust workspace tests')?.wall.n).toBe(6);
    expect(wall('pull_request', 'Native Rust workspace tests')?.queue).toMatchObject({
      n: 5,
      median: 3,
    });
    expect(wall('pull_request', 'Native Rust workspace tests')?.wall.p90).toBe(835);
    // Matrix placeholders only ever appear as skipped jobs.
    const names = summary.pull_request.jobs.map((job) => job.name);
    expect(names.some((name) => name.includes('${{'))).toBe(false);
  });

  it('counts the controlling branch, folding rehearsal stages into one branch', () => {
    expect(summary.merge_group.controlling).toEqual([
      { name: 'Native Rust workspace tests', count: 6, share: 1 },
    ]);
    expect(summary.pull_request.controlling.map(({ name, count }) => [name, count])).toEqual([
      ['Native Rust workspace tests', 4],
      ['Packed install (macOS x64)', 1],
      ['Release rehearsal', 1],
    ]);
  });

  it('flags only values above their budgets and renders them', () => {
    expect(overBudget(summary, budgets).map((item) => item.what)).toEqual(
      expect.arrayContaining(['gate median', 'gate p90'])
    );
    const tight = { ...budgets, gate: { ...budgets.gate } };
    tight.gate.pull_request = { median: 2000, p90: 5000 };
    tight.gate.merge_group = { median: 2000, p90: 5000 };
    const lenient = { gate: tight.gate, jobs: {} };
    expect(overBudget(summary, lenient)).toEqual([]);
    const report = renderReport(summary, budgets);
    expect(report).toContain('| PR | 6 (1) |');
    expect(report).toContain('| Native Rust workspace tests | 6 |');
    expect(report).not.toContain(' / Conclude');
    expect(report).toMatch(/## Over budget\n\n- PR gate median: 1093 s > 800.5 s/);
  });
});

describe('gate measurement edge cases', () => {
  const at = (second: number) => new Date(Date.UTC(2026, 9, 1, 0, 0, second)).toISOString();
  const job = (
    id: number,
    name: string,
    start: number,
    end: number,
    conclusion = 'success'
  ): GateJob => ({
    id,
    name,
    conclusion,
    created_at: at(start - 2),
    started_at: at(start),
    completed_at: at(end),
  });
  const run = (jobs: GateJob[]): GateRun => ({
    id: 1,
    event: 'pull_request',
    createdAt: at(0),
    attempt: 1,
    jobs,
  });
  const measure = (jobs: GateJob[]) => summarizeGate([run(jobs)]).pull_request;

  it('ends the gate at the last selected required context and ignores skipped ones', () => {
    const result = measure([
      job(1, 'Code quality', 2, 100),
      { ...job(2, 'Unit tests', 0, 0, 'skipped'), started_at: at(500), completed_at: at(900) },
      job(3, 'Native Rust workspace tests', 2, 300),
      job(4, 'Native package matrix', 301, 320),
    ]);
    expect(result.gate.median).toBe(320);
    expect(result.controlling[0]?.name).toBe('Native Rust workspace tests');
    expect(result.jobs.find((entry) => entry.name === 'Unit tests')).toBeUndefined();
  });

  it('breaks completion ties by job ID and ignores work finishing after the gate', () => {
    const result = measure([
      job(5, 'Alpha', 2, 200),
      job(9, 'Beta', 2, 200),
      job(6, 'Native package matrix', 201, 210),
      job(7, 'Late advisory work', 2, 400),
    ]);
    expect(result.controlling[0]?.name).toBe('Beta');
  });

  it('lets Code quality control a run whose workers were all skipped', () => {
    const result = measure([
      job(1, 'Select affected areas', 0, 20),
      job(2, 'Code quality', 21, 150),
      job(3, 'Native Rust workspace tests', 0, 0, 'skipped'),
    ]);
    expect(result.gate.median).toBe(150);
    expect(result.controlling).toEqual([{ name: 'Code quality', count: 1, share: 1 }]);
  });

  it('leaves a run unmeasured when no required context ran', () => {
    const result = measure([job(1, 'Alpha', 2, 50), job(2, 'Code quality', 0, 0, 'skipped')]);
    expect(result).toMatchObject({ runs: 1, measured: 0 });
    expect(result.gate.median).toBeNull();
    expect(result.controlling).toEqual([]);
  });

  it('names real ci.yml jobs for every required and aggregate context', () => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    for (const name of [...REQUIRED_CONTEXTS, ...AGGREGATE_JOBS])
      expect(workflow).toMatch(new RegExp(`^ {4}name: ${name}$`, 'm'));
  });
});

describe('collection failures', () => {
  it('refuses too few successful runs and incomplete job lists', () => {
    expect(() => collectGateRuns(recorded(), { count: 9 })).toThrow('need 9');
    expect(() => collectGateRuns(recorded(), { count: 0 })).toThrow('count must be');
    const page = fixture.responses[
      'actions/runs/38871651699/jobs?filter=latest&per_page=100&page=1'
    ] as { jobs: unknown[]; total_count: number };
    const truncated = (path: string) =>
      path.includes('38871651699/jobs') && path.endsWith('page=2')
        ? { total_count: page.total_count, jobs: [] }
        : recorded()(path);
    expect(() => collectGateRuns(truncated, { count: 6 })).toThrow('Incomplete jobs');
    const running = (path: string) => {
      const data = recorded()(path) as { jobs?: { status: string }[] };
      return data.jobs
        ? {
            ...data,
            jobs: data.jobs.map((entry, index) =>
              index ? entry : { ...entry, status: 'in_progress' }
            ),
          }
        : data;
    };
    expect(() => collectGateRuns(running, { count: 6 })).toThrow('not complete');
  });
});

describe('budget file', () => {
  it('records the #3230 baseline for both events and every selected required job', () => {
    expect(budgets.gate.pull_request).toEqual({ median: 800.5, p90: 1095 });
    expect(budgets.gate.merge_group).toEqual({ median: 790, p90: 911 });
    expect(Object.keys(budgets.jobs)).toEqual([
      'Native Rust workspace tests',
      'Docker E2E shard 1/2',
      'Docker E2E shard 2/2',
      'Native process contracts',
      'Code quality',
      'Unit tests',
      'Native package matrix',
    ]);
    expect(budgets.jobs['Native Rust workspace tests']).toEqual({
      pull_request: 866,
      merge_group: 861,
    });
  });

  it('rejects a missing event, a missing job list, or a non-positive figure', () => {
    const edit = (change: (data: Record<string, any>) => void) => {
      const data = JSON.parse(budgetText) as Record<string, any>;
      change(data);
      return JSON.stringify(data);
    };
    expect(() => parseBudgets(edit((data) => delete data.gate.merge_group))).toThrow(
      'Missing gate budget for Merge group'
    );
    expect(() => parseBudgets(edit((data) => (data.jobs = {})))).toThrow('Missing per-job');
    expect(() => parseBudgets(edit((data) => (data.gate.pull_request.p90 = 0)))).toThrow(
      'Invalid budget for PR gate p90'
    );
    expect(() =>
      parseBudgets(edit((data) => delete data.jobs['Code quality'].merge_group))
    ).toThrow('Invalid budget for Code quality Merge group');
  });
});

describe('read-only REST reader', () => {
  const ok = JSON.stringify({ ok: true });
  it('issues only GETs against the repository and retries transient 5xx within the budget', () => {
    const calls: string[][] = [];
    let failures = 2;
    const read = ghReader({
      repo: 'pj-tmt/tmt',
      maxRequests: 5,
      execute: (_command, args) => {
        calls.push(args);
        if (failures-- > 0) throw new Error('gh: Server Error (HTTP 502)');
        return ok;
      },
    });
    expect(read('actions/runs/1/jobs')).toEqual({ ok: true });
    expect(calls).toHaveLength(3);
    expect(calls[0]).toEqual(['api', '--method', 'GET', 'repos/pj-tmt/tmt/actions/runs/1/jobs']);
    expect(read.requests()).toBe(3);
  });

  it('does not retry client errors and stops at the request budget', () => {
    let client = 0;
    const failing = ghReader({
      execute: () => {
        client += 1;
        throw new Error('gh: Not Found (HTTP 404)');
      },
    });
    expect(() => failing('x')).toThrow('HTTP 404');
    expect(client).toBe(1);
    const bounded = ghReader({ maxRequests: 1, execute: () => ok });
    bounded('a');
    expect(() => bounded('b')).toThrow('REST budget 1 exhausted');
    expect(() => ghReader({ repo: '../x' })).toThrow('OWNER/REPO');
  });
});
