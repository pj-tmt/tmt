import { expect, it } from 'vite-plus/test';

const { validateNativeSpecReport } = await import(
  new URL('../../scripts/colab-native-spec-report.mjs', import.meta.url).href
);
const report = () => ({
  suites: [
    {
      suites: [
        {
          specs: [
            { id: 'served-1', tests: [{ projectId: 'chromium', results: [{ status: 'passed' }] }] },
          ],
        },
      ],
    },
  ],
  errors: [] as { message: string }[],
  stats: { expected: 1, skipped: 0, unexpected: 0, flaky: 0 },
});

it('requires every listed served case to run once and pass', () => {
  expect(validateNativeSpecReport(report(), report())).toEqual({
    expected: 1,
    passed: 1,
    skipped: 0,
  });
});

it.each(['empty', 'missing', 'skipped', 'failed', 'retried', 'error', 'unexpected', 'duplicate'])(
  'refuses %s evidence even when the runner exits successfully',
  (kind) => {
    const list = report();
    const result = report();
    const test = result.suites[0].suites[0].specs[0].tests[0];
    if (kind === 'empty') list.suites = [];
    if (kind === 'missing') result.suites = [];
    if (kind === 'skipped') {
      test.results[0].status = 'skipped';
      result.stats.skipped = 1;
    }
    if (kind === 'failed') test.results[0].status = 'failed';
    if (kind === 'retried') test.results.push({ status: 'passed' });
    if (kind === 'error') result.errors.push({ message: 'Fixture suite error' });
    if (kind === 'unexpected') result.stats.unexpected = 1;
    if (kind === 'duplicate') result.suites.push(result.suites[0]);
    expect(() => validateNativeSpecReport(list, result)).toThrow(/without skips or retries/);
  }
);
