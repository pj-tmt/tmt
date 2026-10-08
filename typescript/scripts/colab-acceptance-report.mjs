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

// Public diagnostics identify the checked-out assertion, never its private values.
function outcome(spec, result) {
  const value = { status: result.status, duration: result.duration };
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
