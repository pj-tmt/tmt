#!/usr/bin/env node
// The draft release carries the state of its own build, so a release run can be replaced,
// cancelled or repeated without losing or duplicating work:
//   check         does the draft still need a build?          (todo=true|false, awaiting=true|false)
//   attach        upload the verified bundle, marker last
//   record-failure  upload verification-failed.json with the run URL
// Publication is not done here: a draft that carries the bundle marker is complete, and
// `release-publish.mjs` publishes it once the gates passed.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  appendFileSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import {
  archivePrefix,
  productOfTag,
  releaseFlags,
  releasePolicy,
} from './native-release-policy.mjs';
import { BUNDLE_ASSET, FAILURE_ASSET, HOLD_ASSET, releasesFrom } from './plan-release-builds.mjs';

import {
  createReleaseRecord,
  releaseRecordBytes,
  RELEASE_RECORD,
  RECORD_LIMIT,
} from './release-index.mjs';

const MANIFEST = 'dist-manifest.json';
const DIGEST_ATTEMPTS = 5;

/** The files a product's release carries besides the marker, from the final cargo-dist manifest. */
export function bundleFiles(product, manifest, tag) {
  if (manifest.announcement_tag !== tag) {
    throw new Error(`The bundle is for ${manifest.announcement_tag}, not for ${tag}.`);
  }
  const archives = Object.entries(manifest.artifacts ?? {})
    .filter(([, artifact]) => artifact.kind === 'executable-zip')
    .map(([name]) => name)
    .sort();
  if (archives.length !== 4) {
    throw new Error(`Expected four archives in the bundle of ${tag}, found ${archives.length}.`);
  }
  const prefix = `${archivePrefix(product)}-`;
  for (const name of archives) {
    if (!name.startsWith(prefix) || !name.endsWith('.tar.gz')) {
      throw new Error(`Unexpected archive ${name} in the ${product} bundle.`);
    }
  }
  return [...archives, MANIFEST, ...(product === 'cli' ? ['tmt-installer.sh', 'install.sh'] : [])];
}

const sha256 = (file) => `sha256:${createHash('sha256').update(readFileSync(file)).digest('hex')}`;

function findDraft(releases, tag) {
  const release = releases.find(({ tag_name: name }) => name === tag);
  if (!release) throw new Error(`There is no release ${tag}.`);
  if (release.draft !== true) {
    throw new Error(`Release ${tag} is published; nothing can be attached to it.`);
  }
  return release;
}

const assetsOf = (release) => release.assets ?? [];
const hasAsset = (release, name) => assetsOf(release).some((asset) => asset.name === name);

/**
 * Whether the draft still needs a build. A bundle means it is complete, and `awaiting` says that
 * a complete draft carries no hold and waits for its gates and its publication; a failure marker
 * parks it, and a retry removes the marker so that this run owns the draft again.
 */
export function checkDraft({ api, tag, retry = false }) {
  const release = findDraft(api.listReleases(), tag);
  if (hasAsset(release, BUNDLE_ASSET)) {
    return {
      todo: false,
      awaiting: !hasAsset(release, HOLD_ASSET),
      reason: 'it already carries a bundle',
    };
  }
  if (hasAsset(release, FAILURE_ASSET)) {
    if (!retry) {
      return {
        todo: false,
        awaiting: false,
        reason: 'its failure is recorded; retry it by dispatch',
      };
    }
    for (const asset of assetsOf(release).filter(({ name }) => name === FAILURE_ASSET)) {
      api.deleteAsset(asset.id);
    }
  }
  return { todo: true, awaiting: false, reason: '' };
}

/**
 * Uploads the verified bundle to the draft: the product's archives, final manifest and
 * installers first, their digests checked against the local bytes, and the marker last, so a
 * draft that has the marker has everything. Stale files of an interrupted upload are replaced.
 */
