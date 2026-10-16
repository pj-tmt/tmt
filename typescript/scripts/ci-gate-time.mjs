/**
 * Read-only REST measurement of the required CI gate, following the #3230 method.
 * The last N successful `ci.yml` runs per event (ordered by created_at, then run ID)
 * give the gate wall (run creation to the last selected required context), per-job
 * wall and queue, the controlling job and runner-minutes. Skipped jobs have no wall
 * or queue sample; they are never zeros. Runner-minutes are observed occupancy, not
 * billing. The controlling job is the last non-aggregate job to complete before the
 * gate ends, a completion-ordered approximation of #3230's `needs` trace. Budgets
 * live in ci-gate-budgets.json (see the CI reference); this tool compares and
 * reports, and never blocks a PR.
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { runPackedCommand } from './packed-command.mjs';

const DEFAULT_REPO = 'pj-tmt/tmt';
const EVENTS = ['pull_request', 'merge_group'];
const EVENT_LABELS = { pull_request: 'PR', merge_group: 'Merge group' };
const PAGE_SIZE = 100;
const MAX_RUNS = 100;
const MAX_JOB_PAGES = 10;
/** Branch protection's required contexts; each is an aggregate or a short gate job. */
export const REQUIRED_CONTEXTS = [
  'Code quality',
  'Unit tests',
  'Docker E2E',
  'Native package matrix',
];
/** Jobs that only wait on workers; the controlling job is never one of these. */
export const AGGREGATE_JOBS = ['Docker E2E', 'Native package matrix', 'Native Rust contracts'];
const OTHER_JOB_ROWS = 5;

const seconds = (start, end) => {
  const value = (Date.parse(end) - Date.parse(start)) / 1000;
  return Number.isFinite(value) && value >= 0 ? value : null;
};
const sorted = (values) => values.filter(Number.isFinite).sort((a, b) => a - b);
/** Ordinary median. */
export function median(values) {
  const list = sorted(values);
  if (!list.length) return null;
  const mid = Math.floor(list.length / 2);
  return list.length % 2 ? list[mid] : (list[mid - 1] + list[mid]) / 2;
}
/** Nearest-rank percentile, `ceil(p * n)`. */
export function percentile(values, p) {
  const list = sorted(values);
  return list.length ? list[Math.max(0, Math.ceil(p * list.length) - 1)] : null;
}
function describe(values) {
  const list = sorted(values);
  return {
    n: list.length,
    median: median(list),
    p90: percentile(list, 0.9),
    max: list.at(-1) ?? null,
  };
}
/** Rehearsal stages nest as `Release rehearsal (product) / stage`; count them as one branch. */
const branchLabel = (name) =>
  name.startsWith('Release rehearsal') ? 'Release rehearsal' : name.split(' / ')[0];
const byCompletion = (a, b) => a.completed_at.localeCompare(b.completed_at) || a.id - b.id;

/** Default REST reader: bounded `gh api` GETs against one repository. */
export function ghReader({ repo = DEFAULT_REPO, maxRequests = 400, execute = runPackedCommand }) {
  if (
    !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo) ||
    repo.split('/').some((part) => part === '.' || part === '..')
  )
    throw new Error('repo must be OWNER/REPO.');
  let requests = 0;
  const read = (path) => {
    if (requests >= maxRequests)
      throw new Error(
        `REST budget ${maxRequests} exhausted; lower --count or raise --max-requests.`
      );
    for (let attempt = 1; ; attempt += 1) {
      requests += 1;
      try {
        return JSON.parse(
          execute('gh', ['api', '--method', 'GET', `repos/${repo}/${path}`], {
            cwd: process.cwd(),
            env: process.env,
            timeoutMs: 60_000,
            maxBuffer: 8 * 1024 * 1024,
          })
        );
      } catch (error) {
        // Reads are idempotent; retry only GitHub's transient 5xx, within the request budget.
        if (attempt >= 3 || requests >= maxRequests || !/HTTP 5\d\d/.test(String(error.message)))
          throw error;
      }
    }
  };
  read.requests = () => requests;
  return read;
}

