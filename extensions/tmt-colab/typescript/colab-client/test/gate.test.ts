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
  rows: corpus.map((v) => ({ name: v.name, accepted: v.nativePolicy, raw: v.nativePolicy })),
}));
function run(report: unknown, cases: unknown = corpus, engines?: string[]): number | null {
  const script = `import {validateReport} from ${JSON.stringify(new URL('./gate.mjs', import.meta.url).href)};validateReport(${JSON.stringify(report)},${JSON.stringify(cases)},${JSON.stringify(engines)});`;
  return spawnSync(process.execPath, ['--input-type=module', '-e', script], {
    timeout: 10000,
    encoding: 'utf8',
  }).status;
}
describe('differential report gate', () => {
  it('accepts complete controls and exact row correspondence', () => expect(run(reports)).toBe(0));
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
