// Read-only evidence for the Infra lead; never a review, queue or release controller.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  statfsSync,
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
export const COVERAGE =
  'Run health uses 24 h/interval counts; failure, timed_out, action_required and stale details use bounded created-window searches, plus tracked older runs and older queued/in-progress runs. Previously analysed failure tokens are retained for 24 h without repeat detail reads. An unobserved older rerun that starts and finishes between ticks is outside coverage. Review scope is open PRs updated since the tick boundary plus PRs merged in the interval. Statements are evidence, not scripted acceptance.';
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

/** One shared API-call/deadline owner, including the single fixed query exception. */
export function createTickClient({
  repository,
  cache,
  execute = runPackedCommand,
  clock = Date.now,
}) {
  assert.match(repository, /^[\w.-]+\/[\w.-]+$/);
  const prefix = `repos/${repository}/`;
  let requests = 0,
    queries = 0;
  const deadline = clock() + 120_000;
  const command = (program, args, options) => {
    assert.equal(program, 'gh', 'Only the GitHub read port is allowed.');
    assert.equal(args[0], 'api', 'Only the API read port is allowed.');
    if (args[1] === 'graphql') {
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
      assert.ok(++queries <= 1, 'Only one queue query per tick is allowed.');
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
    const timeBudget = () =>
      new Error(`Infra tick time budget exhausted (120 seconds; ${requests}/80 calls).`);
    if (requests >= 80)
      throw new Error('Infra tick API call budget exhausted (80 calls / 120 seconds).');
    if (clock() >= deadline) throw timeBudget();
    requests++;
    try {
      return execute(program, args, {
        ...options,
        // Hosted Rust logs exceed the packed command's 2 MiB default. Keep the
        // existing owner's hard 16 MiB ceiling, only for raw job-log captures.
        ...(args.includes('--allow-escape-sequences') ? { maxBuffer: 16 * 1024 * 1024 } : {}),
        timeoutMs: Math.min(options.timeoutMs, deadline - clock()),
      });
    } catch (error) {
      if (clock() >= deadline) throw timeBudget();
      throw error;
    }
  };
  const api = restClient({ cache, repo: repository, maxRequests: 80, execute: command });
  return {
    get: api.get,
    qualifyTerminalJobs: api.qualifyTerminalJobs,
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
    evidence: () => ({ ...api.evidence(), requests, queueQueries: queries }),
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

/** Injected evidence ports; a partial collection never becomes an empty healthy report. */
export function collectInfraTick(
  api,
  { repository, since, until, previous = null, reviewMap, local }
) {
  const end = time(until),
    start = time(since),
    day = end - 24 * 60 * 60_000;
  assert.ok(
    start < end && start >= day,
    'Tick interval must be within the stated 24-hour coverage.'
  );
  assert.match(repository, /^[\w.-]+\/[\w.-]+$/);
  validateReviewMap(reviewMap);
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
  const failedRunAttempts = [
    ...new Set([
      ...runs
        .filter((run) => FAILED.has(run.conclusion))
        .map((run) => `${run.id}:${run.run_attempt}`),
      ...failures.map((row) => `${row.run}:${row.attempt}`),
    ]),
  ];
  const priorFailures = new Set(previous?.failedRunAttempts ?? []);
  for (const run of runs.filter(
    (run) => FAILED.has(run.conclusion) && !priorFailures.has(`${run.id}:${run.run_attempt}`)
  )) {
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
    const workers = jobs.rows.filter((job) => FAILED.has(job.conclusion) && !aggregateOnly(job));
    const context =
      run.event === 'merge_group'
        ? `merge_group:${run.id}`
        : run.event === 'pull_request' && run.pull_requests?.length === 1
          ? `pr:${run.pull_requests[0].number}`
          : null;
    for (const job of workers) {
      assert.equal(typeof job.name, 'string', 'Missing worker name.');
      let tests = [],
        logHash = null,
        unavailable = null;
      try {
        const log = api.get(`${root}/actions/jobs/${job.id}/logs`, true, true);
        assert.equal(typeof log, 'string', 'Invalid job log.');
        tests = failingTests(log);
        logHash = hash(log);
      } catch (error) {
        if (error.message.includes('budget')) throw error;
        unavailable = 'test log unavailable';
        anomaly('test log unavailable', { run: run.id, job: job.id });
      }
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
        tests,
        logHash,
        unavailable,
        created: run.created_at,
      };
      failures.push(failure);
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
  const recentPulls = (state) => {
    const collected = [],
      seen = new Set();
    let lastUpdated = Infinity;
    for (let page = 1; ; page++) {
      const rows = api.get(
        `${root}/pulls?state=${state}&sort=updated&direction=desc&per_page=100&page=${page}`
      );
      assert.ok(Array.isArray(rows) && rows.length <= 100, 'Invalid recent PR page.');
      for (const pr of rows) {
        const updated = time(pr.updated_at);
        assert.ok(
          updated <= lastUpdated && !seen.has(identity(pr, 'PR')),
          'Changing recent PR listing.'
        );
        lastUpdated = updated;
        seen.add(pr.id);
        if (updated < start) return collected;
        if (
          state === 'open' ||
          (pr.merged_at && time(pr.merged_at) >= start && time(pr.merged_at) <= end)
        )
          collected.push(pr);
      }
      if (rows.length < 100) return collected;
    }
  };
  const open = recentPulls('open'),
    closed = recentPulls('closed');
  const reviewEvidence = [];
  for (const pr of [...open, ...closed]) {
    assert.ok(
      Number.isSafeInteger(pr.number) && pr.number > 0 && SHA.test(pr.head?.sha),
      'Incomplete listed PR identity.'
    );
    const files = pages(api, `${root}/pulls/${pr.number}/files`, undefined, undefined, (file) => {
      assert.ok(
        typeof file.filename === 'string' && file.filename.length > 0,
        'Missing changed file path.'
      );
      return file.filename;
    }).rows.map((file) => file.filename);
    const shared = files.filter(
      (file) => sharedPath(file) || reviewMap.rules.some((rule) => file.startsWith(rule.prefix))
    );
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
    const comments = shared.length ? pages(api, `${root}/issues/${pr.number}/comments`).rows : [];
    const verified = api.get(`${root}/pulls/${pr.number}`);
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
    coverage: COVERAGE,
    anomalies,
    health,
    baseline: previous
      ? 'incremental; only unseen failed attempts analysed'
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
      [
        'queue coverage unavailable',
        'test log unavailable',
        'public-install release evidence unavailable',
      ].includes(item.kind)
    ),
  };
}

export function renderInfraTick(report) {
  const rows = [
    `# Infra tick ${report.until}`,
    '',
    report.coverage,
    '',
    `Anomalies: ${report.anomalies.length}`,
  ];
  for (const item of report.anomalies.slice(0, 20)) {
    const { occurrences, ...summary } = item;
    const detail = occurrences ? { ...summary, contexts: occurrences.length } : summary;
    rows.push(`- ${item.kind}: ${JSON.stringify(detail)}`);
  }
  if (report.anomalies.length > 20)
    rows.push(
      `- ${report.anomalies.length - 20} more anomalies; full evidence is in summary.json.`
    );
  if (!report.anomalies.length) rows.push('No anomalies observed within the stated coverage.');
  rows.push(
    '',
    `${report.health.createdSinceTick} newly created runs (${report.health.created24h} in 24 h); ${report.olderActive.length} older active runs; ${report.trackedCompletions.length} tracked completions; ${report.failures.length} observed failed workers; ${report.failedDrafts.length} failed drafts; ${new Set(report.postPublicationFailures.map((failure) => failure.releaseId)).size} published releases with failed public checks.`,
    `Host: ${(report.host.freeBytes / 1024 ** 3).toFixed(1)} GiB free; 1-minute load ${report.host.load1}.`,
    `Evidence: ${report.evidence.requests ?? 'fixture'} API calls. Shared-path statements: ${report.reviewEvidence.length} PRs; acceptance requires lead judgement.`,
    ''
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
    },
  });
  assert.ok(
    values.output && values['review-map'],
    'Usage: infra-tick-summary.mjs --output DIR --review-map FILE [--since UTC] [--repo OWNER/REPO]'
  );
  mkdirSync(values.output, { recursive: true });
  const cursor = path.join(values.output, 'cursor.json');
  const until = new Date().toISOString();
  let since = values.since ?? new Date(Date.parse(until) - 60 * 60_000).toISOString();
  const api = createTickClient({
    repository: values.repo,
    cache: path.join(values.output, 'evidence'),
  });
  const secrets = [process.env.GH_TOKEN, process.env.GITHUB_TOKEN].filter(Boolean);
  const safe = (text) =>
    secrets.reduce((result, secret) => result.replaceAll(secret, '[REDACTED]'), text);
  try {
    const previous = existsSync(cursor) ? JSON.parse(readFileSync(cursor, 'utf8')) : null;
    since = values.since ?? previous?.until ?? since;
    const report = collectInfraTick(api, {
      repository: values.repo,
      since,
      until,
      previous,
      reviewMap: JSON.parse(readFileSync(values['review-map'], 'utf8')),
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
        }) + '\n',
        { mode: 0o600 }
      );
      renameSync(temporary, cursor);
    } else process.exitCode = 1;
  } catch (error) {
    const report = {
      schema: 1,
      complete: false,
      since,
      until,
      coverage: COVERAGE,
      anomalies: [{ kind: 'collection refused', message: safe(error.message) }],
      evidence: api.evidence(),
    };
    writeFileSync(
      path.join(values.output, 'summary.json'),
      JSON.stringify(report, null, 2) + '\n',
      { mode: 0o600 }
    );
    const markdown = `# Infra tick refused\n\n${COVERAGE}\n\n- collection refused: ${safe(error.message)}\n- API calls: ${api.evidence().requests}/80. Cursor unchanged. Qualified terminal job/log evidence is retained; retry the same output directory to continue a cold baseline.\n`;
    writeFileSync(path.join(values.output, 'summary.md'), markdown, { mode: 0o600 });
    process.stdout.write(markdown);
    process.exitCode = 1;
  }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
