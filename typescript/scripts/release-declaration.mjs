// Generated extension metadata. Source inputs belong to the captured component map, not archives.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseComponentMap } from './ci-scope.mjs';
import { componentOfProduct } from './native-release-policy.mjs';
import { publishedReleases, versionOfTag } from './release-versions.mjs';
import { ghCliAncestry } from './release-upgrade.mjs';
import { readReleaseCatalog } from './release-cut-read.mjs';
import { withReleaseSourceAtRef } from './release-source-at-ref.mjs';
import { runPackedCommand } from './packed-command.mjs';
import { readBoundedFile } from './native-artifact-policy.mjs';

export const DECLARATION_SUPPORT = 'cb5ca3d09c584dc78d333b548208a2ca90498cb0';
export const DECLARATION_FILE = 'TMT-USES.json';
export const DECLARATION_OUTPUT = `rust/target/native-declaration/${DECLARATION_FILE}`;
const MARKER = 'tmt-release-cli';
const SHA = /^[a-f0-9]{40}$/;
const EXTENSIONS = new Set(['ops', 'digest', 'remote', 'colab']);
// Published beta/rc versions are valid minima too; preparation -dev is not a publication floor.
function minimumVersion(version) {
  const match =
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(
      version ?? ''
    );
  return Boolean(
    match &&
    match[4] !== 'dev' &&
    match.slice(1, 4).every((part) => Number.isSafeInteger(Number(part))) &&
    (!match[4] ||
      match[4]
        .split('.')
        .every((part) => !/^\d+$/.test(part) || part === '0' || !part.startsWith('0')))
  );
}

/** Reuse the release-upgrade REST resolver with one cache, request cap and deadline. */
export function declarationAncestry({ repository }) {
  let requests = 0;
  const deadline = Date.now() + 90_000;
  const observe = ghCliAncestry({
    repository,
    spawn: (command, args, options) => {
      assert(
        ++requests <= 60 && Date.now() < deadline,
        'Declaration ancestry REST budget exceeded.'
      );
      return spawnSync(command, args, {
        ...options,
        timeout: Math.min(options.timeout, deadline - Date.now()),
      });
    },
  });
  const cache = new Map();
  return (release, floor) => {
    const key = `${release.tag_name}:${floor}`;
    if (!cache.has(key)) cache.set(key, observe(release, floor));
    return cache.get(key);
  };
}

export const declaresMinimumCli = (product) => EXTENSIONS.has(product);

/** The cut planner and full-history preparation checkout share the same local ancestry port. */
export function gitDeclarationAncestry(git) {
  const tags = new Map();
  const cache = new Map();
  return (cli, floor) => {
    const key = `${cli.tag_name}:${floor}`;
    if (cache.has(key)) return cache.get(key);
    if (!tags.has(cli.tag_name))
      tags.set(
        cli.tag_name,
        git(['rev-parse', '--verify', `refs/tags/${cli.tag_name}^{commit}`]).trim()
      );
    const sha = tags.get(cli.tag_name);
    assert(SHA.test(sha), 'Missing supporting CLI tag commit.');
    try {
      git(['merge-base', '--is-ancestor', floor, sha]);
    } catch (error) {
      if (error.cause?.status !== 1) throw error;
      const observation = { sha, status: 'behind' };
      cache.set(key, observation);
      return observation;
    }
    const observation = { sha, status: floor === sha ? 'identical' : 'ahead' };
    cache.set(key, observation);
    return observation;
  };
}

export function declarationSource(cut, root = process.cwd()) {
  return withReleaseSourceAtRef(
    cut,
    (source) =>
      parseComponentMap(readFileSync(join(source, '.github/components.json'), 'utf8'), {
        historical: true,
      }),
    { root }
  );
}

function floors(map, product) {
  const registration = componentOfProduct(map, product).requiresCliSha;
  assert(SHA.test(registration ?? ''), 'Extension declaration needs its product registration SHA.');
  return [...new Set([DECLARATION_SUPPORT, registration])];
}

