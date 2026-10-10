import { createHash } from 'node:crypto';
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { e2eShardFiles } from './e2e-shards.mjs';
import { runPackedCommand } from './packed-command.mjs';
import {
  componentOfProduct,
  isComponentRetired,
  isProductRetired,
  productOfComponent,
  predecessorOfProduct,
  releasePolicy,
} from './native-release-policy.mjs';

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

function literalPath(value, label) {
  if (
    typeof value !== 'string' ||
    !value ||
    ['\\', '*', '?', '[', ']', ':'].some((symbol) => value.includes(symbol)) ||
    [...value].some(
      (character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127
    ) ||
    value.split('/').some((part) => !part || part === '.' || part === '..')
  )
    throw new Error(`${label} must be a normalized relative literal path.`);
  return value;
}

function reason(value, label) {
  if (typeof value !== 'string' || !value.trim()) throw new Error(`${label} needs a reason.`);
  return value;
}

function declarationList(value, label, parse) {
  if (value === undefined) return [];
  if (!Array.isArray(value) || !value.length) throw new Error(`${label} must be a non-empty list.`);
  return value.map((entry) => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry))
      throw new Error(`${label} needs declaration objects.`);
    return parse(entry);
  });
}

function parseNeverShipped(name, value) {
  const label = `components.${name}.neverShippedPaths`;
  return declarationList(value, label, (entry) => ({
    root: literalPath(entry.root, label),
    reason: reason(entry.reason, label),
    testOnlyReferences: declarationList(entry.testOnlyReferences, label, (reference) => ({
      file: literalPath(reference.file, label),
      reason: reason(reference.reason, label),
    })),
  }));
}

function parseGenerated(name, value) {
  const label = `components.${name}.generatedInputs`;
  const entries = declarationList(value, label, (entry) => {
    const result = Object.fromEntries(
      [
        'includeSite',
        'generator',
        'buildScript',
        'inputDirectory',
        'packageRoot',
        'releaseScript',
      ].map((key) => [key, literalPath(entry[key], `${label}.${key}`)])
    );
    if (typeof entry.variable !== 'string' || !/^[A-Z][A-Z0-9_]*$/.test(entry.variable))
      throw new Error(`${label} needs a literal environment variable name.`);
    if (typeof entry.expression !== 'string' || !entry.expression.trim())
      throw new Error(`${label} needs its exact include expression.`);
    if (typeof entry.generatorBlob !== 'string' || !/^[a-f0-9]{40}$/.test(entry.generatorBlob))
      throw new Error(`${label} needs the reviewed generator Git blob.`);
    return {
      ...result,
      variable: entry.variable,
      expression: entry.expression,
      generatorBlob: entry.generatorBlob,
      reason: reason(entry.reason, label),
    };
  });
  if (
    entries.length > 2 ||
    entries.some((entry, index) => index > 0 && entries[index - 1].variable >= entry.variable) ||
    new Set(entries.map((entry) => entry.generator)).size !== entries.length ||
    new Set(entries.map((entry) => entry.includeSite)).size !== entries.length
  )
    throw new Error(
      `${label} permits at most two canonical generators in ascending variable order.`
    );
  return entries;
}

/**
 * Parses and validates the component map. A malformed map throws, so the
 * selector job fails visibly instead of selecting the wrong work.
 * historical preserves activation fields from immutable source snapshots before later product retirement.
 */