/** Collect the newest `count` successful runs per event with their latest-view jobs. */
export function collectGateRuns(read, { workflow = 'ci.yml', count = 40, events = EVENTS } = {}) {
  if (!Number.isSafeInteger(count) || count < 1 || count > MAX_RUNS)
    throw new Error(`count must be an integer from 1 to ${MAX_RUNS}.`);
  if (!/^[A-Za-z0-9_.-]+$/.test(workflow)) throw new Error('workflow must be a file name.');
  const runs = [];
  for (const event of events) {
    const page = read(
      `actions/workflows/${workflow}/runs?event=${event}&status=success&per_page=${PAGE_SIZE}&page=1`
    );
    if (!Array.isArray(page?.workflow_runs)) throw new Error(`Missing workflow_runs for ${event}.`);
    const chosen = page.workflow_runs
      .filter((run) => run.conclusion === 'success')
      .sort((a, b) => a.created_at.localeCompare(b.created_at) || a.id - b.id)
      .slice(-count);
    if (chosen.length < count)
      throw new Error(`Only ${chosen.length} successful ${event} runs; need ${count}.`);
    for (const run of chosen) runs.push({ ...runFields(run, event), jobs: readJobs(read, run.id) });
  }
  return runs;
}
const runFields = (run, event) => ({
  id: run.id,
  event,
  createdAt: run.created_at,
  attempt: run.run_attempt ?? 1,
});
function readJobs(read, runId) {
  const jobs = [];
  let total = Infinity;
  for (let page = 1; jobs.length < total; page += 1) {
    if (page > MAX_JOB_PAGES) throw new Error(`Job pagination bound exceeded for run ${runId}.`);
    const data = read(
      `actions/runs/${runId}/jobs?filter=latest&per_page=${PAGE_SIZE}&page=${page}`
    );
    if (!Array.isArray(data?.jobs) || !Number.isSafeInteger(data.total_count))
      throw new Error(`Malformed jobs page for run ${runId}.`);
    total = data.total_count;
    if (!data.jobs.length && jobs.length < total)
      throw new Error(`Incomplete jobs for run ${runId}.`);
    jobs.push(...data.jobs);
  }
  if (jobs.some((job) => job.status !== 'completed' || !job.completed_at))
    throw new Error(`Run ${runId} has a job that is not complete.`);
  return jobs.map(({ id, name, conclusion, created_at, started_at, completed_at }) => ({
    id,
    name,
    conclusion,
    created_at,
    started_at,
    completed_at,
  }));
}

/** Measure one run; `null` when no required context actually ran. */
export function measureRun(run, required = REQUIRED_CONTEXTS) {
  const executed = run.jobs.filter((job) => job.conclusion !== 'skipped' && job.started_at);
  const gate = executed
    .filter((job) => required.includes(job.name))
    .sort(byCompletion)
    .at(-1);
  if (!gate) return null;
  const controlling = executed
    .filter((job) => !AGGREGATE_JOBS.includes(job.name) && job.completed_at <= gate.completed_at)
    .sort(byCompletion)
    .at(-1);
  return {
    id: run.id,
    event: run.event,
    attempt: run.attempt,
    gateSeconds: seconds(run.createdAt, gate.completed_at),
    controlling: branchLabel(controlling?.name ?? gate.name),
    runnerMinutes: executed.reduce(
      (sum, job) => sum + seconds(job.started_at, job.completed_at) / 60,
      0
    ),
    jobs: executed.map((job) => ({
      name: job.name,
      wall: seconds(job.started_at, job.completed_at),
      queue: seconds(job.created_at, job.started_at),
    })),
  };
}

