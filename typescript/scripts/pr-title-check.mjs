// Conventional PR titles: enforced on pull requests that change a released component, and
// reported (never failing) over the cumulative squash merge-group range.
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { componentMap, releasedComponentNamesOfPath } from './ci-scope.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const SHA = /^[a-f0-9]{40}$/;
const QUEUE_COMMITS = 40;

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
  return (
    typeof title === 'string' && /^[a-z][a-z0-9-]*(?:\([^()\r\n]+\))?!?: \S[^\r\n]*$/.test(title)
  );
}

export function checkQueueTitles({ event, reader }) {
  const commits = pendingQueueSubjects({ event, reader });
  return {
    checked: commits.length,
    findings: commits.filter(({ title }) => !conventionalPrTitle(title)),
  };
}

/**
 * A non-conventional squash title on a released component's change is dropped silently by the
 * cut planner's conventional types, so it must be fixed before it enters the queue. Pull
 * requests that change no released component keep any title.
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

// Summary I/O never decides the result; only the pull-request gate's evidence and title do.
function writeTitleReport(text) {
  process.stdout.write(text);
  try {
    if (!process.env.GITHUB_STEP_SUMMARY) throw new Error('GITHUB_STEP_SUMMARY is required.');
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  } catch (error) {
    process.stderr.write(`Title report summary unavailable: ${error.message}\n`);
  }
}

function reportMergeGroupUnavailable(error) {
  writeTitleReport(
    `### Conventional PR titles (report-only)\n\nTitle evidence unavailable: <code>${summaryText(error.message)}</code>. This report does not fail the merge group.\n`
  );
}

function runMergeGroupReport(event, reader) {
  try {
    const { checked, findings } = checkQueueTitles({ event, reader });
    const details = findings
      .map(
        ({ sha, title, number }) =>
          `- PR #${number}, commit <code>${sha}</code>: <code>${summaryText(title)}</code>\n`
      )
      .join('');
    writeTitleReport(
      `### Conventional PR titles (report-only)\n\nChecked ${checked} queued title(s); ${findings.length} finding(s). ` +
        'Expected type(scope)?: subject, with an optional breaking-change marker. Findings do not fail this merge group.\n\n' +
        details
    );
  } catch (error) {
    reportMergeGroupUnavailable(error);
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
      'The cut planner would otherwise list it only under "Other changes". Edit the PR title to type(scope)?: subject (for example fix(remote): keep door origins stable), then rerun Code quality.\n'
  );
  process.exitCode = 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 2) throw new Error('Usage: pr-title-check.mjs');
    const run = (command, args) =>
      runPackedCommand(command, args, { cwd: ROOT, env: process.env, timeoutMs: 30_000 });
    const reader = {
      git: (args) => run('git', args).trim(),
      rest: (endpoint) => run('gh', ['api', endpoint]),
    };
    const eventName = process.env.GITHUB_EVENT_NAME;
    if (!['merge_group', 'pull_request'].includes(eventName))
      throw new Error('The title check runs only on pull requests and merge groups.');
    const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
    if (eventName === 'merge_group') runMergeGroupReport(event, reader);
    else runPullRequestGate(event, reader);
  } catch (error) {
    // Only the merge-group observation phase may pass without evidence.
    if (process.env.GITHUB_EVENT_NAME === 'merge_group') {
      reportMergeGroupUnavailable(error);
    } else {
      writeTitleReport(
        `### Conventional PR title\n\nTitle evidence unavailable: <code>${summaryText(error.message)}</code>. Rerun Code quality.\n`
      );
      process.exitCode = 1;
    }
  }
}
