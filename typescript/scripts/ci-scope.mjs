import { createHash } from 'node:crypto';
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { e2eShardFiles } from './e2e-shards.mjs';
import { runPackedCommand } from './packed-command.mjs';

const COMPONENT_MAP = new URL('../../.github/components.json', import.meta.url);

/**
 * `**` matches any path (newlines included: git paths may contain them), `*` and `?`
 * stay inside one directory; everything else is literal.
 */
export function globToRegExp(glob) {
  let source = '';
  for (let index = 0; index < glob.length; index += 1) {
    const character = glob[index];
    if (character === '*' && glob[index + 1] === '*') {
      source += '[\\s\\S]*';
      index += 1;
    } else if (character === '*') source += '[^/]*';
    else if (character === '?') source += '[^/]';
    else source += character.replace(/[\\^$.|+(){}[\]]/g, '\\$&');
  }
  return new RegExp(`^${source}$`);
}

function nonEmptyStrings(value, label) {
  if (
    !Array.isArray(value) ||
    value.length === 0 ||
    value.some((item) => typeof item !== 'string')
  ) {
    throw new Error(`${label} must be a non-empty list of strings.`);
  }
  return value;
}

/** File names only: they reach a shell word list, so no other character is allowed. */
const TEST_FILE = /^[A-Za-z0-9._-]+$/;

function parseScopedChecks(name, checks) {
  if (checks === undefined) return undefined;
  const files = (key) => {
    const list = nonEmptyStrings(checks[key], `components.${name}.scopedChecks.${key}`);
    if (list.some((file) => !TEST_FILE.test(file))) {
      throw new Error(`components.${name}.scopedChecks.${key} must be plain file names.`);
    }
    return list;
  };
  return { nativeTests: files('nativeTests'), e2eFiles: files('e2eFiles') };
}

/**
 * Parses and validates the component map. A malformed map throws, so the
 * selector job fails visibly instead of selecting the wrong work.
 */
export function parseComponentMap(text) {
  const map = JSON.parse(text);
  const components = Object.entries(map.components ?? {}).map(([name, component]) => ({
    name,
    package: component.package,
    release: component.release,
    releaseConsumers:
      component.releaseConsumers === undefined
        ? []
        : nonEmptyStrings(component.releaseConsumers, `components.${name}.releaseConsumers`),
    owns: nonEmptyStrings(component.owns, `components.${name}.owns`),
    excludes: component.excludes ?? [],
    migrations:
      component.migrations === undefined
        ? []
        : nonEmptyStrings(component.migrations, `components.${name}.migrations`),
    selectedBy: (component.selectedBy ?? []).map((glob) => ({ glob, pattern: globToRegExp(glob) })),
    scopedChecks: parseScopedChecks(name, component.scopedChecks),
  }));
  for (const component of components) {
    if (component.release !== undefined && typeof component.release !== 'boolean')
      throw new Error(`Component ${component.name} release must be boolean.`);
  }
  if (components.length === 0) throw new Error('The component map has no components.');
  const ids = new Set();
  const rules = (map.rules ?? []).map((rule) => {
    if (typeof rule.id !== 'string' || ids.has(rule.id)) {
      throw new Error(`Rule id ${rule.id} is missing or repeated.`);
    }
    ids.add(rule.id);
    if (
      !Array.isArray(rule.consumers) ||
      rule.consumers.some((consumer) => consumer !== 'native')
    ) {
      throw new Error(`Rule ${rule.id} names an unknown consumer.`);
    }
    if (typeof rule.why !== 'string' || rule.why === '')
      throw new Error(`Rule ${rule.id} needs a reason.`);
    const paths = nonEmptyStrings(rule.paths, `Rule ${rule.id} paths`);
    return {
      id: rule.id,
      why: rule.why,
      consumers: rule.consumers,
      globs: paths,
      patterns: paths.map(globToRegExp),
    };
  });
  return {
    components,
    rules,
    digest: createHash('sha256').update(text).digest('hex').slice(0, 12),
  };
}

let defaultMap;
function componentMap() {
  defaultMap ??= parseComponentMap(readFileSync(COMPONENT_MAP, 'utf8'));
  return defaultMap;
}

