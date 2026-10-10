#!/usr/bin/env node
// Proves that a release candidate upgrades from the last published release of its product, with
// the real bytes of both: the candidate's archive from its release (a draft or a published
// release) and the previous archive from the newest published release below it. Two steps, run
// by two jobs, so that the token that can see draft assets never runs the release's own code:
//   fetch  (write token, this repository's default ref) downloads the archive and manifest of
//          the candidate, of the previous release and, for an extension, of the CLI that drives
//          it, for every target, checks each against the digest GitHub recorded and writes them
//          with a plan into one directory
//   prove  (read-only, the release's commit) re-checks those digests and runs the verifier of
//          the product over one target's staged files; it never reaches GitHub
//   node release-upgrade.mjs resolve --tag TAG      the commit of the release
//   node release-upgrade.mjs fetch --product cli|office|ops --tag TAG --directory DIR
//        [--candidate-directory BUNDLE]   stage a rehearsal's verified bundle as the candidate
//   node release-upgrade.mjs assess --directory DIR --sha SHA   proved, nothing or predates
//   node release-upgrade.mjs prove --product P --tag TAG --target T --directory DIR [--skill S]
//   node release-upgrade.mjs acceptance --product cli --tag TAG --target T --directory DIR
//   node release-upgrade.mjs reason --directory DIR   why the proof failed, from the hosts' logs
// A CLI candidate runs the managed-install lifecycle verifier over the two archives. An extension
// candidate is installed and upgraded by the newest published CLI, which is what a user's
// `tmt extension install <extension>` runs, because an extension release carries no CLI.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  appendFileSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { runPackedCommand } from './packed-command.mjs';
import {
  archivePrefix,
  componentOfProduct,
  predecessorOfProduct,
  productOfTag,
  upgradeSupportFloor,
} from './native-release-policy.mjs';
import { componentMap } from './ci-scope.mjs';
import { ghApi, retryReleaseRead } from './release-draft-assets.mjs';
import { compareVersions, publishedReleases, versionOfTag } from './release-versions.mjs';

/** The scripts a release's own commit must carry for the proof to run at that commit. */
export const PROOF_FILES = [
  'release-upgrade.mjs',
  'release-versions.mjs',
  'verify-native-installation.mjs',
  'verify-native-extension-upgrade.mjs',
  'verify-native-driver-upgrade.mjs',
];
const DIGEST = /^sha256:[0-9a-f]{64}$/;
const MANIFEST = 'dist-manifest.json';
const PLAN = 'plan.json';
const ASSET_LIMIT = 80 * 1024 * 1024;

/** Published tag ancestry relative to the successor's CLI registration, not release dates. */
function cliAncestry(release, registration, observeCli) {
  if (!observeCli) throw new Error('Cross-product proof needs published CLI tag ancestry.');
  const observation = observeCli(release, registration);
  if (
    typeof observation?.sha !== 'string' ||
    !COMMIT.test(observation.sha) ||
    !['ahead', 'behind', 'identical', 'diverged'].includes(observation?.status) ||
    (observation.status === 'identical') !== (observation.sha === registration)
  )
    throw new Error(`Unknown CLI ancestry for ${release.tag_name}.`);
  return observation;
}

/** The newest CLI drives the candidate; a rename also needs the newest strict older ancestor. */
function upgradeDrivers({ releases, product, previousProduct, map, observeCli }) {
  const published = publishedReleases(releases, 'cli');
  const [driver] = published;
  if (!driver)
    throw new Error('An extension or driver upgrade proof needs a published CLI release.');
  if (previousProduct === product) return { driver, previousDriver: null, provenance: {} };
  if (predecessorOfProduct(map, product) !== previousProduct)
    throw new Error('Cross-product proof requires the declared immediate predecessor.');
  const requiresCliSha = componentOfProduct(map, product).requiresCliSha;
  if (!COMMIT.test(requiresCliSha ?? ''))
    throw new Error('Cross-product proof needs the exact requiresCliSha registration.');
  const current = cliAncestry(driver, requiresCliSha, observeCli);
  if (!['ahead', 'identical'].includes(current.status))
    throw new Error(`Newest published CLI ${driver.tag_name} does not contain registration.`);
  for (const release of published.slice(1)) {
    const observation = cliAncestry(release, requiresCliSha, observeCli);
    if (observation.status !== 'behind') continue;
    return {
      driver,
      previousDriver: release,
      provenance: {
        previousDriver: release.tag_name,
        previousDriverSha: observation.sha,
        driverSha: current.sha,
        requiresCliSha,
      },
    };
  }
  throw new Error('No published CLI is a strict pre-registration ancestor.');
}

