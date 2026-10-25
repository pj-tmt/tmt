// Pull-request commit identities: no author or committer address that ends in .local.
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const SHA = /^[a-f0-9]{40}$/;
const REF = /^[A-Za-z0-9._][A-Za-z0-9._/-]*$/;
const LISTED = 20;

/** A `.local` domain is a hostname git guessed, not an identity anyone can be credited under. */
export function isHostnameEmail(email) {
  const at = typeof email === 'string' ? email.lastIndexOf('@') : -1;
  return at > 0 && /\.local$/i.test(email.slice(at + 1).trim());
}

/** `git log --format=%H%x09%ae%x09%ce` lines in; the commits with a hostname address out. */
export function hostnameCommits(log) {
  const found = [];
  for (const line of log ? log.split('\n') : []) {
    const [sha, author, committer, ...extra] = line.split('\t');
    if (!SHA.test(sha ?? '') || committer === undefined || extra.length)
      throw new Error('Invalid commit identity line.');
    const roles = [];
    if (isHostnameEmail(author)) roles.push({ role: 'author', email: author });
    if (isHostnameEmail(committer)) roles.push({ role: 'committer', email: committer });
    if (roles.length) found.push({ sha, roles });
  }
  return found;
}

/** The range is this PR's own commits; commits already in the base are never judged. */
export function checkCommitEmails({ event, reader }) {
  const pull = event?.pull_request;
  if (![pull?.base?.sha, pull?.head?.sha].every((s) => SHA.test(s ?? '')))
    throw new Error('Missing pull-request event data.');
  const log = reader.git([
    'log',
    '--format=%H%x09%ae%x09%ce',
    `${pull.base.sha}..${pull.head.sha}`,
  ]);
  const base = typeof pull.base.ref === 'string' && REF.test(pull.base.ref) ? pull.base.ref : null;
  return {
    checked: log ? log.split('\n').length : 0,
    base: base ? `origin/${base}` : '<base>',
    findings: hostnameCommits(log),
  };
}

export function commitEmailReport({ checked, base, findings }) {
  if (!findings.length)
    return `### PR commit emails\n\nChecked ${checked} commit(s); none has an author or committer address ending in .local.\n`;
  const listed = findings
    .slice(0, LISTED)
    .map(
      ({ sha, roles }) =>
        `- ${sha.slice(0, 12)}: ${roles.map((r) => `${r.role} ${r.email}`).join(', ')}`
    )
    .join('\n');
  return (
    `### PR commit emails\n\n${findings.length} of ${checked} commit(s) carry an author or committer address ending in .local ` +
    `(a hostname git guessed because no identity was configured):\n\n${listed}\n` +
    `${findings.length > LISTED ? `- ... and ${findings.length - LISTED} more\n` : ''}\n` +
    `Fix: configure a real identity (\`git config --global user.name\` and \`user.email\`), then re-author this PR's commits and push:\n\n` +
    `    git rebase --exec 'git commit --amend --reset-author --no-edit' ${base}\n` +
    `    git push --force-with-lease\n`
  );
}

// Summary I/O never decides the result; the evidence does.
function report(text) {
  process.stdout.write(text);
  try {
    if (!process.env.GITHUB_STEP_SUMMARY) throw new Error('GITHUB_STEP_SUMMARY is required.');
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  } catch (error) {
    process.stderr.write(`Commit email summary unavailable: ${error.message}\n`);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 2) throw new Error('Usage: pr-commit-email-check.mjs');
    if (process.env.GITHUB_EVENT_NAME !== 'pull_request')
      throw new Error('The commit email check runs only on pull requests.');
    const result = checkCommitEmails({
      event: JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8')),
      reader: {
        git: (args) =>
          runPackedCommand('git', args, { cwd: ROOT, env: process.env, timeoutMs: 30_000 }).trim(),
      },
    });
    report(commitEmailReport(result));
    if (result.findings.length) process.exitCode = 1;
  } catch (error) {
    // Missing evidence is not a pass; a rerun fixes a transient read.
    report(
      `### PR commit emails\n\nCommit identity evidence unavailable: ${JSON.stringify(error.message)}. Rerun Code quality.\n`
    );
    process.exitCode = 1;
  }
}
