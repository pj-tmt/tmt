import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { describe, expect, it } from 'vite-plus/test';
import {
  QUEUED_NOTICE,
  prepareReleaseRefresh,
  queuedReleaseNotesCover,
  enableReleaseAutoMerge,
} from '../../scripts/release-please-queue.mjs';

import { ReleaseNotesRefreshRequiredError } from '../../scripts/release-pr-safety.mjs';

const workflow = readFileSync(
  new URL('../../../.github/workflows/release.yml', import.meta.url),
  'utf8'
);
function workflowShell(name: string): string {
  const step = workflow.split(`      - name: ${name}\n`)[1].split('\n      - ')[0];
  return step
    .split('        run: |\n')[1]
    .split('\n')
    .filter((line) => line.startsWith('          '))
    .map((line) => line.slice(10))
    .join('\n');
}
const releaseShell = workflowShell('Create releases for merged release pull requests');
const shell = workflowShell('Run release-please');
const draftCommand = /run: (node [^\n]+ draft)/.exec(
  workflow.split('      - name: Check for a tagless manifest draft\n')[1]
)?.[1];
const connection = (nodes: unknown[], hasNextPage = false, endCursor: string | null = null) => ({
  data: {
    repository: {
      pullRequests: { nodes, pageInfo: { hasNextPage, endCursor } },
    },
  },
});
let nextNumber = 1;
const release = (
  queued: boolean,
  headRefName = 'release-please--branches--main--components--tmt-cli'
) => ({
  id: `PR_${nextNumber}`,
  number: nextNumber++,
  headRefOid: 'a'.repeat(40),
  baseRefName: 'main',
  isDraft: false,
  headRepository: { nameWithOwner: 'pj-tmt/tmt' },
  autoMergeRequest: null,
  headRefName,
  mergeQueueEntry: queued ? { id: 'queue-entry' } : null,
});
const coveredNotes = async () => ({ tag: 'fixture', linkedCommits: 0 });
function queryExecute(response: unknown) {
  return (command: string, args: string[]) => {
    if (command === 'git') return 'a'.repeat(40);
    if (args[1] === 'graphql') return JSON.stringify(response);
    const candidate = (
      response as ReturnType<typeof connection>
    ).data.repository.pullRequests.nodes.find(
      (node) => (node as ReturnType<typeof release>).number === Number(args[1].split('/').at(-1))
    ) as ReturnType<typeof release>;
    return JSON.stringify({
      node_id: candidate.id,
      number: candidate.number,
      state: 'open',
      head: {
        sha: candidate.headRefOid,
        ref: candidate.headRefName,
        repo: { full_name: 'pj-tmt/tmt' },
      },
      base: { ref: 'main', repo: { full_name: 'pj-tmt/tmt' } },
    });
  };
}
function decision(response: unknown) {
  return queuedReleaseNotesCover(
    { repository: 'pj-tmt/tmt', token: 'app-token' },
    queryExecute(response),
    coveredNotes
  );
}
function execute(
  response: unknown,
  {
    queryFails = false,
    queueQueryFails = false,
    failCommand = 'none',
    live = 'true',
    teeFails = false,
    unknownMergeability = false,
    staleNotes = false,
    dequeueFails = false,
    dequeueUnlocks = true,
    heldPaths = [] as string[],
    sameRunDrafts = [] as string[],
  } = {}
) {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-release-queue-'));
  try {
    const manifest = JSON.parse(
      readFileSync(new URL('../../../.release-please-manifest.json', import.meta.url), 'utf8')
    ) as Record<string, string>;
    const drafts = (paths: string[]) =>
      paths.map((path, index) => ({
        id: 100 + index,
        tag_name: `${path === '.' ? 'v' : 'tmt-squad-v'}${manifest[path]}`,
        draft: true,
        created_at: '2026-10-03T06:33:00Z',
      }));
    writeFileSync(path.join(directory, 'new-drafts.json'), JSON.stringify(drafts(sameRunDrafts)));
    writeFileSync(path.join(directory, 'query.json'), JSON.stringify(response));
    const candidate = (
      response as ReturnType<typeof connection>
    ).data.repository.pullRequests.nodes.find(
      (node) =>
        (node as ReturnType<typeof release>).mergeQueueEntry !== null &&
        (node as ReturnType<typeof release>).headRefName.startsWith(
          'release-please--branches--main--'
        )
    ) as ReturnType<typeof release> | undefined;
    const tag = candidate?.headRefName.endsWith('tmt-squad')
      ? 'tmt-squad-v0.1.0-alpha.8'
      : 'v5.0.0-alpha.34';
    writeFileSync(path.join(directory, 'queued'), String(candidate !== undefined));
    writeFileSync(
      path.join(directory, 'pr.json'),
      JSON.stringify({
        node_id: candidate?.id,
        number: candidate?.number,
        state: 'open',
        head: {
          sha: candidate?.headRefOid,
          ref: candidate?.headRefName,
          repo: { full_name: 'pj-tmt/tmt' },
        },
        base: { ref: 'main', repo: { full_name: 'pj-tmt/tmt' } },
        body: `## [next](https://github.com/pj-tmt/tmt/compare/${tag}...next)`,
      })
    );
    writeFileSync(
      path.join(directory, 'recheck.json'),
      JSON.stringify({
        data: {
          node: {
            ...candidate,
            state: 'OPEN',
            repository: { nameWithOwner: 'pj-tmt/tmt' },
          },
        },
      })
    );
    writeFileSync(
      path.join(directory, 'dequeue.json'),
      JSON.stringify({
        data: {
          dequeuePullRequest: {
            mergeQueueEntry: {
              id: candidate?.mergeQueueEntry?.id,
              pullRequest: { id: candidate?.id },
            },
          },
        },
      })
    );
    writeFileSync(
      path.join(directory, 'releases.json'),
      JSON.stringify([
        { tag_name: tag, draft: false, published_at: '2026-10-02T01:00:00Z' },
        ...drafts(heldPaths),
      ])
    );
    // Anchor equals main HEAD: complete coverage is empty, with no history commands needed.
    writeExecutable(
      path.join(directory, 'git'),
      `#!/bin/sh
case "$1" in
  rev-parse)
    if [ "$STALE_NOTES" = true ] && [ "$3" = origin/main ]; then printf '%s' '${'b'.repeat(40)}';
    else printf '%s' '${'a'.repeat(40)}'; fi ;;
  merge-base) ;;
  rev-list) if [ "$STALE_NOTES" = true ]; then printf '%s' '${'b'.repeat(40)}'; fi ;;
  log) if [ "$STALE_NOTES" = true ]; then printf '%s\\000fix(core): late change\\000' '${'b'.repeat(40)}'; fi ;;
  diff-tree) printf 'rust/crates/tmt-core/src/lib.rs\\000' ;;
  show) cat '${fileURLToPath(new URL('../../../release-please-config.json', import.meta.url))}' ;;
  *) exit 25 ;;
esac
`,
      0o700
    );
    writeExecutable(
      path.join(directory, 'gh'),
      `#!/bin/sh
printf '%s\\n' "$1 $2" >> "$RUNNER_TEMP/queries"
if [ "$GH_TOKEN" != 'fixture-app' ]; then exit 22; fi
if [ "$QUERY_FAILS" = true ]; then echo 'query unavailable' >&2; exit 21; fi
case "$2" in
  graphql)
    if [ "$QUEUE_QUERY_FAILS" = true ]; then echo 'queue query unavailable' >&2; exit 21; fi
    case "$4" in
      *dequeuePullRequest*)
        printf 'dequeue\\n' >> "$RUNNER_TEMP/commands"
        if [ "$DEQUEUE_FAILS" = true ]; then echo 'dequeue denied' >&2; exit 26; fi
        if [ "$DEQUEUE_UNLOCKS" = true ]; then printf false > "$RUNNER_TEMP/queued"; fi
        cat "$RUNNER_TEMP/dequeue.json" ;;
      *'node(id:'*) cat "$RUNNER_TEMP/recheck.json" ;;
      *) cat "$RUNNER_TEMP/query.json" ;;
    esac ;;
  */pulls/*) cat "$RUNNER_TEMP/pr.json" ;;
  */releases*) cat "$RUNNER_TEMP/releases.json" ;;
  */git/matching-refs/*) printf '[]\\n' ;;
  *) exit 24 ;;
esac
`,
      0o700
    );
    writeExecutable(
      path.join(directory, 'node'),
      `#!${process.execPath}
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
(async () => {
if (process.argv[2].endsWith('/release-please-run.mjs')) {
  const command = process.argv[3];
  fs.appendFileSync(process.env.RUNNER_TEMP + '/commands', command + String.fromCharCode(10));
  if (command === process.env.FAIL_COMMAND) {
    console.error('release-please failure');
    process.exit(19);
  }
  if (command === 'github-release' && process.env.LIVE === 'true') {
    const releases = JSON.parse(fs.readFileSync(process.env.RUNNER_TEMP + '/releases.json', 'utf8'));
    const drafts = JSON.parse(fs.readFileSync(process.env.RUNNER_TEMP + '/new-drafts.json', 'utf8'));
    fs.writeFileSync(process.env.RUNNER_TEMP + '/releases.json', JSON.stringify([...releases, ...drafts]));
  }
  if (command === 'release-pr' && process.env.LIVE === 'true' && process.env.STALE_NOTES === 'true' &&
      fs.readFileSync(process.env.RUNNER_TEMP + '/queued', 'utf8') === 'true' &&
      !JSON.parse(process.env.TAGLESS_DRAFT_PATHS).includes('.')) {
    console.error('Error updating ref: queued release branch is locked');
    process.exit(19);
  }
  if (command === 'release-pr' && process.env.UNKNOWN_MERGEABILITY === 'true') {
    const { preserveUnchangedReleasePullRequests } = await import(process.env.RELEASE_WRAPPER);
    const candidate = { headRefName: 'release-please--branches--main--fixture', title: 'release', body: 'notes', updates: [] };
    const existing = { number: 17, sha: 'a'.repeat(40) };
    const github = {
      repository: { owner: 'pj-tmt', repo: 'tmt' },
      getGitHubApi: () => ({ octokit: { pulls: { get: async () => ({ data: {
        state: 'open', title: 'release', body: 'notes', mergeable: null,
        head: { ref: candidate.headRefName, sha: existing.sha, repo: { full_name: 'pj-tmt/tmt' } },
        base: { ref: 'main' },
      } }) } } }),
      buildChangeSet: async () => new Map([['release-file', { content: 'unchanged', mode: '100644' }]]),
      getFileContentsOnBranch: async () => ({ content: Buffer.from('unchanged').toString('base64'), mode: '100644' }),
      getPullRequest: async () => existing,
      updatePullRequest: async () => { throw new Error('unchanged head was rewritten'); },
    };
    preserveUnchangedReleasePullRequests(github, Error);
    const result = await github.updatePullRequest(17, candidate, 'main');
    if (result !== existing) throw new Error('existing PR was not returned');
    console.log('Preserved unchanged release head with unknown mergeability');
  }
  console.log('release-please succeeded');
} else {
  // Exercise the real pre-check; only the wrapper's release mutation is replaced.
  const result = spawnSync(process.execPath, process.argv.slice(2), { stdio: 'inherit' });
  process.exit(result.status ?? 1);
}
})().catch(error => { console.error(error); process.exitCode = 1; });
`,
      0o700
    );
    if (teeFails)
      writeExecutable(path.join(directory, 'tee'), '#!/bin/sh\ncat >/dev/null\nexit 23\n', 0o700);
    const summary = path.join(directory, 'summary');
    writeFileSync(summary, '');
    writeFileSync(path.join(directory, 'commands'), '');
    writeFileSync(path.join(directory, 'queries'), '');
    const env = {
      PATH: `${directory}:${process.env.PATH}`,
      RUNNER_TEMP: directory,
      GITHUB_STEP_SUMMARY: summary,
      GITHUB_OUTPUT: path.join(directory, 'output'),
      GITHUB_REPOSITORY: 'pj-tmt/tmt',
      LIVE: live,
      RELEASE_TOKEN: 'fixture-app',
      GH_TOKEN: 'wrong-user-token',
      FAIL_COMMAND: failCommand,
      UNKNOWN_MERGEABILITY: String(unknownMergeability),
      STALE_NOTES: String(staleNotes),
      DEQUEUE_FAILS: String(dequeueFails),
      DEQUEUE_UNLOCKS: String(dequeueUnlocks),
      TAGLESS_DRAFT_PATHS: JSON.stringify(heldPaths),
      RELEASE_WRAPPER: new URL('../../scripts/release-please-run.mjs', import.meta.url).href,
      QUERY_FAILS: String(queryFails),
      QUEUE_QUERY_FAILS: String(queueQueryFails),
    };
    const runStep = (script: string, cwd: string) =>
      spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], {
        cwd,
        env,
        encoding: 'utf8',
        timeout: 5000,
      });
    const pinnedDirectory = fileURLToPath(
      new URL('../../../.github/release-please', import.meta.url)
    );
    const results = [runStep(releaseShell, pinnedDirectory)];
    if (results.at(-1)?.status === 0) {
      if (!draftCommand) throw new Error('Missing workflow draft command');
      const refreshOutput = env.GITHUB_OUTPUT;
      env.GITHUB_OUTPUT = path.join(directory, 'draft-output');
      results.push(runStep(draftCommand, fileURLToPath(new URL('../../../', import.meta.url))));
      if (results.at(-1)?.status === 0) {
        const draftOutputs = Object.fromEntries(
          readFileSync(env.GITHUB_OUTPUT, 'utf8')
            .trim()
            .split('\n')
            .map((line) => [line.slice(0, line.indexOf('=')), line.slice(line.indexOf('=') + 1)])
        );
        Object.assign(env, {
          TAGLESS_DRAFT: draftOutputs.skip,
          TAGLESS_DRAFT_PATHS: draftOutputs.held_paths,
        });
        env.GITHUB_OUTPUT = refreshOutput;
        results.push(runStep(shell, pinnedDirectory));
      }
    }
    const result = results.at(-1)!;
    if (result.error) throw result.error;
    return {
      status: result.status,
      output: results.map((step) => step.stdout + step.stderr).join(''),
      summary: readFileSync(summary, 'utf8'),
      commands: readFileSync(path.join(directory, 'commands'), 'utf8'),
      queries: readFileSync(path.join(directory, 'queries'), 'utf8'),
      outputs: results.length === 3 ? readFileSync(path.join(directory, 'output'), 'utf8') : '',
      draftOutputs:
        results.length >= 2 && results[1].status === 0
          ? readFileSync(path.join(directory, 'draft-output'), 'utf8')
          : '',
    };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe('release PR queue pre-check', () => {
  it('uses workflow credentials for discovery and notes reads, not inherited agent credentials', async () => {
    let calls = 0;
    const response = connection([release(true)]);
    const queued = await queuedReleaseNotesCover(
      { repository: 'pj-tmt/tmt', token: 'app-token', env: { GH_TOKEN: 'user-token' } },
      (command, args, options) => {
        if (command !== 'gh' || args[1] !== 'graphql') return queryExecute(response)(command, args);
        calls += 1;
        expect(command).toBe('gh');
        expect(args.slice(0, 2)).toEqual(['api', 'graphql']);
        expect(args).toContain('owner=pj-tmt');
        expect(args).toContain('repo=tmt');
        expect(args[3]).toContain('states: OPEN');
        expect(args[3]).toContain('mergeQueueEntry { id }');
        expect(options.env.GH_TOKEN).toBe('app-token');
        return JSON.stringify(response);
      },
      coveredNotes
    );
    expect(queued).toBe(true);
    expect(calls).toBe(1);
  });

  it('skips release-pr when queued notes cover main HEAD, preserving github-release', () => {
    const result = execute(
      connection([
        release(false),
        release(true, 'release-please--branches--main--components--tmt-squad'),
      ])
    );
    expect(result.status).toBe(0);
    expect(result.summary).toContain(QUEUED_NOTICE);
    expect(result.commands).toBe('github-release\n');
    expect(result.queries).toMatch(
      /^api repos\/pj-tmt\/tmt\/releases\?per_page=100&page=1\napi graphql\napi repos\/pj-tmt\/tmt\/pulls\/\d+\napi repos\/pj-tmt\/tmt\/releases/
    );
  });

  it('refreshes stale queued notes in the same run and keeps github-release', () => {
    const result = execute(connection([release(true)]), { staleNotes: true });
    expect(result.status).toBe(0);
    expect(result.commands).toBe('github-release\ndequeue\nrelease-pr\n');
    expect(result.summary).toContain('Dequeued stale release PR');
    expect(result.outputs).not.toContain('queue_blocked=true');
    expect(result.summary).not.toContain(QUEUED_NOTICE);
  });

  it('models the original locked-branch failure if dequeue does not unlock the queued head', () => {
    const result = execute(connection([release(true)]), {
      staleNotes: true,
      dequeueUnlocks: false,
    });
    expect(result.status).toBe(19);
    expect(result.commands).toBe('github-release\ndequeue\nrelease-pr\n');
    expect(result.output).toContain('Error updating ref: queued release branch is locked');
  });

  it('skips refresh and queue enabling on dequeue failure but keeps github-release', () => {
    const result = execute(connection([release(true)]), { staleNotes: true, dequeueFails: true });
    expect(result.status).toBe(0);
    expect(result.commands).toBe('github-release\ndequeue\n');
    expect(result.summary).toContain('Could not safely dequeue stale release PR');
    expect(result.summary).toContain(
      'skipping release-pr and auto-merge enabling. github-release continues'
    );
    expect(result.outputs).toContain('queue_blocked=true');
    expect(workflow).toContain(
      "if: steps.mode.outputs.live == 'true' && steps.release.outputs.queue_blocked != 'true'"
    );
  });

  it('reports github-release failure before attempting refresh or dequeue', () => {
    const result = execute(connection([release(true)]), {
      staleNotes: true,
      dequeueFails: true,
      failCommand: 'github-release',
    });
    expect(result.status).toBe(19);
    expect(result.commands).toBe('github-release\n');
    expect(result.output).toContain('github-release failed; release-pr was not attempted.');
    expect(result.summary).toContain('github-release failed; release-pr was not attempted.');
    expect(result.queries).toBe('');
  });

  it('plans stale queued refresh without mutating in a dry run', () => {
    const result = execute(connection([release(true)]), { staleNotes: true, live: 'false' });
    expect(result.status).toBe(0);
    expect(result.commands).toBe('github-release\nrelease-pr\n');
    expect(result.summary).toContain('Dry run: stale queued release PR would be dequeued');
  });

  it('preserves a queued draft-held candidate while allowing unheld refresh and github-release', () => {
    const result = execute(connection([release(true)]), { staleNotes: true, heldPaths: ['.'] });
    expect(result.status).toBe(0);
    expect(result.commands).toBe('github-release\nrelease-pr\n');
    expect(result.summary).toContain('held by a tagless draft');
    expect(result.queries.split('api graphql')).toHaveLength(2);
  });

  it.each([
    { held: ['.'], refresh: true },
    { held: ['extensions/tmt-squad'], refresh: true },
    { held: ['.', 'extensions/tmt-squad'], refresh: false },
  ])('sees drafts created in this run before deciding held paths: $held', ({ held, refresh }) => {
    const result = execute(connection([]), { sameRunDrafts: held });
    expect(result.status).toBe(0);
    expect(result.draftOutputs).toContain(`held_paths=${JSON.stringify(held)}`);
    expect(result.commands).toBe(refresh ? 'github-release\nrelease-pr\n' : 'github-release\n');
  });

  it('never creates same-run drafts when planning a dry run', () => {
    const result = execute(connection([]), { sameRunDrafts: ['.'], live: 'false' });
    expect(result.status).toBe(0);
    expect(result.draftOutputs).toContain('held_paths=[]');
    expect(result.commands).toBe('github-release\nrelease-pr\n');
  });

  it('preserves a stale queued candidate held by a draft created in this run', () => {
    const result = execute(connection([release(true)]), {
      sameRunDrafts: ['.'],
      staleNotes: true,
    });
    expect(result.status).toBe(0);
    expect(result.draftOutputs).toContain('held_paths=["."]');
    expect(result.commands).toBe('github-release\nrelease-pr\n');
    expect(result.summary).toContain('held by a tagless draft');
    expect(result.queries.split('api graphql')).toHaveLength(2);
  });

  it('fails visibly when notes acquisition fails rather than treating the candidate as covered', async () => {
    const response = connection([release(true)]);
    const acquire = queryExecute(response);
    await expect(
      queuedReleaseNotesCover(
        { repository: 'pj-tmt/tmt', token: 'app' },
        (command, args) => {
          if (command === 'gh' && args[1] !== 'graphql') throw new Error('REST notes unavailable');
          return acquire(command, args);
        },
        coveredNotes
      )
    ).rejects.toThrow('REST notes unavailable');
  });

  it('rejects a PR head race before checking its notes', async () => {
    const response = connection([release(true)]);
    const acquire = queryExecute(response);
    await expect(
      queuedReleaseNotesCover(
        { repository: 'pj-tmt/tmt', token: 'app' },
        (command, args) => {
          const value = acquire(command, args);
          if (command === 'gh' && args[1] !== 'graphql') {
            const pr = JSON.parse(value);
            pr.head.sha = 'b'.repeat(40);
            return JSON.stringify(pr);
          }
          return value;
        },
        coveredNotes
      )
    ).rejects.toThrow('changed during discovery');
  });

  it('fetches full history and tags using the Code quality checkout pattern', () => {
    const job = workflow.split('  release-stall:')[0];
    expect(job).toMatch(/persist-credentials: false\n          fetch-depth: 0/);
  });

  it('runs both commands unchanged when release PRs are not queued', () => {
    const result = execute(connection([release(false), release(true, 'feature-branch')]));
    expect(result.status).toBe(0);
    expect(result.commands).toBe('github-release\nrelease-pr\n');
    expect(result.summary).not.toContain(QUEUED_NOTICE);
    expect(result.queries).toBe('api repos/pj-tmt/tmt/releases?per_page=100&page=1\napi graphql\n');
  });

  it('still runs github-release when an unchanged release PR has unknown mergeability', () => {
    const result = execute(connection([release(false)]), { unknownMergeability: true });
    expect(result.status).toBe(0);
    expect(result.commands).toBe('github-release\nrelease-pr\n');
    expect(result.summary).toContain('Preserved unchanged release head with unknown mergeability');
    expect(result.output).not.toContain('unchanged head was rewritten');
  });

  it('keeps the same pre-check and command planning in dry runs', () => {
    const result = execute(connection([]), { live: 'false' });
    expect(result.status).toBe(0);
    expect(result.commands).toBe('github-release\nrelease-pr\n');
    expect(result.summary).toContain('(dry run)');
  });

  it('fails visibly when draft discovery fails after github-release', () => {
    const result = execute(connection([]), { queryFails: true });
    expect(result.status).toBe(1);
    expect(result.commands).toBe('github-release\n');
    expect(result.output).toContain('query unavailable');
  });

  it('fails visibly when queue discovery fails after release reconciliation and draft inspection', () => {
    const result = execute(connection([]), { queueQueryFails: true });
    expect(result.status).toBe(1);
    expect(result.commands).toBe('github-release\n');
    expect(result.draftOutputs).toContain('held_paths=[]');
    expect(result.output).toContain('queue query unavailable');
  });

  it.each(['release-pr', 'github-release'])(
    'never suppresses a %s command failure',
    (failCommand) => {
      const result = execute(connection([]), { failCommand });
      expect(result.status).toBe(19);
      expect(result.output).toContain('release-please failure');
    }
  );

  it('fails github-release before a covered queued PR can skip refresh', () => {
    const result = execute(connection([release(true)]), { failCommand: 'github-release' });
    expect(result.status).toBe(19);
    expect(result.commands).toBe('github-release\n');
  });

  it('still fails a summary write', () => {
    expect(execute(connection([]), { teeFails: true }).status).toBe(23);
  });

  it.each([
    {},
    { errors: [{ message: 'denied' }] },
    { ...connection([release(true)]), errors: [{ message: 'partial result' }] },
    connection([{ headRefName: 'feature-branch' }]),
    connection([null]),
    connection([{ headRefName: 'release-please--branches--main--x', mergeQueueEntry: {} }]),
    connection([release(false)], true),
  ])('rejects malformed, partial or incomplete query data %#', async (response) => {
    await expect(decision(response)).rejects.toThrow(/query/);
  });

  it('requires complete discovery even when a queued release PR is found', async () => {
    await expect(decision(connection([release(true)], true, 'next'))).rejects.toThrow(/invalid PR/);
  });

  it('does not mistake a prefix look-alike or another target branch for main releases', async () => {
    expect(
      await decision(
        connection([
          release(true, 'release-please--branches--main-other'),
          release(true, 'release-please--branches--v4--x'),
        ])
      )
    ).toBe(false);
  });

  it('refuses missing workflow credentials or malformed repository before running gh', async () => {
    const unexpected = () => {
      throw new Error('must not execute');
    };
    await expect(queuedReleaseNotesCover({ repository: 'pj-tmt/tmt' }, unexpected)).rejects.toThrow(
      'RELEASE_TOKEN'
    );
    await expect(
      queuedReleaseNotesCover({ repository: 'pj-tmt/tmt/extra', token: 'app' }, unexpected)
    ).rejects.toThrow('GITHUB_REPOSITORY');
  });
});