const within = (root, path) => root === '.' || path === root || path.startsWith(`${root}/`);

/**
 * Whether a component is released: `release: false` parks it (release-please skips it, and the
 * release pipeline plans and publishes nothing for it). An unknown component is an error.
 */
export function isReleased(map, name) {
  const component = map.components.find((candidate) => candidate.name === name);
  if (!component) throw new Error(`Unknown component ${name}.`);
  return component.release !== false;
}

/** A `selectedBy` glob wins; otherwise the longest `owns` root the path is not excluded from. */
export function ownerOf(path, map = componentMap()) {
  const selected = map.components.find((component) =>
    component.selectedBy.some(({ pattern }) => pattern.test(path))
  );
  if (selected) return selected.name;
  let owner;
  let length = -1;
  for (const component of map.components) {
    if (component.excludes.some((root) => within(root, path))) continue;
    for (const root of component.owns) {
      if (within(root, path) && root.length > length) {
        owner = component.name;
        length = root.length;
      }
    }
  }
  return owner ?? 'unowned';
}

/**
 * What each changed path selects and why, for the run summary. The first
 * matching rule of the component map decides native work; unknown inputs keep
 * full native verification. Frozen Office verification follows ownership only,
 * with shared dependencies covered by weekly/manual runs.
 */
export function explainCiSelection(paths, map = componentMap()) {
  return paths.map((path) => {
    const rule = map.rules.find(({ patterns }) => patterns.some((pattern) => pattern.test(path)));
    const office = selectOfficeBrowser([path], map);
    return {
      path,
      owner: ownerOf(path, map),
      rule: rule?.id ?? 'unmapped',
      why:
        rule?.why ??
        'Unmapped input retains full native verification; frozen Office follows ownership.',
      native: rule ? rule.consumers.includes('native') : true,
      office,
      nativeOffice: office,
    };
  });
}

/** Deletions select their owner; an empty diff retains full native work only. */
export function selectCiAreas(paths, map = componentMap()) {
  if (paths.length === 0) return { native: true, office: false, nativeOffice: false };
  const rows = explainCiSelection(paths, map);
  return {
    native: rows.some((row) => row.native),
    office: rows.some((row) => row.office),
    nativeOffice: rows.some((row) => row.nativeOffice),
  };
}

// Office-local browser harnesses/build files are already component-owned. These
// are the browser-specific machinery inputs outside that component; shared
// dependency and generic fixture changes rely on the weekly/manual safety net.
const OFFICE_BROWSER_INPUTS = new Set([
  '.github/workflows/office-browser.yml',
  '.dockerignore',
  'typescript/scripts/verify-office-emulators.mjs',
]);

/**
 * Frozen Office PR verification follows ownership plus Office-specific machinery,
 * not shared inputs or core dependencies. Weekly/manual runs cover every partition.
 * Empty or unknown paths select no browser work; required CI stays conservative.
 */
export function selectOfficeBrowser(paths, map = componentMap()) {
  return paths.some((path) => ownerOf(path, map) === 'office' || OFFICE_BROWSER_INPUTS.has(path));
}

// Keep the advisory harness narrower than required native CI. Ownership comes
// from the component map; these are its concrete browser/model/vector inputs.
const COLAB_HARNESS_ROOTS = [
  'extensions/tmt-colab/typescript/colab-client',
  'extensions/tmt-colab/rust/tmt-colab-model',
  'extensions/tmt-colab/contracts/vectors',
];

const COLAB_HARNESS_INPUTS = new Set([
  '.github/workflows/colab-browser.yml',
  'rust/Cargo.lock',
  'rust/Cargo.toml',
  'typescript/pnpm-lock.yaml',
]);

/** Empty/unknown diffs do not select advisory work; weekly/manual runs cover shared drift. */
export function selectColabHarness(paths, map = componentMap()) {
  return paths.some((path) => {
    const owner = ownerOf(path, map);
    return (
      COLAB_HARNESS_INPUTS.has(path) ||
      ((owner === 'colab-client' || owner === 'tmt-colab') &&
        COLAB_HARNESS_ROOTS.some((root) => within(root, path)))
    );
  });
}

