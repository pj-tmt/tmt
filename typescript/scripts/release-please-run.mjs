// #912: release-please 17.11.2 has no additional-paths; attribute private leaves before splitting.
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { parseComponentMap } from './ci-scope.mjs';
import { releaseConsumption } from './release-please-config.mjs';

const requirePinned = createRequire(
  new URL('../../.github/release-please/package.json', import.meta.url)
);

export function loadPinnedReleasePlease() {
  return requirePinned('release-please');
}

export function assertReleasePleaseApi(api) {
  if (
    api.VERSION !== '17.11.2' ||
    typeof api.GitHub?.create !== 'function' ||
    typeof api.Manifest?.fromManifest !== 'function' ||
    typeof api.Errors?.FileNotFoundError !== 'function' ||
    [
      'updatePullRequest',
      'getPullRequest',
      'getGitHubApi',
      'buildChangeSet',
      'getFileContentsOnBranch',
    ].some((name) => typeof api.GitHub.prototype[name] !== 'function') ||
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

/** Compare generated release files at an immutable head; main-only pushes must not reset its CI. */
export function preserveUnchangedReleasePullRequests(github, fileNotFoundError) {
  const original = github.updatePullRequest;
  github.updatePullRequest = async function (number, candidate, targetBranch, options) {
    const { data: existing } = await this.getGitHubApi().octokit.pulls.get({
      ...this.repository,
      pull_number: number,
    });
    if (existing.state !== 'open')
      return original.call(this, number, candidate, targetBranch, options);
    if (
      existing.head.ref !== candidate.headRefName ||
      existing.base.ref !== targetBranch ||
      existing.head.repo?.full_name !== `${this.repository.owner}/${this.repository.repo}` ||
      !/^[a-f0-9]{40}$/.test(existing.head.sha ?? '')
    ) {
      throw new Error(`Release PR #${number} no longer matches its candidate branch.`);
    }
    const body = candidate.body.toString();
    // Overflow handling can write comments; unchanged comparison only admits complete inline notes.
    if (
      existing.title !== candidate.title.toString() ||
      existing.body !== body ||
      body.length > 65536 ||
      existing.mergeable === false
    )
      return original.call(this, number, candidate, targetBranch, options);
    const changes = await this.buildChangeSet(candidate.updates, targetBranch);
    if (changes.size === 0)
      throw new Error(`Release PR #${number} has no generated release files.`);
    for (const [path, change] of changes) {
      let file;
      try {
        file = await this.getFileContentsOnBranch(path, existing.head.sha);
      } catch (error) {
        if (!(error instanceof fileNotFoundError)) throw error;
        return original.call(this, number, candidate, targetBranch, options);
      }
      if (
        Buffer.from(file.content, 'base64').toString('utf8') !== change.content ||
        file.mode !== change.mode
      )
        return original.call(this, number, candidate, targetBranch, options);
    }
    // GitHub computes mergeability lazily. Preserve an unchanged head while it is unknown,
    // so release-pr completes and the workflow still reconciles merged PRs via github-release.
    // Confirmed conflicts take the original updater above; the queue checks the merged result.
    // Keep the pinned API's return shape, without pushing the branch or editing PR metadata.
    return this.getPullRequest(number);
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
  if (command === 'check-install') {
    for (const entry of ['index.js', 'index.d.ts']) {
      const file = new URL(
        `../../.github/release-please/node_modules/release-please/build/src/${entry}`,
        import.meta.url
      );
      if (!existsSync(file))
        throw new Error(
          'Pinned release-please tooling is missing. From the repository root, run: ' +
            'pnpm --dir .github/release-please install --frozen-lockfile --ignore-scripts'
        );
    }
    return;
  }
  if (!['release-pr', 'github-release'].includes(command)) {
    throw new Error('Usage: release-please-run.mjs release-pr | github-release | check-install');
  }
  if (!['true', 'false'].includes(process.env.LIVE)) throw new Error('LIVE must be true or false.');
  const [owner, repo, extra] = (process.env.GITHUB_REPOSITORY ?? '').split('/');
  if (!owner || !repo || extra) throw new Error('GITHUB_REPOSITORY must be owner/repo.');
  const api = loadPinnedReleasePlease();
  assertReleasePleaseApi(api);
  const github = await api.GitHub.create({ owner, repo, token: process.env.RELEASE_TOKEN });
  // Like the manifest/config, read the map from the target branch rather than another checkout.
  const mapFile = await github.getFileContentsOnBranch('.github/components.json', 'main');
  attributeReleaseConsumption(github, parseComponentMap(mapFile.parsedContent).components);
  if (command === 'release-pr' && process.env.LIVE === 'true')
    preserveUnchangedReleasePullRequests(github, api.Errors.FileNotFoundError);
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