describe('stale queued release dequeue', () => {
  const options = {
    repository: 'pj-tmt/tmt',
    token: 'app-token',
    live: true,
    env: { GH_TOKEN: 'wrong-user' },
  };
  const staleNotes = async () => {
    throw new ReleaseNotesRefreshRequiredError('stale');
  };
  function fixture({ change = {}, result = undefined as unknown, failRecheck = false } = {}) {
    const pr = release(true);
    const response = connection([pr]);
    const mutations: string[][] = [];
    const calls: string[] = [];
    return {
      pr,
      mutations,
      calls,
      execute: (
        command: string,
        args: string[],
        config: { env: NodeJS.ProcessEnv; timeoutMs: number }
      ) => {
        expect(config.env.GH_TOKEN).toBe(command === 'gh' ? 'app-token' : 'wrong-user');
        expect(config.timeoutMs).toBe(30_000);
        if (args[3]?.includes('dequeuePullRequest')) {
          calls.push('dequeue');
          mutations.push(args);
          expect(args).toContain(`id=${pr.id}`);
          return JSON.stringify(
            result ?? {
              data: {
                dequeuePullRequest: {
                  mergeQueueEntry: {
                    id: pr.mergeQueueEntry?.id,
                    pullRequest: { id: pr.id },
                  },
                },
              },
            }
          );
        }
        if (args[3]?.includes('node(id:')) {
          calls.push('recheck');
          if (failRecheck) throw new Error('recheck unavailable');
          return JSON.stringify({
            data: {
              node: {
                ...pr,
                state: 'OPEN',
                repository: { nameWithOwner: 'pj-tmt/tmt' },
                ...change,
              },
            },
          });
        }
        calls.push(command === 'git' ? 'history' : args[1] === 'graphql' ? 'discover' : 'notes');
        return queryExecute(response)(command, args);
      },
    };
  }
  it('rechecks identity and dequeues the observed PR once with the App token before allowing refresh', async () => {
    const f = fixture();
    expect(await prepareReleaseRefresh(options, f.execute, staleNotes)).toMatchObject({
      decision: 'run',
    });
    expect(f.calls.slice(-2)).toEqual(['recheck', 'dequeue']);
    expect(f.mutations).toHaveLength(1);
  });
  it.each([
    { id: 'other' },
    { number: 999999 },
    { headRefOid: 'b'.repeat(40) },
    { state: 'CLOSED' },
    { headRefName: 'feature' },
    { baseRefName: 'v4' },
    { repository: { nameWithOwner: 'other/repo' } },
    { headRepository: { nameWithOwner: 'other/repo' } },
    { mergeQueueEntry: null },
    { mergeQueueEntry: { id: 'new-entry' } },
    { isDraft: true },
  ])('blocks refresh without mutation when the PR recheck changes %#', async (change) => {
    const f = fixture({ change });
    expect(await prepareReleaseRefresh(options, f.execute, staleNotes)).toMatchObject({
      decision: 'blocked',
    });
    expect(f.mutations).toHaveLength(0);
  });
  it('blocks refresh without mutation when recheck acquisition fails', async () => {
    const f = fixture({ failRecheck: true });
    expect(await prepareReleaseRefresh(options, f.execute, staleNotes)).toMatchObject({
      decision: 'blocked',
    });
    expect(f.mutations).toHaveLength(0);
  });
  it.each([
    {},
    { errors: [{ message: 'denied' }] },
    { data: { dequeuePullRequest: { mergeQueueEntry: null } } },
    {
      data: {
        dequeuePullRequest: {
          mergeQueueEntry: { id: 'queue-entry', pullRequest: { id: 'other' } },
        },
      },
    },
    {
      data: {
        dequeuePullRequest: {
          mergeQueueEntry: { id: 'other-entry', pullRequest: { id: 'other' } },
        },
      },
    },
  ])('blocks refresh after an uncertain dequeue response without retrying %#', async (result) => {
    const f = fixture({ result });
    expect(await prepareReleaseRefresh(options, f.execute, staleNotes)).toMatchObject({
      decision: 'blocked',
    });
    expect(f.mutations).toHaveLength(1);
  });
  it.each(['errors', 'entry'])(
    'blocks a %s-only mutation defect with otherwise valid dequeue evidence',
    async (defect) => {
      const f = fixture();
      const execute = (...args: Parameters<typeof f.execute>) => {
        const value = f.execute(...args);
        if (!args[1][3]?.includes('dequeuePullRequest')) return value;
        const response = JSON.parse(value);
        if (defect === 'errors') response.errors = [{ message: 'partial response' }];
        else response.data.dequeuePullRequest.mergeQueueEntry.id = 'other-entry';
        return JSON.stringify(response);
      };
      expect(await prepareReleaseRefresh(options, execute, staleNotes)).toMatchObject({
        decision: 'blocked',
      });
      expect(f.mutations).toHaveLength(1);
    }
  );
  it('does not mutate covered or nonqueued candidates', async () => {
    const f = fixture();
    expect(await prepareReleaseRefresh(options, f.execute, coveredNotes)).toEqual({
      decision: 'skip',
      notice: QUEUED_NOTICE,
    });
    expect(f.mutations).toHaveLength(0);
    expect(f.calls).not.toContain('recheck');
    expect(
      await prepareReleaseRefresh(options, queryExecute(connection([release(false)])), staleNotes)
    ).toEqual({ decision: 'run' });
  });
  it('never dequeues a draft-held candidate before the wrapper filters it from generation', async () => {
    const f = fixture();
    expect(
      await prepareReleaseRefresh({ ...options, heldPaths: ['.'] }, f.execute, staleNotes)
    ).toMatchObject({ decision: 'run' });
    expect(f.mutations).toHaveLength(0);
    expect(f.calls).not.toContain('recheck');
  });
  it('blocks multiple queued releases before any dequeue', async () => {
    const response = connection([release(true), release(true)]);
    expect(await prepareReleaseRefresh(options, queryExecute(response), staleNotes)).toMatchObject({
      decision: 'blocked',
      notice: expect.stringContaining('Multiple queued'),
    });
  });
});