export function attachBundle({ api, product, tag, directory, sleep = () => {} }) {
  if (productOfTag(tag) !== product) {
    throw new Error(`Tag ${tag} is not a ${product} tag.`);
  }
  const release = findDraft(api.listReleases(), tag);
  if (hasAsset(release, BUNDLE_ASSET)) throw new Error(`Draft ${tag} already carries a bundle.`);

  const manifest = JSON.parse(readFileSync(path.join(directory, MANIFEST), 'utf8'));
  const names = bundleFiles(product, manifest, tag);
  const marker = path.join(directory, BUNDLE_ASSET);
  const expectedMarker = JSON.stringify({
    ...releasePolicy(product),
    flags: releaseFlags(product),
  });
  if (!existsSync(marker) || readFileSync(marker, 'utf8').trim() !== expectedMarker) {
    throw new Error(`${BUNDLE_ASSET} does not match the ${product} publication policy.`);
  }
  for (const name of names) {
    if (!existsSync(path.join(directory, name))) throw new Error(`The bundle has no ${name}.`);
  }

  const preparedRecord = createReleaseRecord({
    product,
    tag,
    releaseId: release.id,
    sourceSha: release.target_commitish,
    directory,
  });
  const upload = (name) => {
    for (const stale of assetsOf(release).filter((asset) => asset.name === name)) {
      api.deleteAsset(stale.id);
    }
    api.upload(release, name, path.join(directory, name));
  };
  for (const name of names) upload(name);

  const storedAssets = (selectedNames) => {
    for (let attempt = 1; attempt <= DIGEST_ATTEMPTS; attempt += 1) {
      const current = findDraft(api.listReleases(), tag);
      if (current.id !== release.id || current.target_commitish !== release.target_commitish)
        throw new Error(`Draft ${tag} identity changed during upload.`);
      const selected = selectedNames.map((name) => {
        const matches = assetsOf(current).filter((asset) => asset.name === name);
        if (matches.length > 1) throw new Error(`Draft ${tag} has duplicate ${name}.`);
        return matches[0];
      });
      if (selected.every((asset) => asset?.digest)) return selected;
      if (attempt === DIGEST_ATTEMPTS) {
        const missing = selectedNames.filter((_, index) => !selected[index]?.digest);
        throw new Error(`GitHub reports no digest for ${missing.join(', ')}.`);
      }
      sleep(2000);
    }
  };
  const uploaded = storedAssets(names);
  names.forEach((name, index) => {
    const local = sha256(path.join(directory, name));
    if (uploaded[index].digest !== local) {
      throw new Error(`${name} was stored as ${uploaded[index].digest}, expected ${local}.`);
    }
  });
  for (const asset of [preparedRecord.manifest, ...Object.values(preparedRecord.archives)]) {
    if (uploaded.find(({ name }) => name === asset.name)?.digest !== `sha256:${asset.sha256}`)
      throw new Error(`Stored ${asset.name} differs from the prepared record.`);
  }
  const record = releaseRecordBytes(preparedRecord);
  writeFileSync(path.join(directory, RELEASE_RECORD), record);
  upload(RELEASE_RECORD);
  const stored = storedAssets([RELEASE_RECORD]);
  if (stored[0].digest !== `sha256:${createHash('sha256').update(record).digest('hex')}`)
    throw new Error(`Stored ${RELEASE_RECORD} digest mismatch.`);
  const downloaded = api.downloadAsset(stored[0].id);
  if (Buffer.byteLength(downloaded) > RECORD_LIMIT || !Buffer.from(downloaded).equals(record))
    throw new Error(`Stored ${RELEASE_RECORD} bytes mismatch.`);
  upload(BUNDLE_ASSET);
  return { uploaded: [...names, RELEASE_RECORD, BUNDLE_ASSET] };
}

/** Parks the draft: uploads which run failed and on what, unless the draft is already complete. */
export function recordFailure({ api, tag, runUrl, sha, jobs, now = new Date() }) {
  const release = findDraft(api.listReleases(), tag);
  if (hasAsset(release, BUNDLE_ASSET)) return { recorded: false };
  const directory = mkdtempSync(path.join(os.tmpdir(), 'release-failure-'));
  try {
    const file = path.join(directory, FAILURE_ASSET);
    writeFileSync(
      file,
      `${JSON.stringify({ tag, sha, runUrl, failedJobs: jobs, recordedAt: now.toISOString() }, null, 2)}\n`
    );
    for (const stale of assetsOf(release).filter(({ name }) => name === FAILURE_ASSET)) {
      api.deleteAsset(stale.id);
    }
    api.upload(release, FAILURE_ASSET, file);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
  return { recorded: true };
}

/**
 * Parks the complete bundle of a draft: uploads which gate held its publication, why, and which
 * run. A draft that is not complete has nothing to hold, and a draft that already has a marker
 * gets the new one in its place (the gate that failed last is the one to release).
 */
export function recordHold({ api, tag, hold, now = new Date() }) {
  const release = findDraft(api.listReleases(), tag);
  if (!hasAsset(release, BUNDLE_ASSET)) throw new Error(`Draft ${tag} has no bundle to hold.`);
  const directory = mkdtempSync(path.join(os.tmpdir(), 'release-hold-'));
  try {
    const file = path.join(directory, HOLD_ASSET);
    writeFileSync(
      file,
      `${JSON.stringify({ tag, ...hold, recordedAt: now.toISOString() }, null, 2)}\n`
    );
    for (const stale of assetsOf(release).filter(({ name }) => name === HOLD_ASSET)) {
      api.deleteAsset(stale.id);
    }
    api.upload(release, HOLD_ASSET, file);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

/** Removes the hold marker of a draft, if it has one. Returns whether there was one. */
export function clearHold({ api, tag }) {
  const release = findDraft(api.listReleases(), tag);
  const markers = assetsOf(release).filter(({ name }) => name === HOLD_ASSET);
  for (const marker of markers) api.deleteAsset(marker.id);
  return markers.length > 0;
}

/** The hold marker of a draft (its gate and reason), or null. `download` returns an asset's text. */
export function readHold({ api, tag, download }) {
  const marker = assetsOf(findDraft(api.listReleases(), tag)).find(
    ({ name }) => name === HOLD_ASSET
  );
  return marker ? JSON.parse(download(marker)) : null;
}

/** Retries only an acquisition read; validation and writes stay with the caller. */
export function retryReleaseRead(
  read,
  {
    label,
    sleep = (milliseconds) =>
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds),
  }
) {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return read();
    } catch (failure) {
      if (attempt === 3) throw failure;
      const delay = 1000 * 2 ** (attempt - 1);
      process.stderr.write(
        `${label} attempt ${attempt}/3 failed: ${failure.message}\nRetrying in ${delay} ms.\n`
      );
      sleep(delay);
    }
  }
}

