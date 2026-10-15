import fs from 'node:fs';
import path from 'node:path';

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

// This is an independent boundary check of the app's closed own-publication diagnostic.
// Never retain an attachment path, generic error text, record key/value or unknown field.
function ownPublication(attachment) {
  if (
    !attachment ||
    typeof attachment !== 'object' ||
    attachment.name !== 'colab-own-publication' ||
    attachment.contentType !== 'application/json' ||
    typeof attachment.body !== 'string' ||
    attachment.body.length > 1024
  )
    return;
  try {
    const bytes = Buffer.from(attachment.body, 'base64');
    if (bytes.length > 512 || bytes.toString('base64') !== attachment.body) return;
    const row = JSON.parse(bytes.toString('utf8'));
    const fields = ['root', 'kind', 'streamId', 'submittedSeq', 'admittedSeq', 'leaderReceipt'];
    const kinds = {
      threads: ['thread'],
      intents: ['ask', 'attachment-publication'],
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
        results: test.results.map((result) => outcome(spec, result)),
      };
      tests.push(value);
      if (test.status === 'unexpected') console.log(JSON.stringify(value));
    }
  }
  for (const child of suite.suites ?? []) visit(child);
}

if (input && fs.existsSync(input)) {
  const report = JSON.parse(fs.readFileSync(input, 'utf8'));
  for (const suite of report.suites) visit(suite);
}
const output = path.join(process.env.RUNNER_TEMP, 'colab-acceptance-results');
fs.mkdirSync(output, { recursive: true });
fs.writeFileSync(path.join(output, 'outcomes.json'), JSON.stringify({ tests }, null, 2) + '\n');
