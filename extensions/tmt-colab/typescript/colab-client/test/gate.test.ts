import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
const corpus = readFileSync(
  new URL('../../../contracts/vectors/ed25519-829.jsonl', import.meta.url),
  'utf8',
)
  .trim()
  .split('\n')
  .map((line) => JSON.parse(line));
const reports = ['chromium', 'firefox', 'webkit'].map((engine) => ({
  engine,
  version: 'fixture',
  checks: true,
  rows: corpus.map((v) => ({
    name: v.name,
    accepted: v.nativePolicy,
    rawProbed: v.nativePolicy,
    raw: v.nativePolicy,
  })),
}));
function run(
  report: unknown,
  cases: unknown = corpus,
  engines?: string[],
  failure?: RegExp,
): number | null {
  const script = `import {validateReport} from ${JSON.stringify(new URL('./gate.mjs', import.meta.url).href)};validateReport(${JSON.stringify(report)},${JSON.stringify(cases)},${JSON.stringify(engines)});`;
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', script], {
    timeout: 10000,
    encoding: 'utf8',
  });
  if (failure) expect(result.stderr).toMatch(failure);
  return result.status;
}
describe('differential report gate', () => {
  it('accepts complete controls and exact row correspondence', () => expect(run(reports)).toBe(0));
  it('requires a rawProbed boolean for every row', () => {
    for (const rawProbed of [undefined, null, 0, 'true'])
      expect(
        run(
          reports.map((r) => ({
            ...r,
            rows: r.rows.map((row, i) => (i === 1 ? { ...row, rawProbed } : row)),
          })),
          corpus,
          undefined,
          /missing rawProbed boolean/,
        ),
      ).toBe(1);
  });
  it('requires every native positive to be raw-probed and accepted by WebCrypto', () => {
    for (const control of corpus.filter((v) => v.nativePolicy))
      for (const change of [{ rawProbed: false }, { raw: false }])
        expect(
          run(
            reports.map((r) => ({
              ...r,
              rows: r.rows.map((row) => (row.name === control.name ? { ...row, ...change } : row)),
            })),
            corpus,
            undefined,
            /missing raw positive control/,
          ),
        ).toBe(1);
  });
  it('refuses admission or a raw result when the point guard prevented probing', () => {
    for (const change of [{ accepted: true }, { raw: true }])
      expect(
        run(
          reports.map((r) => ({
            ...r,
            rows: r.rows.map((row, i) => (i === 1 ? { ...row, ...change } : row)),
          })),
          corpus,
          undefined,
          /guard-rejected raw probe must refuse/,
        ),
      ).toBe(1);
  });
  it('exits nonzero for missing, skipped, unavailable, duplicate or incomplete engines', () => {
    const mutations = [
      reports.slice(0, 2),
      [...reports, reports[0]],
      reports.map((r, i) => (i === 1 ? { ...r, error: 'unavailable' } : r)),
      reports.map((r, i) => (i === 1 ? { ...r, skipped: true } : r)),
      reports.map((r, i) => (i === 1 ? { ...r, rows: r.rows.slice(1) } : r)),
      reports.map((r, i) => (i === 1 ? { ...r, checks: false } : r)),
    ];
    for (const report of mutations) expect(run(report)).toBe(1);
  });
  it('rejects missing controls, wrong corpus counts and wrong admissions', () => {
    expect(run(reports, corpus.slice(1))).toBe(1);
    expect(
      run(
        reports,
        corpus.map((v) => (v.name === 'positive-normal' ? { ...v, name: 'missing-control' } : v)),
      ),
    ).toBe(1);
    expect(
      run(
        reports.map((r, i) =>
          i === 1
            ? { ...r, rows: r.rows.map((v, n) => (n === 0 ? { ...v, accepted: !v.accepted } : v)) }
            : r,
        ),
      ),
    ).toBe(1);
  });
});