describe('paginated release discovery', () => {
  const options = { repository: 'pj-tmt/tmt', token: 'app-token' };
  it.each([true, false])('finds a queued release beyond 100 unrelated PRs: %s', async (queued) => {
    const pages = [
      connection(
        Array.from({ length: 100 }, () => release(true, 'feature')),
        true,
        'after-100'
      ),
      connection([release(queued)]),
    ];
    let calls = 0;
    expect(
      await queuedReleaseNotesCover(
        options,
        (_command, args, config) => {
          if (_command !== 'gh' || args[1] !== 'graphql')
            return queryExecute(pages[1])(_command, args);
          expect(config.env.GH_TOKEN).toBe('app-token');
          if (calls === 1) expect(args).toContain('cursor=after-100');
          return JSON.stringify(pages[calls++]);
        },
        coveredNotes
      )
    ).toBe(queued);
    expect(calls).toBe(2);
  });
  it('fails on a second-page API error rather than permitting a rewrite', async () => {
    const pages = [connection([release(false)], true, 'next'), { errors: [{ message: 'denied' }] }];
    await expect(
      queuedReleaseNotesCover(options, () => JSON.stringify(pages.shift()))
    ).rejects.toThrow('GraphQL errors');
  });
  it('rejects cursor cycles, duplicate PRs and exhausted discovery before enabling', async () => {
    const pr = release(false);
    let calls = 0;
    await expect(
      queuedReleaseNotesCover(options, () =>
        JSON.stringify(connection([release(false)], true, 'cycle'))
      )
    ).rejects.toThrow('pagination cursor');
    expect(() =>
      enableReleaseAutoMerge(options, () => JSON.stringify(connection([pr], true, 'next')))
    ).toThrow('invalid PR');
    expect(() =>
      enableReleaseAutoMerge(options, () => {
        calls += 1;
        return JSON.stringify(connection([release(false)], true, `page-${calls}`));
      })
    ).toThrow('20 pages');
    expect(calls).toBe(20);
  });
});