/** The newest published release of a product below the candidate's version, or null. */
export function selectPrevious({ releases, product, candidateTag, map = componentMap() }) {
  const candidate = versionOfTag(candidateTag, product);
  const own = publishedReleases(releases, product).find(
    (release) => compareVersions(versionOfTag(release.tag_name, product), candidate) < 0
  );
  if (own) return own;
  let predecessor = predecessorOfProduct(map, product);
  while (predecessor) {
    const previous = publishedReleases(releases, predecessor).find(
      (release) => compareVersions(versionOfTag(release.tag_name, predecessor), candidate) < 0
    );
    if (previous) return previous;
    if (publishedReleases(releases, predecessor).length) return null;
    predecessor = predecessorOfProduct(map, predecessor);
  }
  return null;
}

/** Historical candidates at/below the floor retain their original single-source proof. */
export function selectSupportFloor({ releases, product, candidateTag }) {
  const floor = upgradeSupportFloor(product);
  if (
    !floor ||
    compareVersions(versionOfTag(candidateTag, product), versionOfTag(floor, product)) <= 0
  )
    return null;
  const matches = publishedReleases(releases, product).filter(
    (release) => release.tag_name === floor
  );
  if (matches.length !== 1)
    throw new Error(`Require exactly one published support floor ${floor}.`);
  return matches[0];
}

/** The targets a release carries an archive for, from its asset names. */
export function archiveTargets({ release, product }) {
  const prefix = `${archivePrefix(product)}-`;
  return (release.assets ?? [])
    .map(({ name }) => name)
    .filter((name) => name.startsWith(prefix) && name.endsWith('.tar.gz'))
    .map((name) => name.slice(prefix.length, -'.tar.gz'.length))
    .sort();
}

/** The archive and manifest of a release for one target; both must carry a GitHub digest. */
export function selectAssets({ release, product, target }) {
  const wanted = { archive: `${archivePrefix(product)}-${target}.tar.gz`, manifest: MANIFEST };
  const found = {};
  for (const [role, name] of Object.entries(wanted)) {
    const asset = (release.assets ?? []).find((candidate) => candidate.name === name);
    if (!asset) throw new Error(`Release ${release.tag_name} has no ${name}.`);
    if (!DIGEST.test(asset.digest ?? '')) {
      throw new Error(`GitHub reports no usable digest for ${name} of ${release.tag_name}.`);
    }
    found[role] = asset;
  }
  return found;
}

const sha256 = (file) => `sha256:${createHash('sha256').update(readFileSync(file)).digest('hex')}`;

/**
 * Downloads a release's archive and manifest for the target, each checked against its digest.
 * `digests` maps each staged file, relative to `directory`, to the digest it was checked against.
 */
export function stageRelease({ download, release, product, target, directory }) {
  mkdirSync(directory, { recursive: true });
  const staged = { digests: {} };
  for (const [role, asset] of Object.entries(selectAssets({ release, product, target }))) {
    const file = path.join(directory, asset.name);
    download(asset, file);
    if (sha256(file) !== asset.digest) {
      throw new Error(`${asset.name} of ${release.tag_name} does not match its recorded digest.`);
    }
    staged[role] = file;
    staged.digests[asset.name] = asset.digest;
  }
  return staged;
}

/**
 * The candidate of a rehearsal: the verified bundle a prepare run assembled, read from `directory`
 * as a pseudo-release so the one staging path checks it like a downloaded release. Its tag is the
 * synthetic announcement tag of the assembled manifest. Every archive must match both the
 * `.sha256` file cargo-dist wrote beside it and the digest computed here, so the staged digest is
 * never just the candidate vouching for itself.
 */
export function localCandidate({ directory, product, tag }) {
  const manifestFile = path.join(directory, MANIFEST);
  if (!existsSync(manifestFile)) throw new Error(`The candidate bundle has no ${MANIFEST}.`);
  const announced = JSON.parse(readFileSync(manifestFile, 'utf8')).announcement_tag;
  if (announced !== tag)
    throw new Error(`The candidate bundle announces ${announced}, not ${tag}.`);
  const names = readdirSync(directory).sort();
  const prefix = `${archivePrefix(product)}-`;
  const archives = names.filter((name) => name.startsWith(prefix) && name.endsWith('.tar.gz'));
  if (archives.length === 0) throw new Error(`The candidate bundle has no ${product} archive.`);
  const assets = [MANIFEST, ...archives, ...names.filter((name) => name === 'install.sh')].map(
    (name) => {
      const file = path.join(directory, name);
      const digest = sha256(file);
      if (name.endsWith('.tar.gz')) {
        const recorded = path.join(directory, `${name}.sha256`);
        const expected = existsSync(recorded)
          ? readFileSync(recorded, 'utf8').trim().split(/\s+/)[0]
          : '';
        if (`sha256:${expected}` !== digest)
          throw new Error(`${name} does not match its recorded ${name}.sha256.`);
      }
      return { name, digest, file };
    }
  );
  return {
    release: { tag_name: tag, draft: true, assets },
    download: (asset, file) => copyFileSync(asset.file, file),
  };
}

