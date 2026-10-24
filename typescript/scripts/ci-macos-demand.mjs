/** Read-only, bounded macOS demand census. No sampling or cross-invocation state. */
import assert from 'node:assert/strict';
import { isDeepStrictEqual, parseArgs } from 'node:util';
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { median, percentile } from './ci-gate-time.mjs';
import { runPackedCommand } from './packed-command.mjs';

const iso = (ms) => new Date(ms).toISOString();
const time = (value) => (typeof value === 'string' ? Date.parse(value) : NaN);
const id = (value) => Number.isSafeInteger(value) && value > 0;
const macosLabel = (label) => /^macos-(?:latest|\d+(?:-intel|-large|-xlarge)?)$/i.test(label);
const macos = (labels) => !labels.includes('self-hosted') && labels.some(macosLabel);

/** Conservative source trace, not a YAML expression evaluator. Unknown syntax fails closed. */
export function traceWorkflows(sources) {
  const byPath = new Map(sources.map((source) => [source.path, source.text]));
  assert.equal(byPath.size, sources.length, 'duplicate workflow source');
  const visit = (path, stack = []) => {
    assert.ok(!stack.includes(path), `workflow cycle: ${path}`);
    const text = byPath.get(path);
    assert.ok(typeof text === 'string', `missing workflow source: ${path}`);
    const evidence = new Set();
    const lines = text.split('\n').filter((line) => !/^\s*#/.test(line));
    const runners = lines.flatMap((line) => {
      const match = /^\s+(?:- )?runner:\s*([\w.-]+)\s*(?:#.*)?$/.exec(line);
      return match ? [match[1]] : [];
    });
    for (const line of lines) {
      const match = /^ {4}runs-on:\s*(.*?)\s*(?:#.*)?$/.exec(line);
      if (!match) continue;
      const runner = match[1].replace(/^(['"])(.*)\1$/, '$2');
      const candidates = runner === '${{ matrix.runner }}' ? runners : [runner];
      assert.ok(candidates.length > 0, `unresolved runner in ${path}`);
      for (const candidate of candidates) {
        assert.match(
          candidate,
          /^(?:ubuntu-[\w.-]+|windows-[\w.-]+|macos-[\w.-]+)$/,
          `unknown runner in ${path}: ${candidate}`
        );
        if (candidate.startsWith('macos-')) evidence.add(path);
      }
    }
    for (const line of lines) {
      const match = /^ {4}uses:\s*(\S+)/.exec(line);
      if (!match) continue;
      assert.match(
        match[1],
        /^\.\/\.github\/workflows\/[\w.-]+\.ya?ml$/,
        `external reusable workflow in ${path}`
      );
      for (const child of visit(match[1].slice(2), [...stack, path])) evidence.add(child);
    }
    return [...evidence].sort();
  };
  return sources.map(({ path }) => {
    const evidence = visit(path);
    return { path, macos: evidence.length > 0, evidence };
  });
}

/** All attempts, retries and both REST-core snapshots share one hard ceiling. */
export function demandReader({
  repo = 'pj-tmt/tmt',
  maxRequests = 800,
  execute = runPackedCommand,
} = {}) {
  assert.match(
    repo,
    /^[A-Za-z0-9_-][A-Za-z0-9_.-]*\/[A-Za-z0-9_-][A-Za-z0-9_.-]*$/,
    'invalid repository'
  );
  assert.ok(
    Number.isInteger(maxRequests) && maxRequests >= 2 && maxRequests <= 800,
    'REST budget must be 2..800'
  );
  const ledger = [];
  let finished = false;
  const get = (path, final = false) => {
    assert.ok(!finished, 'reader already finished');
    assert.ok(
      path === '/rate_limit' ||
        /^(?:contents\/\.github\/workflows\/[\w.-]+\.ya?ml\?ref=[a-f0-9]{40}|actions\/(?:workflows(?:\?per_page=100&page=\d+|\/\d+\/runs\?created=[^&]+&per_page=100&page=\d+)|runs\/\d+\/(?:jobs\?filter=all&per_page=(?:100|50)&page=\d+|attempts\/\d+)))$/.test(
          path
        ),
      'unapproved REST path'
    );
    for (let attempt = 1; ; attempt++) {
      assert.ok(
        ledger.length < maxRequests - (final ? 0 : 1),
        `REST budget ${maxRequests} exhausted (final core snapshot reserved)`
      );
      const entry = { path, attempt, outcome: 'error' };
      ledger.push(entry);
      try {
        const value = JSON.parse(
          execute(
            'gh',
            ['api', '--method', 'GET', path === '/rate_limit' ? path : `repos/${repo}/${path}`],
            { timeoutMs: 60_000, maxBuffer: 8 * 1024 * 1024 }
          )
        );
        entry.outcome = 'ok';
        return value;
      } catch (error) {
        if (
          final ||
          attempt >= 3 ||
          !/HTTP 5\d\d/.test(error.message) ||
          ledger.length >= maxRequests - 1
        )
          throw error;
      }
    }
  };
  const snapshot = (data) => {
    const core = data?.resources?.core;
    assert.ok(
      core &&
        Number.isSafeInteger(core.remaining) &&
        core.remaining >= 0 &&
        id(core.limit) &&
        id(core.reset),
      'missing REST core rate snapshot'
    );
    return { remaining: core.remaining, limit: core.limit, reset: core.reset };
  };
  const coreBefore = snapshot(get('/rate_limit'));
  const read = (path) => {
    assert.ok(
      coreBefore.remaining >= 2500,
      `REST core remaining ${coreBefore.remaining} below 2500; no data reads`
    );
    return get(path);
  };
  read.requests = () => ledger.length;
  read.remaining = () => maxRequests - 1 - ledger.length;
  read.finish = () => {
    const coreAfter = snapshot(get('/rate_limit', true));
    finished = true;
    return { calls: ledger.length, cap: maxRequests, coreBefore, coreAfter, ledger };
  };
  return read;
}

function pageRows(read, path, key, admit, perPage = 100, expectedTotal) {
  const rows = [];
  const seen = new Set();
  let total = expectedTotal;
  try {
    for (let page = 1; ; page++) {
      const data = read(`${path}&page=${page}`);
      assert.ok(
        Number.isSafeInteger(data.total_count) && data.total_count >= 0 && Array.isArray(data[key]),
        `invalid ${key} page`
      );
      total ??= data.total_count;
      assert.equal(data.total_count, total, `changing ${key} count`);
      assert.equal(
        data[key].length,
        Math.min(perPage, total - rows.length),
        `truncated ${key} evidence`
      );
      for (const row of data[key]) {
        assert.ok(id(row.id) && !seen.has(row.id), `duplicate/invalid ${key} identity`);
        seen.add(row.id);
        admit(row);
        rows.push(row);
      }
      if (rows.length === total) return rows;
    }
  } catch (error) {
    error.totalCount = total ?? null;
    throw error;
  }
}

// A persistent 100-row 502 discards the partial run and restarts at page one.
function runJobs(read, run) {
  const admit = (job) => {
    assert.equal(job.run_id, run.id, 'foreign job run');
    assert.ok(
      id(job.run_attempt) && job.run_attempt <= run.run_attempt,
      'foreign/missing job attempt'
    );
    assert.ok(
      typeof job.name === 'string' &&
        Array.isArray(job.labels) &&
        job.labels.every((label) => typeof label === 'string'),
      'invalid job metadata'
    );
  };
  let total;
  try {
    try {
      return pageRows(read, `actions/runs/${run.id}/jobs?filter=all&per_page=100`, 'jobs', admit);
    } catch (error) {
      total = error.totalCount;
      if (!/HTTP 502/.test(error.message)) throw error;
    }
    return pageRows(
      read,
      `actions/runs/${run.id}/jobs?filter=all&per_page=50`,
      'jobs',
      admit,
      50,
      total ?? undefined
    );
  } catch (error) {
    throw new Error(
      `Unreadable run ${run.id} (${run.workflow ?? 'fixture'}); job count ${error.totalCount ?? total ?? 'unavailable'}: ${error.message}`
    );
  }
}

function runCohort(read, workflow, start, end) {
  const query = `actions/workflows/${workflow.id}/runs?created=${encodeURIComponent(`${iso(start)}..${iso(end - 1000)}`)}&per_page=100`;
  // GitHub search returns at most 1,000 runs. Split before paging capped searches.
  const first = read(`${query}&page=1`);
  assert.ok(
    Number.isSafeInteger(first.total_count) &&
      first.total_count >= 0 &&
      Array.isArray(first.workflow_runs),
    'invalid run search'
  );
  if (first.total_count >= 1000) {
    assert.ok(end - start > 1000, '1000 runs in one second: search cannot be enumerated');
    const middle = start + Math.floor((end - start) / 2000) * 1000;
    return [...runCohort(read, workflow, start, middle), ...runCohort(read, workflow, middle, end)];
  }
  return pageRows(
    (path) => (path === `${query}&page=1` ? first : read(path)),
    query,
    'workflow_runs',
    (run) => {
      assert.equal(run.workflow_id, workflow.id, 'foreign workflow run');
      assert.ok(time(run.created_at) >= start && time(run.created_at) < end, 'out-of-window run');
      assert.ok(
        id(run.run_attempt) && typeof run.event === 'string' && run.event.length > 0,
        'invalid run metadata'
      );
    }
  );
}

const fingerprint = ({ name, started_at, completed_at, status, conclusion, steps }) => ({
  name,
  started_at,
  completed_at,
  status,
  conclusion,
  steps,
});
function actualJobs(read, run) {
  const actual = [];
  const prior = new Map();
  let retained = 0;
  let previousStart = -Infinity;
  const allJobs = runJobs(read, run);
  for (let attempt = 1; attempt <= run.run_attempt; attempt++) {
    let start;
    if (run.run_attempt > 1) {
      const context = read(`actions/runs/${run.id}/attempts/${attempt}`);
      assert.ok(context.id === run.id && context.run_attempt === attempt, 'foreign attempt');
      start = time(context.run_started_at);
      assert.ok(Number.isFinite(start) && start > previousStart, 'invalid attempt chronology');
      previousStart = start;
    }
    const jobs = allJobs.filter((job) => job.run_attempt === attempt);
    for (const job of jobs) {
      if (job.conclusion === 'skipped') {
        actual.push(job);
        continue;
      }
      if (start !== undefined && time(job.started_at) < start) {
        assert.ok(
          job.conclusion === 'success' &&
            job.status === 'completed' &&
            time(job.completed_at) <= start &&
            prior.has(job.name) &&
            Array.isArray(job.steps) &&
            isDeepStrictEqual(fingerprint(job), fingerprint(prior.get(job.name))),
          `unproved retained success: ${job.name}`
        );
        retained++;
        continue;
      }
      actual.push(job);
      prior.set(job.name, job);
    }
  }
  return { jobs: actual, retained };
}

/** Collection returns no aggregate if any page or attempt cannot be proved complete. */
export function collectDemand(read, { since, until, sources, historicalSources }) {
  const start = time(since);
  const end = time(until);
  assert.ok(
    Number.isFinite(start) &&
      Number.isFinite(end) &&
      start < end &&
      start % 1000 === 0 &&
      end % 1000 === 0 &&
      end - start <= 6 * 3600_000,
    'window must have increasing whole-second UTC boundaries and span at most six hours'
  );
  const trace = traceWorkflows(sources);
  const workflows = [];
  const runs = [];
  const seen = new Set();
  const inventory = pageRows(read, 'actions/workflows?per_page=100', 'workflows', (row) => {
    assert.ok(
      typeof row.name === 'string' && typeof row.path === 'string',
      'invalid workflow inventory'
    );
  });
  for (const { id: workflowId, name, path } of inventory) {
    let source = trace.find((entry) => entry.path === path);
    const workflow = { id: workflowId, name, path };
    // GitHub keeps deleted throwaway workflows in its inventory. Prove absence of
    // cohort runs, or trace their exact run source; never assume they were Linux.
    const cohort = !source || source.macos ? runCohort(read, workflow, start, end) : [];
    const historical = [];
    if (!source && cohort.length) {
      assert.ok(historicalSources, `workflow source unavailable at recorded SHA: ${path}`);
      for (const sha of new Set(cohort.map((run) => run.head_sha))) {
        assert.match(sha ?? '', /^[a-f0-9]{40}$/, 'missing historical source SHA');
        const resolved = traceWorkflows(historicalSources(path, sha)).find(
          (entry) => entry.path === path
        );
        assert.ok(resolved, `historical workflow source unavailable: ${path}`);
        historical.push({ sha, ...resolved });
      }
      source = {
        path,
        macos: historical.some((entry) => entry.macos),
        evidence: [...new Set(historical.flatMap((entry) => entry.evidence))].sort(),
      };
    }
    source ??= { path, macos: false, evidence: [] };
    workflows.push({
      ...workflow,
      ...source,
      historical,
      reason:
        !trace.some((entry) => entry.path === path) && !cohort.length
          ? 'absent from source; zero runs in window'
          : source.macos
            ? 'macOS in source/reusable closure'
            : 'no macOS runner in source/reusable closure',
    });
    for (const run of cohort) {
      assert.ok(!seen.has(run.id), 'duplicate run across ranges/workflows');
      seen.add(run.id);
      if (
        historical.length
          ? historical.find((entry) => entry.sha === run.head_sha).macos
          : source.macos
      )
        runs.push({ ...run, workflow: name, path });
    }
  }
  const minimumCalls = runs.reduce(
    (sum, run) => sum + (run.run_attempt === 1 ? 1 : 1 + run.run_attempt),
    0
  );
  assert.ok(
    minimumCalls <= read.remaining(),
    `hydration preflight: ${runs.length} runs need at least ${minimumCalls} GETs; ${read.remaining()} remain (final core snapshot reserved); narrow --since/--until`
  );
  const measurements = [];
  let retainedCopies = 0;
  let selfHostedJobs = 0;
  let unclassifiedJobs = 0;
  for (const run of runs) {
    assert.ok(
      run.pull_requests === undefined || Array.isArray(run.pull_requests),
      'invalid PR associations'
    );
    const prs = [
      ...new Set(
        (run.pull_requests ?? []).map((pr) => {
          assert.ok(id(pr.number), 'invalid PR association');
          return pr.number;
        })
      ),
    ].sort((a, b) => a - b);
    // A multi-PR association is one shared bucket, never credited in full to each PR.
    const association = prs.length
      ? prs.map((pr) => `#${pr}`).join(', ') + (prs.length > 1 ? ' (shared)' : '')
      : run.event === 'pull_request'
        ? 'PR unavailable'
        : 'no PR association';
    const executions = actualJobs(read, run);
    retainedCopies += executions.retained;
    for (const job of executions.jobs) {
      if (job.labels.includes('self-hosted')) {
        selfHostedJobs++;
        continue;
      }
      if (!job.labels.length) {
        unclassifiedJobs++;
        continue;
      }
      if (!macos(job.labels)) continue;
      const created = time(job.created_at);
      const started = time(job.started_at);
      const completed = time(job.completed_at);
      const skipped = job.conclusion === 'skipped';
      const pending = !skipped && !Number.isFinite(started);
      const executed = !skipped && !pending;
      const finished = executed && job.status === 'completed';
      const queueSeconds =
        executed && Number.isFinite(created) && started >= created
          ? (started - created) / 1000
          : null;
      const runnerMinutes =
        finished && Number.isFinite(completed) && completed >= started
          ? (completed - started) / 60_000
          : null;
      measurements.push({
        run: run.id,
        job: job.id,
        attempt: job.run_attempt,
        name: job.name,
        workflow: run.workflow,
        path: run.path,
        event: run.event,
        prs,
        association,
        labels: job.labels.filter(macosLabel).sort(),
        status: job.status,
        conclusion: job.conclusion,
        startedAt: job.started_at ?? null,
        skipped: Number(skipped),
        pending: Number(pending),
        completed: Number(finished),
        unfinished: Number(executed && !finished),
        invalidTimings: Number(
          executed && (queueSeconds === null || (finished && runnerMinutes === null))
        ),
        queueSeconds,
        runnerMinutes,
      });
    }
  }
  return {
    schema: 1,
    complete: true,
    since: iso(start),
    until: iso(end),
    cohort: 'workflow run created_at, all outcomes; not billed minutes',
    workflows,
    runs: runs.length,
    actualExecutions: measurements.filter(
      (job) => job.conclusion !== 'skipped' && Number.isFinite(time(job.startedAt))
    ).length,
    retainedCopies,
    selfHostedJobs,
    unclassifiedJobs,
    rows: aggregateDemand(measurements, ['workflow', 'path', 'event', 'labels']),
    jobRows: aggregateDemand(measurements, ['workflow', 'path', 'event', 'labels', 'name']),
    prRows: aggregateDemand(measurements, ['association']),
    measurements,
  };
}

// One aggregation owner for disjoint workflow/event, job and PR-association views.
function aggregateDemand(measurements, dimensions) {
  const groups = new Map();
  for (const measurement of measurements) {
    const key = JSON.stringify(dimensions.map((dimension) => measurement[dimension]));
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(measurement);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([, group]) => {
      const row = Object.fromEntries(
        dimensions.map((dimension) => [dimension, group[0][dimension]])
      );
      const queue = group.flatMap((job) => (job.queueSeconds === null ? [] : [job.queueSeconds]));
      const minutes = group.flatMap((job) =>
        job.runnerMinutes === null ? [] : [job.runnerMinutes]
      );
      for (const count of ['completed', 'skipped', 'pending', 'unfinished', 'invalidTimings'])
        row[count] = group.reduce((sum, job) => sum + job[count], 0);
      return {
        ...row,
        jobs: group.length,
        durationSamples: minutes.length,
        runnerMinutes: minutes.length ? minutes.reduce((sum, value) => sum + value, 0) : null,
        queueSeconds: {
          n: queue.length,
          median: median(queue),
          p90: percentile(queue, 0.9),
          max: queue.length ? Math.max(...queue) : null,
        },
      };
    });
}

const cell = (value) =>
  String(value)
    .replaceAll('|', '\\|')
    .replace(/[\r\n]/g, ' ');
const number = (value) => (value === null ? 'unavailable' : value.toFixed(2));
export function renderDemand(report) {
  const lines = [
    '# Measured macOS demand',
    '',
    `Window: ${report.since} ≤ run created_at < ${report.until}.`,
    `Source SHA: ${report.sourceSha ?? 'fixture'}. Complete census: ${report.complete}; ${report.runs} macOS-capable workflow runs; ${report.actualExecutions} actual macOS executions; ${report.retainedCopies} retained copies excluded.`,
    '',
    'Minutes are measured completed-job occupancy, not billing. Pending/unfinished/skipped jobs are not zero-duration samples. Queue statistics are seconds (ordinary median, nearest-rank p90).',
    '',
    `Excluded self-hosted jobs: ${report.selfHostedJobs}; jobs with unavailable runner labels: ${report.unclassifiedJobs}. PR associations use run metadata only; shared/missing associations stay explicit and never multiply occupancy.`,
  ];
  for (const [title, dimensions, rows] of [
    [
      'Workflow and trigger',
      [
        ['workflow', 'Workflow'],
        ['event', 'Trigger'],
        ['labels', 'macOS labels'],
      ],
      report.rows,
    ],
    [
      'Job',
      [
        ['workflow', 'Workflow'],
        ['event', 'Trigger'],
        ['name', 'Job'],
      ],
      report.jobRows,
    ],
    ['PR association', [['association', 'PR association']], report.prRows],
  ]) {
    lines.push(
      '',
      `## ${title}`,
      '',
      `| ${dimensions.map(([, label]) => label).join(' | ')} | Jobs | Complete | Skipped | Pending | Unfinished | Invalid timing | Duration n | Runner-min | Queue n | Median s | p90 s | Max s |`,
      `| ${dimensions.map(() => '---').join(' | ')} | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |`
    );
    for (const row of rows)
      lines.push(
        `| ${[
          ...dimensions.map(([key]) => (Array.isArray(row[key]) ? row[key].join(', ') : row[key])),
          row.jobs,
          row.completed,
          row.skipped,
          row.pending,
          row.unfinished,
          row.invalidTimings,
          row.durationSamples,
          number(row.runnerMinutes),
          row.queueSeconds.n,
          number(row.queueSeconds.median),
          number(row.queueSeconds.p90),
          number(row.queueSeconds.max),
        ]
          .map(cell)
          .join(' | ')} |`
      );
  }
  lines.push(
    '',
    '## Source inventory',
    '',
    '| Workflow | Hydrate | Reason | macOS source closure |',
    '| --- | --- | --- | --- |'
  );
  for (const workflow of report.workflows)
    lines.push(
      `| ${[workflow.path, workflow.macos ? 'yes' : 'no', workflow.reason, workflow.evidence.join(', ')].map(cell).join(' | ')} |`
    );
  if (report.requests)
    lines.push(
      '',
      '## REST core ledger',
      '',
      `${report.requests.calls}/${report.requests.cap} GET attempts, including rate snapshots and retries. Core remaining before: ${report.requests.coreBefore.remaining}; after: ${report.requests.coreAfter.remaining}. This is a shared account; the difference is not attributed exclusively to this invocation. Full endpoint ledger is in JSON.`
    );
  return `${lines.join('\n')}\n`;
}

function main() {
  const { values } = parseArgs({
    options: {
      repo: { type: 'string', default: 'pj-tmt/tmt' },
      since: { type: 'string' },
      until: { type: 'string' },
      sha: { type: 'string', default: 'HEAD' },
      output: { type: 'string' },
      'max-requests': { type: 'string', default: '800' },
    },
  });
  assert.ok(values.output, '--output PREFIX is required (JSON and Markdown)');
  const sourceSha = runPackedCommand(
    'git',
    ['rev-parse', '--verify', `${values.sha}^{commit}`],
    {}
  ).trim();
  assert.match(sourceSha, /^[a-f0-9]{40}$/, 'invalid source SHA');
  const paths = runPackedCommand(
    'git',
    ['ls-tree', '-r', '--name-only', sourceSha, '.github/workflows'],
    {}
  )
    .trim()
    .split('\n')
    .filter((path) => /\.ya?ml$/.test(path));
  const sources = paths.map((path) => ({
    path,
    text: runPackedCommand('git', ['show', `${sourceSha}:${path}`], {}),
  }));
  const until = values.until ?? iso(Math.floor(Date.now() / 1000) * 1000);
  const since = values.since ?? iso(time(until) - 6 * 3600_000);
  const read = demandReader({ repo: values.repo, maxRequests: Number(values['max-requests']) });
  let report;
  try {
    const historicalSources = (path, sha) => {
      const loaded = new Map();
      const load = (file) => {
        if (loaded.has(file)) return;
        assert.match(
          file,
          /^\.github\/workflows\/[\w.-]+\.ya?ml$/,
          'unsafe historical workflow path'
        );
        const data = read(`contents/${file}?ref=${sha}`);
        assert.ok(
          data.encoding === 'base64' && typeof data.content === 'string',
          'missing historical workflow contents'
        );
        const text = Buffer.from(data.content, 'base64').toString('utf8');
        loaded.set(file, { path: file, text });
        for (const match of text.matchAll(
          /^ {4}uses: \.\/(\.github\/workflows\/[\w.-]+\.ya?ml)\s*$/gm
        ))
          load(match[1]);
      };
      load(path);
      return [...loaded.values()];
    };
    report = { ...collectDemand(read, { since, until, sources, historicalSources }), sourceSha };
  } catch (error) {
    report = { schema: 1, complete: false, since, until, sourceSha, error: error.message };
    process.exitCode = 1;
  }
  try {
    report.requests = read.finish();
  } catch (error) {
    report.complete = false;
    report.rateError = error.message;
    process.exitCode = 1;
  }
  writeFileSync(`${values.output}.json`, `${JSON.stringify(report, null, 2)}\n`);
  const markdown = report.complete
    ? renderDemand(report)
    : `# INCOMPLETE macOS census — no totals\n\n${report.error ?? report.rateError}\n\n${JSON.stringify(report.requests ?? { rateError: report.rateError }, null, 2)}\n`;
  writeFileSync(`${values.output}.md`, markdown);
  console.log(markdown);
}
if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
