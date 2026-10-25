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
  watchHeartbeatPath,
  readWatchHeartbeat,
  watchHeartbeat,
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
    const first = collectWatchCycle({ repo: 'pj-tmt/tmt', state, api: f.api, routing, now });
    const second = collectWatchCycle({
      repo: 'pj-tmt/tmt',
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
    const result = collectWatchCycle({ repo: 'pj-tmt/tmt', state, api: f.api, routing, now });
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
    const result = collectWatchCycle({ repo: 'pj-tmt/tmt', state, api: f.api, routing, now });
    expect(result.examined).toEqual([{ run: 99, attempt: 2, conclusion: 'failure' }]);
    expect(result.state.tracked).toEqual({});
    expect(f.calls.filter((c) => c.endpoint === 'repos/pj-tmt/tmt/actions/runs/99')).toHaveLength(
      1
    );
    const again = collectWatchCycle({
      repo: 'pj-tmt/tmt',
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
    const result = collectWatchCycle({
      repo: 'pj-tmt/tmt',
      state,
      api: f.api,
      routing,
      now,
      initial: true,
    });
    expect(result.state.tracked[String(active.id)].run_attempt).toBe(3);
    const restart = collectWatchCycle({
      repo: 'pj-tmt/tmt',
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
    expect(() =>
      collectWatchCycle({ repo: 'pj-tmt/tmt', state, api, routing, now, initial: true })
    ).toThrow('WATCH_COVERAGE');
    expect(state).toEqual(before);
  });
  it('refuses missing cached 304 and duplicate job identities', () => {
    expect(() =>
      collectWatchCycle({
        repo: 'pj-tmt/tmt',
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
      collectWatchCycle({ repo: 'pj-tmt/tmt', state: newWatchState(now), api: f.api, routing, now })
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
    const result = collectWatchCycle({
      repo: 'pj-tmt/tmt',
      state,
      api: f.api,
      routing,
      now,
      dryRun: true,
    });
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

it.each(['env', 'flag', 'gh', 'remote'])(
  'CLI dry-run resolves the renamed repository from %s without live effects',
  (source) => {
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
        html_url: 'https://github.com/tmt-ninja/tmt/actions/runs/40072107154',
        created_at: iso(Date.now() - 60_000),
        updated_at: iso(Date.now() - 1000),
      });
      writeExecutable(
        path.join(bin, 'gh'),
        String.raw`#!/usr/bin/env node
import { appendFileSync } from 'node:fs';
const args = process.argv.slice(2);
appendFileSync(${JSON.stringify(calls)}, JSON.stringify(args) + '\n');
if (args[0] === 'repo') {
 if (${JSON.stringify(source)} === 'remote') process.exit(1);
 process.stdout.write('tmt-ninja/tmt'); process.exit(0);
}
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
      writeExecutable(
        path.join(bin, 'git'),
        "#!/bin/sh\nprintf '%s\\n' git@github.com:tmt-ninja/tmt.git\n",
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
          ...(source === 'flag' ? ['--repo', 'tmt-ninja/tmt'] : []),
        ],
        {
          encoding: 'utf8',
          env: {
            ...process.env,
            PATH: `${bin}:${process.env.PATH}`,
            GITHUB_REPOSITORY:
              source === 'env' ? 'tmt-ninja/tmt' : source === 'flag' ? 'wrong/repo' : undefined,
          },
          timeout: 15_000,
          maxBuffer: 1024 * 1024,
        }
      );
      expect(result.status, result.stderr).toBe(0);
      const report = JSON.parse(result.stdout);
      expect(report.sends).toBe(0);
      expect(report.issuesCreated).toBe(0);
      expect(report.alerts).toHaveLength(1);
      expect(report.alerts[0].runUrl).toContain('https://github.com/tmt-ninja/tmt/');
      expect(readFileSync(stateFile, 'utf8')).toBe(sentinel);
      expect(readFileSync(calls, 'utf8')).not.toContain('POST');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }
);

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
  const first = collectWatchCycle({ repo: 'pj-tmt/tmt', state, api: f.api, routing, now });
  expect(first.examined).toEqual([]);
  expect(first.state.tracked[String(future.id)]).toBeTruthy();
  const next = collectWatchCycle({
    repo: 'pj-tmt/tmt',
    state: first.state,
    api: f.api,
    routing,
    now: now + 60_000,
  });
  expect(next.examined).toHaveLength(1);
});

it('refuses a catch-up response outside its stated created window', () => {
  const old = run({ created_at: iso(now - WATCH_LIMITS.lookbackMs - 1000) });
  expect(() =>
    collectWatchCycle({
      repo: 'pj-tmt/tmt',
      state: newWatchState(now),
      api: apiFor([old]).api,
      routing,
      now,
      dryRun: true,
    })
  ).toThrow('outside catch-up');
});

it('publishes coverage notes in the same heartbeat used during catch-up and completion', () => {
  const state = newWatchState(now);
  const result = collectWatchCycle({
    repo: 'pj-tmt/tmt',
    state,
    api: apiFor([]).api,
    routing,
    now,
    initial: true,
  });
  const heartbeat = watchHeartbeat(result.state, now, 'checking', { reads: 3 });
  expect(heartbeatStatus(heartbeat, now).healthy).toBe(true);
  expect(heartbeat.coverageGaps).toContainEqual({
    updatedAt: iso(now),
    message: expect.stringContaining('restart coverage excludes'),
  });
  expect(watchHeartbeat(result.state, now, 'ok').coverageGaps).toEqual(heartbeat.coverageGaps);
});
it('reads only the shared state-directory heartbeat and reports missing or malformed JSON', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-watch-heartbeat-'));
  try {
    const file = watchHeartbeatPath(directory)!;
    expect(readWatchHeartbeat(directory)).toEqual({ path: file, error: 'ENOENT' });
    writeFileSync(file, '{');
    expect(readWatchHeartbeat(directory)).toEqual({ path: file, error: 'invalid heartbeat JSON' });
    const heartbeat = { updatedAt: iso(now), status: 'ok', coverageGaps: [] };
    writeFileSync(file, JSON.stringify(heartbeat));
    expect(readWatchHeartbeat(directory)).toEqual({ path: file, heartbeat });
    const moduleUrl = new URL('../../scripts/ci-failure-watch.mjs', import.meta.url).href;
    const child = spawnSync(
      process.execPath,
      [
        '--input-type=module',
        '-e',
        `import { readWatchHeartbeat } from ${JSON.stringify(moduleUrl)}; console.log(JSON.stringify(readWatchHeartbeat()));`,
      ],
      {
        encoding: 'utf8',
        env: { ...process.env, TMT_CI_WATCH_STATE: directory },
        timeout: 10_000,
      }
    );
    expect(child.status, child.stderr).toBe(0);
    expect(JSON.parse(child.stdout)).toEqual({ path: file, heartbeat });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

it.each([
  [0, true],
  [WATCH_LIMITS.lookbackMs - 1, true],
  [WATCH_LIMITS.lookbackMs, false],
  [WATCH_LIMITS.lookbackMs + 1, false],
  [-1, false],
])(
  'coverage notes expire at the producer without removing retained alerts (age %i)',
  (age, included) => {
    const result = collectWatchCycle({
      repo: 'pj-tmt/tmt',
      state: newWatchState(now),
      api: apiFor([]).api,
      routing,
      now,
      initial: true,
    });
    const alert = Object.values(result.state.alerts).find((alert) => alert.coverage)!;
    alert.lastSeen = now - age;
    const retained = structuredClone(result.state);
    const heartbeat = watchHeartbeat(result.state, now, 'ok');
    expect(heartbeat.coverageGaps).toEqual(
      included ? [{ updatedAt: iso(now - age), message: alert.job }] : []
    );
    expect(result.state).toEqual(retained);
  }
);

describe.each(['in_progress', 'queued'])('real active-page overflow: %s (#4924)', (status) => {
  function page(count: number, total: number): WatchApi {
    const rows = Array.from({ length: count }, (_, i) =>
      run({ id: i + 1, status, conclusion: null })
    );
    const base = apiFor([], rows).api;
    return {
      get(endpoint, etag) {
        if (new URL(`https://fixture/${endpoint}`).searchParams.get('status') === status)
          return { status: 200, body: { total_count: total, workflow_runs: rows } };
        return base.get(endpoint, etag);
      },
    };
  }
  it('accepts a short 5-of-6 listing without a false coverage note', () => {
    const result = collectWatchCycle({
      repo: 'pj-tmt/tmt',
      state: newWatchState(now),
      api: page(5, 6),
      routing,
      now,
    });
    expect(Object.values(result.state.alerts).filter((a) => a.coverage)).toEqual([]);
    expect(result.groups).toEqual([]);
    expect(Object.keys(result.state.tracked)).toHaveLength(5);
  });
  it('reports one full 100-of-130 overflow and re-arms only after a clean pass', () => {
    const first = collectWatchCycle({
      repo: 'pj-tmt/tmt',
      state: newWatchState(now),
      api: page(100, 130),
      routing,
      now,
    });
    const out = effects();
    deliverWatchAlerts(first.state, out, now);
    const notes = () => out.calls.filter((call) => call.startsWith('talk:'));
    expect(notes()).toHaveLength(1);
    expect(watchHeartbeat(first.state, now, 'ok').coverageGaps).toHaveLength(1);
    const still = collectWatchCycle({
      repo: 'pj-tmt/tmt',
      state: first.state,
      api: page(100, 130),
      routing,
      now: now + 60_000,
    });
    deliverWatchAlerts(still.state, out, now + 60_000);
    expect(notes()).toHaveLength(1);
    const clean = collectWatchCycle({
      repo: 'pj-tmt/tmt',
      state: still.state,
      api: page(100, 100),
      routing,
      now: now + 120_000,
    });
    expect(clean.state.coverage[`active-page:${status}`]).toBeUndefined();
    const again = collectWatchCycle({
      repo: 'pj-tmt/tmt',
      state: clean.state,
      api: page(100, 130),
      routing,
      now: now + 180_000,
    });
    deliverWatchAlerts(again.state, out, now + 180_000);
    expect(notes()).toHaveLength(2);
    expect(Object.values(again.state.alerts).filter((a) => a.coverage)).toMatchObject([
      { count: 2, firstSeen: now, lastSeen: now + 180_000, pending: false },
    ]);
    expect(watchHeartbeat(again.state, now + 180_000, 'ok').coverageGaps).toMatchObject([
      { updatedAt: iso(now + 180_000) },
    ]);
  });
});
describe('one run failure alert (#4951)', () => {
  const cascade = [
    { id: 11, name: 'Native package matrix', conclusion: 'failure', started_at: iso(now - 3000) },
    { id: 12, name: 'Native Rust contracts', conclusion: 'failure', started_at: iso(now - 4000) },
    {
      id: 13,
      name: 'Verify exact workspace test identities',
      conclusion: 'failure',
      started_at: iso(now - 5000),
    },
    { id: 10, name: 'Remote root worker', conclusion: 'failure', started_at: iso(now - 6000) },
  ];
  it('opens one issue and submits once for one root plus three dependent failures', () => {
    const state = newWatchState(now);
    const out = effects();
    observeRun(state, run(), cascade, routing, now);
    expect(Object.values(state.alerts)).toHaveLength(1);
    const alert = Object.values(state.alerts)[0];
    expect(alert.job).toBe('Remote root worker');
    expect(alert.route.squad).toBe('tmt-remote');
    expect(alert.jobs).toEqual([
      'Remote root worker',
      'Verify exact workspace test identities',
      'Native Rust contracts',
      'Native package matrix',
    ]);
    deliverWatchAlerts(state, out, now);
    expect(out.calls.filter((c) => c === 'create')).toHaveLength(1);
    expect(out.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(1);
    for (const job of cascade) expect(formatNotification([alert])).toContain(job.name);
  });
  it('deduplicates the whole failed-job set despite order changes, but a different set is new', () => {
    const state = newWatchState(now);
    const out = effects();
    observeRun(state, run(), cascade, routing, now);
    deliverWatchAlerts(state, out, now);
    observeRun(state, run({ id: 2 }), [...cascade].reverse(), routing, now + 60_000);
    deliverWatchAlerts(state, out, now + 60_000);
    expect(out.calls.filter((c) => c === 'create')).toHaveLength(1);
    expect(out.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(1);
    observeRun(state, run({ id: 3 }), cascade.slice(1), routing, now + 120_000);
    deliverWatchAlerts(state, out, now + 120_000);
    expect(out.calls.filter((c) => c === 'create')).toHaveLength(2);
    expect(out.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(2);
  });
  it('revalidates an old URL and opens a fresh issue when the signature issue closed', () => {
    const state = newWatchState(now);
    const out = effects();
    let next = 100;
    out.createIssue = (alert) => {
      out.calls.push('create');
      const url = `https://github.com/pj-tmt/tmt/issues/${next++}`;
      out.issues.set(alert.signature, url);
      return url;
    };
    observeRun(state, run(), jobs, routing, now);
    deliverWatchAlerts(state, out, now);
    out.issues.clear(); // Only open matches remain visible to the search port.
    observeRun(
      state,
      run({ id: 2, conclusion: 'success', updated_at: iso(now + 1000) }),
      [],
      routing,
      now + 1000
    );
    observeRun(state, run({ id: 3, updated_at: iso(now + 2000) }), jobs, routing, now + 2000);
    deliverWatchAlerts(state, out, now + 2000);
    expect(out.calls.filter((c) => c === 'create')).toHaveLength(2);
    const notices = out.calls.filter((c) => c.startsWith('talk:'));
    expect(notices).toHaveLength(2);
    expect(notices[1]).toContain('/issues/101');
    expect(notices[1]).not.toContain('/issues/100');
  });
  it('does not notify against a cached issue when its fresh open-issue search refuses', () => {
    const state = newWatchState(now);
    const out = effects();
    observeRun(state, run(), jobs, routing, now);
    deliverWatchAlerts(state, out, now);
    observeRun(state, run({ run_attempt: 2 }), jobs, routing, now + WATCH_LIMITS.dedupMs);
    out.findIssues = () => {
      throw new Error('search unavailable');
    };
    expect(() => deliverWatchAlerts(state, out, now + WATCH_LIMITS.dedupMs)).toThrow(
      'search unavailable'
    );
    expect(out.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(1);
  });
  it('retains one uncertain issue-create intent for the aggregate across restart', () => {
    const state = newWatchState(now);
    const out = effects();
    observeRun(state, run(), cascade, routing, now);
    out.createIssue = () => {
      throw new Error('unknown POST result');
    };
    expect(() => deliverWatchAlerts(state, out, now)).toThrow('unknown POST result');
    const recovered = effects();
    let searches = 0;
    recovered.findIssues = () =>
      ++searches === 1 ? ['https://github.com/pj-tmt/tmt/issues/123'] : [];
    deliverWatchAlerts(state, recovered, now + 1000);
    expect(searches).toBe(1);
    expect(recovered.calls).not.toContain('create');
    expect(recovered.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(1);
    expect(state.issueIntent).toBeNull();
  });
});

it('keeps version1 historical per-job alerts and seen/green history unchanged', () => {
  const old = newWatchState(now);
  observeRun(old, run(), jobs, routing, now);
  deliverWatchAlerts(old, effects(), now);
  const signature = Object.keys(old.alerts)[0];
  // The original v1 workflow/job hash must still find this historical record.
  expect(signature).toBe('df066e2e70f9cdbd3c81b3435c203d42724873d99d8d3f5e9e0f42226ece49dd');
  delete old.alerts[signature].jobs;
  old.green['1:schedule'] = now - 1000;
  const state = JSON.parse(JSON.stringify(old)) as WatchState;
  const history = structuredClone(state.alerts[signature]);
  observeRun(
    state,
    run({ id: 2 }),
    [
      { name: 'Remote root', conclusion: 'failure' },
      { name: 'Native package matrix', conclusion: 'failure' },
    ],
    routing,
    now + 1000
  );
  expect(state.version).toBe(1);
  expect(state.alerts[signature]).toEqual(history);
  expect(state.seen[`${run().id}:1`]).toBe(old.seen[`${run().id}:1`]);
  expect(state.green).toEqual(old.green);
});
it('preserves complete job names and defers folded notices that would exceed the frame bound', () => {
  const state = newWatchState(now);
  for (let i = 0; i < 8; i++) {
    observeRun(
      state,
      run({ id: i + 1 }),
      [
        { name: `Remote root${i} ` + 'x'.repeat(1900), conclusion: 'failure' },
        { name: 'aggregate ' + 'y'.repeat(1900), conclusion: 'failure' },
      ],
      routing,
      now
    );
  }
  const groups = notificationGroups(state, now);
  expect(groups).toHaveLength(1);
  expect(groups[0].length).toBeLessThan(8);
  expect(Buffer.byteLength(formatNotification(groups[0]))).toBeLessThanOrEqual(
    WATCH_LIMITS.maxNoticeBytes
  );
  for (const alert of groups[0])
    for (const name of alert.jobs!) expect(formatNotification(groups[0])).toContain(name);
  const out = effects();
  deliverWatchAlerts(state, out, now);
  expect(Object.values(state.alerts).some((a) => a.pending)).toBe(true);
  expect(out.calls.filter((c) => c.startsWith('talk:'))).toHaveLength(1);
});

it('loads v1 history and delivers one aggregate through fake gh/tmt CLI ports', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'tmt-watch-aggregate-'));
  try {
    const bin = path.join(root, 'bin');
    const directory = path.join(root, 'state');
    mkdirSync(bin);
    mkdirSync(directory);
    const time = Math.floor(Date.now() / 1000) * 1000;
    const state = newWatchState(time);
    observeRun(state, run(), jobs, routing, time - 1000);
    deliverWatchAlerts(state, effects(), time - 1000);
    const key = Object.keys(state.alerts)[0];
    delete state.alerts[key].jobs;
    state.coverage['restart-lookback'] = time;
    state.green['1:schedule'] = time - 1000;
    const historical = structuredClone(state.alerts[key]);
    writeFileSync(path.join(directory, 'state.json'), JSON.stringify(state));
    const map = path.join(root, 'routing.json');
    writeFileSync(map, JSON.stringify(routing));
    const calls = path.join(root, 'calls.jsonl');
    const created = path.join(root, 'created.json');
    const sample = run({ id: 123, created_at: iso(time - 60_000), updated_at: iso(time - 1000) });
    const failures = [
      'Remote root',
      'Native package matrix',
      'Native Rust contracts',
      'Verify exact identities',
    ].map((name, index) => ({
      id: index + 1,
      name,
      conclusion: 'failure',
      started_at: iso(time - 5000 + index * 1000),
    }));
    writeExecutable(
      path.join(bin, 'gh'),
      String.raw`#!/usr/bin/env node
import {appendFileSync, readFileSync, writeFileSync} from 'node:fs';
const args = process.argv.slice(2);
appendFileSync(${JSON.stringify(calls)}, JSON.stringify(['gh',...args])+'\n');
if (args.includes('POST')) {
 if (!args.includes('repos/pj-tmt/tmt/issues')) process.exit(77);
 writeFileSync(${JSON.stringify(created)}, readFileSync(args[args.indexOf('--input')+1]));
 console.log(JSON.stringify({html_url:'https://github.com/pj-tmt/tmt/issues/201'}));
 process.exit(0);
}
const endpoint = args[args.indexOf('--include')+1];
const url = new URL('https://fixture/'+endpoint);
let body;
if (endpoint === 'rate_limit') body = {resources:{core:{remaining:4000}}};
else if (endpoint.startsWith('search/issues?')) {
 if (!url.searchParams.get('q').includes('is:open')) process.exit(78);
 body = {incomplete_results:false,total_count:1,items:[{state:'closed',html_url:'https://github.com/pj-tmt/tmt/issues/100',body:url.searchParams.get('q')}]};
} else if (endpoint.includes('/jobs?')) body = {total_count:4,jobs:${JSON.stringify(failures)}};
else if (endpoint.startsWith('repos/pj-tmt/tmt/actions/runs?')) {
 const rows = url.searchParams.get('event') === 'merge_group' ? [${JSON.stringify(sample)}] : [];
 body = {total_count:rows.length,workflow_runs:rows};
} else process.exit(79);
process.stdout.write('HTTP/2.0 200 OK\nETag: "fixture"\nX-RateLimit-Resource: core\nX-RateLimit-Remaining: 4000\n\n'+JSON.stringify(body));
`,
      0o755
    );
    writeExecutable(
      path.join(bin, 'tmt'),
      String.raw`#!/usr/bin/env node
import {appendFileSync} from 'node:fs';
appendFileSync(${JSON.stringify(calls)}, JSON.stringify(['tmt',...process.argv.slice(2)])+'\n');
console.log(JSON.stringify({status:'sent',requestId:'fixture-request'}));
`,
      0o755
    );
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      PATH: `${bin}:${process.env.PATH}`,
      HOME: root,
      XDG_CONFIG_HOME: root,
      TMT_AGENT: '1',
      BROWSER: 'true',
    };
    delete env.TMUX;
    delete env.TMUX_PANE;
    const result = spawnSync(
      process.execPath,
      [
        fileURLToPath(new URL('../../scripts/ci-failure-watch.mjs', import.meta.url)),
        '--repo',
        'pj-tmt/tmt',
        '--once',
        '--state',
        directory,
        '--routing-map',
        map,
        '--identity',
        'infra-fixture',
      ],
      { env, encoding: 'utf8', timeout: 15_000 }
    );
    expect(result.status, result.stderr).toBe(0);
    const commands = readFileSync(calls, 'utf8')
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as string[]);
    expect(commands.filter((args) => args.includes('POST'))).toHaveLength(1);
    const notices = commands.filter((args) => args[0] === 'tmt');
    expect(notices).toHaveLength(1);
    expect(notices[0].slice(1, 3)).toEqual(['talk', 'tmt-infra-lead']);
    const body = JSON.parse(readFileSync(created, 'utf8')).body;
    for (const failure of failures) {
      expect(body).toContain(failure.name);
      expect(notices[0][3]).toContain(failure.name);
    }
    expect(notices[0][3]).toContain('/issues/201');
    expect(notices[0][3]).not.toContain('/issues/100');
    const saved = JSON.parse(
      readFileSync(path.join(directory, 'state.json'), 'utf8')
    ) as WatchState;
    expect(saved.version).toBe(1);
    expect(saved.alerts[key]).toEqual(historical);
    expect(saved.seen[`${run().id}:1`]).toBe(state.seen[`${run().id}:1`]);
    expect(saved.green).toEqual(state.green);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

it('refuses an oversized complete notice before issue or delivery effects', () => {
  const state = newWatchState(now);
  observeRun(
    state,
    run(),
    Array.from({ length: 20 }, (_, i) => ({
      name: `Remote ${i} ` + 'x'.repeat(1900),
      conclusion: 'failure',
    })),
    routing,
    now
  );
  const out = effects();
  expect(() => deliverWatchAlerts(state, out, now)).toThrow('WATCH_NOTICE_LIMIT');
  expect(out.calls).toEqual([]);
});

it('accepts current repository routing provenance without accepting another repository map', () => {
  expect(
    validateRouting(
      { ...routing, source: 'https://github.com/tmt-ninja/tmt/issues/606' },
      'tmt-ninja/tmt'
    )
  ).toMatchObject({ rules: routing.rules });
  expect(() =>
    validateRouting(
      { ...routing, source: 'https://github.com/foreign/repo/issues/606' },
      'tmt-ninja/tmt'
    )
  ).toThrow('WATCH_ROUTING');
});