/**
 * The fetch step. Stages, for every target of the candidate, the candidate, the previous release
 * and, for an extension or driver, the newest published CLI under `directory/<target>/<kind>`, and writes
 * `plan.json` with the tags and the digests. A product with no earlier published release stages
 * nothing: there is nothing to upgrade from.
 */
export function fetchUpgrade({
  releases,
  download,
  product,
  tag,
  directory,
  local,
  map = componentMap(),
  observeCli,
}) {
  const candidate = local ? local.release : releases.find((release) => release.tag_name === tag);
  if (!candidate) throw new Error(`There is no release ${tag}.`);
  if (local) {
    // A synthetic candidate must out-rank everything published, or "previous" would be wrong.
    const [newest] = publishedReleases(releases, product);
    if (
      newest &&
      compareVersions(versionOfTag(tag, product), versionOfTag(newest.tag_name, product)) <= 0
    )
      throw new Error(
        `The synthetic candidate ${tag} is not newer than the published ${newest.tag_name}.`
      );
  }
  const previous = selectPrevious({ releases, product, candidateTag: tag, map });
  const floor = selectSupportFloor({ releases, product, candidateTag: tag });
  const plan = {
    product,
    tag,
    previous: previous?.tag_name ?? null,
    floor: floor?.tag_name ?? null,
    driver: null,
    files: {},
  };
  mkdirSync(directory, { recursive: true });
  if (previous) {
    const targets = archiveTargets({ release: candidate, product });
    if (targets.length === 0) throw new Error(`Release ${tag} has no archive to upgrade to.`);
    let driverRelease = null;
    let previousDriverRelease = null;
    if (product !== 'cli') {
      const drivers = upgradeDrivers({
        releases,
        product,
        previousProduct: productOfTag(previous.tag_name),
        map,
        observeCli,
      });
      driverRelease = drivers.driver;
      previousDriverRelease = drivers.previousDriver;
      plan.driver = driverRelease.tag_name;
      Object.assign(plan, drivers.provenance);
    }
    for (const target of targets) {
      const stage = (release, kind, releaseProduct, source = download) => {
        const { digests } = stageRelease({
          download: source,
          release,
          product: releaseProduct,
          target,
          directory: path.join(directory, target, kind),
        });
        for (const [name, digest] of Object.entries(digests)) {
          plan.files[path.posix.join(target, kind, name)] = digest;
        }
      };
      stage(candidate, 'candidate', product, local?.download);
      stage(previous, 'previous', productOfTag(previous.tag_name));
      if (floor && floor.tag_name !== previous.tag_name) stage(floor, 'floor', product);
      if (floor) {
        const bootstraps = candidate.assets.filter(({ name }) => name === 'install.sh');
        const asset = bootstraps[0];
        if (bootstraps.length !== 1 || !DIGEST.test(asset.digest ?? ''))
          throw new Error(`Release ${tag} must have exactly one digest-checked install.sh.`);
        const name = path.posix.join(target, 'candidate', 'install.sh');
        const file = path.join(directory, name);
        (local?.download ?? download)(asset, file);
        if (sha256(file) !== asset.digest)
          throw new Error(`install.sh of ${tag} does not match its recorded digest.`);
        plan.files[name] = asset.digest;
      }
      if (driverRelease) stage(driverRelease, 'driver', 'cli');
      if (previousDriverRelease) stage(previousDriverRelease, 'previous-driver', 'cli');
    }
  }
  writeFileSync(path.join(directory, PLAN), `${JSON.stringify(plan, null, 2)}\n`);
  return plan;
}