/**
 * How much of the native work a change needs. `none`: nothing native is selected.
 * A component name (only `squad` declares `scopedChecks`): every path that selects
 * native work is owned by that component, which cannot affect the others, so it
 * runs its own checks. `full`: anything else, and an empty diff, fails closed.
 */
export function selectNativeScope(paths, map = componentMap()) {
  if (paths.length === 0) return 'full';
  const native = explainCiSelection(paths, map).filter((row) => row.native);
  if (native.length === 0) return 'none';
  const owner = native[0].owner;
  const component = map.components.find(({ name }) => name === owner);
  return component?.scopedChecks && native.every((row) => row.owner === owner) ? owner : 'full';
}

/** The tests a scoped component runs, for the workflow to pass on as word lists. */
export function scopedChecks(scope, map = componentMap()) {
  const checks = map.components.find(({ name }) => name === scope)?.scopedChecks;
  return { nativeTests: checks?.nativeTests ?? [], e2eFiles: checks?.e2eFiles ?? [] };
}

const EVIDENCE_ROWS = 100;

/** Markdown for `$GITHUB_STEP_SUMMARY` and the log: one row per changed path. */
export function renderSelectionEvidence({
  base,
  head,
  rows,
  areas,
  digest,
  nativeScope,
  range = '...',
}) {
  const selects = (row) =>
    [row.native && 'native', row.office && 'office', row.nativeOffice && 'native_office']
      .filter(Boolean)
      .join(', ') || 'nothing';
  const lines = [
    '### CI selection',
    '',
    `Diff \`${base.slice(0, 12)}${range}${head.slice(0, 12)}\`, ${rows.length} changed path(s), component map \`sha256:${digest}\`.`,
    '',
    `Selected: native=${areas.native}, office=${areas.office}, native_office=${areas.nativeOffice}` +
      (nativeScope ? `, native scope ${nativeScope}.` : '.'),
    '',
    '| Path | Owner | Rule | Selects |',
    '| --- | --- | --- | --- |',
    ...rows
      .slice(0, EVIDENCE_ROWS)
      .map((row) => `| \`${row.path}\` | ${row.owner} | ${row.rule} | ${selects(row)} |`),
  ];
  if (rows.length > EVIDENCE_ROWS) {
    const rest = rows.slice(EVIDENCE_ROWS);
    const byRule = new Map();
    for (const row of rest) byRule.set(row.rule, (byRule.get(row.rule) ?? 0) + 1);
    lines.push(
      '',
      `${rest.length} more path(s) not listed: ${[...byRule].map(([rule, count]) => `${rule} ${count}`).join(', ')}.`
    );
  }
  return `${lines.join('\n')}\n`;
}

export function ciGatePasses(selected, results) {
  if (!['true', 'false'].includes(selected) || results.length === 0) return false;
  const expected = selected === 'true' ? 'success' : 'skipped';
  return results.every((result) => result === expected);
}

const NATIVE_JOBS = [
  'nativeRust',
  'unitTests',
  'e2eShard1',
  'e2eShard2',
  'runtimeBuild',
  'packedInstall',
  'macosRuntimeBuild',
  'macosPackedInstall',
];
const E2E_JOBS = ['e2eShard1', 'e2eShard2'];
/** Positional result order accepted by the gate-rust CLI. */
export const RUST_WORKERS = ['clippy', 'tests', 'office', 'process', 'msrv'];

/**
 * What each native job must have reported for the scope. A scoped component runs
 * its Rust checks and the first E2E shard (which holds its files); the second shard,
 * the CLI runtime builds, packed installs and tooling unit tests are skipped because
 * the CLI is unchanged. `none` skips everything. Anything unexpected, including an
 * unknown scope, fails: a selected job that was skipped, cancelled or missing is as
 * wrong as a job that ran when the selector skipped it.
 */
