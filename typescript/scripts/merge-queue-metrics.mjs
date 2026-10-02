/**
 * Read-only REST evidence, not a queue controller. Half-open run-created cohorts
 * include later reruns; merges use merge timestamps. Group duration excludes
 * unfinished attempts. Runner minutes are measured, not billed or price weighted.
 * Queue-tip coverage is not cumulative membership. Missing timestamps, enqueue
 * events and pending squash incarnations stay unknown; cancellations do not prove
 * invalidation causes. Sole worker failures exclude propagated gate failures.
 * Exact-tree fail/pass pairs are candidates requiring independent log review.
 * Mutable lists refresh; completed attempts/commit comparisons reuse local evidence.
 */
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { runPackedCommand } from './packed-command.mjs';

const DEFAULT_REPO = 'pj-tmt/tmt';
function repositoryRoot(repo) {
  if (
    !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo) ||
    repo.split('/').some((part) => part === '.' || part === '..')
  )
    throw new Error('repo must be OWNER/REPO.');
  return `repos/${repo}`;
}
// Keep verbose PR/timeline responses below the shared bounded command buffer.
const PAGE_SIZE = 30;
const EVENTS = ['pull_request', 'merge_group', 'push'];
const FAILURE = new Set(['failure', 'timed_out', 'startup_failure', 'action_required']);
const AGGREGATES = new Set(['Docker E2E', 'Native Rust contracts', 'Native package matrix']);

function instant(value) {
  const parsed = typeof value === 'string' && value.endsWith('Z') ? Date.parse(value) : NaN;
  if (!Number.isFinite(parsed)) throw new Error(`Expected a UTC timestamp ending in Z: ${value}`);
  return parsed;
}
function minutes(start, end) {
  if (!start || !end) return null;
  const duration = (Date.parse(end) - Date.parse(start)) / 60_000;
  return Number.isFinite(duration) && duration >= 0 ? duration : null;
}
function list(value, label) {
  if (!Array.isArray(value)) throw new Error(`Missing REST array: ${label}`);
  return value;
}
function tip(run) {
  return Number(/\/pr-(\d+)-[a-f0-9]+$/.exec(run.head_branch ?? '')?.[1]) || null;
}
function propagated(job) {
  const failed = (job.steps ?? []).filter((step) => FAILURE.has(step.conclusion));
  return (
    AGGREGATES.has(job.name) ||
    (failed.length > 0 && failed.every((step) => step.name.startsWith('Require ')))
  );
}
function runner(job) {
  const labels = job.labels ?? [];
  if (labels.includes('self-hosted')) return `self-hosted: ${labels.join(',')}`;
  return labels.join(',') || 'unknown';
}
function describe(values) {
  const sorted = values.filter((value) => Number.isFinite(value)).sort((a, b) => a - b);
  if (!sorted.length) return { n: 0, median: null, min: null, max: null, total: 0 };
  const mid = Math.floor(sorted.length / 2);
  return {
    n: sorted.length,
    median: sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2,
    min: sorted[0],
    max: sorted.at(-1),
    total: sorted.reduce((sum, value) => sum + value, 0),
  };
}

/** Read-only, bounded REST requests. Terminal evidence is reused; mutable lists refresh. */
export function restClient({
  cache,
  repo = DEFAULT_REPO,
  offline = false,
  maxRequests = 500,
  execute = runPackedCommand,
}) {
  const ROOT = repositoryRoot(repo);
  mkdirSync(cache, { recursive: true });
  let requests = 0;
  const touched = [];
  return {
    get(path, immutable = false) {
      if (!path.startsWith(`${ROOT}/`)) throw new Error(`Only ${ROOT} REST paths are allowed.`);
      const file = resolve(cache, `${createHash('sha256').update(path).digest('hex')}.json`);
      if ((offline || immutable) && existsSync(file)) {
        const stored = JSON.parse(readFileSync(file, 'utf8'));
        if (stored.path !== path) throw new Error(`Cache path mismatch: ${file}`);
        touched.push(file);
        return stored.data;
      }
      if (offline) throw new Error(`Offline evidence missing: ${path}`);
      if (requests >= maxRequests)
        throw new Error(
          `REST budget ${maxRequests} exhausted; narrow the window or reuse cached evidence.`
        );
      requests += 1;
      const data = JSON.parse(
        execute('gh', ['api', '--method', 'GET', path], {
          cwd: process.cwd(),
          env: process.env,
          timeoutMs: 60_000,
        })
      );
      writeFileSync(
        file,
        `${JSON.stringify({ path, fetchedAt: new Date().toISOString(), data })}\n`,
        { mode: 0o600 }
      );
      touched.push(file);
      return data;
    },
    evidence: () => ({ requests, files: [...new Set(touched)] }),
  };
}

