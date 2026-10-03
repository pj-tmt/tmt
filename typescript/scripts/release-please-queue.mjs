import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { runPackedCommand } from './packed-command.mjs';

import { parseComponentMap } from './ci-scope.mjs';
import {
  checkReleaseNotes,
  createSafetyReader,
  ReleaseNotesRefreshRequiredError,
} from './release-pr-safety.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const PREFIX = 'release-please--branches--main--';
const MAX_PAGES = 20;
const QUERY = `query($owner: String!, $repo: String!, $cursor: String) {
  repository(owner: $owner, name: $repo) {
    pullRequests(first: 100, after: $cursor, states: OPEN,
      orderBy: {field: CREATED_AT, direction: ASC}) {
      nodes {
        number headRefName headRefOid baseRefName isDraft
        headRepository { nameWithOwner }
        mergeQueueEntry { id }
        autoMergeRequest { enabledAt }
      }
      pageInfo { hasNextPage endCursor }
    }
  }
}`;
export const QUEUED_NOTICE =
  'Queued release PR notes cover main HEAD; skipping release-pr and preserving the checked head.';

function workflowOptions({ repository, token, env = process.env }) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? ''))
    throw new Error('GITHUB_REPOSITORY must be owner/repo.');
  if (typeof token !== 'string' || token.length === 0)
    throw new Error('RELEASE_TOKEN is required for the queue query.');
  return { repository, env: { ...env, GH_TOKEN: token }, timeoutMs: 30_000, cwd: process.cwd() };
}

/** Discovery must complete before either generation or auto-merge enabling. */
function* releasePullRequests(options, execute) {
  const [owner, repo] = options.repository.split('/');
  const cursors = new Set();
  const numbers = new Set();
  let cursor;
  for (let page = 0; page < MAX_PAGES; page += 1) {
    const args = [
      'api',
      'graphql',
      '-f',
      `query=${QUERY}`,
      '-f',
      `owner=${owner}`,
      '-f',
      `repo=${repo}`,
    ];
    if (cursor !== undefined) args.push('-f', `cursor=${cursor}`);
    const response = JSON.parse(execute('gh', args, options));
    if (
      response.errors !== undefined &&
      (!Array.isArray(response.errors) || response.errors.length)
    )
      throw new Error('Release queue query returned GraphQL errors.');
    const pulls = response.data?.repository?.pullRequests;
    if (
      !Array.isArray(pulls?.nodes) ||
      pulls.nodes.length > 100 ||
      typeof pulls.pageInfo?.hasNextPage !== 'boolean'
    )
      throw new Error('Release queue query returned an invalid PR connection.');
    for (const pr of pulls.nodes) {
      if (
        !pr ||
        !Number.isSafeInteger(pr.number) ||
        pr.number < 1 ||
        numbers.has(pr.number) ||
        typeof pr.headRefName !== 'string' ||
        !/^[a-f0-9]{40}$/.test(pr.headRefOid ?? '') ||
        typeof pr.baseRefName !== 'string' ||
        typeof pr.isDraft !== 'boolean' ||
        !(pr.headRepository === null || typeof pr.headRepository?.nameWithOwner === 'string') ||
        !(
          pr.mergeQueueEntry === null ||
          (typeof pr.mergeQueueEntry?.id === 'string' && pr.mergeQueueEntry.id.length > 0)
        ) ||
        !(
          pr.autoMergeRequest === null ||
          (typeof pr.autoMergeRequest?.enabledAt === 'string' &&
            pr.autoMergeRequest.enabledAt.length > 0)
        )
      )
        throw new Error('Release queue query returned an invalid PR.');
      numbers.add(pr.number);
    }
    for (const pr of pulls.nodes) {
      if (
        pr.headRefName.startsWith(PREFIX) &&
        pr.baseRefName === 'main' &&
        pr.headRepository?.nameWithOwner === options.repository
      )
        yield pr;
    }
    if (!pulls.pageInfo.hasNextPage) return;
    cursor = pulls.pageInfo.endCursor;
    if (
      typeof cursor !== 'string' ||
      cursor.length === 0 ||
      cursors.has(cursor) ||
      pulls.nodes.length === 0
    )
      throw new Error('Release queue query returned an invalid pagination cursor.');
    cursors.add(cursor);
  }
  throw new Error(`Release queue query exceeds ${MAX_PAGES} pages; refusing incomplete discovery.`);
}