/** Both proofs recheck fetch's staged digests before consuming matching-host archives. */
function stagedUpgrade({ directory, product, tag, target, map = componentMap() }) {
  const plan = JSON.parse(readFileSync(path.join(directory, PLAN), 'utf8'));
  if (plan.product !== product || plan.tag !== tag) {
    throw new Error(`The staged assets are for ${plan.tag}, not for ${tag}.`);
  }
  const floorTag = upgradeSupportFloor(product);
  if (
    floorTag &&
    compareVersions(versionOfTag(tag, product), versionOfTag(floorTag, product)) > 0
  ) {
    if (plan.floor !== floorTag || !plan.previous)
      throw new Error('Staged plan is missing the declared support floor.');
  }
  if (!plan.previous) return { previous: null };
  const previousProduct = productOfTag(plan.previous);
  const crossProduct = previousProduct !== product;
  const provenanceFields = ['previousDriver', 'previousDriverSha', 'driverSha', 'requiresCliSha'];
  if (crossProduct) {
    if (
      product === 'cli' ||
      predecessorOfProduct(map, product) !== previousProduct ||
      typeof plan.previousDriverSha !== 'string' ||
      !COMMIT.test(plan.previousDriverSha) ||
      typeof plan.driverSha !== 'string' ||
      !COMMIT.test(plan.driverSha) ||
      typeof plan.requiresCliSha !== 'string' ||
      !COMMIT.test(plan.requiresCliSha) ||
      componentOfProduct(map, product).requiresCliSha !== plan.requiresCliSha ||
      plan.previousDriverSha === plan.driverSha ||
      plan.previousDriverSha === plan.requiresCliSha ||
      typeof plan.previousDriver !== 'string' ||
      typeof plan.driver !== 'string' ||
      compareVersions(versionOfTag(plan.previousDriver, 'cli'), versionOfTag(plan.driver, 'cli')) >=
        0
    )
      throw new Error('Staged cross-product plan has invalid CLI registration provenance.');
  } else if (provenanceFields.some((field) => Object.hasOwn(plan, field))) {
    throw new Error('Same-product plan must not carry cross-product driver provenance.');
  }
  const prefix = `${target}/`;
  const files = Object.entries(plan.files).filter(([name]) => name.startsWith(prefix));
  if (files.length === 0) throw new Error(`The staged assets have no files for ${target}.`);
  for (const [name, digest] of files) {
    const file = path.join(directory, name);
    if (!existsSync(file) || sha256(file) !== digest) {
      throw new Error(`Staged ${name} is missing or does not match its recorded digest.`);
    }
  }
  const staged = (kind, kindProduct) => {
    const base = path.join(directory, target, kind);
    for (const name of [`${archivePrefix(kindProduct)}-${target}.tar.gz`, MANIFEST]) {
      if (!Object.hasOwn(plan.files, path.posix.join(target, kind, name)))
        throw new Error(`Staged ${kind}/${name} has no recorded digest.`);
    }
    return {
      archive: path.join(base, `${archivePrefix(kindProduct)}-${target}.tar.gz`),
      manifest: path.join(base, MANIFEST),
    };
  };
  const now = staged('candidate', product);
  const before = staged('previous', productOfTag(plan.previous));
  if (plan.floor && !Object.hasOwn(plan.files, path.posix.join(target, 'candidate', 'install.sh')))
    throw new Error('Staged candidate install.sh has no recorded digest.');
  return {
    previous: plan.previous,
    now,
    before,
    driver: product === 'cli' ? null : staged('driver', 'cli'),
    previousProduct,
    previousDriver: crossProduct ? staged('previous-driver', 'cli') : null,
    floor: plan.floor && plan.floor !== plan.previous ? staged('floor', product) : null,
    bootstrap: plan.floor ? path.join(directory, target, 'candidate', 'install.sh') : null,
  };
}

/** Run the existing installer/migration verifier; return null for the first product release. */
export function proveStaged({ directory, product, tag, target, run, skill, sourceRoot, map }) {
  const { previous, now, before, driver, previousProduct, previousDriver, floor, bootstrap } =
    stagedUpgrade({
      directory,
      product,
      tag,
      target,
      map,
    });
  if (!previous) return { previous: null };
  const common = [
    '--archive',
    now.archive,
    '--manifest',
    now.manifest,
    '--previous-archive',
    before.archive,
    '--previous-manifest',
    before.manifest,
    '--target',
    target,
  ];
  if (product === 'cli') {
    if (!skill) throw new Error('A CLI upgrade proof needs --skill.');
    run('verify-native-installation.mjs', [
      ...common,
      '--skill',
      skill,
      ...(sourceRoot ? ['--source-root', sourceRoot] : []),
      ...(bootstrap ? ['--bootstrap', bootstrap] : []),
    ]);
    if (floor) {
      run('verify-native-installation.mjs', [
        '--archive',
        now.archive,
        '--manifest',
        now.manifest,
        '--previous-archive',
        floor.archive,
        '--previous-manifest',
        floor.manifest,
        '--target',
        target,
        '--skill',
        skill,
        '--bootstrap',
        bootstrap,
        ...(sourceRoot ? ['--source-root', sourceRoot] : []),
      ]);
    }
  } else {
    run(
      product === 'driver-herdr'
        ? 'verify-native-driver-upgrade.mjs'
        : 'verify-native-extension-upgrade.mjs',
      [
        ...common,
        '--product',
        product,
        '--driver-archive',
        driver.archive,
        '--driver-manifest',
        driver.manifest,
        ...(previousDriver
          ? [
              '--previous-product',
              previousProduct,
              '--previous-driver-archive',
              previousDriver.archive,
              '--previous-driver-manifest',
              previousDriver.manifest,
            ]
          : []),
      ]
    );
  }
  return { previous };
}