/** Per-event gate, job and controlling-job statistics. */
export function summarizeGate(runs, required = REQUIRED_CONTEXTS) {
  return Object.fromEntries(
    EVENTS.map((event) => {
      const own = runs.filter((run) => run.event === event);
      const measured = own.map((run) => measureRun(run, required)).filter(Boolean);
      const jobNames = [...new Set(measured.flatMap((run) => run.jobs.map((job) => job.name)))];
      const counts = new Map();
      for (const run of measured)
        counts.set(run.controlling, (counts.get(run.controlling) ?? 0) + 1);
      return [
        event,
        {
          runs: own.length,
          measured: measured.length,
          retried: measured.filter((run) => run.attempt > 1).length,
          firstCreatedAt: own.map((run) => run.createdAt).sort()[0] ?? null,
          lastCreatedAt:
            own
              .map((run) => run.createdAt)
              .sort()
              .at(-1) ?? null,
          gate: describe(measured.map((run) => run.gateSeconds)),
          runnerMinutes: {
            median: median(measured.map((run) => run.runnerMinutes)),
            p90: percentile(
              measured.map((run) => run.runnerMinutes),
              0.9
            ),
          },
          jobs: jobNames
            .map((name) => {
              const samples = measured.flatMap((run) =>
                run.jobs.filter((job) => job.name === name)
              );
              return {
                name,
                wall: describe(samples.map((job) => job.wall)),
                queue: describe(samples.map((job) => job.queue)),
              };
            })
            .sort((a, b) => b.wall.p90 - a.wall.p90 || a.name.localeCompare(b.name)),
          controlling: [...counts]
            .map(([name, count]) => ({ name, count, share: count / measured.length }))
            .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name)),
        },
      ];
    })
  );
}

/** Validate the budget file, the single owner of every limit. */
export function parseBudgets(text) {
  const data = JSON.parse(text);
  const limit = (value, what) => {
    if (!Number.isFinite(value) || value <= 0)
      throw new Error(`Invalid budget for ${what}: ${value}`);
    return value;
  };
  const budgets = { gate: {}, jobs: {} };
  for (const event of EVENTS) {
    const gate = data.gate?.[event];
    if (!gate) throw new Error(`Missing gate budget for ${EVENT_LABELS[event]}.`);
    budgets.gate[event] = {
      median: limit(gate.median, `${EVENT_LABELS[event]} gate median`),
      p90: limit(gate.p90, `${EVENT_LABELS[event]} gate p90`),
    };
  }
  const jobs = Object.entries(data.jobs ?? {});
  if (!jobs.length) throw new Error('Missing per-job budgets.');
  for (const [name, row] of jobs)
    budgets.jobs[name] = Object.fromEntries(
      EVENTS.map((event) => [event, limit(row?.[event], `${name} ${EVENT_LABELS[event]}`)])
    );
  return budgets;
}

/** Budget lines over their limit; empty means within budget. */
export function overBudget(summary, budgets) {
  const over = [];
  for (const event of EVENTS) {
    const { gate, jobs } = summary[event];
    const limit = budgets.gate[event];
    if (gate.median > limit.median)
      over.push({ event, what: 'gate median', value: gate.median, budget: limit.median });
    if (gate.p90 > limit.p90)
      over.push({ event, what: 'gate p90', value: gate.p90, budget: limit.p90 });
    for (const [name, budget] of Object.entries(budgets.jobs)) {
      const job = jobs.find((row) => row.name === name);
      if (job && job.wall.p90 > budget[event])
        over.push({ event, what: `${name} p90`, value: job.wall.p90, budget: budget[event] });
    }
  }
  return over;
}

/** Budgeted jobs with no wall sample in the window (renamed, removed or always skipped). */
export function unmeasuredBudgets(summary, budgets) {
  return EVENTS.flatMap((event) =>
    Object.keys(budgets.jobs)
      .filter((name) => !summary[event].jobs.some((row) => row.name === name && row.wall.n > 0))
      .map((job) => ({ event, job }))
  );
}

const fixed = (value, digits = 1) =>
  value === null ? 'n/a' : Number(value.toFixed(digits)).toString();