function expectedNativeResults(scope, map, macos = 'true') {
  if (scope === 'full') {
    return {
      ...Object.fromEntries(NATIVE_JOBS.map((job) => [job, 'success'])),
      macosRuntimeBuild: macos === 'false' ? 'skipped' : 'success',
      macosPackedInstall: macos === 'false' ? 'skipped' : 'success',
    };
  }
  if (scope === 'none') return Object.fromEntries(NATIVE_JOBS.map((job) => [job, 'skipped']));
  if (!map.components.some((component) => component.name === scope && component.scopedChecks)) {
    return undefined;
  }
  return {
    nativeRust: 'success',
    unitTests: 'skipped',
    e2eShard1: 'success',
    e2eShard2: 'skipped',
    runtimeBuild: 'skipped',
    packedInstall: 'skipped',
    macosRuntimeBuild: 'skipped',
    macosPackedInstall: 'skipped',
  };
}

function expectedRustResults(scope, map, officeSelected) {
  const native = expectedNativeResults(scope, map)?.nativeRust;
  if (!native) return undefined;
  return {
    ...Object.fromEntries(RUST_WORKERS.map((worker) => [worker, native])),
    office: scope === 'full' && officeSelected === 'true' ? native : 'skipped',
  };
}

function gatePasses(jobs, scope, results, map, macos) {
  const expected = expectedNativeResults(scope, map, macos);
  if (!expected || jobs.some((job) => typeof results?.[job] !== 'string')) return false;
  return jobs.every((job) => results[job] === expected[job]);
}

/** `Native package matrix`: every native job, the two E2E shards included. */
export function nativeGatePasses(scope, results, macos, map = componentMap()) {
  if (!['true', 'false'].includes(macos)) return false;
  return gatePasses(NATIVE_JOBS, scope, results, map, macos);
}

/** `Docker E2E`: the two shards alone, with the same expectations. */
export function e2eGatePasses(scope, results, map = componentMap()) {
  return gatePasses(E2E_JOBS, scope, results, map);
}

/** `Native Rust contracts`: clippy, tests, Office feature, native fixtures and MSRV. */
export function rustGatePasses(scope, results, officeSelected, map = componentMap()) {
  if (!['true', 'false'].includes(officeSelected)) return false;
  const expected = expectedRustResults(scope, map, officeSelected);
  if (!expected || results?.length !== RUST_WORKERS.length) return false;
  const byWorker = Object.fromEntries(
    RUST_WORKERS.map((worker, index) => [worker, results[index]])
  );
  return RUST_WORKERS.every((worker) => byWorker[worker] === expected[worker]);
}

export function readChangedCiSelection(base, head, cwd, range = '...') {
  if (!['..', '...'].includes(range)) throw new Error('Unknown diff range.');
  if ([base, head].some((sha) => !/^[a-f0-9]{40}$/.test(sha ?? ''))) {
    throw new Error('Expected exact base and head commit SHAs.');
  }
  const changed = runPackedCommand(
    'git',
    ['diff', '--no-renames', '--name-only', '-z', `${base}${range}${head}`, '--'],
    { cwd, env: process.env }
  );
  const paths = changed.split('\0').filter(Boolean);
  const map = componentMap();
  return {
    paths,
    rows: explainCiSelection(paths, map),
    areas: selectCiAreas(paths, map),
    nativeScope: selectNativeScope(paths, map),
    digest: map.digest,
  };
}

export function readChangedCiAreas(base, head, cwd) {
  return readChangedCiSelection(base, head, cwd).areas;
}

/**
 * Standard output is the step's `$GITHUB_OUTPUT`, so it carries only the
 * selection outputs; the evidence table goes to standard error and the step summary.
 */