function pages(api, path, key, immutable = false) {
  const rows = [];
  for (let page = 1; page <= 100; page += 1) {
    const data = api.get(
      `${path}${path.includes('?') ? '&' : '?'}per_page=${PAGE_SIZE}&page=${page}`,
      immutable
    );
    const batch = list(key ? data[key] : data, path);
    rows.push(...batch);
    if (key && data.total_count > 1000 && path.includes('/runs?')) {
      throw new Error('GitHub run search is capped at 1000; split the window.');
    }
    if (batch.length < PAGE_SIZE) {
      if (key && Number.isSafeInteger(data.total_count) && rows.length < data.total_count) {
        throw new Error(
          `Incomplete REST page: ${path}; received ${rows.length} of ${data.total_count}.`
        );
      }
      return rows;
    }
  }
  throw new Error(`Pagination bound exceeded: ${path}`);
}

/** Collect one snapshot. Every rerun attempt has its own jobs and completion evidence. */
export function collectMetrics(
  api,
  { since, until, repo = DEFAULT_REPO, workflow = 'ci.yml', tagPrs = [961, 963] }
) {
  const ROOT = repositoryRoot(repo);
  const start = instant(since),
    end = instant(until);
  if (end <= start) throw new Error('until must be later than since.');
  if (!/^[A-Za-z0-9_.-]+$/.test(workflow)) throw new Error('workflow must be a file name.');
  if (tagPrs.some((number) => !Number.isSafeInteger(number) || number < 1))
    throw new Error('Invalid tag PR.');
  const within = (time) => time && Date.parse(time) >= start && Date.parse(time) < end;
  const runs = EVENTS.flatMap((event) =>
    pages(
      api,
      `${ROOT}/actions/workflows/${workflow}/runs?event=${event}&created=${encodeURIComponent(`${since}..${until}`)}`,
      'workflow_runs'
    )
  ).filter((run) => within(run.created_at));
  const pullRequests = new Map();
  // Pulls are sorted by update time; a merge in the window necessarily updated in it.
  for (let page = 1; page <= 100; page += 1) {
    const batch = list(
      api.get(
        `${ROOT}/pulls?state=closed&sort=updated&direction=desc&per_page=${PAGE_SIZE}&page=${page}`
      ),
      'pulls'
    );
    for (const pr of batch) if (within(pr.merged_at)) pullRequests.set(pr.number, pr);
    if (batch.length < PAGE_SIZE || batch.every((pr) => Date.parse(pr.updated_at) < start)) break;
    if (page === 100) throw new Error('Pull-request pagination bound exceeded.');
  }
  for (const number of new Set([...tagPrs, ...runs.map(tip).filter(Boolean)])) {
    if (!pullRequests.has(number)) pullRequests.set(number, api.get(`${ROOT}/pulls/${number}`));
  }
  const timelines = [...pullRequests.values()].map((pr) => ({
    number: pr.number,
    mergedAt: pr.merged_at,
    events: pages(api, `${ROOT}/issues/${pr.number}/timeline`).filter((event) =>
      ['added_to_merge_queue', 'removed_from_merge_queue', 'merged'].includes(event.event)
    ),
  }));
  const tags = tagPrs.map((number) => ({
    number,
    mergedAt: pullRequests.get(number)?.merged_at,
    mergeSha: pullRequests.get(number)?.merged_at
      ? pullRequests.get(number).merge_commit_sha
      : null,
  }));
  const groups = runs.filter((run) => run.event === 'merge_group');
  // Queue heads contain squash commits, not necessarily the original PR head SHA.
  // Tag by known merged commit ancestry and by observed queue-head ancestry for pending PRs.
  const anchors = tags.map((tag) => ({
    ...tag,
    shas: [
      ...new Set(
        [
          tag.mergeSha,
          ...groups.filter((run) => tip(run) === tag.number).map((run) => run.head_sha),
        ].filter(Boolean)
      ),
    ],
  }));
  const comparisons = new Map();
  const attempts = [];
  for (const run of runs) {
    const inclusion = {};
    for (const tag of anchors) {
      let included = false;
      for (const sha of tag.shas) {
        if (sha === run.head_sha) {
          included = true;
          break;
        }
        const key = `${sha}...${run.head_sha}`;
        if (!comparisons.has(key))
          comparisons.set(key, api.get(`${ROOT}/compare/${key}?per_page=1`, true).status);
        if (['identical', 'ahead'].includes(comparisons.get(key))) {
          included = true;
          break;
        }
      }
      // Pending PRs may have an unobserved queue-head incarnation outside this window.
      inclusion[tag.number] = included ? 'included' : tag.mergeSha ? 'absent' : 'unknown';
    }
    for (let attempt = 1; attempt <= run.run_attempt; attempt += 1) {
      const detail =
        run.run_attempt === 1
          ? run
          : api.get(
              `${ROOT}/actions/runs/${run.id}/attempts/${attempt}`,
              attempt < run.run_attempt || run.status === 'completed'
            );
      const jobs = pages(
        api,
        `${ROOT}/actions/runs/${run.id}/attempts/${attempt}/jobs`,
        'jobs',
        detail.status === 'completed'
      );
      const ends = jobs
        .filter((job) => job.completed_at)
        .map((job) => job.completed_at)
        .sort();
      attempts.push({
        id: run.id,
        attempt,
        event: run.event,
        createdAt: run.created_at,
        startedAt: detail.run_started_at,
        sha: run.head_sha,
        tree: run.head_commit?.tree_id ?? null,
        tipPr: tip(run),
        conclusion: detail.conclusion,
        status: detail.status,
        inclusion,
        jobs,
        duration:
          detail.status === 'completed' && ends.length
            ? minutes(detail.run_started_at, ends.at(-1))
            : null,
        url: run.html_url,
      });
    }
  }
  return {
    since,
    until,
    workflow,
    repo,
    capturedAt: new Date().toISOString(),
    tags,
    runs: attempts,
    timelines,
    evidence: api.evidence?.() ?? { requests: null, files: [] },
  };
}

