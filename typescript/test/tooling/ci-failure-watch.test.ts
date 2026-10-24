import { describe, expect, it } from 'vite-plus/test';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { writeExecutable } from '../support/executable-fixture.mjs';
import {
  WATCH_LIMITS,
  newWatchState,
  validateRouting,
  routeFailure,
  watchedRun,
  failureSignature,
  observeRun,
  heartbeatStatus,
  notificationGroups,
  formatNotification,
  parseGhResponse,
  collectWatchCycle,
  deliverWatchAlerts,
  createWatchApi,
  type WatchRun,
  type WatchState,
  type WatchApi,
  type WatchAlert,
} from '../../scripts/ci-failure-watch.mjs';

const now = Date.parse('2026-10-24T07:20:00Z');
const iso = (time: number) => new Date(time).toISOString();
const routing = validateRouting({
  source: 'https://github.com/pj-tmt/tmt/issues/606',
  rules: [
    {
      workflowPrefix: '.github/workflows/native-release',
      squad: 'tmt-infra',
      source: '#606 line14 CI/release tooling',
    },
    { jobPrefix: 'Native', squad: 'tmt-core', source: '#606 line10 core' },
    { jobPrefix: 'Remote', squad: 'tmt-remote', source: '#606 line11 Remote' },
  ],
});
// Compact recorded Actions shape: #4752 MG40072107154; no job log is read here.
function run(change: Partial<WatchRun> = {}): WatchRun {
  const id = change.id ?? 40072107154;
  return {
    id,
    run_attempt: 1,
    workflow_id: 1,
    path: '.github/workflows/ci.yml',
    name: 'CI',
    event: 'merge_group',
    head_branch: 'gh-readonly-queue/main/pr-4752',
    status: 'completed',
    conclusion: 'failure',
    created_at: iso(now - 60_000),
    updated_at: iso(now - 1000),
    html_url: `https://github.com/pj-tmt/tmt/actions/runs/${id}`,
    ...change,
  };
}
const jobs = [
  { id: 120807763624, name: 'Native Rust workspace partition 2', conclusion: 'failure' },
];
function effects() {
  const calls: string[] = [];
  const issues = new Map<string, string>();
  const saves: WatchState[] = [];
  return {
    calls,
    issues,
    saves,
    save(state: WatchState) {
      saves.push(structuredClone(state));
      calls.push('save');
    },
    findIssues(signature: string) {
      calls.push('search');
      return issues.has(signature) ? [issues.get(signature)!] : [];
    },
    createIssue(alert: WatchAlert) {
      calls.push('create');
      const url = `https://github.com/pj-tmt/tmt/issues/${issues.size + 1}`;
      issues.set(alert.signature, url);
      return url;
    },
    talk(message: string) {
      calls.push(`talk:${message}`);
      return 'req_fixture';
    },
  };
}
function apiFor(completed: WatchRun[], active: WatchRun[] = [], etags = false) {
  const calls: { endpoint: string; etag?: string }[] = [];
  const api: WatchApi = {
    get(endpoint, etag) {
      calls.push({ endpoint, etag });
      if (etag && etags) return { status: 304, body: null };
      if (endpoint.includes('/jobs?'))
        return { status: 200, body: { total_count: jobs.length, jobs } };
      if (/\/runs\/[0-9]+$/.test(endpoint))
        return {
          status: 200,
          body: completed.find((r) => r.id === Number(endpoint.split('/').at(-1))),
        };
      const query = new URL(`https://fixture/${endpoint}`).searchParams;
      const rows =
        query.get('status') === 'completed'
          ? completed.filter((r) => !query.has('event') || r.event === query.get('event'))
          : active.filter((r) => r.status === query.get('status'));
      return {
        status: 200,
        etag: '"fixture"',
        body: { total_count: rows.length, workflow_runs: rows },
      };
    },
  };
  return { api, calls };
}