describe('single active release auto-merge', () => {
  const options = { repository: 'pj-tmt/tmt', token: 'app-token' };
  const enabled = (pr = release(false)) => ({
    ...pr,
    autoMergeRequest: { enabledAt: '2026-10-02T00:00:00Z' },
  });
  function run(pulls: unknown[], failMerge = false) {
    const mutations: string[][] = [];
    const notice = enableReleaseAutoMerge(options, (_command, args, config) => {
      expect(config.env.GH_TOKEN).toBe('app-token');
      if (args[0] === 'api') return JSON.stringify(connection(pulls));
      mutations.push(args);
      if (failMerge) throw new Error('head changed or enabling failed');
      return '';
    });
    return { notice, mutations };
  }
  it('enables only the oldest eligible PR, pinning its head, with normal auto-merge', () => {
    const first = release(false),
      second = release(false);
    expect(run([second, first]).mutations).toEqual([
      [
        'pr',
        'merge',
        String(first.number),
        '--repo',
        'pj-tmt/tmt',
        '--auto',
        '--match-head-commit',
        first.headRefOid,
      ],
    ]);
  });
  it.each([true, false])(
    'retains an already active PR even if a different PR is older (queued=%s)',
    (queued) => {
      const older = release(false),
        active = queued ? release(true) : enabled();
      expect(run([older, active]).mutations).toEqual([]);
    }
  );
  it('does not treat one PR that is both queued and enabled as two active PRs', () => {
    expect(run([enabled(release(true)), release(false)]).mutations).toEqual([]);
  });
  it('refuses multiple already active releases before another mutation', () => {
    expect(() => run([enabled(), release(true)])).toThrow('Multiple release PRs');
  });
  it('advances to the remaining component only after the selected PR disappears', () => {
    const first = enabled(),
      second = release(false);
    expect(run([first, second]).mutations).toEqual([]);
    expect(run([second]).mutations[0][2]).toBe(String(second.number));
  });
  it('never selects drafts, fork branches, look-alikes or another base branch', () => {
    expect(
      run([
        { ...release(false), isDraft: true },
        { ...release(false), headRepository: { nameWithOwner: 'other/tmt' } },
        { ...release(false), baseRefName: 'v4' },
        release(false, 'release-please--branches--main-other'),
      ]).mutations
    ).toEqual([]);
    expect(run([]).mutations).toEqual([]);
  });
  it('propagates a head race or enabling failure instead of trying the second component', () => {
    expect(() => run([release(false), release(false)], true)).toThrow(
      'head changed or enabling failed'
    );
  });
});
