// Read-only evidence for the Infra lead; never a review, queue or release controller.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import {
  existsSync,
  appendFileSync,
  linkSync,
  mkdirSync,
  readFileSync,
  renameSync,
  statfsSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import { loadavg } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { restClient } from './merge-queue-metrics.mjs';
import { readReleaseCatalog } from './release-cut-read.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const SHA = /^[a-f0-9]{40}$/;
const FAILED = new Set(['failure', 'timed_out', 'action_required', 'stale']);
const ACTIVE = new Set(['queued', 'in_progress', 'waiting', 'pending', 'requested']);
const AGGREGATES = new Set([
  'Docker E2E',
  'Native Rust contracts',
  'Native package matrix',
  'Verify exact workspace test identities',
  'Complete trusted-main verification',
  'Report main verification failure',
]);
export const QUEUE_QUERY = `query InfraTickQueue($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    mergeQueue(branch: "main") {
      entries(first: 100) {
        totalCount
        pageInfo { hasNextPage }
        nodes { position state enqueuedAt pullRequest { number headRefOid url } }
      }
    }
  }
}`;
export const REVIEW_QUERY = `query InfraTickReviews($ids: [ID!]!) {
  rateLimit { cost }
  nodes(ids: $ids) {
    __typename
    ... on PullRequest {
      id number headRefOid changedFiles
      repository { nameWithOwner }
      files(first: 100) {
        totalCount
        pageInfo { hasNextPage }
        nodes { path }
      }
      comments(last: 100) {
        totalCount
        pageInfo { hasPreviousPage hasNextPage }
        nodes { id body url }
      }
    }
  }
}`;
export const COVERAGE =
  'Run health uses 24 h/interval counts; failure, timed_out, action_required and stale details use bounded created-window searches, plus tracked older runs and older queued/in-progress runs. Worker metadata is complete; logs are sampled: first failed non-aggregate worker per attempt, newest runs first, at most three per tick. Unread logs are not retried automatically or asserted healthy; recurrence uses observed tokens only, including retained 24 h samples. Workflow startup_failure conclusions are not searched and remain outside detail coverage. An unobserved older rerun that starts and finishes between ticks is outside coverage. Review scope is open PRs updated since the tick boundary plus PRs merged in the interval. Statements are evidence, not scripted acceptance.';
const hash = (text) => createHash('sha256').update(text).digest('hex');
const time = (value) => {
  assert.ok(
    typeof value === 'string' && /Z$/.test(value) && Number.isFinite(Date.parse(value)),
    'Invalid UTC evidence timestamp.'
  );
  return Date.parse(value);
};
function identity(row, label) {
  assert.ok(Number.isSafeInteger(row?.id) && row.id > 0, `Invalid ${label} identity.`);
  return row.id;
}

