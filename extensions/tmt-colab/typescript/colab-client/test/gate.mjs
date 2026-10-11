import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
export const ENGINES = ['chromium', 'firefox', 'webkit'];
function requireEngines(engines) {
  if (
    !Array.isArray(engines) ||
    engines.length === 0 ||
    new Set(engines).size !== engines.length ||
    engines.some((engine) => !ENGINES.includes(engine))
  )
    throw new Error('Expected a nonempty, unique set of known engines: chromium,firefox,webkit');
  return engines;
}

export function parseEngines(args) {
  if (args.length === 0) return [...ENGINES];
  if (args.length !== 2 || args[0] !== '--engines')
    throw new Error('Usage: differential.mjs [--engines chromium,firefox,webkit]');
  return requireEngines(args[1].split(','));
}

/** The normal build of the `@tmt/colab-app` package: the refusal proof serves its entry, never a synthetic notice. */
export const APP_DIST = new URL('../../app/dist/', import.meta.url);

/** Fails up front, naming the expected path, when the refusal proof runs without the built app. */
export function requireBuiltApp(dist = APP_DIST) {
  const entry = fileURLToPath(new URL('index.html', dist));
  if (!existsSync(entry))
    throw new Error(
      `The refusal proof needs the built Colab app, but ${entry} is missing. ` +
        'Build @tmt/colab-app first: corepack pnpm --filter @tmt/colab-app build',
    );
  return dist;
}

/** Refusal is explicit and limited to the divergent Linux WebKit backend. */
export function parseRefused(value, engines, platform) {
  if (!value) return [];
  const refused = value.split(',');
  requireEngines(refused);
  if (platform !== 'linux' || refused.some((name) => name !== 'webkit' || !engines.includes(name)))
    throw new Error('Expected-refused is only supported for selected Linux WebKit');
  return refused;
}

export function validateRefusal(result) {
  if (
    result.diagnostics?.cleanup !== 'closed' ||
    result.error ||
    result.skipped ||
    !result.version ||
    result.probeRefused !== true ||
    !Number.isInteger(result.policyMismatches) ||
    result.policyMismatches < 1 ||
    result.notices !== 1 ||
    result.heading !== 'Update your browser' ||
    result.body !==
      'This browser cannot safely verify Colab signatures. Update it or open Colab in another supported browser.' ||
    result.sdkRequests !== 0 ||
    result.iframes !== 0 ||
    result.actions !== 0
  )
    throw new Error('Incomplete expected browser refusal');
}

export function validateExpectedReport(report, corpus, engines, refused) {
  requireEngines(engines);
  if (
    !Array.isArray(report) ||
    report.length !== engines.length ||
    new Set(report.map((r) => r.engine)).size !== engines.length ||
    report.some((r) => !engines.includes(r.engine))
  )
    throw new Error('Missing or duplicate engine');
  for (const result of report.filter((r) => refused.includes(r.engine))) validateRefusal(result);
  const strict = engines.filter((name) => !refused.includes(name));
  if (strict.length)
    validateReport(
      report.filter((r) => strict.includes(r.engine)),
      corpus,
      strict,
    );
}

export function validateReport(report, corpus, engines = ENGINES) {
  requireEngines(engines);
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
    report.length !== engines.length ||
    new Set(report.map((r) => r.engine)).size !== engines.length
  )
    throw new Error('Missing or duplicate engine');
  for (const name of engines) {
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
      if (typeof row.rawProbed !== 'boolean')
        throw new Error(`${name} missing rawProbed boolean for ${expected.name}`);
      if (expected.nativePolicy && (!row.rawProbed || row.raw !== true))
        throw new Error(`${name} missing raw positive control for ${expected.name}`);
      if (!row.rawProbed && (row.accepted !== false || row.raw !== false))
        throw new Error(`${name} guard-rejected raw probe must refuse ${expected.name}`);
      if (
        row.name !== expected.name ||
        row.accepted !== expected.nativePolicy ||
        typeof row.raw !== 'boolean'
      )
        throw new Error(`${name} failed ${expected.name}`);
    }
  }
}