export function runCiScope(args, { cwd, stdout, stderr, summaryFile }) {
  if (args[0] === 'gate') {
    if (!ciGatePasses(args[1], args.slice(2))) {
      throw new Error(
        'Selected CI work did not complete successfully, or skip evidence is invalid.'
      );
    }
    return;
  }
  if (args[0] === 'gate-rust') {
    const [, scope, officeSelected, ...results] = args;
    if (!rustGatePasses(scope, results, officeSelected)) {
      throw new Error(
        'Selected native Rust workers did not complete successfully, or skip evidence is invalid.'
      );
    }
    return;
  }
  if (args[0] === 'gate-e2e') {
    const [, scope, ...values] = args;
    const results = Object.fromEntries(E2E_JOBS.map((job, index) => [job, values[index]]));
    if (values.length !== E2E_JOBS.length || !e2eGatePasses(scope, results)) {
      throw new Error(
        'Selected Docker E2E shards did not complete successfully, or skip evidence is invalid.'
      );
    }
    return;
  }
  if (args[0] === 'gate-native') {
    const [, macos, scope, ...values] = args;
    const results = Object.fromEntries(NATIVE_JOBS.map((job, index) => [job, values[index]]));
    if (values.length !== NATIVE_JOBS.length || !nativeGatePasses(scope, results, macos)) {
      throw new Error(
        'Selected native CI work did not complete successfully, or skip evidence is invalid.'
      );
    }
    return;
  }
  const full = args.length === 1 && args[0] === 'full';
  const seed = args.length === 1 && args[0] === 'seed';
  const queue = args[0] === 'merge-group';
  if (!full && !seed && !queue && args.length !== 2) {
    throw new Error('Expected exact base and head commit SHAs.');
  }
  let [base, head] = queue ? [undefined, args[1]] : args;
  const range = queue ? '..' : '...';
  let fallback;
  let selection;
  if (!full && !seed) {
    try {
      if (queue) {
        if (args.length !== 2 || !/^[a-f0-9]{40}$/.test(head ?? '')) {
          throw new Error('Expected an exact merge-group head SHA.');
        }
        // HEADGREEN may set the event base to a preceding, still-pending queue commit.
        // Anchor at fetched main so a later prose-only tip retains earlier native work.
        base = runPackedCommand('git', ['merge-base', '--all', 'refs/remotes/origin/main', head], {
          cwd,
          env: process.env,
        }).trim();
        // Multiple merge bases are ambiguous; readChangedCiSelection rejects them.
      }
      selection = readChangedCiSelection(base, head, cwd, range);
      if (queue && selection.paths.length === 0) {
        fallback = 'Merge-group diff is empty; using full verification.';
        selection = null;
      }
    } catch (error) {
      if (!queue) throw error;
      fallback = `Merge-group diff unreadable; using full verification. ${error.message}`;
    }
  }
  selection ??= { paths: [], areas: selectCiAreas([]), nativeScope: selectNativeScope([]) };
  if (full) selection.areas = { native: true, office: true, nativeOffice: true };
  if (queue) {
    selection.areas = { ...selection.areas, office: false, nativeOffice: false };
    selection.rows = selection.rows?.map((row) => ({ ...row, office: false, nativeOffice: false }));
  }
  const officeBrowser = full || (!queue && !seed && selectOfficeBrowser(selection.paths));
  const colabHarness = !queue && !seed && !full && selectColabHarness(selection.paths);
  const evidence =
    (full || seed || fallback
      ? `### CI selection\n\n${fallback ?? (full ? 'Weekly/manual full verification; no path filtering.' : 'Main cache seed; Office verification is frozen.')}\n`
      : renderSelectionEvidence({ base, head, range, ...selection })) +
    `\nOffice browser PR selection (Office ownership or verification machinery): ${officeBrowser}.\n` +
    `\nColab browser PR selection (client, model, vectors or harness workflow): ${colabHarness}.\n`;
  stderr.write(evidence);
  if (summaryFile) appendFileSync(summaryFile, evidence);
  const { areas, nativeScope } = selection;
  const checks = scopedChecks(nativeScope);
  const [firstShard, secondShard] = e2eShardFiles(nativeScope, checks.e2eFiles);
  stdout.write(
    `native=${areas.native}\noffice=${areas.office}\nnative_office=${areas.nativeOffice}\n` +
      `office_browser=${officeBrowser}\n` +
      `colab_harness=${colabHarness}\n` +
      `native_scope=${nativeScope}\n` +
      `scoped_native_tests=${checks.nativeTests.join(' ')}\n` +
      `e2e_shard_1=${firstShard.join(' ')}\n` +
      `e2e_shard_2=${secondShard.join(' ')}\n`
  );
}

function main(args) {
  runCiScope(args, {
    cwd: fileURLToPath(new URL('../../', import.meta.url)),
    stdout: process.stdout,
    stderr: process.stderr,
    summaryFile: process.env.GITHUB_STEP_SUMMARY,
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
