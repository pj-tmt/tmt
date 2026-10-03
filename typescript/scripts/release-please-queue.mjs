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
        id number headRefName headRefOid baseRefName isDraft
        headRepository { nameWithOwner }
        mergeQueueEntry { id }
        autoMergeRequest { enabledAt }
      }
      pageInfo { hasNextPage endCursor }
    }
  }
}`;
const RECHECK = `query($id: ID!) {
  node(id: $id) {
    ... on PullRequest {
      id number state headRefName headRefOid baseRefName isDraft
      repository { nameWithOwner }
      headRepository { nameWithOwner }
      mergeQueueEntry { id }
    }
  }
}`;
const DEQUEUE = `mutation($id: ID!) {
  dequeuePullRequest(input: {id: $id}) {
    mergeQueueEntry { id pullRequest { id } }
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
        typeof pr.id !== 'string' ||
        pr.id.length === 0 ||
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

/** Inspect every queued candidate through the shared notes gate before planning a mutation. */
async function staleQueuedReleasePullRequests(
  options,
  execute = runPackedCommand,
  checkNotes = checkReleaseNotes
) {
  const pulls = [...releasePullRequests(workflowOptions(options), execute)];
  const queued = pulls.filter((pr) => pr.mergeQueueEntry !== null);
  if (!queued.length) return { queued, stale: [] };
  const reader = createSafetyReader(options, execute);
  const base = reader.git(['rev-parse', '--verify', 'origin/main']);
  const components = parseComponentMap(
    readFileSync(`${ROOT}.github/components.json`, 'utf8')
  ).components;
  const stale = [];
  for (const queuedPr of queued) {
    const pr = reader.get(`pulls/${queuedPr.number}`);
    if (
      pr?.node_id !== queuedPr.id ||
      pr.number !== queuedPr.number ||
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
      stale.push(queuedPr);
    }
  }
  return { queued, stale };
}

/** Read-only coverage decision shared with the refresh preparation owner. */
export async function queuedReleaseNotesCover(
  options,
  execute = runPackedCommand,
  checkNotes = checkReleaseNotes
) {
  const { queued, stale } = await staleQueuedReleasePullRequests(options, execute, checkNotes);
  return queued.length > 0 && stale.length === 0;
}

/** Dequeue only proven stale candidates; failed mutation preparation must preserve github-release. */
export async function prepareReleaseRefresh(
  options,
  execute = runPackedCommand,
  checkNotes = checkReleaseNotes
) {
  const { queued, stale } = await staleQueuedReleasePullRequests(options, execute, checkNotes);
  if (!queued.length) return { decision: 'run' };
  if (!stale.length) return { decision: 'skip', notice: QUEUED_NOTICE };
  const heldPaths = options.heldPaths ?? [];
  if (!Array.isArray(heldPaths) || heldPaths.some((path) => typeof path !== 'string'))
    throw new Error('Invalid tagless draft manifest paths.');
  const components = parseComponentMap(
    readFileSync(`${ROOT}.github/components.json`, 'utf8')
  ).components;
  if (
    stale.every((pr) =>
      heldPaths.includes(
        components.find(
          (component) => pr.headRefName === `${PREFIX}components--${component.package}`
        )?.owns[0]
      )
    )
  )
    return {
      decision: 'run',
      notice:
        'Stale queued release PR is held by a tagless draft; preserving its queue entry while unheld components regenerate.',
    };
  if (options.live !== true)
    return {
      decision: 'run',
      notice: 'Dry run: stale queued release PR would be dequeued before refresh.',
    };
  if (queued.length !== 1)
    return {
      decision: 'blocked',
      notice:
        'Multiple queued release PRs; skipping release-pr and auto-merge enabling. github-release continues. Reconcile the queue before retrying.',
    };
  const selected = stale[0];
  const commandOptions = workflowOptions(options);
  try {
    const current = JSON.parse(
      execute(
        'gh',
        ['api', 'graphql', '-f', `query=${RECHECK}`, '-f', `id=${selected.id}`],
        commandOptions
      )
    );
    const pr = current.data?.node;
    if (
      (current.errors !== undefined && (!Array.isArray(current.errors) || current.errors.length)) ||
      pr?.id !== selected.id ||
      pr.number !== selected.number ||
      pr.state !== 'OPEN' ||
      pr.headRefName !== selected.headRefName ||
      pr.headRefOid !== selected.headRefOid ||
      pr.baseRefName !== 'main' ||
      pr.repository?.nameWithOwner !== options.repository ||
      pr.headRepository?.nameWithOwner !== options.repository ||
      pr.isDraft !== selected.isDraft ||
      pr.mergeQueueEntry?.id !== selected.mergeQueueEntry.id
    )
      throw new Error('Queued release PR identity, head or queue entry changed before dequeue.');
    const result = JSON.parse(
      execute(
        'gh',
        ['api', 'graphql', '-f', `query=${DEQUEUE}`, '-f', `id=${selected.id}`],
        commandOptions
      )
    );
    const entry = result.data?.dequeuePullRequest?.mergeQueueEntry;
    if (
      (result.errors !== undefined && (!Array.isArray(result.errors) || result.errors.length)) ||
      entry?.id !== selected.mergeQueueEntry.id ||
      entry.pullRequest?.id !== selected.id
    )
      throw new Error('Dequeue did not return the expected release PR queue entry.');
  } catch (error) {
    const reason =
      error instanceof Error
        ? error.message.split('\n')[0].slice(0, 300)
        : 'Unknown dequeue failure.';
    return {
      decision: 'blocked',
      notice: `Could not safely dequeue stale release PR #${selected.number}; skipping release-pr and auto-merge enabling. github-release continues. Verify the PR and queue state, then retry the Release run. Reason: ${reason}`,
    };
  }
  return {
    decision: 'run',
    notice: `Dequeued stale release PR #${selected.number}; release-pr will refresh it before auto-merge is enabled again.`,
  };
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
      const { decision, notice } = await prepareReleaseRefresh({
        ...options,
        live: process.env.LIVE === 'true',
        heldPaths: JSON.parse(process.env.TAGLESS_DRAFT_PATHS ?? '[]'),
      });
      if (notice) {
        if (!process.env.GITHUB_STEP_SUMMARY) throw new Error('GITHUB_STEP_SUMMARY is required.');
        appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${notice}\n`);
      }
      process.stdout.write(`${decision}\n`);
    }
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