function supportingCli(cli, map, product, observeCli) {
  let sha;
  for (const floor of floors(map, product)) {
    const observation = observeCli(cli, floor);
    assert(SHA.test(observation?.sha ?? ''), 'Missing supporting CLI tag commit.');
    assert(
      ['ahead', 'behind', 'identical', 'diverged'].includes(observation.status),
      'Unknown CLI ancestry.'
    );
    if (sha) assert.equal(observation.sha, sha, 'CLI tag moved during resolution.');
    if (!['ahead', 'identical'].includes(observation.status)) return null;
    sha = observation.sha;
  }
  return sha;
}

/** Select once at allocation; a later, lower-version publication must not change this cut. */
export function resolveDeclaration({ map, product, cut, releases, observeCli }) {
  if (!declaresMinimumCli(product)) return null;
  assert(SHA.test(cut ?? ''), 'Declaration needs an exact cut.');
  for (const cli of publishedReleases(releases, 'cli')
    .reverse()
    .filter((release) => release.draft === false)) {
    const version = versionOfTag(cli.tag_name, 'cli');
    if (!minimumVersion(version)) continue;
    const sha = supportingCli(cli, map, product, observeCli);
    if (sha) {
      const record = { schema: 1, cut, product, cli: { tag: cli.tag_name, version, sha } };
      return verifyDeclaration({ record, map, product, cut, releases, observeCli });
    }
  }
  throw new Error(
    `minimum-cli-unpublished: ${product} needs a published CLI containing product registration and ${DECLARATION_SUPPORT}.`
  );
}

export function declarationMarker(record) {
  return record ? `\n<!-- ${MARKER}: ${JSON.stringify(record)} -->\n` : '';
}

export function readDeclarationMarker(body = '') {
  const lines = body.split('\n').filter((line) => line.includes(`<!-- ${MARKER}:`));
  assert(lines.length <= 1, 'Duplicate frozen CLI declaration metadata.');
  if (!lines.length) return null;
  const match = new RegExp(`^<!-- ${MARKER}: (.+) -->$`).exec(lines[0]);
  assert(match, 'Malformed frozen CLI declaration metadata.');
  return JSON.parse(match[1]);
}

/** Recheck the frozen publication, not whichever CLI would be chosen from today's catalogue. */
export function verifyDeclaration({ record, map, product, cut, releases, observeCli }) {
  if (!declaresMinimumCli(product)) {
    assert.equal(record, null, 'Only extensions carry a CLI declaration.');
    return null;
  }
  declarationBytes({ record, map, product, cut });
  const cli = releases.find(
    (release) => release.draft === false && release.tag_name === record.cli.tag
  );
  assert(cli, `minimum-cli-unpublished: ${record.cli.tag} is not published.`);
  assert.equal(
    supportingCli(cli, map, product, observeCli),
    record.cli.sha,
    'Frozen CLI no longer contains the declaration floor.'
  );
  const uses = componentOfProduct(map, product).uses ?? [];
  for (const use of uses) {
    assert(
      releases.some(
        (release) =>
          release.draft === false &&
          release.tag_name === `tmt-${use.extension}-v${use.requires.slice(2)}`
      ),
      `uses-unpublished: ${use.extension} ${use.requires} is not published.`
    );
  }
  return record;
}

