// Mechanical checkout edits for the shadow native spike; nothing is committed or tagged.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, lstatSync, readlinkSync } from 'node:fs';
import { relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseComponentMap } from './ci-scope.mjs';
import { releasePolicy } from './native-release-policy.mjs';
import { versionOfTag } from './release-versions.mjs';

const LOCK = 'rust/Cargo.lock';
const WORKSPACE = 'rust/Cargo.toml';
const hash = (root, file) =>
  createHash('sha256')
    .update(
      lstatSync(resolve(root, file)).isSymbolicLink()
        ? `symlink:${readlinkSync(resolve(root, file))}`
        : readFileSync(resolve(root, file))
    )
    .digest('hex');
const read = (root, file) => readFileSync(resolve(root, file), 'utf8');
const TOOL = resolve(
  process.env.CARGO_TARGET_DIR ?? fileURLToPath(new URL('../../rust/target', import.meta.url)),
  'debug/release-version'
);
// The developer-only Rust helper owns TOML parsing and formatting-preserving edits.
function tomlCommand(args, source) {
  const result = spawnSync(TOOL, args, {
    input: source,
    encoding: 'utf8',
    timeout: 10_000,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 || result.signal)
    throw new Error(`Rust TOML helper failed: ${result.stderr}`);
  return result.stdout;
}
const parse = (text) => JSON.parse(tomlCommand(['parse'], text));
const normalizeLock = (lock) => ({
  ...lock,
  package: [...lock.package].sort((a, b) =>
    `${a.name}\0${a.version}\0${a.source ?? ''}`.localeCompare(
      `${b.name}\0${b.version}\0${b.source ?? ''}`
    )
  ),
});

/** Builds a version-only edit contract from the source, not a release manifest/config. */
export function captureVersionState({ root, files, metadata, product, tag, cut, map }) {
  const version = versionOfTag(tag, product);
  if (
    !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(
      version
    )
  )
    throw new Error('Invalid injection tag version.');
  const component = map.components.find((c) => c.name === product);
  if (!component?.package || component.release === false)
    throw new Error('Injection requires a released component package.');
  const crates = metadata.packages.filter((p) => metadata.workspace_members.includes(p.id));
  const selected = crates.filter((p) => p.name === component.package);
  if (selected.length !== 1) throw new Error('Missing or ambiguous product Cargo package.');
  const packageManifest = relative(root, selected[0].manifest_path);
  if (!files.includes(packageManifest)) throw new Error('Product manifest is not tracked source.');
  const inherited = parse(read(root, packageManifest)).package.version?.workspace === true;
  const manifest = inherited ? WORKSPACE : packageManifest;
  const section = inherited ? 'workspace.package' : 'package';
  const source = read(root, manifest);
  const original = parse(source);
  const oldVersion = inherited ? original.workspace.package.version : original.package.version;
  if (oldVersion !== selected[0].version || oldVersion === version)
    throw new Error('Product version must match metadata and differ from the injected tag.');
  const packages = inherited
    ? crates
        .filter(
          (p) => parse(readFileSync(p.manifest_path, 'utf8')).package.version?.workspace === true
        )
        .map((p) => p.name)
    : [component.package];
  if (!files.includes(LOCK) || !files.includes(manifest))
    throw new Error('Version source/lock must be tracked.');
  return {
    schema: 1,
    cut,
    product,
    tag,
    version,
    oldVersion,
    manifest,
    section,
    source,
    packages,
    lock: read(root, LOCK),
    hashes: Object.fromEntries(files.map((file) => [file, hash(root, file)])),
  };
}

/** toml_edit preserves surrounding formatting and rejects invalid/ambiguous TOML. */
function versionEditedSource(snapshot) {
  return tomlCommand(
    ['edit', snapshot.section, snapshot.oldVersion, snapshot.version],
    snapshot.source
  );
}

export function injectVersion(root, snapshot) {
  assert.equal(
    read(root, snapshot.manifest),
    snapshot.source,
    'Version source changed before injection.'
  );
  writeFileSync(resolve(root, snapshot.manifest), versionEditedSource(snapshot));
}

/** Strict semantic Cargo.lock equality, allowing only the selected local version changes. */
export function verifyVersionState(root, snapshot, metadata) {
  assert.equal(snapshot.schema, 1);
  const changed = Object.entries(snapshot.hashes)
    .filter(([file, before]) => hash(root, file) !== before)
    .map(([file]) => file)
    .sort();
  assert.deepEqual(
    changed,
    [LOCK, snapshot.manifest].sort(),
    'Source differs beyond version manifest and implied lock entries.'
  );
  assert.equal(
    read(root, snapshot.manifest),
    versionEditedSource(snapshot),
    'Cargo manifest differs beyond the exact version field.'
  );
  const expectedManifest = parse(snapshot.source);
  if (snapshot.section === 'workspace.package')
    expectedManifest.workspace.package.version = snapshot.version;
  else expectedManifest.package.version = snapshot.version;
  assert.deepEqual(
    parse(read(root, snapshot.manifest)),
    expectedManifest,
    'Non-version Cargo manifest edit.'
  );
  const expectedLock = parse(snapshot.lock);
  const updated = new Set();
  for (const entry of expectedLock.package) {
    if (!entry.source && snapshot.packages.includes(entry.name)) {
      assert.equal(entry.version, snapshot.oldVersion);
      entry.version = snapshot.version;
      updated.add(entry.name);
    }
    if (entry.dependencies)
      entry.dependencies = entry.dependencies.map((dependency) => {
        const name = snapshot.packages.find(
          (name) => dependency === `${name} ${snapshot.oldVersion}`
        );
        return name ? `${name} ${snapshot.version}` : dependency;
      });
  }
  assert.deepEqual(
    [...updated].sort(),
    [...snapshot.packages].sort(),
    'Missing implied local Cargo.lock entry.'
  );
  assert.deepEqual(
    normalizeLock(parse(read(root, LOCK))),
    normalizeLock(expectedLock),
    'Non-implied Cargo.lock change.'
  );
  const crates = metadata.packages.filter((p) => metadata.workspace_members.includes(p.id));
  for (const name of snapshot.packages) {
    const found = crates.filter((p) => p.name === name);
    assert.equal(found.length, 1);
    assert.equal(found[0].version, snapshot.version, 'Resolved product version differs from tag.');
  }
  return {
    product: snapshot.product,
    tag: snapshot.tag,
    cut: snapshot.cut,
    changed,
    packages: snapshot.packages,
  };
}

export function verifyDistVersions(snapshot, plan, build, reportedVersion) {
  for (const manifest of [plan, build]) {
    assert.equal(manifest.announcement_tag, snapshot.tag, 'dist tag differs from injected tag.');
    assert.equal(manifest.releases.length, 1, 'dist selected unexpected products.');
    const release = manifest.releases[0];
    assert.equal(
      release.app_name,
      snapshot.product === 'cli' ? 'tmt-cli' : `tmt-${snapshot.product}`
    );
    assert.equal(release.app_version, snapshot.version, 'dist version differs from tag.');
  }
  const expected =
    snapshot.product === 'cli' ? snapshot.version : `${snapshot.product} ${snapshot.version}`;
  assert.equal(reportedVersion.trim(), expected, 'Built binary version differs from tag.');
}

function command(root, executable, args) {
  const result = spawnSync(executable, args, {
    cwd: root,
    env: process.env,
    encoding: 'utf8',
    timeout: 60_000,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 || result.signal)
    throw new Error(`${executable} ${args.join(' ')} failed: ${result.stderr}`);
  return result.stdout.trimEnd();
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [action, rootArg, snapshotFile, ...args] = process.argv.slice(2);
    const root = resolve(rootArg ?? '.');
    if (!snapshotFile || resolve(snapshotFile).startsWith(`${root}/`))
      throw new Error('Injection snapshot must be outside the source checkout.');
    const git = (args) => command(root, 'git', args);
    if (action === 'prepare' && args.length === 2) {
      const [product, tag] = args;
      releasePolicy(product);
      const cut = git(['rev-parse', 'HEAD']);
      git(['diff', '--exit-code', cut, '--']);
      if (git(['ls-files', '--others', '--exclude-standard', '-z']))
        throw new Error('Untracked source before injection.');
      // --no-deps is sufficient for inheritance discovery ONLY. Verification below resolves fully.
      const metadata = JSON.parse(
        command(root, 'cargo', [
          'metadata',
          '--offline',
          '--locked',
          '--no-deps',
          '--format-version',
          '1',
          '--manifest-path',
          WORKSPACE,
        ])
      );
      const snapshot = captureVersionState({
        root,
        files: git(['ls-files', '-z']).split('\0').filter(Boolean),
        metadata,
        product,
        tag,
        cut,
        map: parseComponentMap(read(root, '.github/components.json')),
      });
      writeFileSync(snapshotFile, `${JSON.stringify(snapshot)}\n`);
      injectVersion(root, snapshot);
    } else if (action === 'verify' && !args.length) {
      const snapshot = JSON.parse(readFileSync(snapshotFile, 'utf8'));
      assert.equal(git(['rev-parse', 'HEAD']), snapshot.cut, 'Checkout moved after injection.');
      assert.deepEqual(
        git(['ls-files', '-z']).split('\0').filter(Boolean).sort(),
        Object.keys(snapshot.hashes).sort()
      );
      if (git(['ls-files', '--others', '--exclude-standard', '-z']))
        throw new Error('Untracked source after injection.');
      const metadata = JSON.parse(
        command(root, 'cargo', [
          'metadata',
          '--offline',
          '--locked',
          '--format-version',
          '1',
          '--manifest-path',
          WORKSPACE,
        ])
      );
      console.log(JSON.stringify(verifyVersionState(root, snapshot, metadata), null, 2));
    } else if (action === 'artifact' && args.length === 3) {
      const snapshot = JSON.parse(readFileSync(snapshotFile, 'utf8'));
      verifyDistVersions(
        snapshot,
        JSON.parse(readFileSync(args[0], 'utf8')),
        JSON.parse(readFileSync(args[1], 'utf8')),
        command(root, resolve(args[2]), ['--version'])
      );
      console.log(`Verified ${snapshot.tag}: plan, build and binary agree.`);
    } else
      throw new Error(
        'Usage: release-version-injection.mjs prepare <root> <snapshot> <product> <tag> | verify <root> <snapshot> | artifact <root> <snapshot> <plan> <build> <binary>'
      );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