/** Skip only when every queued release candidate passes the shared notes gate at main HEAD. */
export async function queuedReleaseNotesCover(
  options,
  execute = runPackedCommand,
  checkNotes = checkReleaseNotes
) {
  const pulls = [...releasePullRequests(workflowOptions(options), execute)];
  const queued = pulls.filter((pr) => pr.mergeQueueEntry !== null);
  if (!queued.length) return false;
  const reader = createSafetyReader(options, execute);
  const base = reader.git(['rev-parse', '--verify', 'origin/main']);
  const components = parseComponentMap(
    readFileSync(`${ROOT}.github/components.json`, 'utf8')
  ).components;
  let covered = true;
  for (const queuedPr of queued) {
    const pr = reader.get(`pulls/${queuedPr.number}`);
    if (
      pr?.number !== queuedPr.number ||
      pr.head?.sha !== queuedPr.headRefOid ||
      pr.head?.ref !== queuedPr.headRefName ||
      pr.head?.repo?.full_name !== options.repository ||
      pr.base?.ref !== 'main' ||
      pr.base?.repo?.full_name !== options.repository ||
      pr.state !== 'open'
    )
      throw new Error('Queued release PR changed during discovery; retry the run.');
    try {
      await checkNotes({ pr, base, components, reader });
    } catch (error) {
      if (!(error instanceof ReleaseNotesRefreshRequiredError)) throw error;
      covered = false;
    }
  }
  return covered;
}

/** Workflow concurrency serializes this owner. External enqueues are not coordinated by it. */
export function enableReleaseAutoMerge(options, execute = runPackedCommand) {
  const commandOptions = workflowOptions(options);
  const pulls = [...releasePullRequests(commandOptions, execute)];
  const active = pulls.filter((pr) => pr.mergeQueueEntry !== null || pr.autoMergeRequest !== null);
  if (active.length > 1)
    throw new Error(
      'Multiple release PRs are already enabled or queued; reconcile them before retrying.'
    );
  if (active.length === 1)
    return `Release PR #${active[0].number} is already active; no second PR enabled.`;
  const selected = pulls.filter((pr) => !pr.isDraft).sort((a, b) => a.number - b.number)[0];
  if (!selected) return 'No open release PR needs auto-merge.';
  execute(
    'gh',
    [
      'pr',
      'merge',
      String(selected.number),
      '--repo',
      options.repository,
      '--auto',
      '--match-head-commit',
      selected.headRefOid,
    ],
    commandOptions
  );
  return `Enabled auto-merge for release PR #${selected.number}; other release PRs wait until it merges.`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const options = { repository: process.env.GITHUB_REPOSITORY, token: process.env.RELEASE_TOKEN };
    if (process.argv[2] === 'enable') {
      if (process.env.LIVE !== 'true')
        throw new Error('Enabling release auto-merge requires LIVE=true.');
      const notice = enableReleaseAutoMerge(options);
      if (process.env.GITHUB_STEP_SUMMARY)
        appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${notice}\n`);
      process.stdout.write(`${notice}\n`);
    } else {
      if (process.argv[2] !== undefined)
        throw new Error('Usage: release-please-queue.mjs [enable]');
      const queued = await queuedReleaseNotesCover(options);
      if (queued) {
        if (!process.env.GITHUB_STEP_SUMMARY) throw new Error('GITHUB_STEP_SUMMARY is required.');
        appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${QUEUED_NOTICE}\n`);
      }
      process.stdout.write(queued ? 'skip\n' : 'run\n');
    }
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