/** One shared API-call/deadline owner, including the two fixed read-only queries. */
export function createTickClient({
  repository,
  cache,
  execute = runPackedCommand,
  clock = Date.now,
  trace,
}) {
  assert.match(repository, /^[\w.-]+\/[\w.-]+$/);
  const prefix = `repos/${repository}/`;
  let requests = 0,
    queueQueries = 0,
    reviewQueries = 0,
    reviewPoints = 0,
    stage = 'acquisition',
    prCount = 0;
  const deadline = clock() + 120_000;
  let logDeadline, logTimeoutMs;
  const timeBudget = () =>
    new Error(
      `Infra tick time budget exhausted (120 seconds; ${requests}/80 calls; stage=${stage}; PRs=${prCount}).`
    );
  const command = (program, args, options) => {
    assert.equal(program, 'gh', 'Only the GitHub read port is allowed.');
    assert.equal(args[0], 'api', 'Only the API read port is allowed.');
    if (args[1] === 'graphql') {
      if (args[5] === `query=${QUEUE_QUERY}`) {
        assert.deepEqual(
          args,
          [
            'api',
            'graphql',
            '--method',
            'POST',
            '-f',
            `query=${QUEUE_QUERY}`,
            '-f',
            `owner=${repository.split('/')[0]}`,
            '-f',
            `name=${repository.split('/')[1]}`,
          ],
          'Only the fixed queue query is allowed.'
        );
        assert.ok(++queueQueries <= 1, 'Only one queue query per tick is allowed.');
      } else {
        assert.deepEqual(
          args.slice(0, 6),
          ['api', 'graphql', '--method', 'POST', '-f', `query=${REVIEW_QUERY}`],
          'Only the fixed review query is allowed.'
        );
        const fields = args.slice(6);
        assert.ok(
          fields.length > 0 && fields.length <= 200 && fields.length % 2 === 0,
          'Review query needs at most 100 node IDs.'
        );
        assert.ok(
          fields.every((field, index) =>
            index % 2 === 0 ? field === '-F' : /^ids\[\]=[A-Za-z0-9_+\/=-]{1,256}$/.test(field)
          ),
          'Only review node-ID variables are allowed.'
        );
        assert.equal(
          new Set(fields.filter((_, index) => index % 2 === 1)).size,
          fields.length / 2,
          'Duplicate review node ID.'
        );
        assert.ok(++reviewQueries <= 1, 'Only one review query per tick is allowed.');
      }
    } else {
      const endpoint = args.find((arg) => arg === prefix.slice(0, -1) || arg.startsWith(prefix));
      assert.ok(
        endpoint &&
          !decodeURIComponent(endpoint.split('?')[0])
            .split('/')
            .some((part) => part === '.' || part === '..'),
        'Foreign REST path.'
      );
      assert.equal(
        args.filter((arg) => arg === '--method').length,
        1,
        'Explicit read method required.'
      );
      assert.equal(args[args.indexOf('--method') + 1], 'GET', 'REST writes are forbidden.');
      assert.ok(
        !args.some((arg) => ['-f', '-F', '--input', '--raw-field', '--field'].includes(arg)),
        'REST fields are forbidden.'
      );
    }
    if (requests >= 80)
      throw new Error(
        `Infra tick API call budget exhausted (80 calls / 120 seconds; stage=${stage}; PRs=${prCount}).`
      );
    if (clock() >= deadline) throw timeBudget();
    requests++;
    const started = clock();
    let outcome = 'error',
      graphqlCost;
    try {
      const result = execute(program, args, {
        ...options,
        // Hosted Rust logs exceed the packed command's 2 MiB default. Keep the
        // existing owner's hard 16 MiB ceiling, only for raw job-log captures.
        ...(args.includes('--allow-escape-sequences') ? { maxBuffer: 16 * 1024 * 1024 } : {}),
        timeoutMs: Math.min(options.timeoutMs, deadline - clock(), logTimeoutMs ?? Infinity),
      });
      if (args[5] === `query=${REVIEW_QUERY}`) {
        const response = JSON.parse(result);
        graphqlCost = response.data?.rateLimit?.cost;
        assert.ok(
          Number.isSafeInteger(graphqlCost) && graphqlCost > 0,
          'Review GraphQL cost missing, invalid or above 20 points.'
        );
        reviewPoints += graphqlCost;
        assert.ok(graphqlCost <= 20, 'Review GraphQL cost above 20 points.');
        assert.ok(!response.errors?.length, 'GraphQL review errors.');
      }
      outcome = 'success';
      return result;
    } catch (error) {
      if (clock() >= deadline) throw timeBudget();
      throw error;
    } finally {
      trace?.({
        request: requests,
        method: args[1] === 'graphql' ? 'POST' : 'GET',
        endpoint:
          args[1] === 'graphql'
            ? args[5] === `query=${QUEUE_QUERY}`
              ? 'graphql:InfraTickQueue'
              : 'graphql:InfraTickReviews'
            : args.find((arg) => arg.startsWith(prefix.slice(0, -1))),
        outcome,
        milliseconds: Math.max(0, clock() - started),
        ...(Number.isSafeInteger(graphqlCost) ? { graphqlCost } : {}),
      });
    }
  };
  const api = restClient({ cache, repo: repository, maxRequests: 80, execute: command });
  return {
    get: api.get,
    failureLog: (job) => {
      identity({ id: job }, 'failed job');
      if (clock() >= deadline) throw timeBudget();
      logDeadline ??= clock() + 60_000;
      if (requests >= 78) return { attempted: false, reason: 'two-call headroom' };
      const timeoutMs = Math.min(20_000, logDeadline - clock(), deadline - clock() - 10_000);
      if (timeoutMs <= 0) return { attempted: false, reason: 'diagnostic time/headroom' };
      try {
        logTimeoutMs = timeoutMs;
        const text = api.get(`${prefix}actions/jobs/${job}/logs`, true, true);
        if (clock() >= deadline) throw timeBudget();
        assert.equal(typeof text, 'string', 'Invalid job log.');
        return { attempted: true, text };
      } catch (error) {
        if (error.message.includes('budget')) throw error;
        return { attempted: true, reason: 'download unavailable or timed out' };
      } finally {
        logTimeoutMs = undefined;
      }
    },
    qualifyTerminalJobs: api.qualifyTerminalJobs,
    stage: (name, count = 0) => {
      stage = name;
      prCount = count;
      if (clock() >= deadline) throw timeBudget();
    },
    releases: () => {
      const repositoryEvidence = JSON.parse(
        command('gh', ['api', prefix.slice(0, -1), '--method', 'GET'], {
          cwd: ROOT,
          env: process.env,
          timeoutMs: 10_000,
        })
      );
      assert.equal(repositoryEvidence.permissions?.push, true, 'Draft visibility unavailable.');
      return readReleaseCatalog({ repository }, command);
    },
    queue: () => {
      const result = JSON.parse(
        command(
          'gh',
          [
            'api',
            'graphql',
            '--method',
            'POST',
            '-f',
            `query=${QUEUE_QUERY}`,
            '-f',
            `owner=${repository.split('/')[0]}`,
            '-f',
            `name=${repository.split('/')[1]}`,
          ],
          { cwd: ROOT, env: process.env, timeoutMs: 30_000 }
        )
      );
      assert.ok(!result.errors?.length, 'GraphQL queue errors.');
      return result.data?.repository?.mergeQueue?.entries;
    },
    reviews: (ids) => {
      const result = JSON.parse(
        command(
          'gh',
          [
            'api',
            'graphql',
            '--method',
            'POST',
            '-f',
            `query=${REVIEW_QUERY}`,
            ...ids.flatMap((id) => ['-F', `ids[]=${id}`]),
          ],
          { cwd: ROOT, env: process.env, timeoutMs: 30_000 }
        )
      );
      return result.data?.nodes;
    },
    evidence: () => ({
      ...api.evidence(),
      requests,
      queueQueries,
      reviewQueries,
      reviewPoints,
      stage,
      prCount,
    }),
  };
}

function pages(
  api,
  endpoint,
  key,
  expected,
  rowKey = (row) => identity(row, 'REST record'),
  immutable = false
) {
  const rows = [],
    paths = [],
    seen = new Set();
  let total = expected;
  for (let page = 1; ; page++) {
    const url = `${endpoint}${endpoint.includes('?') ? '&' : '?'}per_page=100&page=${page}`;
    paths.push(url);
    const result = api.get(url, immutable);
    const batch = key ? result?.[key] : result;
    assert.ok(Array.isArray(batch) && batch.length <= 100, 'Incomplete REST page.');
    if (key) {
      assert.ok(
        Number.isSafeInteger(result.total_count) && result.total_count >= 0,
        'Missing REST count.'
      );
      if (total !== undefined) assert.equal(result.total_count, total, 'Changing REST count.');
      total = result.total_count;
      if (key === 'workflow_runs')
        assert.ok(total < 1_000, `Workflow run search reaches the 1,000-result limit: ${endpoint}`);
    }
    for (const row of batch) {
      const id = rowKey(row);
      assert.ok(!seen.has(id), 'Duplicate REST identity.');
      seen.add(id);
      rows.push(row);
    }
    if (total !== undefined && rows.length === total) return { rows, paths };
    assert.ok(total === undefined || rows.length < total, 'REST count exceeded.');
    if (batch.length < 100) {
      assert.equal(total, undefined, 'Truncated REST evidence.');
      return { rows, paths };
    }
  }
}