export function parseComponentMap(text, { historical = false } = {}) {
  const map = JSON.parse(text);
  const components = Object.entries(map.components ?? {}).map(([name, component]) => ({
    name,
    package: component.package,
    skills: component.skills,
    release: component.release,
    releaseStatus: component.releaseStatus,
    bootstrapSha: component.bootstrapSha,
    initialVersion: component.initialVersion,
    requiresCliSha: component.requiresCliSha,
    predecessor: component.predecessor,
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
    neverShippedPaths: parseNeverShipped(name, component.neverShippedPaths),
    generatedInputs: parseGenerated(name, component.generatedInputs),
  }));
  for (const component of components) {
    if (
      component.bootstrapSha !== undefined &&
      (typeof component.bootstrapSha !== 'string' || !/^[a-f0-9]{40}$/.test(component.bootstrapSha))
    )
      throw new Error(`Component ${component.name} bootstrapSha must be a commit SHA.`);
    if (
      component.requiresCliSha !== undefined &&
      (typeof component.requiresCliSha !== 'string' ||
        !/^[a-f0-9]{40}$/.test(component.requiresCliSha) ||
        !component.package)
    )
      throw new Error(
        `Component ${component.name} requiresCliSha must be a registration commit SHA for a package.`
      );
    if (component.initialVersion !== undefined) {
      const alpha = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-alpha\.(0|[1-9]\d*)$/.exec(
        component.initialVersion
      );
      if (
        !component.package ||
        !component.bootstrapSha ||
        !alpha ||
        alpha.slice(1).some((part) => !Number.isSafeInteger(Number(part))) ||
        Number(alpha[4]) >= Number.MAX_SAFE_INTEGER
      )
        throw new Error(
          `Component ${component.name} initialVersion needs a package, bootstrapSha and canonical bounded alpha version.`
        );
    }
    if (
      component.skills !== undefined &&
      (typeof component.skills !== 'boolean' || (component.skills && !component.package))
    )
      throw new Error(`Component ${component.name} skills must be boolean and needs a package.`);
    if (component.release !== undefined && typeof component.release !== 'boolean')
      throw new Error(`Component ${component.name} release must be boolean.`);
    if (
      component.releaseStatus !== undefined &&
      (component.release !== false || !['never', 'parked'].includes(component.releaseStatus))
    )
      throw new Error(`Invalid releaseStatus of ${component.name}.`);
    if (component.releaseStatus === 'never' && component.releaseConsumers.length)
      throw new Error(`Never-shipped component ${component.name} cannot have release consumers.`);
    for (const name of component.releaseConsumers) {
      const consumer = components.find((candidate) => candidate.name === name);
      if (component.release !== false || !consumer?.package || consumer.releaseStatus === 'never')
        throw new Error(`Invalid release consumer ${name} of ${component.name}.`);
    }
  }
  const productMap = { components };
  for (const component of components) {
    if (!historical && isComponentRetired(component.name) && component.release !== false)
      throw new Error(`Retired component ${component.name} must declare release: false.`);
    if (component.predecessor === undefined) continue;
    if (!component.package || typeof component.predecessor !== 'string' || !component.predecessor)
      throw new Error(`Component ${component.name} predecessor needs a package and product key.`);
    const product = productOfComponent(component.name);
    componentOfProduct(productMap, product);
    releasePolicy(component.predecessor);
    if (component.predecessor === product)
      throw new Error(`Component ${component.name} cannot be its own predecessor.`);
    if (!isProductRetired(component.predecessor)) {
      const predecessor = componentOfProduct(productMap, component.predecessor);
      if (!predecessor.package || predecessor.release === false)
        throw new Error(
          `Predecessor ${component.predecessor} must be a released or retired product.`
        );
    }
    const visited = new Set([product]);
    let predecessor = component.predecessor;
    while (predecessor !== undefined) {
      if (visited.has(predecessor)) throw new Error(`Predecessor cycle for ${component.name}.`);
      visited.add(predecessor);
      predecessor = predecessorOfProduct(productMap, predecessor);
    }
  }
  if (components.length === 0) throw new Error('The component map has no components.');
  const declarationMap = { components };
  const roots = [];
  for (const component of components) {
    if (
      (component.neverShippedPaths.length || component.generatedInputs.length) &&
      !component.package
    )
      throw new Error(`Component ${component.name} declarations need a Cargo package.`);
    for (const declaration of component.neverShippedPaths) {
      if (ownerOf(declaration.root, declarationMap) !== component.name)
        throw new Error(`Never-shipped root ${declaration.root} belongs to another component.`);
      for (const candidate of components) {
        for (const root of candidate.owns) {
          if (
            root !== '.' &&
            within(declaration.root, root) &&
            ownerOf(root, declarationMap) !== component.name
          )
            throw new Error(`Never-shipped root ${declaration.root} contains another component.`);
        }
      }
      if (roots.some((root) => within(root, declaration.root) || within(declaration.root, root)))
        throw new Error(`Overlapping never-shipped root ${declaration.root}.`);
      roots.push(declaration.root);
      const files = declaration.testOnlyReferences.map(({ file }) => file);
      if (
        new Set(files).size !== files.length ||
        files.some((file) => within(declaration.root, file))
      )
        throw new Error(`Invalid test-only references for ${declaration.root}.`);
    }
  }
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
/** The checked-in component map, parsed once. */
export function componentMap() {
  defaultMap ??= parseComponentMap(readFileSync(COMPONENT_MAP, 'utf8'));
  return defaultMap;
}

const within = (root, path) => root === '.' || path === root || path.startsWith(`${root}/`);

/** Released roots plus Cargo normal/build workspace closure; CI selection remains independent. */
export function releasedComponentsForPath(path, map = componentMap(), workspace) {
  return map.components.filter(
    (component) =>
      component.package &&
      component.release !== false &&
      ((component.owns.some((root) => within(root, path)) &&
        !component.excludes.some((root) => within(root, path))) ||
        workspace
          ?.closure(component.package, ['normal', 'build'])
          .some((crate) => within(crate.dir, path)))
  );
}

/** Names of released components a path changes: owned or closure roots plus declared private-leaf consumers. */
export function releasedComponentNamesOfPath(path, map, workspace) {
  const names = new Set(
    releasedComponentsForPath(path, map, workspace).map((component) => component.name)
  );
  const owner = map.components.find((component) => component.name === ownerOf(path, map));
  for (const consumer of owner?.releaseConsumers ?? []) names.add(consumer);
  return names;
}

/**
 * Whether a component is released: `release: false` keeps it out of automatic cuts and
 * publication. An unknown component is an error.
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
 * full native verification. Office product verification is retired independently
 * of ownership and release attribution.
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
        'Unmapped input retains full native verification; Office product checks are retired.',
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

/** Changes to dependency inventories and notice-generation inputs require release notices. */
export function selectNativeNotices(paths) {
  const inputs = new Set([
    'rust/Cargo.lock',
    'rust/rust-toolchain.toml',
    'rust/about.toml',
    'rust/about.hbs',
    'dist-workspace.toml',
    '.github/components.json',
    '.github/workflows/ci.yml',
    'scripts/build-native-artifact.sh',
    'scripts/native-cargo.sh',
    'typescript/scripts/verify-native-notices.mjs',
    'typescript/scripts/native-artifact-policy.mjs',
    'typescript/scripts/native-release-policy.mjs',
    'typescript/scripts/packed-command.mjs',
    'typescript/scripts/ci-scope.mjs',
  ]);
  return (
    paths.length === 0 ||
    paths.some(
      (path) =>
        inputs.has(path) ||
        path.startsWith('rust/licenses/') ||
        /^(rust|extensions)\/(?:.*\/)?Cargo\.toml$/.test(path)
    )
  );
}

/** Office product verification is retired for every event; ownership and release attribution remain. */
export function selectOfficeBrowser(_paths, _map = componentMap()) {
  return false;
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

// The app component suite renders the app in Chromium against its own dev server, so it
// reads the app, the client and design system it imports, and the lockfile; it needs no
// Rust. Native-fixture specs skip there and stay in the weekly/manual acceptance.
const COLAB_APP_OWNERS = new Set(['colab-app', 'colab-client', 'browser-ui']);

const COLAB_APP_ROOTS = [
  'extensions/tmt-colab/typescript/app',
  'extensions/tmt-colab/typescript/colab-client',
  'design/browser-ui',
];

const COLAB_APP_INPUTS = new Set([
  '.github/workflows/colab-browser.yml',
  'typescript/pnpm-lock.yaml',
]);

/** Empty/unknown diffs do not select advisory work; weekly/manual runs cover shared drift. */
export function selectColabApp(paths, map = componentMap()) {
  return paths.some(
    (path) =>
      COLAB_APP_INPUTS.has(path) ||
      (COLAB_APP_OWNERS.has(ownerOf(path, map)) &&
        COLAB_APP_ROOTS.some((root) => within(root, path)))
  );
}

// The Remote Firestore Rules emulator suite (Java 21 plus firebase-tools, no Docker) runs as
// steps of the Unit tests job. Remote is a CLI-scope owner, so every selected path below is a
// full native scope and that job always runs for it; selection stays a plain path match.
const REMOTE_FIRESTORE_ROOTS = [
  'extensions/tmt-remote/rust/tmt-remote/tests/emulator',
  'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/rules',
];

const REMOTE_FIRESTORE_INPUTS = new Set([
  '.github/workflows/ci.yml',
  'extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js',
  'extensions/tmt-remote/rust/tmt-remote/src/rules.rs',
]);

/** The suite, its fixtures, the composer, the SDK bundle its guard test imports or the pinned CI step changed. Empty diffs select nothing. */
export function selectRemoteFirestore(paths) {
  return paths.some(
    (path) =>
      REMOTE_FIRESTORE_INPUTS.has(path) || REMOTE_FIRESTORE_ROOTS.some((root) => within(root, path))
  );
}

// Colab uses the same hosted emulator lane and full-native Unit tests gate as Remote.
const COLAB_FIRESTORE_ROOTS = [
  'extensions/tmt-colab/firestore',
  'extensions/tmt-colab/rust/tmt-colab/tests/emulator',
];

/** Admission sources, composed declaration vectors, suite or CI wiring changed. */
export function selectColabFirestore(paths) {
  return paths.some(
    (path) =>
      path === '.github/workflows/ci.yml' ||
      path.startsWith('extensions/tmt-colab/contracts/vectors/deploy-declaration-') ||
      COLAB_FIRESTORE_ROOTS.some((root) => within(root, path))
  );
}

/**
 * How much of the native work a change needs. `none`: nothing native is selected.
 * A component name (only `ops` declares `scopedChecks`): every path that selects
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
  if (full) selection.areas = { native: true, office: false, nativeOffice: false };
  if (queue) {
    selection.areas = { ...selection.areas, office: false, nativeOffice: false };
    selection.rows = selection.rows?.map((row) => ({ ...row, office: false, nativeOffice: false }));
  }
  const officeBrowser = selectOfficeBrowser(selection.paths);
  const colabHarness = !queue && !seed && !full && selectColabHarness(selection.paths);
  const colabApp = !queue && !seed && !full && selectColabApp(selection.paths);
  const nativeNotices = !seed && (full || selectNativeNotices(selection.paths));
  const remoteFirestore = !seed && !full && selectRemoteFirestore(selection.paths);
  const colabFirestore = !seed && !full && selectColabFirestore(selection.paths);
  const evidence =
    (full || seed || fallback
      ? `### CI selection\n\n${fallback ?? (full ? 'Weekly/manual retained-product verification; Office retired.' : 'Main cache seed; Office product verification is retired.')}\n`
      : renderSelectionEvidence({ base, head, range, ...selection })) +
    `\nOffice browser selection (retired product): ${officeBrowser}.\n` +
    `\nColab browser PR selection (client, model, vectors or harness workflow): ${colabHarness}.\n` +
    `\nColab app component suite PR selection (app, client, browser UI, workflow or lockfile): ${colabApp}.\n` +
    `\nNative dependency notices selection: ${nativeNotices}.\n` +
    `\nRemote Firestore Rules emulator selection (Unit tests job): ${remoteFirestore}.\n` +
    `\nColab Firestore Rules emulator selection (Unit tests job): ${colabFirestore}.\n`;
  stderr.write(evidence);
  if (summaryFile) appendFileSync(summaryFile, evidence);
  const { areas, nativeScope } = selection;
  const checks = scopedChecks(nativeScope);
  const [firstShard, secondShard] = e2eShardFiles(nativeScope, checks.e2eFiles);
  stdout.write(
    `native=${areas.native}\noffice=${areas.office}\nnative_office=${areas.nativeOffice}\n` +
      `office_browser=${officeBrowser}\n` +
      `colab_harness=${colabHarness}\n` +
      `colab_app=${colabApp}\n` +
      `native_notices=${nativeNotices}\n` +
      `remote_firestore=${remoteFirestore}\n` +
      `colab_firestore=${colabFirestore}\n` +
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
