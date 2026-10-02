#!/usr/bin/env node
// Generates release-please-config.json from the component map, the Cargo
// workspace and the publication policy's tags, so they cannot drift apart.
//   node typescript/scripts/release-please-config.mjs --check   fail when the committed file is stale
//   node typescript/scripts/release-please-config.mjs --write   regenerate it
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { ownerOf, parseComponentMap } from './ci-scope.mjs';
import { releasePolicy } from './native-release-policy.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const CONFIG_FILE = 'release-please-config.json';
const WORKSPACE_MANIFEST = 'rust/Cargo.toml';
const LOCK_FILE = 'rust/Cargo.lock';

/**
 * Crates, their resolved version and version declaration, their workspace dependencies, the crates that have a
 * `Cargo.lock` entry and every tracked file, read from the repository at `root`.
 */
export function readWorkspace(root = ROOT) {
  const metadata = JSON.parse(
    runPackedCommand(
      'cargo',
      [
        'metadata',
        '--no-deps',
        '--offline',
        '--locked',
        '--format-version',
        '1',
        '--manifest-path',
        `${root}${WORKSPACE_MANIFEST}`,
      ],
      { cwd: root, env: process.env, timeoutMs: 60_000 }
    )
  );
  const names = new Set(metadata.packages.map(({ name }) => name));
  const crates = metadata.packages.map((crate) => {
    const manifest = relative(root, crate.manifest_path);
    return {
      name: crate.name,
      version: crate.version,
      manifest,
      dir: dirname(manifest),
      hasBinary: crate.targets.some(({ kind }) => kind.includes('bin')),
      dist: crate.metadata?.dist?.dist,
      inheritsVersion: /^version\.workspace\s*=\s*true\s*$/m.test(
        readFileSync(crate.manifest_path, 'utf8')
      ),
      dependencies: crate.dependencies
        .filter((dependency) => dependency.kind !== 'dev' && names.has(dependency.name))
        .map(({ name }) => name),
    };
  });
  const lockNames = new Set(
    [...readFileSync(`${root}${LOCK_FILE}`, 'utf8').matchAll(/^name = "([^"]+)"$/gm)].map(
      ([, name]) => name
    )
  );
  const files = runPackedCommand('git', ['ls-files', '-z'], { cwd: root, env: process.env })
    .split('\0')
    .filter(Boolean);
  return { crates, lockNames, files };
}

/** Where release-please looks for `file`: relative to the package, or `/`-anchored at the root. */
function fromPackage(packagePath, file) {
  if (packagePath === '.') return file;
  return file.startsWith(`${packagePath}/`) ? file.slice(packagePath.length + 1) : `/${file}`;
}

/** The workspace crates `owned` links, directly or through each other. */
function linkedCrates(owned, crates) {
  const byName = new Map(crates.map((crate) => [crate.name, crate]));
  const seen = new Set();
  const pending = [...owned];
  while (pending.length > 0) {
    const crate = pending.pop();
    if (seen.has(crate.name)) continue;
    seen.add(crate.name);
    for (const name of crate.dependencies) pending.push(byName.get(name));
  }
  return [...seen].map((name) => byName.get(name));
}

/**
 * release-please attributes a commit to a package by the files it touches under the package
 * path, and has no option to add other directories (#912). The release wrapper supplies
 * declared private-leaf consumption before this split. A package can only drop paths, so to keep
 * the linked crates of an excluded root, everything else under that root is listed instead.
 */
function excludeAllBut(root, kept, files) {
  const entries = new Set(
    files
      .filter((file) => file.startsWith(`${root}/`))
      .map((file) => file.slice(root.length + 1).split('/')[0])
  );
  const excluded = [];
  for (const entry of [...entries].sort()) {
    const path = `${root}/${entry}`;
    if (files.includes(path)) {
      throw new Error(`${path} is a file above a linked crate; it cannot be excluded by prefix.`);
    }
    if (kept.includes(path)) continue;
    if (kept.some((dir) => dir.startsWith(`${path}/`))) {
      excluded.push(...excludeAllBut(path, kept, files));
    } else {
      excluded.push(path);
    }
  }
  return excluded;
}

/** The `exclude-paths` of a component: its excluded roots, minus the crates it links there. */
function excludedPaths(component, linked, files) {
  return component.excludes.flatMap((root) => {
    const kept = linked.map((crate) => crate.dir).filter((dir) => dir.startsWith(`${root}/`));
    return kept.length === 0 ? [root] : excludeAllBut(root, kept.sort(), files);
  });
}

/** Private source roots and their released consumers, independent of version/CI ownership. */
export function releaseConsumption(components) {
  return components.flatMap((leaf) => {
    if (!leaf.releaseConsumers?.length) return [];
    if (leaf.release !== false || leaf.owns.length !== 1 || leaf.excludes.length) {
      throw new Error(`Release consumption requires a private single-root leaf: ${leaf.name}.`);
    }
    return [...new Set(leaf.releaseConsumers)].map((name) => {
      const consumer = components.find((component) => component.name === name);
      if (
        !consumer ||
        consumer.release === false ||
        consumer.owns.length !== 1 ||
        !consumer.package
      ) {
        throw new Error(`Invalid release consumer ${name} of ${leaf.name}.`);
      }
      const [source] = leaf.owns;
      const [target] = consumer.owns;
      if (source === '.' || target === '.' || source === target) {
        throw new Error(`Release consumption requires distinct non-root paths: ${leaf.name}.`);
      }
      return { source, target };
    });
  });
}

