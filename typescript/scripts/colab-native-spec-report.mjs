import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

function cases(report) {
  const found = [];
  function visit(suite) {
    for (const spec of suite.specs ?? [])
      for (const test of spec.tests ?? []) found.push({ id: `${spec.id}:${test.projectId}`, test });
    for (const child of suite.suites ?? []) visit(child);
  }
  for (const suite of report.suites ?? []) visit(suite);
  return found;
}

/** A successful exit alone can mean every native test skipped. Require the complete listed run (served and chrome lanes). */
export function validateNativeSpecReport(list, result) {
  const expected = cases(list)
    .map((row) => row.id)
    .sort();
  const actual = cases(result);
  const ids = actual.map((row) => row.id).sort();
  if (
    expected.length === 0 ||
    new Set(expected).size !== expected.length ||
    JSON.stringify(expected) !== JSON.stringify(ids) ||
    (result.errors ?? []).length !== 0 ||
    result.stats?.expected !== expected.length ||
    result.stats?.skipped !== 0 ||
    result.stats?.unexpected !== 0 ||
    result.stats?.flaky !== 0 ||
    !actual.every(({ test }) => test.results?.length === 1 && test.results[0].status === 'passed')
  )
    throw new Error(
      'Native spec coverage must be nonempty, complete, and pass without skips or retries.'
    );
  return { expected: expected.length, passed: actual.length, skipped: 0 };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [list, result] = process.argv.slice(2);
  process.stdout.write(
    JSON.stringify(
      validateNativeSpecReport(
        JSON.parse(readFileSync(list, 'utf8')),
        JSON.parse(readFileSync(result, 'utf8'))
      )
    ) + '\n'
  );
}