export function failingTests(log) {
  const found = new Set();
  for (const line of log.split(/\r?\n/)) {
    const nextest = /\bFAIL\s+\[[^\]]+\]\s+(\S+)\s+(.+?)\s*$/.exec(line);
    const libtest = /\btest (\S+) \.\.\. FAILED\s*$/.exec(line);
    if (nextest) found.add(`${nextest[1]}::${nextest[2]}`);
    else if (libtest) found.add(libtest[1]);
  }
  return [...found].sort();
}
function aggregateOnly(job) {
  const failures = (job.steps ?? []).filter((step) => FAILED.has(step.conclusion));
  return (
    AGGREGATES.has(job.name.split(' / ').at(-1)) ||
    (failures.length > 0 && failures.every((step) => step.name.startsWith('Require ')))
  );
}
function sharedPath(file) {
  return (
    file.startsWith('.github/') ||
    file.startsWith('contracts/') ||
    [
      'AGENTS.md',
      'ARCHITECTURE.md',
      'DEVELOPMENT.md',
      'CONVENTIONS.md',
      'Cargo.toml',
      'package.json',
      'pnpm-workspace.yaml',
      'rust/Cargo.toml',
    ].includes(file) ||
    /(?:^|\/)(?:Cargo\.lock|pnpm-lock\.yaml|package-lock\.json|yarn\.lock|bun\.lockb?|package\.json|pnpm-workspace\.yaml|tsconfig[^/]*\.json|vitest\.config\.[cm]?[jt]s)$/.test(
      file
    )
  );
}
function validateReviewMap(map) {
  assert.ok(
    Array.isArray(map?.rules) && map.rules.length > 0,
    'Reviewed path-to-lead map is required.'
  );
  for (const rule of map.rules) {
    assert.ok(
      typeof rule.prefix === 'string' &&
        rule.prefix &&
        typeof rule.source === 'string' &&
        rule.source.includes('606'),
      'Review map needs a #606 source line.'
    );
    assert.ok(
      Array.isArray(rule.leads) &&
        rule.leads.length > 0 &&
        rule.leads.every((lead) => /^tmt-(?:[a-z]+-)*lead$/.test(lead)),
      'Invalid stated lead.'
    );
  }
}

function completeReviewConnection(connection, label, tail = false) {
  assert.ok(
    Number.isSafeInteger(connection?.totalCount) &&
      connection.totalCount >= 0 &&
      Array.isArray(connection.nodes) &&
      connection.nodes.length <= 100 &&
      connection.nodes.length <= connection.totalCount &&
      typeof connection.pageInfo?.hasNextPage === 'boolean' &&
      (!tail || typeof connection.pageInfo.hasPreviousPage === 'boolean'),
    `Invalid GraphQL ${label} connection.`
  );
  if (connection.pageInfo.hasNextPage || (tail && connection.pageInfo.hasPreviousPage))
    return false;
  assert.equal(
    connection.nodes.length,
    connection.totalCount,
    `Truncated GraphQL ${label} evidence.`
  );
  return true;
}