export const ACCEPTANCE_TEST =
  'native_install::upgrade::artifact_tests::cargo_dist_upgrade_refreshes_real_artifacts_and_preserves_conflicts';
const ACCEPTANCE_SOURCE = 'rust/crates/tmt-adapters/src/native_install/upgrade_artifact_tests.rs';

/** The release's source, rather than repaired rerun tooling, decides historical applicability. */
export function acceptanceApplicability(sourceRoot) {
  if (!existsSync(sourceRoot)) throw new Error('The release-source checkout is missing.');
  const file = path.join(sourceRoot, ACCEPTANCE_SOURCE);
  if (!existsSync(file)) return 'predates';
  const source = readFileSync(file, 'utf8');
  return /fn cargo_dist_upgrade_refreshes_real_artifacts_and_preserves_conflicts\(\)/.test(
    source
  ) && source.includes('skill-content transition: skipped (old and candidate text are identical)')
    ? 'applicable'
    : 'predates';
}

/** Compile only the adapter's lib tests, then require one discovered and one executed real-archive test. */
export function proveArchiveAcceptance({
  directory,
  product,
  tag,
  target,
  sourceRoot,
  execute = runPackedCommand,
  environment = process.env,
  report = () => {},
  clock = () => performance.now(),
}) {
  if (product !== 'cli') throw new Error('The adapter archive acceptance proof is CLI-only.');
  const { previous, now, before, floor } = stagedUpgrade({ directory, product, tag, target });
  if (!previous) {
    report('Real-archive adapter acceptance: not applicable (no previous published CLI release).');
    return { outcome: 'nothing' };
  }
  if (sourceRoot && acceptanceApplicability(sourceRoot) === 'predates') {
    report(
      'Real-archive adapter acceptance: predates; not applicable (release source predates the release-gate test form). Existing installer/migration proof remains required.'
    );
    return { outcome: 'predates' };
  }
  const root = sourceRoot ? path.resolve(sourceRoot) : path.resolve(here, '../..');
  const rustRoot = path.join(root, 'rust');
  const env = {
    ...environment,
    CARGO_PROFILE_DEV_DEBUG: '0',
    CARGO_INCREMENTAL: '0',
    TMT_UPGRADE_OLD_ARCHIVE: before.archive,
    TMT_UPGRADE_OLD_MANIFEST: before.manifest,
    TMT_UPGRADE_NEW_ARCHIVE: now.archive,
    TMT_UPGRADE_NEW_MANIFEST: now.manifest,
    TMT_UPGRADE_TARGET: target,
  };
  const executePhase = (phase, executable, args, options) => {
    const started = clock();
    const log = (status) => {
      try {
        const elapsed =
          status === 'started' ? '' : ` seconds=${((clock() - started) / 1000).toFixed(3)}`;
        process.stderr.write(
          `Adapter phase timing: target=${target} phase=${phase} status=${status}${elapsed}\n`
        );
      } catch {
        // Passive diagnostics cannot replace a proof result or its original error.
      }
    };
    log('started');
    try {
      const output = execute(executable, args, options);
      log('returned');
      return output;
    } catch (error) {
      log('failed');
      throw error;
    }
  };
  const started = performance.now();
  const compiled = executePhase(
    'compile',
    'cargo',
    [
      'test',
      '--quiet',
      '--locked',
      '--manifest-path',
      'Cargo.toml',
      '-p',
      'tmt-adapters',
      '--lib',
      '--no-run',
      '--message-format=json',
    ],
    { cwd: rustRoot, env, timeoutMs: 600_000 }
  );
  report(
    `Real-archive adapter acceptance compile: ${Math.ceil((performance.now() - started) / 1000)} seconds.`
  );
  const binaries = compiled
    .trim()
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line))
    .filter(
      (item) =>
        item.reason === 'compiler-artifact' &&
        item.target.name === 'tmt_adapters' &&
        item.profile.test &&
        item.executable
    )
    .map((item) => item.executable);
  if (binaries.length !== 1)
    throw new Error('Expected exactly one tmt-adapters lib-test executable.');
  const options = { cwd: rustRoot, env, timeoutMs: 120_000 };
  const listed = executePhase(
    'discovery',
    binaries[0],
    [ACCEPTANCE_TEST, '--exact', '--ignored', '--list'],
    options
  );
  const tests = listed.split('\n').filter((line) => line.endsWith(': test'));
  if (tests.length !== 1 || tests[0] !== `${ACCEPTANCE_TEST}: test`) {
    throw new Error('Expected exactly one discovered real-archive upgrade acceptance test.');
  }
  for (const source of [before, ...(floor ? [floor] : [])]) {
    const started = performance.now();
    const output = executePhase(
      source === before ? 'run-previous' : 'run-floor',
      binaries[0],
      [ACCEPTANCE_TEST, '--exact', '--ignored', '--nocapture'],
      {
        ...options,
        env: {
          ...env,
          TMT_UPGRADE_OLD_ARCHIVE: source.archive,
          TMT_UPGRADE_OLD_MANIFEST: source.manifest,
        },
      }
    );
    report(output.trim());
    if (
      !/^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; \d+ filtered out;/m.test(
        output
      )
    )
      throw new Error(
        'Expected exactly one passing executed real-archive upgrade acceptance test.'
      );
    report(
      `Real-archive adapter acceptance: passed (${source === before ? previous : upgradeSupportFloor(product)} -> ${tag}, ${target}); acquisition injected, real old/new binaries executed; ${Math.ceil((performance.now() - started) / 1000)} seconds.`
    );
  }
  return { outcome: 'proved' };
}

