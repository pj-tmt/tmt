import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { runPackedCommand } from './packed-command.mjs';

const PREFIX = 'release-please--branches--main--';
const QUERY = `query($owner: String!, $repo: String!) {
  repository(owner: $owner, name: $repo) {
    pullRequests(first: 100, states: OPEN) {
      nodes { headRefName mergeQueueEntry { id } }
      pageInfo { hasNextPage }
    }
  }
}`;
export const QUEUED_NOTICE =
  'A release PR is in the merge queue; skipping release-pr. It refreshes on the first main push after it merges.';

/** A single workflow-token query; incomplete discovery cannot authorize an update. */
export function releasePrQueued(
  { repository, token, env = process.env },
  execute = runPackedCommand
) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? ''))
    throw new Error('GITHUB_REPOSITORY must be owner/repo.');
  if (typeof token !== 'string' || token.length === 0)
    throw new Error('RELEASE_TOKEN is required for the queue query.');
  const [owner, repo] = repository.split('/');
  const response = JSON.parse(
    execute(
      'gh',
      ['api', 'graphql', '-f', `query=${QUERY}`, '-f', `owner=${owner}`, '-f', `repo=${repo}`],
      { cwd: process.cwd(), env: { ...env, GH_TOKEN: token }, timeoutMs: 30_000 }
    )
  );
  if (
    response.errors !== undefined &&
    (!Array.isArray(response.errors) || response.errors.length)
  ) {
    throw new Error('Release queue query returned GraphQL errors.');
  }
  const pulls = response.data?.repository?.pullRequests;
  if (!Array.isArray(pulls?.nodes) || typeof pulls.pageInfo?.hasNextPage !== 'boolean') {
    throw new Error('Release queue query returned an invalid PR connection.');
  }
  for (const pr of pulls.nodes) {
    if (
      !pr ||
      typeof pr.headRefName !== 'string' ||
      !(
        pr.mergeQueueEntry === null ||
        (typeof pr.mergeQueueEntry?.id === 'string' && pr.mergeQueueEntry.id.length > 0)
      )
    ) {
      throw new Error('Release queue query returned an invalid PR.');
    }
  }
  if (pulls.nodes.some((pr) => pr.headRefName.startsWith(PREFIX) && pr.mergeQueueEntry !== null))
    return true;
  if (pulls.pageInfo.hasNextPage)
    throw new Error('Release queue query exceeds 100 open PRs; refusing an incomplete pre-check.');
  return false;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const queued = releasePrQueued({
      repository: process.env.GITHUB_REPOSITORY,
      token: process.env.RELEASE_TOKEN,
    });
    if (queued) {
      if (!process.env.GITHUB_STEP_SUMMARY) throw new Error('GITHUB_STEP_SUMMARY is required.');
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${QUEUED_NOTICE}\n`);
    }
    process.stdout.write(queued ? 'skip\n' : 'run\n');
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