describe('finished-run coverage and ETag observations', () => {
  it('uses cached bodies on 304 without repeating jobs or advancing a false failure', () => {
    const state = newWatchState(now);
    state.cursor = iso(now - 60_000);
    const f = apiFor([run()], [], true);
    const first = collectWatchCycle({ state, api: f.api, routing, now });
    const second = collectWatchCycle({
      state: first.state,
      api: f.api,
      routing,
      now: now + 60_000,
    });
    expect(second.examined).toEqual([]);
    expect(f.calls.filter((c) => c.endpoint.includes('/jobs?'))).toHaveLength(1);
    expect(f.calls.filter((c) => c.etag === '"fixture"')).toHaveLength(3);
    expect(Object.values(second.state.alerts)[0].count).toBe(1);
  });
  it('keeps only permitted events, main pushes and an inclusive cursor boundary', () => {
    const state = newWatchState(now);
    state.cursor = iso(now - 1000);
    const rows = [
      run(),
      run({ id: 2, event: 'pull_request' }),
      run({ id: 3, event: 'push', head_branch: 'topic' }),
      run({ id: 4, event: 'push', head_branch: 'main' }),
      run({ id: 5, updated_at: iso(now - 1001) }),
    ];
    const f = apiFor(rows);
    const result = collectWatchCycle({ state, api: f.api, routing, now });
    expect(result.examined.map((x) => x.run)).toEqual([4, 40072107154]);
    expect(watchedRun(run({ event: 'workflow_dispatch' }))).toBe(true);
    expect(watchedRun(run({ event: 'schedule' }))).toBe(true);
  });
  it('checks an old tracked rerun when it leaves active page one, once per attempt', () => {
    const state = newWatchState(now);
    state.cursor = iso(now - 60_000);
    const old = run({
      id: 99,
      created_at: iso(now - 2 * WATCH_LIMITS.lookbackMs),
      status: 'in_progress',
      run_attempt: 2,
    });
    state.tracked['99'] = old;
    const done = { ...old, status: 'completed', conclusion: 'failure' };
    const f = apiFor([done]);
    const result = collectWatchCycle({ state, api: f.api, routing, now });
    expect(result.examined).toEqual([{ run: 99, attempt: 2, conclusion: 'failure' }]);
    expect(result.state.tracked).toEqual({});
    expect(f.calls.filter((c) => c.endpoint === 'repos/pj-tmt/tmt/actions/runs/99')).toHaveLength(
      1
    );
    const again = collectWatchCycle({
      state: result.state,
      api: f.api,
      routing,
      now: now + 60_000,
    });
    expect(again.examined).toEqual([]);
  });
  it('records only one restart gap and persists the current observed attempt', () => {
    const state = newWatchState(now);
    const active = run({ status: 'in_progress', run_attempt: 3 });
    const f = apiFor([], [active]);
    const result = collectWatchCycle({ state, api: f.api, routing, now, initial: true });
    expect(result.state.tracked[String(active.id)].run_attempt).toBe(3);
    const restart = collectWatchCycle({
      state: result.state,
      api: f.api,
      routing,
      now: now + 60_000,
      initial: true,
    });
    expect(Object.values(restart.state.alerts).filter((a) => a.coverage)).toHaveLength(1);
  });
  it('refuses incomplete catch-up rather than silently accepting a capped search', () => {
    const state = newWatchState(now);
    const before = structuredClone(state);
    const api: WatchApi = {
      get: () => ({ status: 200, body: { total_count: 1000, workflow_runs: [] } }),
    };
    expect(() => collectWatchCycle({ state, api, routing, now, initial: true })).toThrow(
      'WATCH_COVERAGE'
    );
    expect(state).toEqual(before);
  });
  it('refuses missing cached 304 and duplicate job identities', () => {
    expect(() =>
      collectWatchCycle({
        state: newWatchState(now),
        api: { get: () => ({ status: 304, body: null }) },
        routing,
        now,
      })
    ).toThrow('304 without');
    const f = apiFor([run()]);
    const original = f.api.get;
    f.api.get = (endpoint, etag) =>
      endpoint.includes('/jobs?')
        ? { status: 200, body: { total_count: 2, jobs: [jobs[0], jobs[0]] } }
        : original(endpoint, etag);
    expect(() =>
      collectWatchCycle({ state: newWatchState(now), api: f.api, routing, now })
    ).toThrow('incomplete jobs');
  });
});