/** Markdown report for the weekly hygiene pass. */
export function renderReport(summary, budgets, { workflow = 'ci.yml' } = {}) {
  const lines = [
    '# Required-gate time',
    '',
    `Workflow ${workflow}. Gate wall = run creation to the last selected required context (${REQUIRED_CONTEXTS.join(', ')}). Job wall = started to completed; queue = created to started. Skipped jobs have no sample. Median is ordinary; p90 is nearest rank. Seconds unless noted.`,
    '',
    '| Event | Runs (retried) | Created window (UTC) | Gate median / p90 / max | Budget median / p90 | Runner-min median / p90 |',
    '| --- | ---: | --- | ---: | ---: | ---: |',
  ];
  for (const event of EVENTS) {
    const row = summary[event];
    lines.push(
      `| ${EVENT_LABELS[event]} | ${row.measured} (${row.retried}) | ${row.firstCreatedAt} – ${row.lastCreatedAt} | ${fixed(row.gate.median)} / ${fixed(row.gate.p90)} / ${fixed(row.gate.max)} | ${budgets.gate[event].median} / ${budgets.gate[event].p90} | ${fixed(row.runnerMinutes.median, 2)} / ${fixed(row.runnerMinutes.p90, 2)} |`
    );
  }
  for (const event of EVENTS) {
    const row = summary[event];
    lines.push(
      '',
      `## ${EVENT_LABELS[event]} jobs`,
      '',
      '| Job | n | Wall median / p90 | Queue median / p90 | p90 budget |',
      '| --- | ---: | ---: | ---: | ---: |'
    );
    const shown = row.jobs.filter((job) => !job.name.includes(' / '));
    const budgeted = shown.filter((job) => budgets.jobs[job.name]);
    const others = shown.filter((job) => !budgets.jobs[job.name]).slice(0, OTHER_JOB_ROWS);
    for (const job of [...budgeted, ...others])
      lines.push(
        `| ${job.name} | ${job.wall.n} | ${fixed(job.wall.median)} / ${fixed(job.wall.p90)} | ${fixed(job.queue.median)} / ${fixed(job.queue.p90)} | ${budgets.jobs[job.name]?.[event] ?? '–'} |`
      );
    lines.push(
      '',
      `Budgeted jobs plus the ${OTHER_JOB_ROWS} slowest others; nested rehearsal stages and the full list are in --json.`,
      '',
      `Controlling job (${EVENT_LABELS[event]}):`
    );
    for (const job of row.controlling)
      lines.push(`- ${job.name}: ${job.count}/${row.measured} (${fixed(job.share * 100)}%)`);
  }
  const over = overBudget(summary, budgets);
  lines.push('', '## Over budget', '');
  if (over.length)
    for (const item of over)
      lines.push(
        `- ${EVENT_LABELS[item.event]} ${item.what}: ${fixed(item.value)} s > ${item.budget} s`
      );
  else lines.push('None.');
  const unmeasured = unmeasuredBudgets(summary, budgets);
  lines.push(
    '',
    '## Budgeted jobs without samples',
    '',
    'A budget that matches no sample is not being checked; rename or remove it in ci-gate-budgets.json.',
    ''
  );
  if (unmeasured.length)
    for (const item of unmeasured) lines.push(`- ${EVENT_LABELS[item.event]} ${item.job}`);
  else lines.push('None.');
  return `${lines.join('\n')}\n`;
}

const DEFAULT_BUDGETS = fileURLToPath(new URL('./ci-gate-budgets.json', import.meta.url));
function main(argv) {
  const { values } = parseArgs({
    args: argv,
    options: {
      repo: { type: 'string', default: DEFAULT_REPO },
      workflow: { type: 'string', default: 'ci.yml' },
      count: { type: 'string', default: '40' },
      'max-requests': { type: 'string', default: '400' },
      budgets: { type: 'string', default: DEFAULT_BUDGETS },
      output: { type: 'string' },
      json: { type: 'string' },
    },
  });
  const budgets = parseBudgets(readFileSync(values.budgets, 'utf8'));
  const read = ghReader({ repo: values.repo, maxRequests: Number(values['max-requests']) });
  const runs = collectGateRuns(read, { workflow: values.workflow, count: Number(values.count) });
  const summary = summarizeGate(runs);
  const report = renderReport(summary, budgets, { workflow: values.workflow });
  if (values.json)
    writeFileSync(
      values.json,
      `${JSON.stringify(
        {
          summary,
          over: overBudget(summary, budgets),
          unmeasured: unmeasuredBudgets(summary, budgets),
          requests: read.requests(),
        },
        null,
        2
      )}\n`
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
