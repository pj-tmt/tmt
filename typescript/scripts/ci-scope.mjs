import { createHash } from 'node:crypto';
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { e2eShardFiles } from './e2e-shards.mjs';
import { runPackedCommand } from './packed-command.mjs';

const COMPONENT_MAP = new URL('../../.github/components.json', import.meta.url);
const CONSUMERS = ['native', 'office'];

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
      rule.consumers.some((consumer) => !CONSUMERS.includes(consumer))
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
 * Every workspace crate except tmt-cli is Office-affecting by default: the
 * Office crates import the shared crates and call `tmt api` at runtime. Only
 * these top-level modules are verified unreachable from the Office crates and
 * the API module; ci-scope.test.ts recomputes that closure so the list cannot
 * silently rot.
 */
export const NATIVE_OFFICE_UNREACHABLE = {
  'tmt-adapters': ['setup'],
};

/**
 * CLI surfaces Office drives: its facade, the API command, and the installers
 * that publish Office releases through its verifier.
 */
const NATIVE_OFFICE_CLI = [
  'rust/crates/tmt-cli/src/office_facade.rs',
  'rust/crates/tmt-cli/src/api_command.rs',
  'rust/crates/tmt-cli/src/native_install_command.rs',
  'rust/crates/tmt-cli/src/extension_install_command.rs',
  'rust/crates/tmt-cli/src/native_upgrade_command.rs',
];

function consumedCratePath(path) {
  const [, crate, rest] = /^rust\/crates\/([^/]+)\/(.*)$/.exec(path) ?? [];
  if (!crate) return true;
  if (crate === 'tmt-cli') return NATIVE_OFFICE_CLI.includes(path);
  const module = /^src\/([^/.]+)/.exec(rest)?.[1];
  return !(module && (NATIVE_OFFICE_UNREACHABLE[crate] ?? []).includes(module));
}

/**
 * Shared paths the native Office image never reads: prose outside Office,
 * core-only test suites, and E2E scenarios (Office imports only the harness
 * and test support) with their separate image.
 */
const NATIVE_OFFICE_UNRELATED = [
  /^(?!docs\/office\/)(?:docs\/.+|[^/]+)\.md$/,
  /^typescript\/test\/(?:native|tooling)\//,
  /^typescript\/test\/e2e\/(?:[^/]+\.e2e\.test\.ts|Dockerfile)$/,
];

/** Unknown paths fail closed, as they do for the other areas. */
function consumedByNativeOffice(path) {
  if (path.startsWith('rust/crates/')) return consumedCratePath(path);
  if (path.startsWith('rust/')) return true;
  if (path.startsWith('skills/') || path.startsWith('extensions/tmt-squad/')) return false;
  return !NATIVE_OFFICE_UNRELATED.some((pattern) => pattern.test(path));
}

/**
 * What each changed path selects and why, for the run summary. The first
 * matching rule of the component map decides `native` and `office`; a path no
 * rule matches fails closed to both. `nativeOffice` retains the local Office
 * verification impact of core surfaces it consumes; advisory PR scheduling
 * uses the separate ownership-only selectOfficeBrowser policy.
 */
export function explainCiSelection(paths, map = componentMap()) {
  return paths.map((path) => {
    const rule = map.rules.find(({ patterns }) => patterns.some((pattern) => pattern.test(path)));
    const consumers = rule ? rule.consumers : CONSUMERS;
    return {
      path,
      owner: ownerOf(path, map),
      rule: rule?.id ?? 'unmapped',
      why: rule?.why ?? 'Not covered by any rule, so it fails closed to every consumer.',
      native: consumers.includes('native'),
      office: consumers.includes('office'),
      // A rule with no consumers means no CI job reads the path, the Office shards included.
      nativeOffice: consumers.length > 0 && consumedByNativeOffice(path),
    };
  });
}

