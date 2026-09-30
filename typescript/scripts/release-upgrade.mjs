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
//   node release-upgrade.mjs fetch --product cli|office|squad --tag TAG --directory DIR
//   node release-upgrade.mjs prove --product P --tag TAG --target T --directory DIR [--skill S]
// A CLI candidate runs the managed-install lifecycle verifier over the two archives. An extension
// candidate is installed and upgraded by the newest published CLI, which is what a user's
// `tmt <extension> install` runs, because an extension release carries no CLI.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { archivePrefix } from './native-release-policy.mjs';
import { ghApi } from './release-draft-assets.mjs';
import { compareVersions, publishedReleases, versionOfTag } from './release-versions.mjs';

const DIGEST = /^sha256:[0-9a-f]{64}$/;
const MANIFEST = 'dist-manifest.json';
const PLAN = 'plan.json';
const ASSET_LIMIT = 80 * 1024 * 1024;

/** The newest published release of a product below the candidate's version, or null. */
export function selectPrevious({ releases, product, candidateTag }) {
  const candidate = versionOfTag(candidateTag, product);
  return (
    publishedReleases(releases, product).find(
      (release) => compareVersions(versionOfTag(release.tag_name, product), candidate) < 0
    ) ?? null
  );
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
 * The fetch step. Stages, for every target of the candidate, the candidate, the previous release
 * and, for an extension, the newest published CLI under `directory/<target>/<kind>`, and writes
 * `plan.json` with the tags and the digests. A product with no earlier published release stages
 * nothing: there is nothing to upgrade from.
 */
export function fetchUpgrade({ releases, download, product, tag, directory }) {
  const candidate = releases.find((release) => release.tag_name === tag);
  if (!candidate) throw new Error(`There is no release ${tag}.`);
  const previous = selectPrevious({ releases, product, candidateTag: tag });
  const plan = { product, tag, previous: previous?.tag_name ?? null, driver: null, files: {} };
  mkdirSync(directory, { recursive: true });
  if (previous) {
    const targets = archiveTargets({ release: candidate, product });
    if (targets.length === 0) throw new Error(`Release ${tag} has no archive to upgrade to.`);
    let driverRelease = null;
    if (product !== 'cli') {
      [driverRelease] = publishedReleases(releases, 'cli');
      if (!driverRelease)
        throw new Error('An extension upgrade proof needs a published CLI release.');
      plan.driver = driverRelease.tag_name;
    }
    for (const target of targets) {
      const stage = (release, kind, releaseProduct) => {
        const { digests } = stageRelease({
          download,
          release,
          product: releaseProduct,
          target,
          directory: path.join(directory, target, kind),
        });
        for (const [name, digest] of Object.entries(digests)) {
          plan.files[path.posix.join(target, kind, name)] = digest;
        }
      };
      stage(candidate, 'candidate', product);
      stage(previous, 'previous', product);
      if (driverRelease) stage(driverRelease, 'driver', 'cli');
    }
  }
  writeFileSync(path.join(directory, PLAN), `${JSON.stringify(plan, null, 2)}\n`);
  return plan;
}

/**
 * The prove step, over the directory `fetchUpgrade` wrote. Every file is checked against the
 * digest in the plan again, since it crossed a job boundary, and `run` executes the product's
 * verifier script over the target's files and throws when it fails. Returns the previous tag, or
 * null when there was nothing to upgrade from.
 */
export function proveStaged({ directory, product, tag, target, run, skill }) {
  const plan = JSON.parse(readFileSync(path.join(directory, PLAN), 'utf8'));
  if (plan.product !== product || plan.tag !== tag) {
    throw new Error(`The staged assets are for ${plan.tag}, not for ${tag}.`);
  }
  if (!plan.previous) return { previous: null };
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
    return {
      archive: path.join(base, `${archivePrefix(kindProduct)}-${target}.tar.gz`),
      manifest: path.join(base, MANIFEST),
    };
  };
  const now = staged('candidate', product);
  const before = staged('previous', product);
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
    run('verify-native-installation.mjs', [...common, '--skill', skill]);
  } else {
    const driver = staged('driver', 'cli');
    run('verify-native-extension-upgrade.mjs', [
      ...common,
      '--product',
      product,
      '--driver-archive',
      driver.archive,
      '--driver-manifest',
      driver.manifest,
    ]);
  }
  return { previous: plan.previous };
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

/** Downloads one release asset by id, which also works for the assets of a draft. */
export function ghAssetDownloader({ repository, env = process.env, spawn = spawnSync }) {
  return (asset, file) => {
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
      throw new Error(`gh could not download ${asset.name} (${result.status}): ${result.stderr}`);
    }
    writeFileSync(file, result.stdout);
  };
}

const here = path.dirname(fileURLToPath(import.meta.url));

function main(argv, environment) {
  const [command, ...rest] = argv;
  const { values } = parseArgs({
    args: rest,
    options: {
      product: { type: 'string' },
      tag: { type: 'string' },
      target: { type: 'string' },
      directory: { type: 'string' },
      skill: { type: 'string', default: 'skills/tmux-team/SKILL.md' },
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
  if (command === 'prove') {
    // Offline by design: the release's own code runs here, so no token and no GitHub.
    required(['product', 'tag', 'target', 'directory']);
    const { previous } = proveStaged({
      directory: values.directory,
      product: values.product,
      tag: values.tag,
      target: values.target,
      skill: values.skill,
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
  const repository = environment.GITHUB_REPOSITORY;
  if (!repository) throw new Error('GITHUB_REPOSITORY is not set.');
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
      product: values.product,
      tag: values.tag,
      directory: values.directory,
    });
    report(
      plan.previous
        ? `Fetched ${values.tag} and ${plan.previous}${plan.driver ? ` with ${plan.driver}` : ''} for ${Object.keys(plan.files).length} files.`
        : `No published ${values.product} release precedes ${values.tag}; nothing was fetched.`
    );
  } else {
    throw new Error('Usage: release-upgrade.mjs resolve|fetch|prove --tag TAG ...');
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