/**
 * What the proof can do for a release, from the plan `fetchUpgrade` wrote: `nothing` when the
 * product has no earlier published release, `predates` (with the reason, worded for the owner)
 * when the release's commit lacks the proof's scripts, `proved` when the proof can run.
 * `hasFileAt` says whether a file exists at the release's commit.
 */
export function assessUpgrade({ plan, hasFileAt }) {
  if (!plan.previous) return { outcome: 'nothing', reason: '' };
  const required = PROOF_FILES.filter(
    (file) => file !== 'verify-native-driver-upgrade.mjs' || plan.product === 'driver-herdr'
  );
  const missing = required.find((file) => !hasFileAt(`typescript/scripts/${file}`));
  if (missing) {
    return {
      outcome: 'predates',
      reason: `This release's commit has no ${missing}: it predates the automated upgrade proof, so prove the upgrade by hand, as the native release verification guide describes.`,
    };
  }
  return { outcome: 'proved', reason: '' };
}

/** `text` as one line of at most `limit` characters: control characters become spaces. */
function oneLine(text, limit) {
  const line = text
    .replace(/\p{Cc}+/gu, ' ')
    .replace(/ {2,}/g, ' ')
    .trim();
  return line.length > limit ? `${line.slice(0, limit - 3)}...` : line;
}

/**
 * Why one host's proof failed, from its log: the first error a verifier threw, on one line. The
 * release commit's own scripts wrote the log, so it is data, never trusted beyond that.
 */
export function failureCause(log) {
  const error = /^(?:Assertion)?Error(?: \[[A-Z_]+\])?: (.+)$/m.exec(log);
  return oneLine(error?.[1] ?? 'the proof failed without an error message', 300);
}

/**
 * One sentence for the hold reason from the logs of the hosts that failed: each distinct cause
 * once, with the hosts it happened on.
 */
export function combineFailures(failures) {
  const hosts = new Map();
  for (const { target, log } of failures) {
    const cause = failureCause(log);
    hosts.set(cause, [...(hosts.get(cause) ?? []), target].sort());
  }
  return oneLine(
    [...hosts].map(([cause, targets]) => `${cause} (${targets.join(', ')})`).join('; '),
    600
  );
}

const COMMIT = /^[0-9a-f]{40}$/;

/**
 * The commit a release was made from: a draft points at it directly, a published release may
 * name a branch, in which case its tag does. `commitOfTag` resolves a tag name to a commit.
 */
