import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
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
  const root = mkdtempSync(path.join(os.tmpdir(), 'dev-worktree-remove-'));
  try {
    const bin = path.join(root, 'bin');
    mkdirSync(bin);
    // The fake `gh` reports FAKE_PR_STATE, or fails when FAKE_GH=fail.
    writeExecutable(
      path.join(bin, 'gh'),
      '#!/bin/sh\n[ "${FAKE_GH:-}" = fail ] && exit 1\nprintf "%s\\n" "${FAKE_PR_STATE:-OPEN}"\n',
      0o755
    );
    const env: NodeJS.ProcessEnv = {
      PATH: `${bin}${path.delimiter}${process.env.PATH ?? ''}`,
      HOME: root,
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
  it('removes a clean worktree whose commits are all on its upstream', async () => {
    await withScratch((scratch) => {
      const result = scratch.remove();
      expect(result.status, result.stderr).toBe(0);
      expect(existsSync(scratch.worktree)).toBe(false);
      expect(scratch.git(scratch.repo, 'worktree', 'list')).not.toContain('wt');
    });
  });

  it('removes after a server-side rebase when the PR is merged, and refuses while it is not', async () => {
    await withScratch((scratch) => {
      rewriteRemoteBranch(scratch);
      // The trap: `git log @{u}..` is not empty although nothing is lost.
      expect(scratch.git(scratch.worktree, 'log', '--oneline', '@{u}..')).not.toBe('');
      scratch.env.FAKE_PR_STATE = 'OPEN';
      const open = scratch.remove();
      expect(open.status).toBe(1);
      expect(open.stderr).toContain('Stop and ask the maintainer');
      expect(existsSync(scratch.worktree)).toBe(true);
      scratch.env.FAKE_PR_STATE = 'MERGED';
      const merged = scratch.remove();
      expect(merged.status, merged.stderr).toBe(0);
      expect(existsSync(scratch.worktree)).toBe(false);
    });
  });

  it('refuses a dirty worktree even when the PR is merged', async () => {
    await withScratch((scratch) => {
      scratch.env.FAKE_PR_STATE = 'MERGED';
      writeFileSync(path.join(scratch.worktree, 'scratch.txt'), 'untracked\n');
      const result = scratch.remove();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('uncommitted or untracked');
      expect(existsSync(scratch.worktree)).toBe(true);
    });
  });

  it('refuses unpushed commits on an unmerged PR, including when gh cannot answer', async () => {
    await withScratch((scratch) => {
      writeFileSync(path.join(scratch.worktree, 'c'), 'three\n');
      scratch.git(scratch.worktree, 'add', 'c');
      scratch.git(scratch.worktree, 'commit', '-q', '-m', 'three');
      for (const gh of ['open', 'fail']) {
        if (gh === 'fail') scratch.env.FAKE_GH = 'fail';
        const result = scratch.remove();
        expect(result.status, gh).toBe(1);
        expect(result.stderr).toContain('commits its upstream lacks');
        expect(existsSync(scratch.worktree)).toBe(true);
      }
    });
  });

  it('refuses a branch with no upstream unless the PR is merged', async () => {
    await withScratch((scratch) => {
      scratch.git(scratch.worktree, 'branch', '--unset-upstream');
      const result = scratch.remove();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('has no upstream');
      expect(existsSync(scratch.worktree)).toBe(true);
      scratch.env.FAKE_PR_STATE = 'MERGED';
      expect(scratch.remove().status).toBe(0);
    });
  });

  it('refuses the main checkout, a non-worktree and a malformed PR number', async () => {
    await withScratch((scratch) => {
      scratch.env.FAKE_PR_STATE = 'MERGED';
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