/** Injected evidence ports; a partial collection never becomes an empty healthy report. */
export function collectInfraTick(
  api,
  { repository, since, until, previous = null, reviewMap, local }
) {
  api.stage?.('run evidence');
  const end = time(until),
    start = time(since),
    day = end - 24 * 60 * 60_000;
  assert.ok(
    start < end && start >= day,
    'Tick interval must be within the stated 24-hour coverage.'
  );
  assert.match(repository, /^[\w.-]+\/[\w.-]+$/);
  validateReviewMap(reviewMap);
  const needsReview = (file) =>
    sharedPath(file) || reviewMap.rules.some((rule) => file.startsWith(rule.prefix));
  if (previous) {
    assert.equal(previous.schema, 1, 'Invalid tick cursor schema.');
    assert.ok(
      time(previous.until) <= start &&
        Array.isArray(previous.failedDraftIds) &&
        previous.failedDraftIds.every((id) => Number.isSafeInteger(id) && id > 0) &&
        Array.isArray(previous.failedRunAttempts) &&
        previous.failedRunAttempts.every((key) => /^[1-9]\d*:[1-9]\d*$/.test(key)) &&
        Array.isArray(previous.activeRunIds) &&
        previous.activeRunIds.every((id) => Number.isSafeInteger(id) && id > 0),
      'Invalid tick cursor.'
    );
    if (previous.baselineUntil !== undefined)
      assert.ok(time(previous.baselineUntil) <= start, 'Bootstrap boundary cannot move backwards.');
  }
  const root = `repos/${repository}`,
    anomalies = [];
  const anomaly = (kind, evidence) => anomalies.push({ kind, ...evidence });
  const createdSince = new Date(day).toISOString();
  const createdWindow = encodeURIComponent(`${createdSince}..${until}`);
  // Counts are not enumerated: GitHub can report >1,000 matches but only return
  // the first 1,000 rows. Every enumerated status slice below must stay <1,000.
  const countCreated = (from) => {
    const endpoint = `${root}/actions/runs?created=${encodeURIComponent(`${from}..${until}`)}&per_page=1&page=1`;
    const result = api.get(endpoint);
    assert.ok(
      Number.isSafeInteger(result?.total_count) && result.total_count >= 0,
      'Missing run health count.'
    );
    assert.ok(
      Array.isArray(result.workflow_runs) &&
        result.workflow_runs.length === Math.min(result.total_count, 1),
      'Incomplete run health count.'
    );
    return result.total_count;
  };
  const health = { created24h: countCreated(createdSince), createdSinceTick: countCreated(since) };
  const discovered = [
    ...[...FAILED].map(
      (status) =>
        pages(
          api,
          `${root}/actions/runs?status=${status}&created=${createdWindow}`,
          'workflow_runs'
        ).rows
    ),
    ...[...ACTIVE].map(
      (status) =>
        pages(
          api,
          `${root}/actions/runs?status=${status}${['queued', 'in_progress'].includes(status) ? '' : `&created=${createdWindow}`}`,
          'workflow_runs'
        ).rows
    ),
  ];
  const joined = new Map();
  for (const batch of discovered)
    for (const run of batch) {
      identity(run, 'run');
      time(run.created_at);
      assert.ok(
        SHA.test(run.head_sha) && Number.isSafeInteger(run.run_attempt) && run.run_attempt > 0,
        'Incomplete run provenance.'
      );
      const existing = joined.get(run.id);
      if (existing)
        assert.ok(
          existing.head_sha === run.head_sha && existing.run_attempt === run.run_attempt,
          'Changing run attempt/source during acquisition.'
        );
      // These discovery sets intentionally overlap; identity joins are explicit, not page deduplication.
      joined.set(run.id, run);
    }
  const tracked = new Set(previous?.activeRunIds ?? []);
  for (const id of tracked)
    if (!joined.has(id)) {
      const run = api.get(`${root}/actions/runs/${id}`);
      assert.equal(identity(run, 'tracked run'), id, 'Tracked run identity changed.');
      assert.ok(
        SHA.test(run.head_sha) && Number.isSafeInteger(run.run_attempt) && run.run_attempt > 0,
        'Incomplete tracked run provenance.'
      );
      time(run.created_at);
      joined.set(id, run);
    }
  const runs = [...joined.values()].filter(
    (run) =>
      time(run.created_at) <= end &&
      (time(run.created_at) >= day || ACTIVE.has(run.status) || tracked.has(run.id))
  );
  assert.ok(
    previous?.failureEvidence === undefined || Array.isArray(previous.failureEvidence),
    'Invalid retained failure evidence.'
  );
  const failures = (previous?.failureEvidence ?? []).filter((row) => time(row.created) >= day),
    recurrence = new Map();
  assert.ok(
    failures.every(
      (row) =>
        Number.isSafeInteger(row.run) &&
        row.run > 0 &&
        Number.isSafeInteger(row.attempt) &&
        row.attempt > 0 &&
        time(row.created) <= end &&
        previous.failedRunAttempts.includes(`${row.run}:${row.attempt}`) &&
        Array.isArray(row.tests) &&
        row.tests.every((test) => typeof test === 'string')
    ),
    'Invalid retained failure evidence.'
  );
  const eligibleFailures = runs.filter((run) => {
    if (!FAILED.has(run.conclusion)) return false;
    if (!previous?.baselineUntil || tracked.has(run.id)) return true;
    // Bootstrap deliberately omits historical details, including old queued work
    // that finished before the first observation. Later transitions remain visible.
    return (
      time(run.created_at) > time(previous.baselineUntil) ||
      time(run.updated_at) > time(previous.baselineUntil)
    );
  });
  const failedRunAttempts = [
    ...new Set([
      ...eligibleFailures.map((run) => `${run.id}:${run.run_attempt}`),
      ...failures.map((row) => `${row.run}:${row.attempt}`),
    ]),
  ];
  const priorFailures = new Set(previous?.failedRunAttempts ?? []);
  api.stage?.('merge queue');
  const queue = [];
  try {
    const evidence = api.queue();
    assert.ok(
      evidence &&
        evidence.pageInfo?.hasNextPage === false &&
        Array.isArray(evidence.nodes) &&
        evidence.nodes.length <= 100 &&
        evidence.totalCount === evidence.nodes.length,
      'Incomplete queue evidence.'
    );
    const positions = new Set(),
      prs = new Set();
    for (const entry of evidence.nodes) {
      assert.ok(
        Number.isSafeInteger(entry.position) &&
          entry.position > 0 &&
          !positions.has(entry.position),
        'Invalid queue position.'
      );
      assert.ok(
        Number.isSafeInteger(entry.pullRequest?.number) &&
          entry.pullRequest.number > 0 &&
          !prs.has(entry.pullRequest.number) &&
          SHA.test(entry.pullRequest.headRefOid),
        'Invalid queue PR.'
      );
      assert.ok(
        ['AWAITING_CHECKS', 'LOCKED', 'MERGEABLE', 'QUEUED', 'UNMERGEABLE'].includes(entry.state),
        'Invalid queue state.'
      );
      positions.add(entry.position);
      prs.add(entry.pullRequest.number);
      const ageMinutes = (end - time(entry.enqueuedAt)) / 60_000;
      assert.ok(ageMinutes >= 0, 'Queue enqueue time is in the future.');
      const row = {
        pr: entry.pullRequest.number,
        head: entry.pullRequest.headRefOid,
        position: entry.position,
        state: entry.state,
        ageMinutes,
        url: entry.pullRequest.url,
      };
      queue.push(row);
      if (ageMinutes > 30) anomaly('merge queue entry older than 30 minutes', row);
    }
  } catch {
    queue.length = 0;
    anomaly('queue coverage unavailable', {});
  }
  api.stage?.('release catalogue');
  const releases = api.releases();
  for (const release of releases) identity(release, 'release');
  const failedDrafts = releases
    .filter(
      (release) =>
        release.draft && release.assets.some((asset) => asset.name === 'verification-failed.json')
    )
    .map((release) => ({ id: release.id, tag: release.tag_name }));
  const previousFailed = new Set(previous?.failedDraftIds ?? []);
  for (const release of failedDrafts)
    if (!previousFailed.has(release.id))
      anomaly(
        previous ? 'newly observed failed draft' : 'failed draft at initial baseline',
        release
      );
  api.stage?.('PR cohort');
  const stableListing = (endpoint) => {
    const first = pages(api, endpoint),
      verified = pages(api, endpoint);
    for (const row of [...first.rows, ...verified.rows]) time(row.updated_at);
    assert.equal(first.paths.length, verified.paths.length, 'Changing recent PR listing.');
    // A touch can reorder members across pages; only membership must remain stable.
    // pages() rejects repeated identities before this comparison, never dedupes them.
    assert.deepEqual(
      new Set(first.rows.map((row) => row.id)),
      new Set(verified.rows.map((row) => row.id)),
      'Changing recent PR listing.'
    );
    return first.rows;
  };
  const open = stableListing(`${root}/pulls?state=open&sort=updated&direction=desc`).filter(
    (pr) => time(pr.updated_at) >= start
  );
  // The server filter, not listing order, bounds closed history. Overlap one second
  // before the rounded-down boundary so local filtering retains inclusive `since`.
  const overlap = new Date(Math.floor(start / 1000) * 1000 - 1000).toISOString();
  const candidates = stableListing(
    `${root}/issues?state=closed&since=${encodeURIComponent(overlap)}&sort=updated&direction=desc`
  ).filter((row) => time(row.updated_at) >= start && row.pull_request != null);
  for (const row of candidates)
    assert.ok(
      Number.isSafeInteger(row.number) &&
        row.number > 0 &&
        typeof row.node_id === 'string' &&
        row.node_id.length > 0 &&
        row.pull_request?.url === `https://api.github.com/${root}/pulls/${row.number}`,
      'Incomplete closed PR candidate identity.'
    );
  api.stage?.('PR hydration', open.length + candidates.length);
  const used = api.evidence?.().requests;
  assert.ok(Number.isSafeInteger(used) && used >= 0 && used <= 80, 'Missing API budget evidence.');
  const required = candidates.length + (open.length || candidates.length ? 1 : 0);
  assert.ok(
    required <= 80 - used,
    `Infra tick API call budget exhausted (80 calls / 120 seconds; stage=PR hydration; PRs=${open.length + candidates.length}; H=${candidates.length}; required=${required}; remaining=${80 - used}).`
  );
  const closed = [];
  for (const candidate of candidates) {
    const pr = api.get(`${root}/pulls/${candidate.number}`);
    identity(pr, 'PR');
    assert.ok(
      pr.number === candidate.number && pr.node_id === candidate.node_id && pr.state === 'closed',
      'Changing closed PR candidate identity or state.'
    );
    time(pr.updated_at);
    assert.ok(pr.merged_at === null || typeof pr.merged_at === 'string', 'Missing merge evidence.');
    if (pr.merged_at && time(pr.merged_at) >= start && time(pr.merged_at) <= end) closed.push(pr);
  }
  const cohort = [...open, ...closed];
  api.stage?.('PR review', cohort.length);
  assert.ok(cohort.length <= 100, `PR review stage: ${cohort.length} PRs exceeds 100-node bound.`);
  for (const pr of cohort)
    assert.ok(
      Number.isSafeInteger(pr.number) &&
        pr.number > 0 &&
        SHA.test(pr.head?.sha) &&
        typeof pr.node_id === 'string' &&
        pr.node_id.length > 0,
      'Incomplete listed PR identity.'
    );
  const batch = cohort.length ? api.reviews(cohort.map((pr) => pr.node_id)) : [];
  assert.ok(
    Array.isArray(batch) && batch.length === cohort.length,
    'Incomplete GraphQL review cohort.'
  );
  const seen = new Set();
  const reviewEvidence = [];
  for (const [index, pr] of cohort.entries()) {
    const row = batch[index];
    assert.ok(
      row?.__typename === 'PullRequest' &&
        row.id === pr.node_id &&
        !seen.has(row.id) &&
        row.number === pr.number &&
        row.repository?.nameWithOwner === repository &&
        Number.isSafeInteger(row.changedFiles) &&
        row.changedFiles >= 0,
      'Foreign, duplicate or incomplete GraphQL PR identity.'
    );
    seen.add(row.id);
    assert.equal(row.headRefOid, pr.head.sha, 'PR head changed while reading review evidence.');
    assert.equal(row.files?.totalCount, row.changedFiles, 'Incomplete PR file count.');
    let files, comments, verified;
    const completeFiles = completeReviewConnection(row.files, 'files');
    if (completeFiles) {
      files = row.files.nodes.map((file) => {
        assert.ok(
          typeof file.path === 'string' && file.path.length > 0,
          'Missing changed file path.'
        );
        return file.path;
      });
      assert.equal(new Set(files).size, files.length, 'Duplicate changed file path.');
      const shared = files.some(needsReview);
      if (!shared) comments = [];
      else if (completeReviewConnection(row.comments, 'comments', true)) {
        const ids = new Set();
        comments = row.comments.nodes.map((comment) => {
          assert.ok(
            typeof comment.id === 'string' &&
              comment.id &&
              !ids.has(comment.id) &&
              typeof comment.body === 'string' &&
              typeof comment.url === 'string' &&
              comment.url,
            'Invalid GraphQL comment identity or evidence.'
          );
          ids.add(comment.id);
          return { body: comment.body, html_url: comment.url };
        });
      }
    }
    // Only a paginated PR pays for the old complete REST walk. Batched heads,
    // file counts and complete comment bodies otherwise replace all three reads.
    if (comments === undefined) {
      files = pages(api, `${root}/pulls/${pr.number}/files`, undefined, undefined, (file) => {
        assert.ok(
          typeof file.filename === 'string' && file.filename.length > 0,
          'Missing changed file path.'
        );
        return file.filename;
      }).rows.map((file) => file.filename);
      const shared = files.some(needsReview);
      comments = shared ? pages(api, `${root}/issues/${pr.number}/comments`).rows : [];
      verified = api.get(`${root}/pulls/${pr.number}`);
    } else
      verified = {
        number: row.number,
        head: { sha: row.headRefOid },
        changed_files: row.changedFiles,
      };
    const shared = files.filter(needsReview);
    const requirements = new Map();
    for (const file of shared) {
      const rules = reviewMap.rules.filter((rule) => file.startsWith(rule.prefix));
      if (!rules.length) anomaly('unmapped shared path', { pr: pr.number, file });
      for (const rule of rules)
        for (const lead of rule.leads) {
          if (!requirements.has(lead)) requirements.set(lead, []);
          requirements.get(lead).push({ file, source: rule.source });
        }
    }
    assert.equal(verified.head?.sha, pr.head.sha, 'PR head changed while reading review evidence.');
    assert.ok(
      verified.number === pr.number &&
        Number.isSafeInteger(verified.changed_files) &&
        verified.changed_files === files.length,
      'Incomplete PR file count.'
    );
    if (!shared.length) continue;
    const statements = comments
      .filter((comment) => typeof comment.body === 'string' && comment.body.includes(pr.head.sha))
      .flatMap((comment) =>
        [...requirements.keys()]
          .filter((lead) => new RegExp(`(?:^|\\n)Agent: ${lead}(?:\\s|$)`).test(comment.body))
          .map((lead) => ({
            lead,
            head: pr.head.sha,
            comment: comment.body,
            url: comment.html_url,
          }))
      );
    for (const lead of requirements.keys())
      if (!statements.some((statement) => statement.lead === lead))
        anomaly('missing head-named lead statement; judgement required', {
          pr: pr.number,
          head: pr.head.sha,
          lead,
        });
    reviewEvidence.push({
      pr: pr.number,
      head: pr.head.sha,
      shared,
      requirements: [...requirements].map(([lead, sources]) => ({ lead, sources })),
      statements,
      disposition: 'Lead must judge acceptance; statements are not interpreted as approval.',
    });
  }
  api.stage?.('failed run metadata', cohort.length);
  const pendingFailureLogs = [];
  for (const run of eligibleFailures
    .filter((run) => !priorFailures.has(`${run.id}:${run.run_attempt}`))
    .sort((a, b) => time(b.created_at) - time(a.created_at) || b.id - a.id)) {
    assert.equal(run.status, 'completed', 'Failed run evidence is not terminal.');
    const jobs = pages(
      api,
      `${root}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`,
      'jobs',
      undefined,
      undefined,
      true
    );
    assert.ok(
      jobs.rows.every(
        (job) =>
          job.status === 'completed' &&
          job.completed_at &&
          job.run_id === run.id &&
          job.run_attempt === run.run_attempt &&
          job.head_sha === run.head_sha
      ),
      'Incomplete or foreign terminal job evidence.'
    );
    api.qualifyTerminalJobs?.(jobs.paths);
    const workers = jobs.rows
      .filter((job) => FAILED.has(job.conclusion) && !aggregateOnly(job))
      .sort((a, b) => a.id - b.id);
    const context =
      run.event === 'merge_group'
        ? `merge_group:${run.id}`
        : run.event === 'pull_request' && run.pull_requests?.length === 1
          ? `pr:${run.pull_requests[0].number}`
          : null;
    for (const job of workers) {
      assert.equal(typeof job.name, 'string', 'Missing worker name.');
      const failure = {
        run: run.id,
        attempt: run.run_attempt,
        event: run.event,
        context,
        workflow: run.name,
        workflowPath: run.path ?? null,
        publicInstall: (job.steps ?? []).some(
          (step) => step.name === 'Install from the public release'
        ),
        worker: job.name,
        job: job.id,
        url: job.html_url,
        tests: [],
        logHash: null,
        unavailable: 'log not read: pending',
        created: run.created_at,
      };
      failures.push(failure);
      pendingFailureLogs.push(failure);
    }
    if (!priorFailures.has(`${run.id}:${run.run_attempt}`))
      anomaly(previous ? 'newly observed failed run' : 'failed run at initial baseline', {
        run: run.id,
        attempt: run.run_attempt,
        workflow: run.name,
        url: run.html_url,
        workers: workers.map((job) => job.name),
        attribution: workers.length
          ? 'observed failed worker jobs'
          : 'no independent failed worker identified',
      });
    if (
      !priorFailures.has(`${run.id}:${run.run_attempt}`) &&
      /(?:^|\/)native-release(?:-smoke)?\.yml(?:@|$)/.test(run.path ?? '')
    )
      anomaly('failed release or post-publication check', {
        run: run.id,
        workflow: run.name,
        url: run.html_url,
        workers: workers.map((job) => job.name),
      });
  }
  const postPublicationFailures = [];
  for (const failure of failures) {
    if (!/(?:^|\/)native-release(?:-smoke)?\.yml(?:@|$)/.test(failure.workflowPath ?? '')) continue;
    // This exact tag-bearing job label is owned by native-release-smoke.yml.
    const smoke = /^Install (\S+) from the public release \([^)]+\)$/.exec(
      failure.worker.split(' / ').at(-1)
    );
    if (!smoke) {
      if (failure.publicInstall)
        anomaly('public-install release evidence unavailable', {
          run: failure.run,
          job: failure.job,
        });
      continue;
    }
    const matching = releases.filter((release) => release.tag_name === smoke[1]);
    if (matching.length !== 1 || matching[0].draft) {
      anomaly('public-install release evidence unavailable', {
        tag: smoke[1],
        run: failure.run,
        job: failure.job,
      });
      continue;
    }
    const published = {
      tag: smoke[1],
      releaseId: matching[0].id,
      run: failure.run,
      attempt: failure.attempt,
      worker: failure.worker,
      url: failure.url,
    };
    postPublicationFailures.push(published);
    if (!priorFailures.has(`${failure.run}:${failure.attempt}`))
      anomaly('published release post-publication check failed', published);
  }
  api.stage?.('failure logs', cohort.length);
  const sampledAttempts = new Set();
  let logAttempts = 0;
  for (const failure of pendingFailureLogs) {
    const attempt = `${failure.run}:${failure.attempt}`;
    // After the first worker, the attempt cap takes precedence over tick/time caps.
    if (sampledAttempts.has(attempt)) failure.unavailable = 'log not read: one-per-attempt cap';
    else {
      sampledAttempts.add(attempt);
      if (logAttempts >= 3) failure.unavailable = 'log not read: three-per-tick cap';
      else {
        const result = api.failureLog(failure.job);
        if (result.attempted) logAttempts++;
        if (typeof result.text === 'string') {
          failure.tests = failingTests(result.text);
          failure.logHash = hash(result.text);
          failure.unavailable = null;
        } else {
          failure.unavailable = `log not read: ${result.reason}`;
          if (result.attempted)
            anomaly('test log unavailable', {
              run: failure.run,
              job: failure.job,
              reason: failure.unavailable,
            });
        }
      }
    }
  }
  const logCoverage = {
    read: failures.filter((failure) => /^[a-f0-9]{64}$/.test(failure.logHash ?? '')).length,
    total: failures.length,
    attempted: logAttempts,
  };
  for (const failure of failures)
    for (const test of failure.tests)
      if (failure.context) {
        if (!recurrence.has(test)) recurrence.set(test, new Map());
        recurrence.get(test).set(failure.context, failure);
      }
  const repeated = [...recurrence]
    .filter(([, contexts]) => contexts.size > 1)
    .map(([test, contexts]) => ({ test, occurrences: [...contexts.values()] }));
  for (const item of repeated) anomaly('test failed across distinct PR/merge-group contexts', item);
  api.stage?.('local guides and host', cohort.length);
  assert.deepEqual(
    Object.keys(local.guides).sort(),
    ['ARCHITECTURE.md', 'DEVELOPMENT.md'],
    'Both guide texts are required.'
  );
  const guides = Object.entries(local.guides).map(([file, text]) => {
    const lines = text.endsWith('\n') ? text.split('\n').length - 1 : text.split('\n').length;
    const cap = local.budgets.lines[file],
      bytes = Buffer.byteLength(text),
      byteCap = local.budgets.bytes[file];
    assert.ok(Number.isSafeInteger(cap) && Number.isSafeInteger(byteCap), 'Missing guide budget.');
    const guide = { file, lines, cap, bytes, byteCap };
    if (cap - lines <= 10 || bytes > byteCap) anomaly('guide near or over budget', guide);
    return guide;
  });
  assert.ok(
    Number.isFinite(local.freeBytes) &&
      local.freeBytes >= 0 &&
      Number.isFinite(local.load1) &&
      local.load1 >= 0,
    'Invalid host evidence.'
  );
  return {
    schema: 1,
    repository,
    since,
    until,
    coverage:
      COVERAGE +
      (previous?.baselineUntil
        ? ` Failure details begin at bootstrap boundary ${previous.baselineUntil}; earlier history was intentionally not analysed. Older active work completed before the first observation is outside coverage.`
        : ''),
    anomalies,
    health,
    baseline: previous
      ? `incremental; only unseen failed attempts analysed${previous.baselineUntil ? ` after bootstrap ${previous.baselineUntil}` : ''}`
      : 'initial; qualified terminal job/log cache resumes after a budget refusal',
    olderActive: runs
      .filter((run) => time(run.created_at) < day && ACTIVE.has(run.status))
      .map((run) => ({ run: run.id, attempt: run.run_attempt, status: run.status })),
    trackedCompletions: runs
      .filter((run) => tracked.has(run.id) && !ACTIVE.has(run.status))
      .map((run) => ({
        run: run.id,
        attempt: run.run_attempt,
        status: run.status,
        conclusion: run.conclusion,
      })),
    failures,
    logCoverage,
    repeated,
    queue,
    failedDrafts,
    postPublicationFailures,
    failedRunAttempts,
    activeRunIds: runs.filter((run) => ACTIVE.has(run.status)).map((run) => run.id),
    reviewEvidence,
    guides,
    host: { freeBytes: local.freeBytes, load1: local.load1 },
    evidence: api.evidence?.() ?? {},
    complete: !anomalies.some((item) =>
      ['queue coverage unavailable', 'public-install release evidence unavailable'].includes(
        item.kind
      )
    ),
  };
}

