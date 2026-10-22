import fs from 'node:fs';
import path from 'node:path';
import {
  loadSoakDictionary,
  policy,
} from '../../extensions/tmt-colab/typescript/app/acceptance/harness/soak-literals.ts';
import { projectSoakFailure } from '../../extensions/tmt-colab/typescript/app/acceptance/harness/soak-diagnostics.ts';

const input = process.env.PLAYWRIGHT_JSON_OUTPUT_FILE;
const acceptance = path.resolve('extensions/tmt-colab/typescript/app/acceptance');
const sources = new Set(
  fs.existsSync(acceptance)
    ? fs
        .readdirSync(acceptance, { withFileTypes: true })
        .filter((entry) => entry.isFile())
        .map((entry) => entry.name)
    : []
);
const tests = [];
const dictionary = loadSoakDictionary();
let soakEvidence;

// Only this synthetic failing case may create the separate, bounded diagnostic artifact.
function retainSoak(spec, result) {
  if (
    path.basename(spec.file ?? '') !== 'connection-soak.spec.ts' ||
    spec.title !== policy.scenario ||
    !['failed', 'timedOut', 'interrupted'].includes(result.status)
  )
    return;
  for (const attachment of (Array.isArray(result.attachments) ? result.attachments : []).slice(
    0,
    64
  )) {
    if (
      attachment?.name !== policy.attachment ||
      attachment.contentType !== 'application/json' ||
      typeof attachment.body !== 'string' ||
      attachment.body.length > 131072
    )
      continue;
    try {
      const bytes = Buffer.from(attachment.body, 'base64');
      if (bytes.length > 65536 || bytes.toString('base64') !== attachment.body) continue;
      const value = projectSoakFailure(JSON.parse(bytes.toString('utf8')), dictionary);
      if (value && !soakEvidence) soakEvidence = value;
    } catch {
      // Unknown private input and attachment paths never belong in retained evidence.
    }
  }
}

// An attachment's JSON row by exact name, or nothing. Never follows an attachment path.
function attachedRow(attachment, name) {
  if (
    !attachment ||
    typeof attachment !== 'object' ||
    attachment.name !== name ||
    attachment.contentType !== 'application/json' ||
    typeof attachment.body !== 'string' ||
    attachment.body.length > 1024
  )
    return;
  try {
    const bytes = Buffer.from(attachment.body, 'base64');
    if (bytes.length > 512 || bytes.toString('base64') !== attachment.body) return;
    return JSON.parse(bytes.toString('utf8'));
  } catch {
    return;
  }
}

// An independent boundary check of the harness's closed Live-status diagnostic: which state a page
// stayed in, whether its renderer frame existed, the tab's visibility and the cause its terminal
// notice names. Never page text.
function liveState(attachment) {
  const row = attachedRow(attachment, 'colab-live-state');
  const states = ['live', 'opening', 'stopped', 'reconnecting', 'archived', 'none', 'other'];
  const causes = [
    'access',
    'revoked',
    'expired',
    'mismatch',
    'unpaired',
    'gone',
    'session',
    'large',
    'capacity',
    'generic',
    'none',
    'other',
  ];
  if (
    !row ||
    typeof row !== 'object' ||
    Array.isArray(row) ||
    Object.keys(row).length !== 4 ||
    !states.includes(row.state) ||
    typeof row.frame !== 'boolean' ||
    !['visible', 'hidden'].includes(row.visibility) ||
    !causes.includes(row.ended)
  )
    return;
  return { state: row.state, frame: row.frame, visibility: row.visibility, ended: row.ended };
}

// This is an independent boundary check of the app's closed own-publication diagnostic.
// Never retain an attachment path, generic error text, record key/value or unknown field.
function ownPublication(attachment) {
  const row = attachedRow(attachment, 'colab-own-publication');
  if (!row) return;
  try {
    const fields = ['root', 'kind', 'streamId', 'submittedSeq', 'admittedSeq', 'leaderReceipt'];
    const kinds = {
      threads: ['thread'],
      intents: ['ask', 'attachment-publication', 'edit-request'],
      messages: [
        'comment',
        'ask-state',
        'thread-status',
        'thread-notification',
        'proposal-decision',
      ],
      replies: ['ask-reply'],
    };
    const seq = (value) =>
      typeof value === 'string' &&
      /^(0|[1-9][0-9]{0,19})$/.test(value) &&
      BigInt(value) <= 18446744073709551615n;
    if (
      !row ||
      typeof row !== 'object' ||
      Array.isArray(row) ||
      Object.keys(row).length !== fields.length ||
      fields.some((key) => !Object.hasOwn(row, key)) ||
      typeof row.root !== 'string' ||
      !Object.hasOwn(kinds, row.root) ||
      typeof row.kind !== 'string' ||
      (row.kind !== 'unknown' && !kinds[row.root].includes(row.kind)) ||
      typeof row.streamId !== 'string' ||
      !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(row.streamId) ||
      (row.submittedSeq !== null && !seq(row.submittedSeq)) ||
      !seq(row.admittedSeq) ||
      typeof row.leaderReceipt !== 'boolean'
    )
      return;
    return {
      root: row.root,
      kind: row.kind,
      streamId: row.streamId,
      submittedSeq: row.submittedSeq,
      admittedSeq: row.admittedSeq,
      leaderReceipt: row.leaderReceipt,
    };
  } catch {
    return;
  }
}

