// PR-body attribution; queue enforcement follows its explicit report-only rollout.
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { pendingQueueSubjects } from './pr-title-check.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
export const AGENT_PRESETS = Object.freeze([
  'codex-sol-high',
  'codex-sol-med',
  'codex-luna-med',
  'codex-luna-low',
  'claude-opus-med',
  'claude-sonnet-high',
  'claude-sonnet-xhigh',
]);
const SEAT = /^[a-z0-9][a-z0-9-]*$/;
const TOKEN = /^[A-Za-z0-9._-]+$/;

/** Attribution is visible prose, not a fenced example or an HTML comment. */
export function parseAgentLines(body) {
  if (typeof body !== 'string') return { agents: [], findings: ['Missing PR body.'] };
  const agents = [];
  const findings = [];
  let fence;
  let comment = false;
  for (const [index, raw] of body.split(/\r?\n/).entries()) {
    if (fence) {
      const close = /^ {0,3}(`{3,}|~{3,})\s*$/.exec(raw);
      if (close && close[1][0] === fence[0] && close[1].length >= fence.length) fence = undefined;
      continue;
    }
    let line = '';
    let rest = raw;
    while (rest) {
      const position = rest.indexOf(comment ? '-->' : '<!--');
      if (position < 0) {
        if (!comment) line += rest;
        break;
      }
      if (!comment) line += rest.slice(0, position);
      rest = rest.slice(position + (comment ? 3 : 4));
      comment = !comment;
    }
    const open = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (open) {
      fence = open[1];
      continue;
    }
    line = line.trim();
    if (!/^agent:/i.test(line)) continue;
    const tokens = line.slice(6).trim().split(/\s+/);
    const [seat, ...value] = tokens;
    const valid =
      line.startsWith('Agent:') &&
      SEAT.test(seat) &&
      ((seat === 'ben' && value.length === 0) ||
        (value.length === 1 && AGENT_PRESETS.includes(value[0])) ||
        (value.length === 3 && value.every((token) => TOKEN.test(token))));
    if (valid) agents.push(tokens.join(' '));
    else findings.push(`Invalid Agent line ${index + 1}.`);
  }
  if (!agents.length) findings.push('No valid visible Agent line.');
  return { agents, findings };
}

function currentPull(number, reader, repository) {
  if (!Number.isSafeInteger(number) || number <= 0) throw new Error('Missing PR number.');
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? ''))
    throw new Error('Missing repository identity.');
  const pull = JSON.parse(reader.rest(`repos/${repository}/pulls/${number}`));
  if (
    pull?.number !== number ||
    typeof pull.title !== 'string' ||
    typeof pull.user?.login !== 'string' ||
    !Object.hasOwn(pull, 'body') ||
    (pull.body !== null && typeof pull.body !== 'string')
  )
    throw new Error(`Unavailable current PR #${number} evidence.`);
  const exempt =
    pull.user.login === 'tmt-ci-bot[bot]' && /^chore\(main\): \S[^\r\n]*$/.test(pull.title);
  return {
    number,
    exempt,
    ...(exempt ? { agents: [], findings: [] } : parseAgentLines(pull.body)),
  };
}

/** Current REST evidence replaces a stale rerun event body; queue subjects supply only PR IDs. */
export function checkPrAgents({ eventName, event, reader, repository, reportOnly = false }) {
  if (!['pull_request', 'merge_group'].includes(eventName))
    throw new Error('Agent checks require a pull request or merge group.');
  if (reportOnly && eventName !== 'merge_group')
    throw new Error('--report-only is available only for merge groups.');
  const numbers =
    eventName === 'pull_request'
      ? [event?.pull_request?.number]
      : [...new Set(pendingQueueSubjects({ event, reader }).map(({ number }) => number))];
  const pulls = numbers.map((number) => currentPull(number, reader, repository));
  return {
    checked: pulls.length,
    exempted: pulls.filter(({ exempt }) => exempt).map(({ number }) => number),
    findings: pulls
      .filter(({ findings }) => findings.length)
      .map(({ number, findings }) => ({ number, findings })),
  };
}

function report(text) {
  process.stdout.write(text);
  try {
    if (!process.env.GITHUB_STEP_SUMMARY) throw new Error('GITHUB_STEP_SUMMARY is required.');
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  } catch (error) {
    process.stderr.write(`Agent report summary unavailable: ${error.message}\n`);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const reportOnly = process.argv.length === 3 && process.argv[2] === '--report-only';
  const observation = reportOnly && process.env.GITHUB_EVENT_NAME === 'merge_group';
  try {
    if (process.argv.length !== 2 && !reportOnly)
      throw new Error('Usage: pr-agent-check.mjs [--report-only]');
    const run = (command, args) =>
      runPackedCommand(command, args, { cwd: ROOT, env: process.env, timeoutMs: 30_000 });
    const result = checkPrAgents({
      eventName: process.env.GITHUB_EVENT_NAME,
      event: JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8')),
      repository: process.env.GITHUB_REPOSITORY,
      reportOnly,
      reader: {
        git: (args) => run('git', args).trim(),
        rest: (endpoint) => run('gh', ['api', endpoint]),
      },
    });
    report(
      `### PR Agent attribution${observation ? ' (report-only)' : ''}\n\n${JSON.stringify(result)}\n${observation ? 'Findings do not fail this merge group.' : 'Every non-exempt PR needs a valid visible Agent line.'}\n`
    );
    if (result.findings.length && !observation) process.exitCode = 1;
  } catch (error) {
    // Report-only applies solely to the queue rollout; PR failures remain visible and failing.
    report(
      `### PR Agent attribution${observation ? ' (report-only)' : ''}\n\nAgent evidence unavailable: ${JSON.stringify(error.message)}\n`
    );
    if (!observation) process.exitCode = 1;
  }
}
