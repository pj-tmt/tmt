#!/usr/bin/env node
// Proves that a release candidate upgrades from the last published release of its product, with
// the real bytes of both: the candidate's archive from its release (a draft or a published
// release), the previous archive from the newest published release below it, each checked
// against the digest GitHub recorded for the asset before anything runs.
//   node release-upgrade.mjs prove --product cli|office|squad --tag TAG --target TARGET \
//     --directory DIR [--skill skills/tmux-team/SKILL.md]
//   node release-upgrade.mjs resolve --tag TAG      the commit of the release, on stdout
// A CLI candidate runs the managed-install lifecycle verifier over the two archives. An extension
// candidate is installed and upgraded by the newest published CLI, which is what a user's
// `tmt <extension> install` runs, because an extension release carries no CLI.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { archivePrefix } from './native-release-policy.mjs';
import { ghApi } from './release-draft-assets.mjs';
import { compareVersions, publishedReleases, versionOfTag } from './release-versions.mjs';

const DIGEST = /^sha256:[0-9a-f]{64}$/;
const MANIFEST = 'dist-manifest.json';
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

/** Downloads a release's archive and manifest for the target, each checked against its digest. */
export function stageRelease({ download, release, product, target, directory }) {
  mkdirSync(directory, { recursive: true });
  const staged = {};
  for (const [role, asset] of Object.entries(selectAssets({ release, product, target }))) {
    const file = path.join(directory, asset.name);
    download(asset, file);
    if (sha256(file) !== asset.digest) {
      throw new Error(`${asset.name} of ${release.tag_name} does not match its recorded digest.`);
    }
    staged[role] = file;
  }
  return staged;
}

/**
 * Proves the candidate's upgrade from the previous published release. `run` executes one
 * verifier script and throws when it fails. Returns the previous tag, or null when the product
 * has no earlier published release and there is nothing to upgrade from.
 */
export function proveUpgrade({ releases, download, run, product, tag, target, directory, skill }) {
  const candidate = releases.find((release) => release.tag_name === tag);
  if (!candidate) throw new Error(`There is no release ${tag}.`);
  const previous = selectPrevious({ releases, product, candidateTag: tag });
  if (!previous) return { previous: null };

  const stage = (release, kind, releaseProduct = product) =>
    stageRelease({
      download,
      release,
      product: releaseProduct,
      target,
      directory: path.join(directory, kind),
    });
  const now = stage(candidate, 'candidate');
  const before = stage(previous, 'previous');
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
    const [driverRelease] = publishedReleases(releases, 'cli');
    if (!driverRelease)
      throw new Error('An extension upgrade proof needs a published CLI release.');
    const driver = stage(driverRelease, 'driver', 'cli');
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
  return { previous: previous.tag_name };
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
  const repository = environment.GITHUB_REPOSITORY;
  if (!repository) throw new Error('GITHUB_REPOSITORY is not set.');
  const required = (names) => {
    for (const name of names) if (!values[name]) throw new Error(`--${name} is required.`);
  };
  if (command === 'resolve') {
    required(['tag']);
    const release = ghApi({ repository })
      .listReleases()
      .find((candidate) => candidate.tag_name === values.tag);
    if (!release) throw new Error(`There is no release ${values.tag}.`);
    const commitOfTag = (tag) =>
      spawnSync('gh', ['api', `repos/${repository}/commits/${tag}`, '--jq', '.sha'], {
        encoding: 'utf8',
        timeout: 60_000,
      }).stdout.trim();
    const commit = releaseCommit({ release, commitOfTag });
    if (environment.GITHUB_OUTPUT) appendFileSync(environment.GITHUB_OUTPUT, `sha=${commit}\n`);
    else process.stdout.write(`${commit}\n`);
    return;
  }
  if (command !== 'prove') {
    throw new Error('Usage: release-upgrade.mjs prove|resolve --tag TAG ...');
  }
  required(['product', 'tag', 'target', 'directory']);
  const { previous } = proveUpgrade({
    releases: ghApi({ repository }).listReleases(),
    download: ghAssetDownloader({ repository }),
    run: (script, args) => {
      const result = spawnSync('node', [path.join(here, script), ...args], { stdio: 'inherit' });
      if (result.error) throw result.error;
      if (result.status !== 0) throw new Error(`${script} failed with ${result.status}.`);
    },
    product: values.product,
    tag: values.tag,
    target: values.target,
    directory: values.directory,
    skill: values.skill,
  });
  const line = previous
    ? `Upgrade proof passed: ${previous} -> ${values.tag} (${values.product}, ${values.target}).`
    : `No published ${values.product} release precedes ${values.tag}; there is nothing to upgrade from.`;
  process.stderr.write(`${line}\n`);
  if (environment.GITHUB_STEP_SUMMARY) appendFileSync(environment.GITHUB_STEP_SUMMARY, `${line}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2), process.env);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
