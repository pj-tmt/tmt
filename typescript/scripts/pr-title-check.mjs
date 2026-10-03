// Report-only conventional titles from the cumulative squash merge-group range.
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
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

function summaryText(text) {
  return String(text).replace(
    /[&<>"']/g,
    (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]
  );
}

// The observation phase must not turn missing evidence or summary I/O into a queue gate.
function writeTitleReport(text) {
  process.stdout.write(text);
  try {
    if (!process.env.GITHUB_STEP_SUMMARY) throw new Error('GITHUB_STEP_SUMMARY is required.');
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  } catch (error) {
    process.stderr.write(`Title report summary unavailable: ${error.message}\n`);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 2) throw new Error('Usage: pr-title-check.mjs');
    const reader = {
      git: (args) =>
        runPackedCommand('git', args, { cwd: ROOT, env: process.env, timeoutMs: 30_000 }).trim(),
    };

    if (process.env.GITHUB_EVENT_NAME !== 'merge_group')
      throw new Error('The title report runs only on merge groups.');
    const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
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
    writeTitleReport(
      `### Conventional PR titles (report-only)\n\nTitle evidence unavailable: <code>${summaryText(error.message)}</code>. This report does not fail the merge group.\n`
    );
  }
}
