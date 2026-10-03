import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { execFileSync, spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

const script = fileURLToPath(new URL('../../../scripts/dev-worktree-remove.sh', import.meta.url));

interface Scratch {
  root: string;
  repo: string;
  worktree: string;
  env: NodeJS.ProcessEnv;
  git: (cwd: string, ...args: string[]) => string;
  remove: (path?: string) => { status: number | null; stdout: string; stderr: string };
}

/** A bare origin, a clone, and a linked worktree whose branch is pushed with an upstream. */
async function withScratch(run: (scratch: Scratch) => void | Promise<void>) {
  const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'dev-worktree-remove-')));
  try {
    const bin = path.join(root, 'bin');
    mkdirSync(bin);
    // Stub the REST response projection, retaining closed-unmerged versus merged.
    writeExecutable(
      path.join(bin, 'gh'),
      `#!${process.execPath}
const fs = require('node:fs');
fs.writeFileSync(process.env.FAKE_GH_LOG, JSON.stringify({ cwd: process.cwd(), args: process.argv.slice(2) }));
if (process.env.FAKE_GH === 'fail') process.exit(1);
const response = { state: process.env.FAKE_PR_STATE || 'open', merged_at: process.env.FAKE_PR_MERGED_AT || null };
process.stdout.write((response.merged_at ? 'MERGED' : response.state.toUpperCase()) + '\\n');
`,
      0o755
    );
    const env: NodeJS.ProcessEnv = {
      PATH: `${bin}${path.delimiter}${process.env.PATH ?? ''}`,
      HOME: root,
      FAKE_GH_LOG: path.join(root, 'gh-call.json'),
      GIT_AUTHOR_NAME: 't',
      GIT_AUTHOR_EMAIL: 't@example.com',
      GIT_COMMITTER_NAME: 't',
      GIT_COMMITTER_EMAIL: 't@example.com',
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_CONFIG_SYSTEM: '/dev/null',
    };
    const git = (cwd: string, ...args: string[]) =>
      execFileSync('git', args, { cwd, env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    const origin = path.join(root, 'origin.git');
    const repo = path.join(root, 'repo');
    const worktree = path.join(root, 'wt');
    git(root, 'init', '--bare', '-q', '-b', 'main', origin);
    git(root, 'clone', '-q', origin, repo);
    writeFileSync(path.join(repo, 'a'), 'one\n');
    git(repo, 'add', 'a');
    git(repo, 'commit', '-q', '-m', 'one');
    git(repo, 'push', '-q', 'origin', 'HEAD:main');
    git(repo, 'worktree', 'add', '-q', '-b', 'feat', worktree, 'HEAD');
    writeFileSync(path.join(worktree, 'b'), 'two\n');
    git(worktree, 'add', 'b');
    git(worktree, 'commit', '-q', '-m', 'two');
    git(worktree, 'push', '-q', '-u', 'origin', 'feat');
    const remove = (target = worktree) => {
      const result = spawnSync('/bin/sh', [script, target, '7'], { env, encoding: 'utf8' });
      return { status: result.status, stdout: result.stdout, stderr: result.stderr };
    };
    await run({ root, repo, worktree, env, git, remove });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

/** The maintainer rebases the PR branch on the server: local commits look unpushed. */
function rewriteRemoteBranch(scratch: Scratch) {
  const other = path.join(scratch.root, 'other');
  scratch.git(scratch.root, 'clone', '-q', path.join(scratch.root, 'origin.git'), other);
  scratch.git(other, 'checkout', '-q', 'feat');
  scratch.git(other, 'commit', '-q', '--amend', '-m', 'two (rebased)');
  scratch.git(other, 'push', '-q', '--force', 'origin', 'feat');
  scratch.git(scratch.worktree, 'fetch', '-q');
}

describe('scripts/dev-worktree-remove.sh', () => {
  it.each(['open', 'closed', 'unknown'])(
    'removes a clean pushed worktree when the PR lookup is %s',
    async (state) => {
      await withScratch((scratch) => {
        scratch.env.FAKE_PR_STATE = state;
        if (state === 'unknown') scratch.env.FAKE_GH = 'fail';
        const listed = () =>
          scratch.git(scratch.repo, 'worktree', 'list', '--porcelain').split('\n');
        const entry = `worktree ${scratch.worktree}`;
        expect(listed()).toContain(entry);
        const result = scratch.remove();
        expect(result.status, result.stderr).toBe(0);
        expect(JSON.parse(readFileSync(path.join(scratch.root, 'gh-call.json'), 'utf8'))).toEqual({
          cwd: scratch.worktree,
          args: [
            'api',
            'repos/{owner}/{repo}/pulls/7',
            '--jq',
            'if .merged_at then "MERGED" else (.state | ascii_upcase) end',
          ],
        });
        expect(existsSync(scratch.worktree)).toBe(false);
        // Exact entry: the random temp parent may itself contain any short substring.
        expect(listed()).not.toContain(entry);
      });
    }
  );

  it('removes after a server-side rebase when the PR is merged, and refuses while it is not', async () => {
    await withScratch((scratch) => {
      rewriteRemoteBranch(scratch);
      // The trap: `git log @{u}..` is not empty although nothing is lost.
      expect(scratch.git(scratch.worktree, 'log', '--oneline', '@{u}..')).not.toBe('');
      scratch.env.FAKE_PR_STATE = 'open';
      const open = scratch.remove();
      expect(open.status).toBe(1);
      expect(open.stderr).toContain('Stop and ask the maintainer');
      expect(existsSync(scratch.worktree)).toBe(true);
      scratch.env.FAKE_PR_STATE = 'closed';
      scratch.env.FAKE_PR_MERGED_AT = '2026-10-03T00:00:00Z';
      const merged = scratch.remove();
      expect(merged.status, merged.stderr).toBe(0);
      expect(existsSync(scratch.worktree)).toBe(false);
    });
  });

  it('refuses a dirty worktree even when the PR is merged', async () => {
    await withScratch((scratch) => {
      scratch.env.FAKE_PR_STATE = 'closed';
      scratch.env.FAKE_PR_MERGED_AT = '2026-10-03T00:00:00Z';
      writeFileSync(path.join(scratch.worktree, 'scratch.txt'), 'untracked\n');
      const result = scratch.remove();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('uncommitted or untracked');
      expect(existsSync(scratch.worktree)).toBe(true);
    });
  });

  it('refuses unpushed commits on open, closed-unmerged and unknown PRs', async () => {
    await withScratch((scratch) => {
      writeFileSync(path.join(scratch.worktree, 'c'), 'three\n');
      scratch.git(scratch.worktree, 'add', 'c');
      scratch.git(scratch.worktree, 'commit', '-q', '-m', 'three');
      for (const state of ['open', 'closed', 'unknown']) {
        scratch.env.FAKE_PR_STATE = state;
        if (state === 'unknown') scratch.env.FAKE_GH = 'fail';
        const result = scratch.remove();
        expect(result.status, state).toBe(1);
        expect(result.stderr).toContain(
          `state: ${state === 'unknown' ? 'unknown' : state.toUpperCase()}`
        );
        expect(result.stderr).toContain('commits its upstream lacks');
        expect(existsSync(scratch.worktree)).toBe(true);
      }
    });
  });

  it('refuses a branch with no upstream unless the PR is merged', async () => {
    await withScratch((scratch) => {
      scratch.git(scratch.worktree, 'branch', '--unset-upstream');
      for (const state of ['open', 'closed', 'unknown']) {
        scratch.env.FAKE_PR_STATE = state;
        if (state === 'unknown') scratch.env.FAKE_GH = 'fail';
        const result = scratch.remove();
        expect(result.status, state).toBe(1);
        expect(result.stderr).toContain('has no upstream');
        expect(existsSync(scratch.worktree)).toBe(true);
      }
      delete scratch.env.FAKE_GH;
      scratch.env.FAKE_PR_STATE = 'closed';
      scratch.env.FAKE_PR_MERGED_AT = '2026-10-03T00:00:00Z';
      expect(scratch.remove().status).toBe(0);
    });
  });

  it('refuses the main checkout, a non-worktree and a malformed PR number', async () => {
    await withScratch((scratch) => {
      scratch.env.FAKE_PR_STATE = 'closed';
      scratch.env.FAKE_PR_MERGED_AT = '2026-10-03T00:00:00Z';
      const main = scratch.remove(scratch.repo);
      expect(main.status).toBe(1);
      expect(main.stderr).toContain('main checkout');
      expect(existsSync(scratch.repo)).toBe(true);
      const plain = path.join(scratch.root, 'plain');
      mkdirSync(plain);
      expect(scratch.remove(plain).stderr).toContain('not a git worktree');
      const bad = spawnSync('/bin/sh', [script, scratch.worktree, '7; rm'], {
        env: scratch.env,
        encoding: 'utf8',
      });
      expect(bad.status).toBe(2);
      expect(existsSync(scratch.worktree)).toBe(true);
    });
  });
});