/** Deletions are still changes, and an empty diff fails closed. */
export function selectCiAreas(paths, map = componentMap()) {
  if (paths.length === 0) return { native: true, office: true, nativeOffice: true };
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
 * Parked Office browser PRs follow ownership plus browser-specific machinery,
 * not shared inputs or core dependencies. Weekly/manual runs cover every partition.
 * Empty or unknown paths select no browser work; required CI stays conservative.
 */
export function selectOfficeBrowser(paths, map = componentMap()) {
  return paths.some(
    (path) =>
      ownerOf(path, map) === 'office' ||
      path.startsWith('docs/office/') ||
      OFFICE_BROWSER_INPUTS.has(path)
  );
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
export function renderSelectionEvidence({ base, head, rows, areas, digest, nativeScope }) {
  const selects = (row) =>
    [row.native && 'native', row.office && 'office', row.nativeOffice && 'native_office']
      .filter(Boolean)
      .join(', ') || 'nothing';
  const lines = [
    '### CI selection',
    '',
    `Diff \`${base.slice(0, 12)}...${head.slice(0, 12)}\`, ${rows.length} changed path(s), component map \`sha256:${digest}\`.`,
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

/**
 * What each native job must have reported for the scope. A scoped component runs
 * its Rust checks and the first E2E shard (which holds its files); the second shard,
 * the CLI runtime builds, packed installs and tooling unit tests are skipped because
 * the CLI is unchanged. `none` skips everything. Anything unexpected, including an
 * unknown scope, fails: a selected job that was skipped, cancelled or missing is as
 * wrong as a job that ran when the selector skipped it.
 */
function expectedNativeResults(scope, map, event = 'pull_request') {
  if (!['pull_request', 'merge_group', 'push', 'schedule', 'workflow_dispatch'].includes(event)) {
    return undefined;
  }
  if (scope === 'full') {
    return {
      ...Object.fromEntries(NATIVE_JOBS.map((job) => [job, 'success'])),
      macosRuntimeBuild: event === 'merge_group' ? 'skipped' : 'success',
      macosPackedInstall: event === 'merge_group' ? 'skipped' : 'success',
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

function gatePasses(jobs, scope, results, map, event) {
  const expected = expectedNativeResults(scope, map, event);
  if (!expected || jobs.some((job) => typeof results?.[job] !== 'string')) return false;
  return jobs.every((job) => results[job] === expected[job]);
}

/** `Native package matrix`: every native job, the two E2E shards included. */
export function nativeGatePasses(scope, results, map = componentMap(), event = 'pull_request') {
  return gatePasses(NATIVE_JOBS, scope, results, map, event);
}

/** `Docker E2E`: the two shards alone, with the same expectations. */
export function e2eGatePasses(scope, results, map = componentMap()) {
  return gatePasses(E2E_JOBS, scope, results, map);
}

/** `Native Rust contracts`: runtime checks and MSRV, selected together. */
export function rustGatePasses(scope, results, map = componentMap()) {
  const expected = expectedNativeResults(scope, map)?.nativeRust;
  if (!expected || results?.length !== 2) return false;
  return results.every((result) => result === expected);
}

export function readChangedCiSelection(base, head, cwd) {
  if ([base, head].some((sha) => !/^[a-f0-9]{40}$/.test(sha ?? ''))) {
    throw new Error('Expected exact base and head commit SHAs.');
  }
  const changed = runPackedCommand(
    'git',
    ['diff', '--no-renames', '--name-only', '-z', `${base}...${head}`, '--'],
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
    const [, scope, ...results] = args;
    if (!rustGatePasses(scope, results)) {
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
    const [, event, scope, ...values] = args;
    const results = Object.fromEntries(NATIVE_JOBS.map((job, index) => [job, values[index]]));
    if (
      values.length !== NATIVE_JOBS.length ||
      !nativeGatePasses(scope, results, undefined, event ?? '')
    ) {
      throw new Error(
        'Selected native CI work did not complete successfully, or skip evidence is invalid.'
      );
    }
    return;
  }
  const full = args.length === 1 && args[0] === 'full';
  if (!full && args.length !== 2) {
    throw new Error('Expected exact base and head commit SHAs.');
  }
  const selection = full
    ? { paths: [], areas: selectCiAreas([]), nativeScope: selectNativeScope([]) }
    : readChangedCiSelection(args[0], args[1], cwd);
  const officeBrowser = selectOfficeBrowser(selection.paths);
  const evidence =
    (full
      ? '### CI selection\n\nFull verification for a merge-group candidate; no path filtering.\n'
      : renderSelectionEvidence({ base: args[0], head: args[1], ...selection })) +
    `\nOffice browser PR selection (Office ownership or verification machinery): ${officeBrowser}.\n`;
  stderr.write(evidence);
  if (summaryFile) appendFileSync(summaryFile, evidence);
  const { areas, nativeScope } = selection;
  const checks = scopedChecks(nativeScope);
  const [firstShard, secondShard] = e2eShardFiles(nativeScope, checks.e2eFiles);
  stdout.write(
    `native=${areas.native}\noffice=${areas.office}\nnative_office=${areas.nativeOffice}\n` +
      `office_browser=${officeBrowser}\n` +
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