// Public diagnostics identify the checked-out assertion, never its private values.
function outcome(spec, result) {
  const value = { status: result.status, duration: result.duration };
  if (['failed', 'timedOut', 'interrupted'].includes(result.status)) {
    const attachments = Array.isArray(result.attachments) ? result.attachments : [];
    const diagnostics = attachments
      .slice(0, 64)
      .flatMap((attachment) => {
        const diagnostic = ownPublication(attachment);
        return diagnostic ? [diagnostic] : [];
      })
      .slice(0, 8);
    if (diagnostics.length) value.ownPublications = diagnostics;
    const states = attachments
      .slice(0, 64)
      .flatMap((attachment) => {
        const state = liveState(attachment);
        return state ? [state] : [];
      })
      .slice(0, 4);
    if (states.length) value.liveStates = states;
  }
  const name = path.basename(spec.file ?? '');
  const location = result.errorLocation;
  if (
    !/^[a-z0-9-]+\.spec\.ts$/.test(name) ||
    !sources.has(name) ||
    !location ||
    location.file !== path.join(acceptance, name) ||
    !Number.isSafeInteger(location.line) ||
    location.line < 1 ||
    !Number.isSafeInteger(location.column) ||
    location.column < 1
  )
    return value;
  const timeout = /Timeout (\d+)ms exceeded/.exec(result.error?.message ?? '');
  const timeoutMs = timeout ? Number(timeout[1]) : 0;
  value.failure = {
    file: `acceptance/${name}`,
    line: location.line,
    column: location.column,
    ...(Number.isSafeInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 1800000
      ? { timeoutMs }
      : {}),
  };
  return value;
}

function visit(suite) {
  for (const spec of suite.specs ?? []) {
    for (const test of spec.tests ?? []) {
      const value = {
        title: spec.title,
        outcome: test.status,
        expectedStatus: test.expectedStatus,
        results: test.results.map((result) => {
          retainSoak(spec, result);
          return outcome(spec, result);
        }),
      };
      tests.push(value);
      if (test.status === 'unexpected') console.log(JSON.stringify(value));
    }
  }
  for (const child of suite.suites ?? []) visit(child);
}

// A closed run verdict distinguishes an interrupted/missing report from an empty successful run.
let reportState = 'missing-report';
try {
  if (input && fs.existsSync(input)) {
    const report = JSON.parse(fs.readFileSync(input, 'utf8'));
    if (!Array.isArray(report.suites)) throw new Error('Invalid report.');
    for (const suite of report.suites) visit(suite);
    reportState = tests.length ? 'complete' : 'empty-report';
    if (Array.isArray(report.errors) && report.errors.length) reportState = 'report-errors';
    if (tests.some((test) => test.results.some((result) => result.status === 'interrupted')))
      reportState = 'interrupted-report';
  }
} catch {
  reportState = 'malformed-report';
}
let executionState = 'missing-execution';
let exitCode;
try {
  const execution = JSON.parse(
    fs.readFileSync(
      path.join(process.env.RUNNER_TEMP, 'colab-acceptance-private/execution.json'),
      'utf8'
    )
  );
  if (
    ['running', 'deadline', 'interrupted', 'startup-failed', 'runner-failed'].includes(
      execution.state
    )
  )
    executionState = execution.state;
  else if (
    execution.state === 'completed' &&
    Number.isInteger(execution.exitCode) &&
    execution.exitCode >= 0 &&
    execution.exitCode <= 255
  ) {
    executionState = 'completed';
    exitCode = execution.exitCode;
  } else executionState = 'invalid-execution';
} catch {
  // Neither private parse errors nor unknown verdict fields belong in public evidence.
}
const incomplete = executionState !== 'completed' || reportState !== 'complete';
const failed = exitCode !== 0 || tests.some((test) => test.outcome === 'unexpected');
const run = {
  status: incomplete ? 'incomplete' : failed ? 'failed' : 'passed',
  reason: executionState !== 'completed' ? executionState : reportState,
};
const output = path.join(process.env.RUNNER_TEMP, 'colab-acceptance-results');
fs.mkdirSync(output, { recursive: true });
fs.writeFileSync(
  path.join(output, 'outcomes.json'),
  JSON.stringify({ run, tests }, null, 2) + '\n'
);
if (soakEvidence) {
  const diagnostics = path.join(process.env.RUNNER_TEMP, 'colab-soak-diagnostics');
  fs.mkdirSync(diagnostics, { recursive: true });
  fs.writeFileSync(path.join(diagnostics, 'soak-failure.json'), JSON.stringify(soakEvidence));
}
if (run.status === 'incomplete') console.log(JSON.stringify({ run }));
process.exitCode = run.status === 'passed' ? 0 : 1;