export function releaseCommit({ release, commitOfTag }) {
  if (COMMIT.test(release.target_commitish ?? '')) return release.target_commitish;
  if (release.draft === true) {
    throw new Error(
      `Draft ${release.tag_name} points at ${release.target_commitish}, not a commit.`
    );
  }
  const commit = commitOfTag(release.tag_name);
  if (!COMMIT.test(commit))
    throw new Error(`Tag ${release.tag_name} does not resolve to a commit.`);
  return commit;
}

/** Downloads one immutable release asset by id; only acquisition failures get bounded retries. */
export function ghAssetDownloader({
  repository,
  env = process.env,
  spawn = spawnSync,
  sleep = (milliseconds) =>
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds),
}) {
  return (asset, file) => {
    const bytes = retryReleaseRead(
      () => {
        const result = spawn(
          'gh',
          [
            'api',
            '-H',
            'Accept: application/octet-stream',
            `repos/${repository}/releases/assets/${asset.id}`,
          ],
          { env, encoding: 'buffer', timeout: 300_000, maxBuffer: ASSET_LIMIT }
        );
        if (result.error) throw result.error;
        if (result.status !== 0) {
          throw new Error(
            `gh could not download ${asset.name} (${result.status}): ${result.stderr}`
          );
        }
        return result.stdout;
      },
      { label: 'Asset download', sleep }
    );
    // File writes and the caller's byte verification are never retried.
    writeFileSync(file, bytes);
  };
}

const here = path.dirname(fileURLToPath(import.meta.url));

/** REST-only tag/ancestry evidence: fetching draft-visible assets never executes release code. */
export function ghCliAncestry({ repository, env = process.env, spawn = spawnSync }) {
  const read = (endpoint, filter) => {
    const result = spawn('gh', ['api', `repos/${repository}/${endpoint}`, '--jq', filter], {
      env,
      encoding: 'utf8',
      timeout: 60_000,
      maxBuffer: 4 * 1024 * 1024,
    });
    if (result.error) throw result.error;
    if (result.status !== 0)
      throw new Error(`CLI ancestry REST read failed for ${endpoint} (${result.status}).`);
    return JSON.parse(result.stdout);
  };
  return (release, registration) => {
    if (release.draft !== false || !COMMIT.test(registration))
      throw new Error('CLI ancestry requires a published release and registration SHA.');
    // target_commitish may describe the historical release branch, not its actual tag.
    const sha = read(`commits/${encodeURIComponent(release.tag_name)}`, '{sha: .sha}')?.sha;
    if (typeof sha !== 'string' || !COMMIT.test(sha))
      throw new Error(`Published CLI tag ${release.tag_name} has no resolved commit.`);
    const comparison = read(
      `compare/${registration}...${sha}`,
      '{status: .status, base_commit: {sha: .base_commit.sha}, merge_base_commit: {sha: .merge_base_commit.sha}}'
    );
    if (
      comparison?.base_commit?.sha !== registration ||
      !['ahead', 'behind', 'identical', 'diverged'].includes(comparison.status) ||
      (comparison.status === 'identical') !== (sha === registration) ||
      (comparison.status === 'behind' && comparison.merge_base_commit?.sha !== sha) ||
      (['ahead', 'identical'].includes(comparison.status) &&
        comparison.merge_base_commit?.sha !== registration)
    )
      throw new Error(`Unknown REST CLI ancestry for ${release.tag_name}.`);
    return { sha, status: comparison.status };
  };
}

