import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { describe, expect, it } from 'vitest';
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
function run(report: unknown, cases: unknown = corpus): number | null {
  const script = `import {validateReport} from ${JSON.stringify(new URL('./gate.mjs', import.meta.url).href)};validateReport(${JSON.stringify(report)},${JSON.stringify(cases)});`;
  return spawnSync(process.execPath, ['--input-type=module', '-e', script], {
    timeout: 10000,
    encoding: 'utf8',
  }).status;
}
describe('three-engine differential report gate', () => {
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
