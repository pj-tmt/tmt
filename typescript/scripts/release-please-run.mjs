// #912: release-please 17.11.2 has no additional-paths; attribute private leaves before splitting.
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { parseComponentMap } from './ci-scope.mjs';
import { releaseConsumption } from './release-please-config.mjs';

const requirePinned = createRequire(
  new URL('../../.github/release-please/package.json', import.meta.url)
);

export function assertReleasePleaseApi(api) {
  if (
    api.VERSION !== '17.11.2' ||
    typeof api.GitHub?.create !== 'function' ||
    typeof api.Manifest?.fromManifest !== 'function' ||
    api.GitHub.prototype.mergeCommitIterator?.constructor.name !== 'AsyncGeneratorFunction' ||
    api.GitHub.prototype.mergeCommitIterator.length !== 1
  ) {
    throw new Error(
      'Unsupported release-please API; re-verify private-leaf attribution before upgrading.'
    );
  }
}

/** Preserve original files and iterator ordering so excludes and per-product cutoffs still apply. */
export function attributeReleaseConsumption(github, components) {
  const mappings = releaseConsumption(components);
  const original = github.mergeCommitIterator;
  if (typeof original !== 'function' || original.length !== 1) {
    throw new Error('Unsupported release-please mergeCommitIterator.');
  }
  github.mergeCommitIterator = async function* (branch, options) {
    for await (const commit of original.call(this, branch, options)) {
      if (!Array.isArray(commit.files) || commit.files.some((file) => typeof file !== 'string')) {
        throw new Error(`Release commit ${commit.sha} is missing valid backfilled files.`);
      }
      const targets = mappings
        .filter(({ source }) =>
          commit.files.some((file) => file === source || file.startsWith(`${source}/`))
        )
        .map(({ target }) => `${target}/.release-please-consumption`);
      yield targets.length
        ? { ...commit, files: [...new Set([...commit.files, ...targets])] }
        : commit;
    }
  };
  return github;
}

/** The existing mode gate selects planning or mutation; no publication is performed here. */
export async function executeReleasePlease(manifest, command, live) {
  if (typeof live !== 'boolean') throw new Error('Release mode must be boolean.');
  if (command === 'release-pr') {
    return live ? manifest.createPullRequests() : manifest.buildPullRequests();
  }
  if (command === 'github-release') {
    return live ? manifest.createReleases() : manifest.buildReleases();
  }
  throw new Error('Unknown release-please command.');
}

async function main(command) {
  if (!['release-pr', 'github-release'].includes(command)) {
    throw new Error('Usage: release-please-run.mjs release-pr | github-release');
  }
  if (!['true', 'false'].includes(process.env.LIVE)) throw new Error('LIVE must be true or false.');
  const [owner, repo, extra] = (process.env.GITHUB_REPOSITORY ?? '').split('/');
  if (!owner || !repo || extra) throw new Error('GITHUB_REPOSITORY must be owner/repo.');
  const api = requirePinned('release-please');
  assertReleasePleaseApi(api);
  const github = await api.GitHub.create({ owner, repo, token: process.env.RELEASE_TOKEN });
  // Like the manifest/config, read the map from the target branch rather than another checkout.
  const mapFile = await github.getFileContentsOnBranch('.github/components.json', 'main');
  attributeReleaseConsumption(github, parseComponentMap(mapFile.parsedContent).components);
  const manifest = await api.Manifest.fromManifest(
    github,
    'main',
    'release-please-config.json',
    '.release-please-manifest.json'
  );
  const result = await executeReleasePlease(manifest, command, process.env.LIVE === 'true');
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main(process.argv[2]).catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