describe('signature recovery, deduplication and routing', () => {
  it('routes workflow before job prefix and labels an unmapped suggestion honestly', () => {
    expect(routeFailure(routing, '.github/workflows/native-release.yml', 'Native test').squad).toBe(
      'tmt-infra'
    );
    expect(routeFailure(routing, '.github/workflows/ci.yml', 'Native test').squad).toBe('tmt-core');
    expect(routeFailure(routing, '.github/workflows/other.yml', 'other').squad).toBe('unmapped');
    expect(() => validateRouting({ source: 'unreviewed', rules: [] })).toThrow('WATCH_ROUTING');
  });
  it('updates one signature inside six hours without another notification or issue', () => {
    const state = newWatchState(now);
    const e = effects();
    observeRun(state, run(), jobs, routing, now);
    deliverWatchAlerts(state, e, now);
    observeRun(state, run({ id: 2 }), jobs, routing, now + 60_000);
    deliverWatchAlerts(state, e, now + 60_000);
    expect(Object.values(state.alerts)[0].count).toBe(2);
    expect(e.calls.filter((c) => c === 'create')).toHaveLength(1);
    expect(e.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(1);
    expect(e.calls.indexOf('search')).toBeLessThan(e.calls.indexOf('create'));
    expect(e.saves.some((s) => s.delivery && s.notifications.length === 1)).toBe(true);
  });
  it('re-arms after green on the same event, never a green on another event', () => {
    const state = newWatchState(now);
    const e = effects();
    observeRun(state, run(), jobs, routing, now);
    deliverWatchAlerts(state, e, now);
    observeRun(
      state,
      run({ id: 2, conclusion: 'success', event: 'schedule', updated_at: iso(now + 1000) }),
      [],
      routing,
      now + 1000
    );
    observeRun(state, run({ id: 3, updated_at: iso(now + 2000) }), jobs, routing, now + 2000);
    deliverWatchAlerts(state, e, now + 2000);
    expect(e.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(1);
    observeRun(
      state,
      run({ id: 4, conclusion: 'success', updated_at: iso(now + 3000) }),
      [],
      routing,
      now + 3000
    );
    observeRun(state, run({ id: 5, updated_at: iso(now + 4000) }), jobs, routing, now + 4000);
    deliverWatchAlerts(state, e, now + 4000);
    expect(e.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(2);
    expect(e.calls.filter((c) => c === 'create')).toHaveLength(1);
  });
  it('reuses the issue after six hours and ignores a previously observed failed attempt', () => {
    const state = newWatchState(now);
    const e = effects();
    const failed = run();
    observeRun(state, failed, jobs, routing, now);
    deliverWatchAlerts(state, e, now);
    observeRun(state, failed, jobs, routing, now + WATCH_LIMITS.dedupMs);
    expect(Object.values(state.alerts)[0].count).toBe(1);
    observeRun(
      state,
      run({ run_attempt: 2, updated_at: iso(now + WATCH_LIMITS.dedupMs) }),
      jobs,
      routing,
      now + WATCH_LIMITS.dedupMs
    );
    deliverWatchAlerts(state, e, now + WATCH_LIMITS.dedupMs);
    expect(e.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(2);
    expect(e.calls.filter((c) => c === 'create')).toHaveLength(1);
    expect(failureSignature(failed, jobs[0].name)).not.toBe(
      failureSignature(failed, 'different job')
    );
  });
});

describe('notification and crash boundaries', () => {
  it('folds a burst by squad, preserves all queued signatures and enforces six calls per hour', () => {
    const state = newWatchState(now);
    for (let i = 0; i < 20; i++)
      observeRun(
        state,
        run({ id: i + 1 }),
        [{ name: `${i % 2 ? 'Native' : 'Remote'} job${i}`, conclusion: 'failure' }],
        routing,
        now
      );
    expect(notificationGroups(state, now)).toHaveLength(2);
    const e = effects();
    deliverWatchAlerts(state, e, now);
    expect(e.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(2);
    expect(Object.values(state.alerts).every((a) => !a.pending)).toBe(true);
    state.notifications = Array(6).fill(now);
    observeRun(
      state,
      run({ id: 50 }),
      [{ name: 'Native new', conclusion: 'failure' }],
      routing,
      now
    );
    expect(notificationGroups(state, now)).toEqual([]);
    expect(notificationGroups(state, now + 3600_000)).toHaveLength(1);
  });
  it('uses the existing searched issue rather than creating a second one', () => {
    const state = newWatchState(now);
    const e = effects();
    observeRun(state, run(), jobs, routing, now);
    const a = Object.values(state.alerts)[0];
    e.issues.set(a.signature, 'https://github.com/pj-tmt/tmt/issues/4758');
    deliverWatchAlerts(state, e, now);
    expect(a.issueUrl).toContain('4758');
    expect(e.calls).not.toContain('create');
    expect(formatNotification([a])).toContain('suggested tmt-core');
  });
  it('does not replay an uncertain talk after a crash or transport refusal', () => {
    const state = newWatchState(now);
    const e = effects();
    observeRun(state, run(), jobs, routing, now);
    e.talk = () => {
      throw new Error('uncertain submission');
    };
    expect(() => deliverWatchAlerts(state, e, now)).toThrow('uncertain submission');
    expect(state.delivery?.signatures).toHaveLength(1);
    expect(() => deliverWatchAlerts(state, effects(), now + 60_000)).toThrow(
      'WATCH_DELIVERY_UNCERTAIN'
    );
  });
  it('searches an uncertain issue create on restart and never blindly creates again', () => {
    const state = newWatchState(now);
    const e = effects();
    observeRun(state, run(), jobs, routing, now);
    e.createIssue = () => {
      throw new Error('unknown POST result');
    };
    expect(() => deliverWatchAlerts(state, e, now)).toThrow('unknown POST');
    expect(state.issueIntent).toBeTruthy();
    expect(() => deliverWatchAlerts(state, effects(), now + 60_000)).toThrow(
      'WATCH_ISSUE_UNCERTAIN'
    );
    const recovered = effects();
    recovered.issues.set(state.issueIntent!, 'https://github.com/pj-tmt/tmt/issues/1');
    deliverWatchAlerts(state, recovered, now + 120_000);
    expect(recovered.calls).not.toContain('create');
    expect(state.issueIntent).toBeNull();
    expect(state.delivery).toBeNull();
  });
  it('returns a dry-run plan without any effect port or mutation of the supplied state', () => {
    const state = newWatchState(now);
    const before = structuredClone(state);
    const f = apiFor([run()]);
    const result = collectWatchCycle({ state, api: f.api, routing, now, dryRun: true });
    expect(state).toEqual(before);
    expect(result.groups).toHaveLength(1);
    expect(result.groups[0][0].issueUrl).toBeUndefined();
    expect(result.examined).toHaveLength(1);
  });
});

describe('GET admission, quota and heartbeat', () => {
  const response = (status = 200, resource = 'core', remaining = 3000) =>
    `HTTP/2.0 ${status} OK\r\nETag: "v1"\r\nX-RateLimit-Resource: ${resource}\r\nX-RateLimit-Remaining: ${remaining}\r\n\r\n${status === 304 ? '' : '{}'}\n`;
  it('parses 304 without JSON and sends If-None-Match without a mutation or log route', () => {
    const calls: string[][] = [];
    const api = createWatchApi({
      execute: (_exe, args) => {
        calls.push(args);
        return response(304);
      },
    });
    expect(api.get('repos/pj-tmt/tmt/actions/runs?status=completed', '"v1"').status).toBe(304);
    expect(calls[0]).toContain('If-None-Match: "v1"');
    expect(calls[0]).toContain('GET');
    expect(calls[0]).not.toContain('POST');
    expect(() => api.get('repos/pj-tmt/tmt/actions/runs/1/logs')).toThrow('WATCH_ENDPOINT');
    expect(parseGhResponse(response(304)).body).toBeNull();
  });
  it('counts conditional and search reads against 800 and refuses below the core floor', () => {
    const api = createWatchApi({ execute: () => response(304) });
    for (let i = 0; i < 800; i++) api.get('repos/pj-tmt/tmt/actions/runs?status=completed');
    expect(() => api.get('rate_limit')).toThrow('WATCH_BUDGET');
    expect(() =>
      createWatchApi({ execute: () => response(200, 'core', 2499) }).get('rate_limit')
    ).toThrow('WATCH_RATE_FLOOR');
    expect(() =>
      createWatchApi({ execute: () => response(200, 'search', 29) }).get('search/issues?q=fixture')
    ).not.toThrow();
  });
  it('bounds each pass and fails closed on stale, future, malformed or error heartbeat', () => {
    let time = 0;
    const api = createWatchApi({ execute: () => response(), clock: () => time });
    time = WATCH_LIMITS.deadlineMs;
    expect(() => api.get('rate_limit')).toThrow('WATCH_BUDGET');
    expect(heartbeatStatus({ updatedAt: iso(now), status: 'ok' }, now).healthy).toBe(true);
    expect(heartbeatStatus({ updatedAt: iso(now), status: 'checking' }, now).healthy).toBe(true);
    expect(heartbeatStatus({ updatedAt: iso(now - 7200_000), status: 'ok' }, now).healthy).toBe(
      true
    );
    for (const h of [
      { updatedAt: iso(now - 7200_001), status: 'ok' },
      { updatedAt: iso(now + 1), status: 'ok' },
      { updatedAt: iso(now), status: 'error' },
      {},
    ])
      expect(heartbeatStatus(h, now).healthy).toBe(false);
  });
});

it('CLI dry-run ignores persistent state, writes nothing and cannot send or create an issue', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'tmt-watch-preview-'));
  try {
    const bin = path.join(root, 'bin');
    mkdirSync(bin);
    const stateRoot = path.join(root, 'state');
    mkdirSync(stateRoot);
    const stateFile = path.join(stateRoot, 'state.json');
    const sentinel = 'Persistent live state must not be read or modified by dry-run.';
    writeFileSync(stateFile, sentinel);
    const map = path.join(root, 'routing.json');
    writeFileSync(map, JSON.stringify(routing));
    const calls = path.join(root, 'commands.jsonl');
    const sample = run({
      created_at: iso(Date.now() - 60_000),
      updated_at: iso(Date.now() - 1000),
    });
    writeExecutable(
      path.join(bin, 'gh'),
      String.raw`#!/usr/bin/env node
import { appendFileSync } from 'node:fs';
const args = process.argv.slice(2);
appendFileSync(${JSON.stringify(calls)}, JSON.stringify(args) + '\n');
if (!args.includes('GET') || args.includes('POST')) process.exit(77);
const endpoint = args[args.indexOf('--include') + 1];
const run = ${JSON.stringify(sample)};
const jobs = ${JSON.stringify(jobs)};
let body;
if (endpoint === 'rate_limit') body = {resources: {core: {remaining: 4000}}};
else if (endpoint.includes('/jobs?')) body = {total_count: jobs.length, jobs};
else {
 const rows = new URL('https://fixture/' + endpoint).searchParams.get('event') === 'merge_group' ? [run] : [];
 body = {total_count: rows.length, workflow_runs: rows};
}
process.stdout.write('HTTP/2.0 200 OK\nETag: "fixture"\nX-RateLimit-Resource: core\nX-RateLimit-Remaining: 4000\n\n' + JSON.stringify(body));
`,
      0o755
    );
    writeExecutable(path.join(bin, 'tmt'), '#!/bin/sh\nexit 99\n', 0o755);
    const result = spawnSync(
      process.execPath,
      [
        fileURLToPath(new URL('../../scripts/ci-failure-watch.mjs', import.meta.url)),
        '--dry-run',
        '--routing-map',
        map,
        '--state',
        stateRoot,
      ],
      {
        encoding: 'utf8',
        env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
        timeout: 15_000,
        maxBuffer: 1024 * 1024,
      }
    );
    expect(result.status, result.stderr).toBe(0);
    const report = JSON.parse(result.stdout);
    expect(report.sends).toBe(0);
    expect(report.issuesCreated).toBe(0);
    expect(report.alerts).toHaveLength(1);
    expect(readFileSync(stateFile, 'utf8')).toBe(sentinel);
    expect(readFileSync(calls, 'utf8')).not.toContain('POST');
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

it('replays the actual #4752 merge-group envelope without inventing a failing test name', () => {
  const state = newWatchState(now);
  const recorded = run({
    workflow_id: 348439940,
    head_branch: 'gh-readonly-queue/main/pr-4752-cbebcc20bae48ea3882e6907929dcaf2fc4d768a',
    created_at: '2026-10-24T06:55:26Z',
    updated_at: '2026-10-24T07:08:31Z',
  });
  observeRun(
    state,
    recorded,
    [{ id: 120807763624, name: 'Docker E2E shard 2/2', conclusion: 'failure' }],
    routing,
    now
  );
  const alert = Object.values(state.alerts)[0];
  expect(alert.runUrl).toBe('https://github.com/pj-tmt/tmt/actions/runs/40072107154');
  expect(alert.job).toBe('Docker E2E shard 2/2');
  expect(formatNotification([alert])).not.toContain('workspace-snapshot');
});

it('leaves a budget-deferred delivery pending without falsely marking it uncertain', () => {
  const state = newWatchState(now);
  const e = effects();
  observeRun(state, run(), jobs, routing, now);
  const limited = {
    ...e,
    reserve: () => {
      throw new Error('WATCH_BUDGET');
    },
  };
  expect(() => deliverWatchAlerts(state, limited, now)).toThrow('WATCH_BUDGET');
  expect(state.delivery).toBeNull();
  expect(state.issueIntent).toBeNull();
  expect(Object.values(state.alerts)[0].pending).toBe(true);
  expect(e.calls).not.toContain('create');
  expect(e.calls.some((c) => c.startsWith('talk:'))).toBe(false);
});

it('refuses ambiguous routing prefixes', () => {
  expect(() =>
    validateRouting({
      ...routing,
      rules: [routing.rules[0], { ...routing.rules[0], squad: 'tmt-core' }],
    })
  ).toThrow('duplicate prefix');
});

it('accepts the recorded gh exit1/HTTP304 shape without accepting another failed command', () => {
  const stdout =
    'HTTP/2.0 304 Not Modified\nETag: "recorded"\nX-RateLimit-Resource: core\nX-RateLimit-Remaining: 4000\n\n';
  const refused = Object.assign(new Error('packed command exit1'), {
    cause: { status: 1, stdout, stderr: 'gh: HTTP 304\n' },
  });
  const api = createWatchApi({
    execute: () => {
      throw refused;
    },
  });
  expect(api.get('repos/pj-tmt/tmt/actions/runs?status=completed', '"recorded"').status).toBe(304);
  const wrong = Object.assign(new Error('other failure'), {
    cause: {
      status: 1,
      stdout: stdout.replace('304 Not Modified', '503 Unavailable'),
      stderr: 'gh: Server Error\n',
    },
  });
  expect(() =>
    createWatchApi({
      execute: () => {
        throw wrong;
      },
    }).get('rate_limit')
  ).toThrow('WATCH_READ');
});

it('does not lose a completion read after the fixed cursor boundary', () => {
  const state = newWatchState(now);
  state.cursor = iso(now - 60_000);
  const future = run({ updated_at: iso(now + 1000) });
  const f = apiFor([future]);
  const first = collectWatchCycle({ state, api: f.api, routing, now });
  expect(first.examined).toEqual([]);
  expect(first.state.tracked[String(future.id)]).toBeTruthy();
  const next = collectWatchCycle({ state: first.state, api: f.api, routing, now: now + 60_000 });
  expect(next.examined).toHaveLength(1);
});

it('refuses a catch-up response outside its stated created window', () => {
  const old = run({ created_at: iso(now - WATCH_LIMITS.lookbackMs - 1000) });
  expect(() =>
    collectWatchCycle({
      state: newWatchState(now),
      api: apiFor([old]).api,
      routing,
      now,
      dryRun: true,
    })
  ).toThrow('outside catch-up');
});