/** Summaries use run-created cohorts; merge throughput uses merge timestamps. */
export function summarizeMetrics(snapshot, since = snapshot.since, until = snapshot.until) {
  const start = instant(since),
    end = instant(until);
  if (end <= start) throw new Error('until must be later than since.');
  const within = (time) => time && Date.parse(time) >= start && Date.parse(time) < end;
  const runs = snapshot.runs.filter((run) => within(run.createdAt));
  const groups = runs.filter((run) => run.event === 'merge_group');
  const merged = snapshot.timelines.filter(
    (pr) =>
      within(pr.mergedAt) &&
      pr.events.some(
        (event) =>
          event.event === 'added_to_merge_queue' &&
          Date.parse(event.created_at) <= Date.parse(pr.mergedAt)
      )
  );
  const latency = merged.map((pr) => {
    let enqueue = null;
    for (const event of [...pr.events].sort((a, b) => a.created_at.localeCompare(b.created_at))) {
      if (Date.parse(event.created_at) > Date.parse(pr.mergedAt)) continue;
      if (event.event === 'added_to_merge_queue') enqueue = event.created_at;
      // Merge queue itself emits removal immediately before/at merge. The
      // final recorded enqueue is measurable; uninterrupted residency is not.
    }
    return { pr: pr.number, minutes: minutes(enqueue, pr.mergedAt) };
  });
  const rows = new Map();
  const flakes = [];
  const history = new Map();
  for (const run of [...snapshot.runs].sort(
    (a, b) => a.createdAt.localeCompare(b.createdAt) || a.attempt - b.attempt
  )) {
    const failures = run.jobs.filter((job) => FAILURE.has(job.conclusion) && !propagated(job));
    for (const job of run.jobs) {
      // Match workflow, event, job and exact git tree; same SHA without a tree is not claimed.
      const treeKey = `${run.event}\0${job.name}\0${runner(job)}\0${run.tree}`;
      if (
        run.tree &&
        job.conclusion === 'success' &&
        history.has(treeKey) &&
        within(run.createdAt)
      ) {
        const failed = history.get(treeKey);
        if (Date.parse(job.started_at) >= Date.parse(failed.job.completed_at)) {
          flakes.push({
            job: job.name,
            tree: run.tree,
            failed: failed.job.html_url,
            passed: job.html_url,
          });
          history.delete(treeKey);
        }
      }
      if (run.tree && FAILURE.has(job.conclusion) && !propagated(job))
        history.set(treeKey, { run, job });
      if (!within(run.createdAt)) continue;
      const key = `${run.event}\0${job.name}\0${runner(job)}`;
      if (!rows.has(key))
        rows.set(key, {
          event: run.event,
          job: job.name,
          runner: runner(job),
          invocations: 0,
          executed: 0,
          skipped: 0,
          failures: 0,
          soleFailures: 0,
          durations: [],
          waits: [],
          steps: new Map(),
        });
      const row = rows.get(key);
      row.invocations += 1;
      if (job.conclusion === 'skipped') {
        row.skipped += 1;
        continue;
      }
      if (job.runner_id && job.started_at) row.executed += 1;
      if (FAILURE.has(job.conclusion)) row.failures += 1;
      if (run.status === 'completed' && failures.length === 1 && failures[0].id === job.id)
        row.soleFailures += 1;
      row.durations.push(minutes(job.started_at, job.completed_at));
      row.waits.push(minutes(job.created_at, job.started_at));
      for (const step of job.steps ?? []) {
        if (!row.steps.has(step.name)) row.steps.set(step.name, []);
        row.steps.get(step.name).push(minutes(step.started_at, step.completed_at));
      }
    }
  }
  const removals = snapshot.timelines.flatMap((pr) =>
    pr.events
      .filter((event) => event.event === 'removed_from_merge_queue' && within(event.created_at))
      .map((event) => ({
        pr: pr.number,
        time: event.created_at,
        reason: event.reason ?? 'unknown (REST timeline gives no reason)',
      }))
  );
  const strata = snapshot.tags.map((tag) => ({
    pr: tag.number,
    mergedAt: tag.mergedAt,
    counts: Object.fromEntries(
      ['included', 'absent', 'unknown'].map((state) => [
        state,
        groups.filter((run) => run.inclusion[tag.number] === state).length,
      ])
    ),
    durations: Object.fromEntries(
      ['included', 'absent', 'unknown'].map((state) => [
        state,
        describe(
          groups.filter((run) => run.inclusion[tag.number] === state).map((run) => run.duration)
        ),
      ])
    ),
  }));
  return {
    since,
    until,
    hours: (end - start) / 3_600_000,
    groupRuns: new Set(groups.map((run) => run.id)).size,
    groupAttempts: groups.length,
    tipPrs: new Set(groups.map((run) => run.tipPr).filter(Boolean)).size,
    mergedPrs: merged.length,
    mergesPerHour: merged.length / ((end - start) / 3_600_000),
    runsPerMerge: merged.length ? new Set(groups.map((run) => run.id)).size / merged.length : null,
    groupDuration: describe(groups.map((run) => run.duration)),
    groupRunnerWait: describe(
      groups.flatMap((run) =>
        run.jobs
          .filter((job) => job.conclusion !== 'skipped')
          .map((job) => minutes(job.created_at, job.started_at))
      )
    ),
    inFlight: groups.filter((run) => run.status !== 'completed').length,
    failedGroups: groups.filter((run) => FAILURE.has(run.conclusion)).length,
    cancelledGroups: groups.filter((run) => run.conclusion === 'cancelled').length,
    latency: { ...describe(latency.map((value) => value.minutes)), prs: latency },
    removals,
    flakes,
    strata,
    jobs: [...rows.values()].map(({ durations, waits, steps, ...row }) => ({
      ...row,
      duration: describe(durations),
      wait: describe(waits),
      steps: [...steps].map(([name, values]) => ({ name, ...describe(values) })),
      eventAttempts: runs.filter((run) => run.event === row.event).length,
    })),
  };
}