/**
 * `gh api` for one repository. gh may write notices to stderr on success, which
 * `runPackedCommand` would reject, so this runs it under its own bound and reads the status.
 */
export function ghApi({ repository, env = process.env, spawn = spawnSync, sleep }) {
  const gh = (args, timeout = 60_000, maxBuffer = 64 * 1024 * 1024) => {
    const result = spawn('gh', args, {
      env,
      encoding: 'utf8',
      timeout,
      maxBuffer,
    });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(
        `gh ${args.slice(0, 3).join(' ')} failed with ${result.status}: ${result.stderr}`
      );
    }
    return result.stdout;
  };
  return {
    listReleases: () =>
      releasesFrom(
        retryReleaseRead(
          () => JSON.parse(gh(['api', '--paginate', '--slurp', `repos/${repository}/releases`])),
          { label: 'Release listing', sleep }
        )
      ),
    upload: (release, name, file) =>
      gh(
        [
          'api',
          '--method',
          'POST',
          '-H',
          'Content-Type: application/octet-stream',
          `https://uploads.github.com/repos/${repository}/releases/${release.id}/assets?name=${encodeURIComponent(name)}`,
          '--input',
          file,
        ],
        300_000
      ),
    downloadAsset: (id) => {
      if (!Number.isSafeInteger(id) || id <= 0)
        throw new Error('Record readback requires an asset ID.');
      return gh(
        [
          'api',
          '-H',
          'Accept: application/octet-stream',
          `repos/${repository}/releases/assets/${id}`,
        ],
        60_000,
        RECORD_LIMIT
      );
    },
    deleteAsset: (id) =>
      gh(['api', '--method', 'DELETE', `repos/${repository}/releases/assets/${id}`]),
  };
}

function main(argv) {
  const [command, ...rest] = argv;
  const { values } = parseArgs({
    args: rest,
    options: {
      product: { type: 'string' },
      tag: { type: 'string' },
      directory: { type: 'string' },
      retry: { type: 'boolean', default: false },
      'run-url': { type: 'string' },
      sha: { type: 'string' },
      jobs: { type: 'string', default: '' },
    },
  });
  if (!values.tag) throw new Error('--tag is required.');
  const repository = process.env.GITHUB_REPOSITORY;
  if (!repository) throw new Error('GITHUB_REPOSITORY is not set.');
  const api = ghApi({ repository });
  if (command === 'check') {
    const { todo, awaiting, reason } = checkDraft({
      api,
      tag: values.tag,
      retry: values.retry,
    });
    process.stderr.write(
      `${values.tag}: ${todo ? 'needs a build' : awaiting ? 'complete, waits for its publication' : `skipped, ${reason}`}.\n`
    );
    const lines = `todo=${todo}\nawaiting=${awaiting}\n`;
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, lines);
    else process.stdout.write(lines);
  } else if (command === 'attach') {
    if (!values.product || !values.directory)
      throw new Error('attach needs --product and --directory.');
    const { uploaded } = attachBundle({
      api,
      product: values.product,
      tag: values.tag,
      directory: values.directory,
      sleep: (milliseconds) =>
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds),
    });
    process.stderr.write(`Attached to ${values.tag}: ${uploaded.join(', ')}.\n`);
  } else if (command === 'record-failure') {
    if (!values['run-url'] || !values.sha)
      throw new Error('record-failure needs --run-url and --sha.');
    const { recorded } = recordFailure({
      api,
      tag: values.tag,
      runUrl: values['run-url'],
      sha: values.sha,
      jobs: values.jobs.split(',').filter(Boolean),
    });
    process.stderr.write(
      recorded
        ? `Recorded the failure on ${values.tag}.\n`
        : `${values.tag} is complete; nothing recorded.\n`
    );
  } else {
    throw new Error(
      'Usage: release-draft-assets.mjs <check|attach|record-failure> --tag <tag> ...'
    );
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