const byName = (left, right) => left.name.localeCompare(right.name);

/**
 * The release-please configuration for the component map and workspace. Every choice that is not
 * ownership comes from a named source: tags and prerelease flags from the publication policy,
 * versions from where each crate declares them, and lock entries from the crates that have one.
 */
export function generateReleasePleaseConfig({ components, workspace }) {
  releaseConsumption(components);
  const { crates, lockNames, files } = workspace;
  const map = { components };
  const ownerOfCrate = (crate) => ownerOf(crate.manifest, map);
  const workspaceOwner = ownerOf(WORKSPACE_MANIFEST, map);
  // A crate that inherits the workspace version changes with the workspace owner's release, so its
  // lock entry stays in that release even when a private component owns the crate.
  const versionOwnerOf = (crate) => (crate.inheritsVersion ? workspaceOwner : ownerOfCrate(crate));
  const releasedCrates = crates.filter(
    (crate) => components.find(({ name }) => name === versionOwnerOf(crate))?.release !== false
  );
  const packages = {};

  for (const component of components) {
    if (component.owns.length !== 1) {
      throw new Error(`Component ${component.name} must own exactly one root to be a package.`);
    }
    const owned = crates.filter((crate) => ownerOfCrate(crate) === component.name);
    if (component.release === false) {
      if (owned.some((crate) => crate.hasBinary && crate.dist !== false))
        throw new Error(`Private component ${component.name} owns a binary without dist=false.`);
      continue;
    }
    const [packagePath] = component.owns;
    const policy = releasePolicy(component.name);
    const includeComponent = policy.tagPrefix !== 'v';
    const expectedPrefix = includeComponent ? `${component.package}-v` : 'v';
    if (policy.tagPrefix !== expectedPrefix) {
      throw new Error(
        `Tag prefix ${policy.tagPrefix} of ${component.name} is not ${expectedPrefix}, which release-please would create.`
      );
    }
    const extraFiles = [];
    const toml = (file, jsonpath) =>
      extraFiles.push({ type: 'toml', path: fromPackage(packagePath, file), jsonpath });
    if (workspaceOwner === component.name) {
      toml(WORKSPACE_MANIFEST, '$.workspace.package.version');
    }
    for (const crate of [...owned].sort(byName)) {
      if (!crate.inheritsVersion) toml(crate.manifest, '$.package.version');
    }
    for (const crate of [...releasedCrates].sort(byName)) {
      if (versionOwnerOf(crate) !== component.name) continue;
      if (!/^[a-z0-9-]+$/.test(crate.name)) {
        throw new Error(`Crate name ${crate.name} cannot be written into a JSONPath filter.`);
      }
      if (!lockNames.has(crate.name))
        throw new Error(`${LOCK_FILE} has no entry for ${crate.name}.`);
      toml(LOCK_FILE, `$.package[?(@.name.value=='${crate.name}')].version`);
    }
    const linked = linkedCrates(owned, crates).filter(
      (crate) => ownerOfCrate(crate) !== component.name
    );
    const exclude = excludedPaths(component, linked, files);
    packages[packagePath] = {
      'release-type': 'simple',
      'prerelease-type': 'alpha',
      component: component.package,
      'include-component-in-tag': includeComponent,
      ...(exclude.length > 0 ? { 'exclude-paths': exclude } : {}),
      'extra-files': extraFiles,
    };
  }

  return {
    $schema:
      'https://raw.githubusercontent.com/googleapis/release-please/v17.11.2/schemas/config.json',
    'separate-pull-requests': true,
    'include-v-in-tag': true,
    versioning: 'prerelease',
    // release-please reads this twice: for the GitHub prerelease flag and for the version line.
    // Without it, 5.0.0-alpha.8 graduates to 5.0.0. The flags a published release actually
    // carries come from the publication policy when the draft is published, not from here.
    prerelease: true,
    // A published release cannot receive assets (immutability), so the workflow attaches and
    // verifies the bundle on the draft and publishes it afterwards.
    draft: true,
    // release-please leaves an open release pull request alone while its notes are unchanged, and
    // the workflow's `gh pr update-branch` cannot resolve a conflict (every release pull request
    // edits the shared manifest). With this, each run rebuilds them from `main`'s current files.
    'always-update': true,
    'skip-changelog': true,
    'pull-request-title-pattern': 'chore${scope}: release${component} ${version}',
    packages,
  };
}

export function renderReleasePleaseConfig(input) {
  return `${JSON.stringify(generateReleasePleaseConfig(input), null, 2)}\n`;
}

function main(argument) {
  if (argument !== '--check' && argument !== '--write') {
    throw new Error('Usage: release-please-config.mjs --check | --write');
  }
  const { components } = parseComponentMap(readFileSync(`${ROOT}.github/components.json`, 'utf8'));
  const expected = renderReleasePleaseConfig({ components, workspace: readWorkspace() });
  const file = `${ROOT}${CONFIG_FILE}`;
  if (argument === '--write') {
    writeFileSync(file, expected);
    process.stdout.write(`Wrote ${CONFIG_FILE}.\n`);
  } else if (readFileSync(file, 'utf8') !== expected) {
    throw new Error(
      `${CONFIG_FILE} is stale. Regenerate it with: node typescript/scripts/release-please-config.mjs --write`
    );
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv[2]);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