const number = (value) => (value === null ? 'n/a' : value.toFixed(1));
const cell = (value) =>
  String(value)
    .replaceAll('|', '\\|')
    .replace(/[\r\n]/g, ' ');
function distribution(stats) {
  return `${number(stats.median)} (${number(stats.min)}–${number(stats.max)}), n=${stats.n}`;
}
export function renderMetrics(snapshot, boundary, { details = false } = {}) {
  if (
    boundary &&
    (instant(boundary) <= instant(snapshot.since) || instant(boundary) >= instant(snapshot.until))
  ) {
    throw new Error('boundary must lie strictly inside the snapshot window.');
  }
  const windows = boundary
    ? [
        ['Before', snapshot.since, boundary],
        ['After', boundary, snapshot.until],
      ]
    : [['Window', snapshot.since, snapshot.until]];
  const lines = [
    '# Merge queue health',
    '',
    `UTC snapshot: ${snapshot.since} ≤ run created_at / merge time < ${snapshot.until}. Repository: ${snapshot.repo ?? DEFAULT_REPO}. Workflow: ${snapshot.workflow}.`,
    '',
    'Queue trial: 2026-10-02T06:08:40Z (wait 5 minutes, build concurrency 3). Confounders: #961 merged 05:21:13Z; #963 changes cumulative scope. Compare strata, not queue settings alone.',
    '',
  ];
  for (const [name, since, until] of windows) {
    const summary = summarizeMetrics(snapshot, since, until);
    lines.push(
      `## ${name}: ${since} → ${until}`,
      '',
      `${summary.groupRuns} merge_group runs (${summary.groupAttempts} attempts) for ${summary.tipPrs} distinct queue-tip PRs; ${summary.mergedPrs} queued PRs merged; ${number(summary.runsPerMerge)} runs/merge; ${number(summary.mergesPerHour)} merges/hour.`,
      `Group CI minutes median (range): ${distribution(summary.groupDuration)}; ${summary.inFlight} attempts in flight, ${summary.failedGroups} failed, ${summary.cancelledGroups} cancelled.`,
      `Group job created-to-start delay minutes median (range): ${distribution(summary.groupRunnerWait)}.`,
      `Last recorded enqueue-to-merge minutes: ${distribution(summary.latency)} (${summary.latency.n}/${summary.mergedPrs} merged PRs derivable).`,
      '',
      '| Tree includes | Included / absent / unknown attempts | Included CI min | Absent CI min |',
      '| --- | --- | --- | --- |'
    );
    for (const tag of summary.strata)
      lines.push(
        `| #${tag.pr} | ${tag.counts.included} / ${tag.counts.absent} / ${tag.counts.unknown} | ${distribution(tag.durations.included)} | ${distribution(tag.durations.absent)} |`
      );
    lines.push(
      '',
      '| Event | Job | Runner labels | Executed / event attempts | Skipped | CI min median (range), n | Total runner min | Created-to-start min, n | Failures / sole worker failures |',
      '| --- | --- | --- | --- | --- | --- | --- | --- | --- |'
    );
    for (const row of details
      ? summary.jobs
      : [...summary.jobs].sort((a, b) => b.duration.total - a.duration.total).slice(0, 5))
      lines.push(
        `| ${row.event} | ${cell(row.job)} | ${cell(row.runner)} | ${row.executed} / ${row.eventAttempts} | ${row.skipped} | ${distribution(row.duration)} | ${number(row.duration.total)} | ${distribution(row.wait)} | ${row.failures} / ${row.soleFailures} |`
      );
    if (!details)
      lines.push(
        '',
        'Five largest job/event/runner cost rows shown; complete breakdown and step timings are in JSON or --details.'
      );
    lines.push('', `Queue removals: ${summary.removals.length}.`);
    for (const removal of details ? summary.removals : [])
      lines.push(`- #${removal.pr} at ${removal.time}: ${cell(removal.reason)}.`);
    lines.push(
      '',
      `Same-tree fail→pass candidates: ${summary.flakes.length} (candidate flakiness, not a diagnosed test).`
    );
    for (const flake of details ? summary.flakes : summary.flakes.slice(0, 3))
      lines.push(
        `- ${cell(flake.job)}: [failed](${flake.failed}) → [passed](${flake.passed}); tree ${flake.tree}.`
      );
    lines.push('');
  }
  lines.push(
    'Method: run IDs and attempts are distinct; run cohorts use initial created_at, including later reruns. Group duration is run_started_at to last completed job; in-flight attempts are excluded. Created-to-start delay includes dependency scheduling and runner wait; REST does not isolate pure runner queue time. Missing timestamps are excluded, never zero-filled. Skipped jobs have no runner cost. Minutes are observed execution time, not billed minutes or price multipliers. Runner labels distinguish macOS/Linux/self-hosted.',
    '',
    'Sole worker failures exclude aggregate-only checks and jobs whose only failed steps start with “Require ”. These counts require a completed attempt; totals retain all failed jobs. Costs/steps are available in the JSON snapshot/summary. Distinct PR count is queue-tip coverage, not full cumulative membership. Merged PRs require a queue-enqueue timeline event. Latency uses the last recorded enqueue; REST removal at merge does not establish interrupted residency. Missing enqueue events remain unknown.',
    '',
    'Invalidation cause is not inferred from cancellation: REST removal reasons are reported when exposed; failure/conflict/dequeue-ahead attribution otherwise remains unknown. Pending PR inclusion is proven against observed queue-head ancestry; a missing anchor/incarnation remains unknown. A merged PR uses its merge commit ancestry. Every run carries inclusion tags and job/step evidence in JSON. Reports are generated locally; posting is a separate requested action.',
    '',
    `REST requests this collection: ${snapshot.evidence.requests ?? 'fixture'}; evidence files: ${snapshot.evidence.files.length}.`
  );
  return `${lines.join('\n')}\n`;
}