function anomalyPriority({ kind }) {
  switch (kind) {
    case 'collection refused':
    case 'newly observed failed draft':
    case 'failed draft at initial baseline':
    case 'failed release or post-publication check':
    case 'published release post-publication check failed':
    case 'public-install release evidence unavailable':
      return 0;
    case 'guide near or over budget':
      return 2;
    case 'merge queue entry older than 30 minutes':
    case 'queue coverage unavailable':
      return 3;
    case 'unmapped shared path':
      return 4;
    case 'missing head-named lead statements; judgement required':
      return 5;
    default:
      // Failed runs, repeated tests and other diagnostic failures precede budgets.
      return 1;
  }
}

export function renderInfraTick(report) {
  const rows = [
    `# Infra tick ${report.until}`,
    `logs read: ${report.logCoverage.read} of ${report.logCoverage.total} failed workers (retained samples included). Unread logs are not retried automatically or asserted healthy.`,
    '',
    report.coverage,
    '',
    `Anomalies: ${report.anomalies.length}`,
  ];
  const statements = report.anomalies.filter(
    (item) => item.kind === 'missing head-named lead statement; judgement required'
  );
  const printed = report.anomalies
    .filter((item) => !statements.includes(item))
    .map((item) => ({ item, count: 1 }));
  if (statements.length) {
    const prs = new Map();
    for (const { pr, head, lead } of statements) {
      if (!prs.has(pr)) prs.set(pr, { pr, head, leads: [] });
      prs.get(pr).leads.push(lead);
    }
    printed.push({
      item: {
        kind: 'missing head-named lead statements; judgement required',
        count: statements.length,
        prs: [...prs.values()].slice(0, 3),
        remainingPrs: Math.max(0, prs.size - 3),
      },
      count: statements.length,
    });
  }
  printed.sort((a, b) => anomalyPriority(a.item) - anomalyPriority(b.item));
  for (const { item } of printed.slice(0, 20)) {
    const { occurrences, ...summary } = item;
    const detail = occurrences ? { ...summary, contexts: occurrences.length } : summary;
    rows.push(`- ${item.kind}: ${JSON.stringify(detail)}`);
  }
  if (printed.length > 20)
    rows.push(
      `- ${printed.slice(20).reduce((total, entry) => total + entry.count, 0)} more anomalies; full evidence is in summary.json.`
    );
  if (!report.anomalies.length) rows.push('No anomalies observed within the stated coverage.');
  rows.push(
    '',
    `${report.health.createdSinceTick} newly created runs (${report.health.created24h} in 24 h); ${report.olderActive.length} older active runs; ${report.trackedCompletions.length} tracked completions; ${report.failures.length} observed failed workers; ${report.failedDrafts.length} failed drafts; ${new Set(report.postPublicationFailures.map((failure) => failure.releaseId)).size} published releases with failed public checks.`,
    `Host: ${(report.host.freeBytes / 1024 ** 3).toFixed(1)} GiB free; 1-minute load ${report.host.load1}.`,
    `Evidence: ${report.evidence.requests ?? 'fixture'} API calls. Shared-path statements: ${report.reviewEvidence.length} PRs; acceptance requires lead judgement.`,
    ''
  );
  for (const failure of report.failures)
    if (failure.unavailable)
      rows.push(
        `- log not read: ${JSON.stringify({ run: failure.run, job: failure.job, worker: failure.worker, url: failure.url, reason: failure.unavailable })}`
      );
  return rows.join('\n');
}

