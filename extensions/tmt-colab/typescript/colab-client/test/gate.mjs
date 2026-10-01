export const ENGINES = ['chromium', 'firefox', 'webkit'];
export function validateReport(report, corpus) {
  const names = corpus.map((c) => c.name);
  const controls = ['positive-normal', 'mixed-A0-R0', 'small-A-0-identity-R-zero-S-forgery'];
  if (
    names.length !== 148 ||
    new Set(names).size !== 148 ||
    corpus.filter((c) => c.nativePolicy).length !== 9 ||
    controls.some((n) => !names.includes(n))
  )
    throw new Error('Incomplete Ed25519 corpus');
  if (
    !Array.isArray(report) ||
    report.length !== 3 ||
    new Set(report.map((r) => r.engine)).size !== 3
  )
    throw new Error('Missing or duplicate engine');
  for (const name of ENGINES) {
    const result = report.find((r) => r.engine === name);
    if (
      !result ||
      result.error ||
      result.skipped ||
      !result.version ||
      !Array.isArray(result.rows) ||
      result.rows.length !== 148 ||
      result.checks !== true
    )
      throw new Error(`Unavailable or incomplete ${name}`);
    for (let i = 0; i < 148; i++) {
      const row = result.rows[i],
        expected = corpus[i];
      if (
        row.name !== expected.name ||
        row.accepted !== expected.nativePolicy ||
        typeof row.raw !== 'boolean'
      )
        throw new Error(`${name} failed ${expected.name}`);
    }
  }
}