function main(argv, environment) {
  const [command, ...rest] = argv;
  const { values } = parseArgs({
    args: rest,
    options: {
      product: { type: 'string' },
      tag: { type: 'string' },
      target: { type: 'string' },
      directory: { type: 'string' },
      sha: { type: 'string' },
      'candidate-directory': { type: 'string' },
      'source-root': { type: 'string' },
      skill: { type: 'string', default: 'skills/tmt/SKILL.md' },
    },
  });
  const required = (names) => {
    for (const name of names) if (!values[name]) throw new Error(`--${name} is required.`);
  };
  const report = (line) => {
    process.stderr.write(`${line}\n`);
    if (environment.GITHUB_STEP_SUMMARY)
      appendFileSync(environment.GITHUB_STEP_SUMMARY, `${line}\n`);
  };
  if (command === 'acceptance') {
    required(['product', 'tag', 'target', 'directory']);
    proveArchiveAcceptance({
      directory: path.resolve(values.directory),
      product: values.product,
      tag: values.tag,
      target: values.target,
      sourceRoot: values['source-root'],
      environment,
      report,
    });
    return;
  }
  if (command === 'prove') {
    // Offline by design: the release's own code runs here, so no token and no GitHub.
    required(['product', 'tag', 'target', 'directory']);
    const { previous } = proveStaged({
      directory: values.directory,
      product: values.product,
      tag: values.tag,
      target: values.target,
      skill: values.skill,
      sourceRoot: values['source-root'],
      run: (script, args) => {
        const result = spawnSync('node', [path.join(here, script), ...args], { stdio: 'inherit' });
        if (result.error) throw result.error;
        if (result.status !== 0) throw new Error(`${script} failed with ${result.status}.`);
      },
    });
    report(
      previous
        ? `Upgrade proof passed: ${previous} -> ${values.tag} (${values.product}, ${values.target}).`
        : `No published ${values.product} release precedes ${values.tag}; there is nothing to upgrade from.`
    );
    return;
  }
  if (command === 'reason') {
    // The proof's hosts leave `upgrade-proof-log-<target>/upgrade-proof.log`; a missing directory
    // means no host failed, which is no reason at all.
    required(['directory']);
    const failures = existsSync(values.directory)
      ? readdirSync(values.directory)
          .filter((name) => name.startsWith('upgrade-proof-log-'))
          .map((name) => ({
            target: name.slice('upgrade-proof-log-'.length),
            log: readFileSync(path.join(values.directory, name, 'upgrade-proof.log'), 'utf8'),
          }))
      : [];
    const reason = failures.length === 0 ? '' : combineFailures(failures);
    if (reason) report(`The upgrade proof failed: ${reason}`);
    if (environment.GITHUB_OUTPUT) appendFileSync(environment.GITHUB_OUTPUT, `reason=${reason}\n`);
    else process.stdout.write(`${reason}\n`);
    return;
  }
  const repository = environment.GITHUB_REPOSITORY;
  if (!repository) throw new Error('GITHUB_REPOSITORY is not set.');
  if (command === 'assess') {
    required(['directory', 'sha']);
    const plan = JSON.parse(readFileSync(path.join(values.directory, PLAN), 'utf8'));
    const { outcome, reason } = assessUpgrade({
      plan,
      hasFileAt: (file) =>
        spawnSync(
          'gh',
          ['api', '--silent', `repos/${repository}/contents/${file}?ref=${values.sha}`],
          {
            encoding: 'utf8',
            timeout: 60_000,
          }
        ).status === 0,
    });
    if (reason) report(reason);
    // A reason may hold any character a file name can; the output is one line, so escape newlines.
    const lines = [`outcome=${outcome}`, `reason=${reason.replaceAll('\n', ' ')}`].join('\n');
    if (environment.GITHUB_OUTPUT) appendFileSync(environment.GITHUB_OUTPUT, `${lines}\n`);
    else process.stdout.write(`${lines}\n`);
    return;
  }
  const releases = ghApi({ repository }).listReleases();
  if (command === 'resolve') {
    required(['tag']);
    const release = releases.find((candidate) => candidate.tag_name === values.tag);
    if (!release) throw new Error(`There is no release ${values.tag}.`);
    const commitOfTag = (tag) =>
      spawnSync('gh', ['api', `repos/${repository}/commits/${tag}`, '--jq', '.sha'], {
        encoding: 'utf8',
        timeout: 60_000,
      }).stdout.trim();
    const commit = releaseCommit({ release, commitOfTag });
    if (environment.GITHUB_OUTPUT) appendFileSync(environment.GITHUB_OUTPUT, `sha=${commit}\n`);
    else process.stdout.write(`${commit}\n`);
  } else if (command === 'fetch') {
    required(['product', 'tag', 'directory']);
    const plan = fetchUpgrade({
      releases,
      download: ghAssetDownloader({ repository }),
      observeCli: ghCliAncestry({ repository }),
      product: values.product,
      tag: values.tag,
      directory: values.directory,
      // A rehearsal stages the verified bundle of its own run instead of a GitHub release.
      local: values['candidate-directory']
        ? localCandidate({
            directory: path.resolve(values['candidate-directory']),
            product: values.product,
            tag: values.tag,
          })
        : undefined,
    });
    report(
      plan.previous
        ? `Fetched ${values.tag} and ${plan.previous}${plan.driver ? ` with ${plan.driver}` : ''} for ${Object.keys(plan.files).length} files.`
        : `No published ${values.product} release precedes ${values.tag}; nothing was fetched.`
    );
  } else {
    throw new Error(
      'Usage: release-upgrade.mjs resolve|fetch|assess|prove|acceptance|reason --tag TAG ...'
    );
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2), process.env);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