function main(argv) {
  const { values } = parseArgs({
    args: argv,
    options: {
      repo: { type: 'string', default: DEFAULT_REPO },
      since: { type: 'string' },
      until: { type: 'string' },
      boundary: { type: 'string' },
      cache: { type: 'string' },
      output: { type: 'string' },
      json: { type: 'string' },
      offline: { type: 'boolean', default: false },
      details: { type: 'boolean', default: false },
      workflow: { type: 'string', default: 'ci.yml' },
      'max-requests': { type: 'string', default: '500' },
      'tag-pr': { type: 'string', multiple: true },
    },
  });
  if (!values.since || !values.until || !values.cache)
    throw new Error(
      'Required: --since UTC --until UTC --cache DIR; optional --boundary UTC --output FILE --json FILE --offline --tag-pr N.'
    );
  const start = instant(values.since),
    end = instant(values.until);
  if (end <= start) throw new Error('until must be later than since.');
  if (values.boundary && (instant(values.boundary) <= start || instant(values.boundary) >= end)) {
    throw new Error('boundary must lie strictly inside the snapshot window.');
  }
  const maxRequests = Number(values['max-requests']);
  if (!Number.isSafeInteger(maxRequests) || maxRequests < 1)
    throw new Error('max-requests must be positive.');
  const api = restClient({
    cache: resolve(values.cache),
    repo: values.repo,
    offline: values.offline,
    maxRequests,
  });
  const snapshot = collectMetrics(api, {
    repo: values.repo,
    since: values.since,
    until: values.until,
    workflow: values.workflow,
    tagPrs: values['tag-pr']?.map(Number) ?? [961, 963],
  });
  const report = renderMetrics(snapshot, values.boundary, { details: values.details });
  if (values.json)
    writeFileSync(
      values.json,
      `${JSON.stringify({ snapshot, summary: summarizeMetrics(snapshot) }, null, 2)}\n`
    );
  if (values.output) writeFileSync(values.output, report);
  else process.stdout.write(report);
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