/** The source's authored uses are preserved; the CLI minimum is always generated. */
export function declarationBytes({ record, map, product, cut }) {
  assert.deepEqual(Object.keys(record ?? {}).sort(), ['cli', 'cut', 'product', 'schema']);
  assert.equal(record.schema, 1);
  assert.equal(record.cut, cut, 'Declaration cut changed.');
  assert.equal(record.product, product, 'Declaration product changed.');
  assert.deepEqual(Object.keys(record.cli ?? {}).sort(), ['sha', 'tag', 'version']);
  assert(SHA.test(record.cli.sha ?? ''), 'Frozen CLI needs an exact tag commit.');
  assert.equal(versionOfTag(record.cli.tag, 'cli'), record.cli.version);
  assert(
    minimumVersion(record.cli.version),
    'Frozen CLI minimum must be a published release version.'
  );
  assert(Buffer.byteLength(`>=${record.cli.version}`) <= 64, 'CLI minimum exceeds 64 bytes.');
  const uses = componentOfProduct(map, product).uses ?? [];
  const bytes = Buffer.from(
    `${JSON.stringify({ version: 1, requiresCli: `>=${record.cli.version}`, uses }, null, 2)}\n`
  );
  assert(bytes.length <= 4096, 'Generated extension declaration exceeds 4 KiB.');
  return bytes;
}

export function writeDeclaration(root, snapshot, map) {
  if (!snapshot.declaration) return;
  assert(
    !Object.keys(snapshot.hashes).some((file) => file.split('/').at(-1) === DECLARATION_FILE),
    'TMT-USES.json must be generated, never source-copied.'
  );
  const file = join(root, DECLARATION_OUTPUT);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, declarationBytes({ record: snapshot.declaration, map, ...snapshot }));
}

export function verifyDeclarationBytes(file, snapshot, map) {
  if (!snapshot.declaration) return;
  assert.deepEqual(
    readBoundedFile(file, 4096),
    declarationBytes({ record: snapshot.declaration, map, ...snapshot }),
    'Extension declaration differs from generated bytes.'
  );
}

// A bounded REST read and exact-ref source data only; no captured code is executed.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [product, cutArg, tag = '', expectedProduct] = process.argv.slice(2);
    if (product === 'verify' || product === 'generate') {
      const root = resolve(cutArg);
      const snapshot = JSON.parse(readFileSync(tag, 'utf8'));
      assert.equal(
        snapshot.product,
        expectedProduct,
        'Packaging product differs from captured cut.'
      );
      if (declaresMinimumCli(expectedProduct))
        assert(snapshot.declaration, 'Extension packaging needs frozen CLI metadata.');
      const map = parseComponentMap(readFileSync(join(root, '.github/components.json'), 'utf8'));
      if (product === 'generate') writeDeclaration(root, snapshot, map);
      verifyDeclarationBytes(join(root, DECLARATION_OUTPUT), snapshot, map);
    } else {
      assert(
        ['all', 'cli', 'driver-herdr', ...EXTENSIONS].includes(product),
        'Unknown declaration product.'
      );
      const cut = cutArg || process.env.GITHUB_SHA;
      const products = product === 'all' ? [...EXTENSIONS] : [product];
      const records = {};
      const extensions = products.filter(declaresMinimumCli);
      if (extensions.length) {
        const map = declarationSource(cut);
        const releases = readReleaseCatalog({
          repository: process.env.GITHUB_REPOSITORY,
          token: process.env.GH_TOKEN,
        });
        const observeCli = declarationAncestry({ repository: process.env.GITHUB_REPOSITORY });
        const localAncestry = gitDeclarationAncestry((args) =>
          runPackedCommand('git', args, { cwd: process.cwd(), env: process.env }).trimEnd()
        );
        for (const extension of extensions) {
          const context = { map, product: extension, cut, releases };
          const record = process.env.TMT_RELEASE_DECLARATION
            ? JSON.parse(process.env.TMT_RELEASE_DECLARATION)
            : tag
              ? readDeclarationMarker(releases.find((release) => release.tag_name === tag)?.body)
              : resolveDeclaration({ ...context, observeCli: localAncestry });
          records[extension] = verifyDeclaration({ ...context, record, observeCli });
        }
      }
      const value = JSON.stringify(product === 'all' ? records : (records[product] ?? null));
      if (process.env.GITHUB_OUTPUT)
        appendFileSync(
          process.env.GITHUB_OUTPUT,
          `${product === 'all' ? 'declarations' : 'declaration'}=${value}\n`
        );
      console.log(value);
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