describe('explicit engine selection', () => {
  const parse = (args: string[]) =>
    spawnSync(
      process.execPath,
      [
        '--input-type=module',
        '-e',
        `import {parseEngines} from ${JSON.stringify(new URL('./gate.mjs', import.meta.url).href)};console.log(JSON.stringify(parseEngines(${JSON.stringify(args)})));`,
      ],
      { encoding: 'utf8', timeout: 10000 },
    );

  it('defaults to all three and accepts explicit known sets', () => {
    expect(JSON.parse(parse([]).stdout)).toEqual(['chromium', 'firefox', 'webkit']);
    expect(JSON.parse(parse(['--engines', 'chromium']).stdout)).toEqual(['chromium']);
    expect(JSON.parse(parse(['--engines', 'webkit,firefox']).stdout)).toEqual([
      'webkit',
      'firefox',
    ]);
  });

  it('rejects unknown, empty and duplicate sets before starting the harness', () => {
    for (const args of [
      ['--engines', 'unknown'],
      ['--engines', ''],
      ['--engines', 'chromium,chromium'],
      ['--engines'],
    ]) {
      const result = spawnSync(
        process.execPath,
        [fileURLToPath(new URL('./differential.mjs', import.meta.url)), ...args],
        { encoding: 'utf8', timeout: 10000 },
      );
      expect(result.error).toBeUndefined();
      expect(result.status).toBe(1);
      expect(result.stderr).toMatch(/Expected a nonempty|Usage: differential/);
    }
  });

  it('requires exactly the selected engines without weakening rows or availability', () => {
    expect(run([reports[0]], corpus, ['chromium'])).toBe(0);
    expect(run([reports[0]])).toBe(1); // Chromium alone cannot satisfy the default L1 gate.
    expect(run(reports, corpus, ['chromium'])).toBe(1);
    for (const report of [
      [],
      [{ ...reports[0], skipped: true }],
      [{ ...reports[0], error: 'unavailable' }],
      [{ ...reports[0], rows: [] }],
    ])
      expect(run(report, corpus, ['chromium'])).toBe(1);
    expect(run([reports[0]], corpus, [])).toBe(1);
    expect(run([reports[0]], corpus, ['unknown'])).toBe(1);
  });
});

describe('explicit Linux WebKit refusal', () => {
  const call = (source: string) =>
    spawnSync(
      process.execPath,
      [
        '--input-type=module',
        '-e',
        `import {parseRefused,validateRefusal,validateExpectedReport} from ${JSON.stringify(new URL('./gate.mjs', import.meta.url).href)};${source}`,
      ],
      { encoding: 'utf8', timeout: 10000 },
    );
  const refusal = {
    version: 'fixture',
    diagnostics: { cleanup: 'closed' },
    probeRefused: true,
    policyMismatches: 8,
    notices: 1,
    heading: 'Update your browser',
    body: 'This browser cannot safely verify Colab signatures. Update it or open Colab in another supported browser.',
    sdkRequests: 0,
    iframes: 0,
    actions: 0,
  };
  it('keeps strict default and admits only explicit selected Linux WebKit', () => {
    expect(call("parseRefused(undefined,['webkit'],'darwin')").status).toBe(0);
    expect(call("parseRefused('webkit',['webkit'],'linux')").status).toBe(0);
    for (const args of [
      ['webkit', ['webkit'], 'darwin'],
      ['firefox', ['firefox'], 'linux'],
      ['webkit', ['chromium'], 'linux'],
      ['webkit,webkit', ['webkit'], 'linux'],
      ['unknown', ['webkit'], 'linux'],
    ])
      expect(call(`parseRefused(...${JSON.stringify(args)})`).status).toBe(1);
  });
  it('cannot omit or duplicate an expected-refused engine', () => {
    const report = [...reports.slice(0, 2), { ...refusal, engine: 'webkit' }];
    const verify = (result: unknown) =>
      call(
        `validateExpectedReport(${JSON.stringify(result)},${JSON.stringify(corpus)},['chromium','firefox','webkit'],['webkit'])`,
      );
    expect(verify(report).status).toBe(0);
    expect(verify(report.slice(0, 2)).status).toBe(1);
    expect(verify([...report, report[2]]).status).toBe(1);
    expect(verify(reports).status).toBe(1);
    expect(run(report)).toBe(1); // The same refusal cannot satisfy default strict mode.
  });
  it('requires real policy refusal and one notice without activation or actions', () => {
    expect(call(`validateRefusal(${JSON.stringify(refusal)})`).status).toBe(0);
    for (const change of [
      { diagnostics: { cleanup: 'failed' } },
      { probeRefused: false },
      { policyMismatches: 0 },
      { policyMismatches: null },
      { notices: 0 },
      { notices: 2 },
      { heading: 'wrong' },
      { body: 'wrong' },
      { sdkRequests: 1 },
      { iframes: 1 },
      { actions: 1 },
      { error: 'crashed' },
      { skipped: true },
      { version: '' },
    ])
      expect(call(`validateRefusal(${JSON.stringify({ ...refusal, ...change })})`).status).toBe(1);
  });
});
