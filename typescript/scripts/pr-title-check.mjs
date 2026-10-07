// Conventional PR titles: enforced on pull requests that change a released component, and
// enforced over the cumulative squash merge-group range; --report-only retains observation mode.
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { componentMap, releasedComponentNamesOfPath } from './ci-scope.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const SHA = /^[a-f0-9]{40}$/;
const QUEUE_COMMITS = 40;

export const CONVENTIONAL_PR_TYPES = Object.freeze([
  'feat',
  'fix',
  'perf',
  'revert',
  'refactor',
  'docs',
  'test',
  'ci',
  'chore',
  'build',
]);
const EXPECTED_TITLE = `Expected type(scope)?: subject, with an optional breaking-change marker. Approved types: ${CONVENTIONAL_PR_TYPES.join(', ')}.`;

export function pendingQueueSubjects({ event, reader }) {
  if (!SHA.test(event?.merge_group?.head_sha ?? ''))
    throw new Error('Missing merge-group event data.');
  const head = event.merge_group.head_sha;
  const base = reader.git(['merge-base', '--all', 'refs/remotes/origin/main', head]);
  if (!SHA.test(base)) throw new Error('Missing or ambiguous cumulative queue base.');
  const log = reader.git(['log', '--reverse', '--format=%H%x09%s', `${base}..${head}`]);
  const commits = log ? log.split('\n') : [];
  if (!commits.length || commits.length > QUEUE_COMMITS)
    throw new Error('Missing or oversized pending queue range.');
  return commits.map((line) => {
    const sha = line.slice(0, 40);
    const subject = line.slice(41);
    if (!SHA.test(sha) || line[40] !== '\t') throw new Error('Invalid pending queue commit.');
    // GitHub's squash queue appends the PR number; only remove that final suffix.
    const match = / \(#(\d+)\)$/.exec(subject);
    if (!match) throw new Error('Pending queue commit has no PR number.');
    return { sha, title: subject.slice(0, match.index), number: Number(match[1]) };
  });
}

/** Syntax feedback only; release-cut owns release attribution and changelog rules. */
export function conventionalPrTitle(title) {
  const match =
    typeof title === 'string' && /^([a-z][a-z0-9-]*)(?:\([^()\r\n]+\))?!?: \S[^\r\n]*$/.exec(title);
  return Boolean(match && CONVENTIONAL_PR_TYPES.includes(match[1]));
}

export function checkQueueTitles({ event, reader }) {
  const commits = pendingQueueSubjects({ event, reader });
  return {
    checked: commits.length,
    findings: commits.filter(({ title }) => !conventionalPrTitle(title)),
  };
}

/**
 * The cut planner retains non-conventional released-component changes under Other changes.
 * This gate enforces the approved title policy before queue admission; pull requests that
 * change no released component keep any title.
 */
export function checkPullRequestTitle({ title, paths, map }) {
  const components = new Set();
  for (const path of paths)
    for (const name of releasedComponentNamesOfPath(path, map)) components.add(name);
  return {
    components: [...components].sort(),
    ok: conventionalPrTitle(title) || components.size === 0,
  };
}

function enforcePullRequestTitle({ event, reader }) {
  const pull = event?.pull_request;
  if (
    !Number.isInteger(pull?.number) ||
    ![pull.base?.sha, pull.head?.sha].every((s) => SHA.test(s ?? ''))
  )
    throw new Error('Missing pull-request event data.');
  // A rerun reuses the original event payload, so the current title comes from REST.
  const title = JSON.parse(
    reader.rest(`repos/${process.env.GITHUB_REPOSITORY}/pulls/${pull.number}`)
  ).title;
  if (typeof title !== 'string') throw new Error('Missing current pull-request title.');
  const changed = reader.git([
    'diff',
    '--no-renames',
    '--name-only',
    '-z',
    `${pull.base.sha}...${pull.head.sha}`,
    '--',
  ]);
  return {
    title,
    ...checkPullRequestTitle({
      title,
      paths: changed.split('\0').filter(Boolean),
      map: componentMap(),
    }),
  };
}

function summaryText(text) {
  return String(text).replace(
    /[&<>"']/g,
    (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]
  );
}

// Summary I/O never decides the result; evidence and the selected gate policy do.
function writeTitleReport(text) {
  process.stdout.write(text);
  try {
    if (!process.env.GITHUB_STEP_SUMMARY) throw new Error('GITHUB_STEP_SUMMARY is required.');
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  } catch (error) {
    process.stderr.write(`Title report summary unavailable: ${error.message}\n`);
  }
}

function reportMergeGroupUnavailable(error, reportOnly) {
  writeTitleReport(
    `### Conventional PR titles${reportOnly ? ' (report-only)' : ''}\n\nTitle evidence unavailable: <code>${summaryText(error.message)}</code>. ${reportOnly ? 'This report does not fail the merge group.' : 'The merge-group title gate fails closed.'}\n`
  );
  if (!reportOnly) process.exitCode = 1;
}

function runMergeGroupCheck(event, reader, reportOnly) {
  try {
    const { checked, findings } = checkQueueTitles({ event, reader });
    const details = findings
      .map(
        ({ sha, title, number }) =>
          `- PR #${number}, commit <code>${sha}</code>: <code>${summaryText(title)}</code>\n`
      )
      .join('');
    writeTitleReport(
      `### Conventional PR titles${reportOnly ? ' (report-only)' : ''}\n\nChecked ${checked} queued title(s); ${findings.length} finding(s). ` +
        EXPECTED_TITLE +
        (reportOnly
          ? ' Findings do not fail this merge group.'
          : ' Every pending squash subject must pass before admission.') +
        '\n\n' +
        details
    );
    if (findings.length && !reportOnly) process.exitCode = 1;
  } catch (error) {
    reportMergeGroupUnavailable(error, reportOnly);
  }
}

// Enforcement fails closed: missing evidence is not a pass, and a rerun fixes a transient read.
function runPullRequestGate(event, reader) {
  const { title, components, ok } = enforcePullRequestTitle({ event, reader });
  if (ok) {
    writeTitleReport(
      `### Conventional PR title\n\n${components.length ? 'Conventional title for a PR that changes released component(s).' : 'No released component changed; any title is accepted.'}\n`
    );
    return;
  }
  writeTitleReport(
    `### Conventional PR title\n\nThis PR changes released component(s) ${components.join(', ')}, but its title is not conventional: <code>${summaryText(title)}</code>. ` +
      `${EXPECTED_TITLE} The cut planner would otherwise list it only under "Other changes". Edit the PR title (for example fix(remote): keep door origins stable); the separate PR title check runs on edits. Rerun Code quality if its previous title check failed.\n`
  );
  process.exitCode = 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const reportOnly = process.argv.length === 3 && process.argv[2] === '--report-only';
  try {
    if (process.argv.length !== 2 && !reportOnly)
      throw new Error('Usage: pr-title-check.mjs [--report-only]');
    const run = (command, args) =>
      runPackedCommand(command, args, { cwd: ROOT, env: process.env, timeoutMs: 30_000 });
    const reader = {
      git: (args) => run('git', args).trim(),
      rest: (endpoint) => run('gh', ['api', endpoint]),
    };
    const eventName = process.env.GITHUB_EVENT_NAME;
    if (!['merge_group', 'pull_request'].includes(eventName))
      throw new Error('The title check runs only on pull requests and merge groups.');
    if (reportOnly && eventName !== 'merge_group')
      throw new Error('--report-only is available only for merge groups.');
    const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
    if (eventName === 'merge_group') runMergeGroupCheck(event, reader, reportOnly);
    else runPullRequestGate(event, reader);
  } catch (error) {
    // Only the explicit merge-group observation command may pass without evidence.
    if (process.env.GITHUB_EVENT_NAME === 'merge_group') {
      reportMergeGroupUnavailable(error, reportOnly);
    } else {
      writeTitleReport(
        `### Conventional PR title\n\nTitle evidence unavailable: <code>${summaryText(error.message)}</code>. Rerun Code quality.\n`
      );
      process.exitCode = 1;
    }
  }
}