function main() {
  const { values } = parseArgs({
    options: {
      repo: { type: 'string', default: 'pj-tmt/tmt' },
      output: { type: 'string' },
      'review-map': { type: 'string' },
      since: { type: 'string' },
      until: { type: 'string' },
      bootstrap: { type: 'boolean', default: false },
    },
  });
  assert.ok(
    values.output && values['review-map'],
    'Usage: infra-tick-summary.mjs --output DIR --review-map FILE [--bootstrap | --since UTC] [--until UTC] [--repo OWNER/REPO]'
  );
  mkdirSync(values.output, { recursive: true });
  const cursor = path.join(values.output, 'cursor.json');
  const until = values.until ?? new Date().toISOString();
  assert.ok(time(until) <= Date.now(), 'Tick end cannot be in the future.');
  const secrets = [process.env.GH_TOKEN, process.env.GITHUB_TOKEN].filter(Boolean);
  const safe = (text) =>
    secrets.reduce((result, secret) => result.replaceAll(secret, '[REDACTED]'), text);
  const invocation = `${until}:${process.pid}`;
  const callLog = path.join(values.output, 'calls.jsonl');
  const trace = (entry) =>
    appendFileSync(callLog, safe(JSON.stringify({ invocation, ...entry })) + '\n', { mode: 0o600 });
  trace({ event: 'invocation', until });
  let since = values.since ?? new Date(Date.parse(until) - 60 * 60_000).toISOString();
  const api = createTickClient({
    repository: values.repo,
    cache: path.join(values.output, 'evidence'),
    trace,
  });
  try {
    const previous = existsSync(cursor) ? JSON.parse(readFileSync(cursor, 'utf8')) : null;
    since = values.since ?? previous?.until ?? since;
    const reviewMap = JSON.parse(readFileSync(values['review-map'], 'utf8'));
    validateReviewMap(reviewMap);
    if (values.bootstrap || (!previous && !values.since)) {
      assert.equal(previous, null, 'Cannot bootstrap an existing cursor.');
      assert.ok(!values.since, '--bootstrap and --since are mutually exclusive.');
      const report = {
        schema: 1,
        kind: 'baseline established',
        complete: false,
        baselineEstablished: true,
        until,
        coverage:
          'Baseline established; details from the next tick. No historical detail reads or health assessment. Run the same command again for evidence after this boundary.',
        evidence: api.evidence(),
      };
      const markdown = `# Infra tick baseline established\n\n${report.coverage}\n\nBoundary: ${until}. API calls: 0.\n`;
      writeFileSync(
        path.join(values.output, 'summary.json'),
        JSON.stringify(report, null, 2) + '\n',
        { mode: 0o600 }
      );
      writeFileSync(path.join(values.output, 'summary.md'), markdown, { mode: 0o600 });
      const temporary = `${cursor}.${process.pid}.tmp`;
      writeFileSync(
        temporary,
        JSON.stringify({
          schema: 1,
          until,
          baselineUntil: until,
          failedDraftIds: [],
          failedRunAttempts: [],
          failureEvidence: [],
          activeRunIds: [],
        }) + '\n',
        { mode: 0o600, flag: 'wx' }
      );
      try {
        // Publish an entire cursor without replacing another observer's boundary.
        linkSync(temporary, cursor);
      } finally {
        unlinkSync(temporary);
      }
      trace({ event: 'baseline established', requests: 0, complete: false });
      process.stdout.write(markdown);
      return;
    }
    const report = collectInfraTick(api, {
      repository: values.repo,
      since,
      until,
      previous,
      reviewMap,
      local: {
        budgets: JSON.parse(readFileSync(new URL('./guide-budgets.json', import.meta.url), 'utf8')),
        guides: Object.fromEntries(
          ['ARCHITECTURE.md', 'DEVELOPMENT.md'].map((file) => [
            file,
            readFileSync(path.join(ROOT, file), 'utf8'),
          ])
        ),
        freeBytes: (() => {
          const fs = statfsSync(ROOT);
          return fs.bavail * fs.bsize;
        })(),
        load1: loadavg()[0],
      },
    });
    writeFileSync(
      path.join(values.output, 'summary.json'),
      safe(JSON.stringify(report, null, 2) + '\n'),
      { mode: 0o600 }
    );
    const markdown = safe(renderInfraTick(report));
    writeFileSync(path.join(values.output, 'summary.md'), markdown, { mode: 0o600 });
    process.stdout.write(markdown);
    if (report.complete) {
      const temporary = `${cursor}.${process.pid}.tmp`;
      writeFileSync(
        temporary,
        JSON.stringify({
          schema: 1,
          until,
          failedDraftIds: report.failedDrafts.map((release) => release.id),
          failedRunAttempts: report.failedRunAttempts,
          failureEvidence: report.failures,
          activeRunIds: report.activeRunIds,
          ...(previous?.baselineUntil ? { baselineUntil: previous.baselineUntil } : {}),
        }) + '\n',
        { mode: 0o600 }
      );
      renameSync(temporary, cursor);
    } else process.exitCode = 1;
    trace({ event: 'completed', requests: api.evidence().requests, complete: report.complete });
  } catch (error) {
    const { stage, prCount } = api.evidence();
    const report = {
      schema: 1,
      complete: false,
      since,
      until,
      coverage: COVERAGE,
      anomalies: [{ kind: 'collection refused', message: safe(error.message), stage, prCount }],
      evidence: api.evidence(),
    };
    writeFileSync(
      path.join(values.output, 'summary.json'),
      JSON.stringify(report, null, 2) + '\n',
      { mode: 0o600 }
    );
    const markdown = `# Infra tick refused\n\n${COVERAGE}\n\n- collection refused: ${safe(error.message)}\n- stage: ${stage}; PRs: ${prCount}\n- API calls: ${api.evidence().requests}/80. Cursor unchanged. Qualified terminal job/log evidence is retained. Retry a transient refusal once; after a second refusal stop and split the window with --until or escalate.\n`;
    writeFileSync(path.join(values.output, 'summary.md'), markdown, { mode: 0o600 });
    process.stdout.write(markdown);
    trace({ event: 'refused', requests: api.evidence().requests, complete: false });
    process.exitCode = 1;
  }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
