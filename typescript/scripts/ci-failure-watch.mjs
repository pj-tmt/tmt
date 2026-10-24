// Private, operator-started CI watcher. No log downloads, PR alerts or new credentials.
// Coverage: new runs and observed in-flight reruns within about two minutes;
// restart catch-up is bounded to runs created in the last 24 hours, not all history.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import {
  closeSync,
  existsSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';
import { setTimeout } from 'node:timers/promises';
import { runPackedCommand } from './packed-command.mjs';

export const WATCH_LIMITS = Object.freeze({
  intervalMs: 60_000,
  lookbackMs: 86400_000,
  dedupMs: 21600_000,
  notificationsPerHour: 6,
  maxReads: 800,
  coreFloor: 2500,
  deadlineMs: 120_000,
  maxPages: 10,
  maxSignatures: 2000,
});
const events = ['push', 'merge_group', 'workflow_dispatch', 'schedule'];
const clean = (value, limit = 240) =>
  String(value)
    .replace(/\p{Cc}/gu, ' ')
    .slice(0, limit);
const timestamp = (value) => {
  const n = Date.parse(value);
  assert(Number.isFinite(n), 'WATCH_METADATA: invalid timestamp');
  return n;
};
export function newWatchState(now) {
  now = Math.floor(now / 1000) * 1000;
  return {
    version: 1,
    cursor: new Date(now - WATCH_LIMITS.lookbackMs).toISOString(),
    pages: {},
    tracked: {},
    seen: {},
    alerts: {},
    green: {},
    notifications: [],
    coverage: {},
    delivery: null,
    issueIntent: null,
  };
}
export function validateRouting(map) {
  assert(
    map?.source === 'https://github.com/pj-tmt/tmt/issues/606' &&
      Array.isArray(map.rules) &&
      map.rules.length <= 100,
    'WATCH_ROUTING: reviewed #606 map required'
  );
  const prefixes = new Set();
  for (const rule of map.rules) {
    assert(
      (typeof rule.workflowPrefix === 'string') !== (typeof rule.jobPrefix === 'string'),
      'WATCH_ROUTING: one workflow or job prefix per rule'
    );
    const prefix = rule.workflowPrefix ?? rule.jobPrefix;
    const key = `${rule.workflowPrefix === undefined ? 'job' : 'workflow'}:${prefix}`;
    assert(!prefixes.has(key), 'WATCH_ROUTING: duplicate prefix');
    prefixes.add(key);
    assert(
      prefix.length > 0 &&
        prefix.length < 200 &&
        !/\p{Cc}/u.test(prefix) &&
        /^tmt-[a-z]+$/.test(rule.squad) &&
        /^#606\b/.test(rule.source),
      'WATCH_ROUTING: invalid rule'
    );
  }
  return map;
}
export function routeFailure(map, workflow, job) {
  for (const kind of ['workflowPrefix', 'jobPrefix']) {
    const value = kind === 'workflowPrefix' ? workflow : job;
    const rule = map.rules
      .filter((r) => r[kind] && value.startsWith(r[kind]))
      .sort((a, b) => b[kind].length - a[kind].length)[0];
    if (rule) return { squad: rule.squad, source: rule.source };
  }
  return { squad: 'unmapped', source: '#606: no applicable reviewed rule' };
}
export function watchedRun(run) {
  return events.includes(run.event) && (run.event !== 'push' || run.head_branch === 'main');
}
function normalizeRun(run, repo) {
  assert(
    Number.isSafeInteger(run.id) &&
      run.id > 0 &&
      Number.isSafeInteger(run.run_attempt) &&
      run.run_attempt > 0 &&
      Number.isSafeInteger(run.workflow_id),
    'WATCH_METADATA: invalid run identity'
  );
  assert(
    typeof run.path === 'string' &&
      run.path.startsWith('.github/workflows/') &&
      typeof run.name === 'string',
    'WATCH_METADATA: workflow missing'
  );
  timestamp(run.updated_at);
  timestamp(run.created_at);
  assert(
    run.html_url === `https://github.com/${repo}/actions/runs/${run.id}`,
    'WATCH_METADATA: foreign run URL'
  );
  return Object.fromEntries(
    [
      'id',
      'run_attempt',
      'workflow_id',
      'path',
      'name',
      'event',
      'head_branch',
      'status',
      'conclusion',
      'created_at',
      'updated_at',
      'html_url',
    ].map((k) => [k, run[k]])
  );
}
export function failureSignature(run, job) {
  // v1 has no test-name component: obtaining one would require forbidden log reads.
  return createHash('sha256')
    .update(JSON.stringify([run.workflow_id, job]))
    .digest('hex');
}
export function observeRun(state, run, jobs, routing, now) {
  if (!watchedRun(run) || run.status !== 'completed') return;
  const token = `${run.id}:${run.run_attempt}`;
  if (state.seen[token]) return;
  const ended = timestamp(run.updated_at);
  const eventKey = `${run.workflow_id}:${run.event}`;
  if (run.conclusion === 'success')
    state.green[eventKey] = Math.max(state.green[eventKey] ?? 0, ended);
  if (run.conclusion === 'failure') {
    const failed = jobs.filter((job) => ['failure', 'timed_out'].includes(job.conclusion));
    const names = failed.length
      ? failed.map((job) => job.name)
      : ['[failure has no failed-job metadata]'];
    assert(
      new Set(names).size === names.length,
      'WATCH_METADATA: ambiguous duplicate failed job names'
    );
    for (const job of names) {
      assert(
        typeof job === 'string' && job.length > 0 && job.length < 2000,
        'WATCH_METADATA: invalid job name'
      );
      const signature = failureSignature(run, job);
      const old = state.alerts[signature];
      const recovered = (state.green[eventKey] ?? 0) > (old?.lastFailure ?? Infinity);
      const notify =
        !old ||
        old.lastNotifiedAt === undefined ||
        now - old.lastNotifiedAt >= WATCH_LIMITS.dedupMs ||
        recovered;
      state.alerts[signature] = {
        ...old,
        signature,
        workflow: run.name,
        workflowPath: run.path,
        event: run.event,
        job,
        route: routeFailure(routing, run.path, job),
        firstSeen: old?.firstSeen ?? now,
        lastSeen: now,
        lastFailure: ended,
        count: (old?.count ?? 0) + 1,
        runUrl: run.html_url,
        runId: run.id,
        attempt: run.run_attempt,
        pending: Boolean(old?.pending || notify),
      };
    }
  }
  state.seen[token] = ended;
  assert(
    Object.keys(state.alerts).length <= WATCH_LIMITS.maxSignatures,
    'WATCH_STATE_LIMIT: signature history full; operator review required'
  );
}
export const WATCH_STATE_ENV = 'TMT_CI_WATCH_STATE';
export function watchHeartbeatPath(directory = process.env[WATCH_STATE_ENV]) {
  return directory ? path.join(path.resolve(directory), 'heartbeat.json') : null;
}
export function readWatchHeartbeat(directory, read = readFileSync) {
  const file = watchHeartbeatPath(directory);
  if (!file) return { path: null, error: `${WATCH_STATE_ENV} is not configured` };
  try {
    return { path: file, heartbeat: JSON.parse(read(file, 'utf8')) };
  } catch (error) {
    return { path: file, error: error.code ?? 'invalid heartbeat JSON' };
  }
}
export function watchHeartbeat(state, now, status, extra = {}) {
  return {
    updatedAt: new Date(now).toISOString(),
    status,
    cursor: state.cursor,
    pending: Object.values(state.alerts).filter((alert) => alert.pending).length,
    coverageGaps: Object.values(state.alerts)
      .filter(
        (alert) =>
          alert.coverage && now >= alert.lastSeen && now - alert.lastSeen < WATCH_LIMITS.lookbackMs
      )
      .map((alert) => ({ updatedAt: new Date(alert.lastSeen).toISOString(), message: alert.job })),
    ...extra,
  };
}
export function heartbeatStatus(heartbeat, now) {
  const ageMs = now - Date.parse(heartbeat?.updatedAt);
  return {
    healthy:
      Number.isFinite(ageMs) &&
      ageMs >= 0 &&
      ageMs <= 7200_000 &&
      ['ok', 'checking'].includes(heartbeat?.status),
    ageMs: Number.isFinite(ageMs) ? ageMs : null,
  };
}
export function notificationGroups(state, now) {
  state.notifications = state.notifications.filter((time) => now - time < 3600_000);
  const available = Math.max(0, WATCH_LIMITS.notificationsPerHour - state.notifications.length);
  const pending = Object.values(state.alerts)
    .filter((a) => a.pending)
    .sort((a, b) => a.firstSeen - b.firstSeen || a.signature.localeCompare(b.signature));
  if (pending.length <= available) return pending.map((a) => [a]);
  const squads = new Map();
  for (const a of pending) {
    const key = a.route.squad;
    if (!squads.has(key)) squads.set(key, []);
    if (squads.get(key).length < 12) squads.get(key).push(a);
  }
  return [...squads.values()].slice(0, available);
}
export function formatNotification(group) {
  return (
    `CI failure${group.length === 1 ? '' : ' burst'}; suggested ${group[0].route.squad}: ` +
    group
      .map(
        (a) =>
          `${clean(a.workflow, 80)} / ${clean(a.job, 160)} — ${a.runUrl}${a.issueUrl ? ` (${a.issueUrl})` : ''}`
      )
      .join('; ')
  );
}
function coverageGap(state, key, text, now) {
  if (state.coverage[key]) return;
  state.coverage[key] = now;
  const signature = createHash('sha256').update(`coverage:${key}`).digest('hex');
  state.alerts[signature] = {
    signature,
    workflow: 'CI watcher coverage',
    workflowPath: '',
    event: 'coverage',
    job: text,
    route: { squad: 'tmt-infra', source: '#606 Infra CI tooling' },
    firstSeen: now,
    lastSeen: now,
    lastFailure: now,
    count: 1,
    runUrl: 'https://github.com/pj-tmt/tmt/issues/4825',
    pending: true,
    coverage: true,
  };
}
export function parseGhResponse(text) {
  const boundary = text.search(/\r?\n\r?\n/);
  assert(boundary >= 0, 'WATCH_HTTP: response headers missing');
  const header = text.slice(0, boundary).split(/\r?\n/);
  const status = Number(/^HTTP\/\S+\s+(\d+)/.exec(header.shift())?.[1]);
  assert([200, 201, 304].includes(status), `WATCH_HTTP: unexpected status ${status}`);
  const headers = Object.fromEntries(
    header.map((line) => {
      const i = line.indexOf(':');
      return [line.slice(0, i).toLowerCase(), line.slice(i + 1).trim()];
    })
  );
  const body = text.slice(boundary).trim();
  return {
    status,
    etag: headers.etag,
    remaining:
      headers['x-ratelimit-remaining'] === undefined
        ? undefined
        : Number(headers['x-ratelimit-remaining']),
    resource: headers['x-ratelimit-resource'],
    body: status === 304 ? null : JSON.parse(body),
  };
}
function readPage(state, api, endpoint, repo) {
  const previous = state.pages[endpoint];
  const response = api.get(endpoint, previous?.etag);
  if (response.status === 304) {
    assert(previous, 'WATCH_HTTP: 304 without cached body');
    return previous;
  }
  const { workflow_runs: rows, total_count: total } = response.body;
  assert(
    Number.isSafeInteger(total) &&
      total >= 0 &&
      Array.isArray(rows) &&
      rows.length <= 100 &&
      rows.length <= total,
    'WATCH_METADATA: invalid run listing'
  );
  const page = { etag: response.etag, total, rows: rows.map((r) => normalizeRun(r, repo)) };
  state.pages[endpoint] = page;
  return page;
}
function jobsFor(api, repo, run) {
  const jobs = [];
  let total;
  for (let page = 1; page <= 10; page++) {
    const data = api.get(
      `repos/${repo}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100&page=${page}`
    ).body;
    assert(
      Number.isSafeInteger(data.total_count) &&
        data.total_count >= 0 &&
        Array.isArray(data.jobs) &&
        data.jobs.every((j) => Number.isSafeInteger(j.id) && typeof j.name === 'string'),
      'WATCH_METADATA: invalid jobs'
    );
    total ??= data.total_count;
    assert(total === data.total_count, 'WATCH_METADATA: changing job count');
    jobs.push(...data.jobs);
    if (jobs.length >= total) break;
  }
  assert(
    jobs.length === total && new Set(jobs.map((j) => j.id)).size === total,
    `WATCH_METADATA: incomplete jobs for run ${run.id}`
  );
  return jobs;
}
export function collectWatchCycle({
  state: input,
  api,
  routing,
  repo = 'pj-tmt/tmt',
  now,
  initial = false,
  dryRun = false,
}) {
  const state = structuredClone(input);
  const cutoff = Math.max(timestamp(state.cursor), now - WATCH_LIMITS.lookbackMs);
  if (initial && !dryRun)
    coverageGap(
      state,
      'restart-lookback',
      `restart coverage excludes unobserved runs created before ${new Date(now - WATCH_LIMITS.lookbackMs).toISOString()}; older history unknown`,
      now
    );
  if (timestamp(state.cursor) < now - WATCH_LIMITS.lookbackMs - 1000 && !dryRun)
    coverageGap(
      state,
      `cursor-gap:${state.cursor}`,
      `cursor backlog before ${new Date(cutoff).toISOString()} is outside 24h catch-up`,
      now
    );
  const runs = new Map();
  if (initial || dryRun) {
    for (const event of events) {
      let total;
      const found = [];
      for (let page = 1; page <= WATCH_LIMITS.maxPages; page++) {
        const query = new URLSearchParams({
          status: 'completed',
          event,
          created: `${new Date(now - WATCH_LIMITS.lookbackMs).toISOString().replace(/\.\d{3}Z$/, 'Z')}..${new Date(now).toISOString().replace(/\.\d{3}Z$/, 'Z')}`,
          per_page: '100',
          page: String(page),
          ...(event === 'push' ? { branch: 'main' } : {}),
        });
        const data = readPage(state, api, `repos/${repo}/actions/runs?${query}`, repo);
        total ??= data.total;
        assert(
          total === data.total && total < 1000,
          'WATCH_COVERAGE: changing or capped catch-up cohort'
        );
        found.push(...data.rows);
        if (found.length >= total) break;
      }
      assert(
        found.length === total && new Set(found.map((r) => r.id)).size === total,
        'WATCH_COVERAGE: incomplete catch-up pages'
      );
      for (const r of found) {
        assert(
          timestamp(r.created_at) >= now - WATCH_LIMITS.lookbackMs &&
            timestamp(r.created_at) <= now,
          'WATCH_COVERAGE: run outside catch-up created range'
        );
        runs.set(r.id, r);
      }
    }
  } else {
    const page = readPage(
      state,
      api,
      `repos/${repo}/actions/runs?status=completed&per_page=100`,
      repo
    );
    for (const r of page.rows) runs.set(r.id, r);
    if (
      page.rows.length === 100 &&
      page.rows.every((r) => timestamp(r.created_at) > timestamp(state.cursor))
    )
      throw new Error(
        'WATCH_COVERAGE: more than one completed page since cursor; restart catch-up required'
      );
  }
  if (!dryRun) {
    const active = new Map();
    for (const status of ['in_progress', 'queued']) {
      const endpoint = `repos/${repo}/actions/runs?status=${status}&per_page=100`;
      const data = readPage(state, api, endpoint, repo);
      if (data.total > data.rows.length)
        coverageGap(
          state,
          `active-page:${status}`,
          `active ${status} page covers ${data.rows.length} of ${data.total}; unobserved older runs unknown`,
          now
        );
      for (const r of data.rows) if (watchedRun(r)) active.set(r.id, r);
    }
    for (const [id, old] of Object.entries(state.tracked)) {
      if (active.has(Number(id))) continue;
      const r = normalizeRun(api.get(`repos/${repo}/actions/runs/${id}`).body, repo);
      if (r.status === 'completed') runs.set(r.id, r);
      else active.set(r.id, r);
      assert(r.run_attempt >= old.run_attempt, 'WATCH_METADATA: attempt moved backwards');
    }
    state.tracked = Object.fromEntries(active);
  }
  const examined = [];
  for (const run of [...runs.values()].sort(
    (a, b) => timestamp(a.updated_at) - timestamp(b.updated_at) || a.id - b.id
  )) {
    if (timestamp(run.updated_at) > now) {
      state.tracked[run.id] = run;
      continue;
    }
    const tracked = Boolean(input.tracked[run.id]);
    if (
      !watchedRun(run) ||
      (!tracked && timestamp(run.updated_at) < cutoff) ||
      state.seen[`${run.id}:${run.run_attempt}`]
    )
      continue;
    const jobs = run.conclusion === 'failure' ? jobsFor(api, repo, run) : [];
    observeRun(state, run, jobs, routing, now);
    examined.push({ run: run.id, attempt: run.run_attempt, conclusion: run.conclusion });
  }
  state.cursor = new Date(now).toISOString();
  for (const [key, time] of Object.entries(state.seen))
    if (time < now - 2 * WATCH_LIMITS.lookbackMs) delete state.seen[key];
  return { state, examined, groups: notificationGroups(state, now) };
}
export function deliverWatchAlerts(state, effects, now) {
  assert(
    !state.delivery,
    'WATCH_DELIVERY_UNCERTAIN: inspect the recorded tmt request before clearing intent'
  );
  if (state.issueIntent) {
    const matches = effects.findIssues(state.issueIntent);
    assert(
      matches.length === 1 && state.alerts[state.issueIntent],
      'WATCH_ISSUE_UNCERTAIN: resolve the recorded create before retry'
    );
    state.alerts[state.issueIntent].issueUrl = matches[0];
    state.issueIntent = null;
    effects.save(state);
  }
  for (const group of notificationGroups(state, now)) {
    for (const alert of group) {
      if (alert.coverage || alert.issueUrl) continue;
      const matches = effects.findIssues(alert.signature);
      assert(matches.length <= 1, 'WATCH_ISSUE: duplicate signature issues');
      if (matches.length) {
        alert.issueUrl = matches[0];
        effects.save(state);
        continue;
      }
      assert(
        !state.issueIntent,
        'WATCH_ISSUE_UNCERTAIN: prior create not visible in search; inspect before retry'
      );
      effects.reserve?.(25_000);
      state.issueIntent = alert.signature;
      effects.save(state);
      alert.issueUrl = effects.createIssue(alert);
      state.issueIntent = null;
      effects.save(state);
    }
    effects.reserve?.(15_000);
    state.delivery = {
      signatures: group.map((a) => a.signature),
      startedAt: now,
      message: formatNotification(group),
    };
    state.notifications.push(now);
    effects.save(state);
    const receipt = effects.talk(state.delivery.message);
    for (const alert of group) {
      alert.pending = false;
      alert.lastNotifiedAt = now;
      alert.lastRequestId = receipt;
      alert.detectionDelaySeconds = Math.max(0, (now - alert.lastFailure) / 1000);
    }
    state.delivery = null;
    effects.save(state);
  }
}
function atomicJson(file, data) {
  const temporary = `${file}.${process.pid}.${randomUUID()}.tmp`;
  const fd = openSync(temporary, 'wx', 0o600);
  try {
    writeFileSync(fd, JSON.stringify(data, null, 2) + '\n');
    fsyncSync(fd);
  } finally {
    closeSync(fd);
  }
  renameSync(temporary, file);
  const directory = openSync(path.dirname(file), 'r');
  try {
    fsyncSync(directory);
  } finally {
    closeSync(directory);
  }
}
function command(exe, args, timeoutMs = 10_000) {
  return runPackedCommand(exe, args, { timeoutMs });
}
export function createWatchApi({ execute = command, clock = Date.now, progress = () => {} } = {}) {
  let calls = 0;
  const ledger = [];
  const started = clock();
  return {
    get calls() {
      return calls;
    },
    get ledger() {
      return ledger;
    },
    get remainingMs() {
      return WATCH_LIMITS.deadlineMs - (clock() - started);
    },
    get(endpoint, etag) {
      assert(
        calls < WATCH_LIMITS.maxReads && clock() - started < WATCH_LIMITS.deadlineMs,
        'WATCH_BUDGET: read/deadline ceiling'
      );
      assert(
        endpoint === 'rate_limit' ||
          /^repos\/[\w.-]+\/[\w.-]+\/actions\/runs(?:\?[^#]*|\/[1-9][0-9]*(?:\/attempts\/[1-9][0-9]*\/jobs\?[^#]*)?)?$/.test(
            endpoint
          ) ||
          endpoint.startsWith('search/issues?'),
        'WATCH_ENDPOINT: GET outside watcher allowlist'
      );
      assert(etag === undefined || !/[\r\n]/.test(etag), 'WATCH_HTTP: invalid ETag');
      progress();
      const remainingMs = WATCH_LIMITS.deadlineMs - (clock() - started);
      assert(remainingMs > 0, 'WATCH_BUDGET: read deadline exhausted');
      calls += 1;
      const readStarted = clock();
      let response;
      try {
        let output;
        try {
          output = execute(
            'gh',
            [
              'api',
              '--method',
              'GET',
              '--include',
              endpoint,
              ...(etag ? ['-H', `If-None-Match: ${etag}`] : []),
            ],
            Math.min(20_000, remainingMs)
          );
        } catch (failure) {
          const detail = failure.cause;
          // gh uses exit 1 for an HTTP 304, even though conditional GET succeeded.
          // Keep every other process/HTTP failure as an error; never parse prose as success.
          if (
            etag &&
            detail?.status === 1 &&
            /^HTTP\/\S+ 304\b/.test(detail.stdout ?? '') &&
            /^gh: HTTP 304[^\r\n]*$/.test((detail.stderr ?? '').trim())
          )
            output = detail.stdout;
          else throw failure;
        }
        response = parseGhResponse(output);
        ledger.push({
          endpoint,
          status: response.status,
          elapsedMs: clock() - readStarted,
          remaining: response.remaining,
          resource: response.resource,
        });
      } catch (cause) {
        ledger.push({ endpoint, status: 'error', elapsedMs: clock() - readStarted });
        const detail = cause.cause;
        const reason = detail?.stdout?.startsWith('HTTP/')
          ? `${detail.stdout.split(/\r?\n/)[0]} ${clean(detail.stderr, 180)}`
          : cause.message;
        throw new Error(`WATCH_READ: ${endpoint}: ${reason}`, { cause });
      }
      if (response.resource === 'core' && response.remaining !== undefined)
        assert(
          response.remaining >= WATCH_LIMITS.coreFloor,
          'WATCH_RATE_FLOOR: core remaining below 2500'
        );
      return response;
    },
  };
}
function issueUrl(value, repo) {
  assert(
    typeof value === 'string' &&
      value.startsWith(`https://github.com/${repo}/issues/`) &&
      /^[1-9][0-9]*$/.test(value.split('/').at(-1)),
    'WATCH_ISSUE: foreign or missing URL'
  );
  return value;
}
function liveEffects(root, repo, identity, api) {
  const save = (state) => atomicJson(path.join(root, 'state.json'), state);
  return {
    save,
    reserve(ms) {
      assert(api.remainingMs >= ms, 'WATCH_BUDGET: leave pending alerts for the next pass');
    },
    findIssues(signature) {
      const q = `repo:${repo} is:issue in:body "ci-watch:${signature}"`;
      const result = api.get(`search/issues?${new URLSearchParams({ q, per_page: '100' })}`).body;
      assert(
        !result.incomplete_results && result.total_count === result.items.length,
        'WATCH_ISSUE: incomplete signature search'
      );
      return result.items
        .filter((r) => r.body?.includes(`ci-watch:${signature}`))
        .map((r) => issueUrl(r.html_url, repo));
    },
    createIssue(alert) {
      const request = path.join(root, 'issue-request.json');
      atomicJson(request, {
        title: `ci: ${clean(alert.workflow, 60)} / ${clean(alert.job, 100)}`,
        body: `Part of #4742. CI failure watcher (#4825).\n\nSuggested squad: ${alert.route.squad} (${alert.route.source}).\nWorkflow: ${clean(alert.workflow)}\nFailed job: ${clean(alert.job)}\nRun: ${alert.runUrl} (attempt ${alert.attempt})\nNo logs read; cause and test name are unknown.\n\nci-watch:${alert.signature}\n`,
      });
      try {
        return issueUrl(
          JSON.parse(
            command('gh', ['api', '--method', 'POST', `repos/${repo}/issues`, '--input', request])
          ).html_url,
          repo
        );
      } finally {
        rmSync(request, { force: true });
      }
    },
    talk(message) {
      const result = JSON.parse(
        command(
          'tmt',
          ['talk', 'tmt-infra-lead', message, '--identity', identity, '--detach', '--json'],
          15_000
        )
      );
      assert(
        ['sent', 'queued', 'completed'].includes(result.status) &&
          typeof result.requestId === 'string',
        'WATCH_DELIVERY: durable request missing'
      );
      return result.requestId;
    },
  };
}
async function main() {
  const { values } = parseArgs({
    options: {
      state: { type: 'string' },
      'routing-map': { type: 'string' },
      identity: { type: 'string' },
      repo: { type: 'string', default: 'pj-tmt/tmt' },
      once: { type: 'boolean' },
      'dry-run': { type: 'boolean' },
    },
  });
  assert(
    values['routing-map'] &&
      (values['dry-run'] || ((values.state || process.env[WATCH_STATE_ENV]) && values.identity)),
    'Usage: ci-failure-watch.mjs --routing-map FILE [--dry-run | --identity NAME [--state DIR | TMT_CI_WATCH_STATE=DIR] [--once]] [--repo OWNER/REPO]'
  );
  assert(/^[\w.-]+\/[\w.-]+$/.test(values.repo), 'WATCH_REPO: invalid repository');
  const routing = validateRouting(JSON.parse(readFileSync(values['routing-map'], 'utf8')));
  const directory = values.state ?? process.env[WATCH_STATE_ENV];
  const root = directory && path.resolve(directory);
  let lock;
  if (!values['dry-run']) {
    mkdirSync(root, { recursive: true, mode: 0o700 });
    assert(
      lstatSync(root).isDirectory() && !lstatSync(root).isSymbolicLink(),
      'WATCH_STATE: private directory required'
    );
    lock = path.join(root, 'watch.lock');
    try {
      writeFileSync(lock, JSON.stringify({ pid: process.pid }), { flag: 'wx', mode: 0o600 });
    } catch (error) {
      throw new Error(
        `WATCH_LOCK: existing holder ${readFileSync(lock, 'utf8')}; stop and inspect before removing`,
        { cause: error }
      );
    }
  }
  let initial = true;
  let stopped = false;
  const stop = () => {
    stopped = true;
  };
  process.once('SIGINT', stop);
  process.once('SIGTERM', stop);
  try {
    let state =
      !values['dry-run'] && root && existsSync(path.join(root, 'state.json'))
        ? JSON.parse(readFileSync(path.join(root, 'state.json'), 'utf8'))
        : newWatchState(Date.now());
    assert(state.version === 1, 'WATCH_STATE: unsupported version');
    do {
      const now = Math.floor(Date.now() / 1000) * 1000;
      let status = 'ok';
      let error;
      let lastHeartbeat = 0;
      const progress = () => {
        if (!values['dry-run'] && Date.now() - lastHeartbeat >= WATCH_LIMITS.intervalMs) {
          atomicJson(
            watchHeartbeatPath(root),
            watchHeartbeat(state, Date.now(), 'checking', { reads: api.calls })
          );
          lastHeartbeat = Date.now();
        }
      };
      const api = createWatchApi({ progress });
      try {
        const core = api.get('rate_limit').body.resources.core;
        assert(
          core.remaining >= WATCH_LIMITS.coreFloor,
          'WATCH_RATE_FLOOR: core remaining below 2500'
        );
        const result = collectWatchCycle({
          state,
          api,
          routing,
          repo: values.repo,
          now,
          initial,
          dryRun: values['dry-run'],
        });
        if (values['dry-run']) {
          const endpoint = Object.keys(result.state.pages)[0];
          const etag = result.state.pages[endpoint]?.etag;
          const probes = [];
          if (etag) {
            const first = api.get(endpoint, etag);
            probes.push(first.status);
            if (first.status === 200 && first.etag)
              probes.push(api.get(endpoint, first.etag).status);
          }
          process.stdout.write(
            JSON.stringify(
              {
                dryRun: true,
                complete: true,
                conditionalProbe: probes,
                coreBefore: core.remaining,
                ledger: api.ledger,
                since: new Date(now - WATCH_LIMITS.lookbackMs).toISOString(),
                until: new Date(now).toISOString(),
                reads: api.calls,
                examined: result.examined,
                alerts: Object.values(result.state.alerts),
                notifications: result.groups.map(formatNotification),
                sends: 0,
                issuesCreated: 0,
              },
              null,
              2
            ) + '\n'
          );
          return;
        }
        state = result.state;
        atomicJson(path.join(root, 'state.json'), state);
        initial = false;
        deliverWatchAlerts(state, liveEffects(root, values.repo, values.identity, api), Date.now());
      } catch (cause) {
        status = 'error';
        error = clean(cause.message, 1200);
        process.stderr.write(error + '\n');
        if (values['dry-run'])
          process.stdout.write(
            JSON.stringify(
              {
                dryRun: true,
                complete: false,
                since: new Date(now - WATCH_LIMITS.lookbackMs).toISOString(),
                until: new Date(now).toISOString(),
                error,
                reads: api.calls,
                ledger: api.ledger,
                sends: 0,
                issuesCreated: 0,
              },
              null,
              2
            ) + '\n'
          );
        if (values['dry-run'] || values.once) process.exitCode = 1;
      }
      if (!values['dry-run'])
        atomicJson(
          watchHeartbeatPath(root),
          watchHeartbeat(state, Date.now(), status, { error, reads: api.calls })
        );
      if (values.once || values['dry-run'] || stopped) break;
      await setTimeout(Math.max(0, WATCH_LIMITS.intervalMs - (Date.now() - now)));
    } while (!stopped);
  } finally {
    if (lock) rmSync(lock);
    process.removeListener('SIGINT', stop);
    process.removeListener('SIGTERM', stop);
  }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  await main();
