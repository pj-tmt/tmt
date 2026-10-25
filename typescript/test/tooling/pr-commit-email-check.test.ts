import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import {
  checkCommitEmails,
  commitEmailReport,
  hostnameCommits,
  isHostnameEmail,
} from '../../scripts/pr-commit-email-check.mjs';

const script = fileURLToPath(new URL('../../scripts/pr-commit-email-check.mjs', import.meta.url));
const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

// A private repository with no inherited identity, so the host's own git config can neither
// satisfy nor spoil a case.
function repository() {
  const root = mkdtempSync(path.join(tmpdir(), 'tmt-commit-email-'));
  roots.push(root);
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_CONFIG_SYSTEM: '/dev/null',
    GIT_CONFIG_NOSYSTEM: '1',
  };
  const git = (args: string[], extra: Record<string, string> = {}) => {
    const run = spawnSync('git', args, {
      cwd: root,
      env: { ...env, ...extra },
      encoding: 'utf8',
    });
    if (run.status !== 0) throw new Error(`git ${args.join(' ')}: ${run.stderr}`);
    return run.stdout.trim();
  };
  git(['init', '-q', '-b', 'main']);
  let step = 0;
  const commit = (author: string, committer = author) => {
    writeFileSync(path.join(root, 'file'), `${(step += 1)}\n`);
    git(['add', 'file']);
    git(['commit', '-q', '-m', `change ${step}`], {
      GIT_AUTHOR_NAME: 'A',
      GIT_AUTHOR_EMAIL: author,
      GIT_COMMITTER_NAME: 'C',
      GIT_COMMITTER_EMAIL: committer,
    });
    return git(['rev-parse', 'HEAD']);
  };
  return { git, commit };
}
const eventFor = (base: string, head: string, ref = 'main') => ({
  pull_request: { base: { sha: base, ref }, head: { sha: head } },
});

describe('hostname addresses', () => {
  it.each(['benhsieh@Bens-MacBook-Pro.local', 'a@host.LOCAL', 'a@b.c.local'])('flags %s', (e) => {
    expect(isHostnameEmail(e)).toBe(true);
  });
  it.each([
    'xeiyan@gmail.com',
    'a@localhost',
    'a@local',
    'local@example.com',
    'a@local.example.com',
    'a@mylocal.example',
    '',
    'no-at-sign.local',
    undefined,
    null,
  ])('accepts %s', (e) => {
    expect(isHostnameEmail(e)).toBe(false);
  });
  it('refuses a malformed identity line instead of reading past it', () => {
    expect(() => hostnameCommits('not-a-sha\ta@b.c\ta@b.c')).toThrow(/Invalid commit identity/);
    expect(() => hostnameCommits(`${'a'.repeat(40)}\ta@b.c`)).toThrow(/Invalid commit identity/);
    expect(() => hostnameCommits(`${'a'.repeat(40)}\ta@b.c\ta@b.c\textra`)).toThrow(
      /Invalid commit identity/
    );
    expect(hostnameCommits('')).toEqual([]);
  });
});

describe('the pull request commit range', () => {
  it('passes a clean range and an empty range', () => {
    const { git, commit } = repository();
    const base = commit('base@example.com');
    const head = commit('xeiyan@gmail.com', 'noreply@github.com');
    const reader = { git: (args: string[]) => git(args) };
    expect(checkCommitEmails({ event: eventFor(base, head), reader })).toEqual({
      checked: 1,
      base: 'origin/main',
      findings: [],
    });
    expect(checkCommitEmails({ event: eventFor(head, head), reader }).checked).toBe(0);
  });
  it('flags a hostname author and a hostname committer, each by commit', () => {
    const { git, commit } = repository();
    const base = commit('base@example.com');
    const byAuthor = commit('Ben@Bens-MacBook-Pro.local', 'ok@example.com');
    const byCommitter = commit('ok@example.com', 'ben@host.local');
    commit('ok@example.com');
    const head = git(['rev-parse', 'HEAD']);
    const result = checkCommitEmails({
      event: eventFor(base, head),
      reader: { git: (args: string[]) => git(args) },
    });
    expect(result.checked).toBe(3);
    expect(result.findings).toEqual([
      { sha: byCommitter, roles: [{ role: 'committer', email: 'ben@host.local' }] },
      {
        sha: byAuthor,
        roles: [{ role: 'author', email: 'Ben@Bens-MacBook-Pro.local' }],
      },
    ]);
  });
  it('never judges commits that are already in the base', () => {
    const { git, commit } = repository();
    commit('old@Old-Mac.local');
    const base = git(['rev-parse', 'HEAD']);
    const head = commit('xeiyan@gmail.com');
    const result = checkCommitEmails({
      event: eventFor(base, head),
      reader: { git: (args: string[]) => git(args) },
    });
    expect(result.findings).toEqual([]);
  });
  it('fails closed on missing event data', () => {
    const reader = {
      git: () => {
        throw new Error('unexpected read');
      },
    };
    for (const event of [undefined, {}, { pull_request: {} }, eventFor('x', 'y')])
      expect(() => checkCommitEmails({ event, reader })).toThrow(/Missing pull-request event data/);
  });
});

describe('the failure message', () => {
  const sha = 'a'.repeat(40);
  it('names each commit and the exact fix against the PR base', () => {
    const text = commitEmailReport({
      checked: 3,
      base: 'origin/main',
      findings: [{ sha, roles: [{ role: 'author', email: 'ben@Bens-MacBook-Pro.local' }] }],
    });
    expect(text).toContain('1 of 3 commit(s)');
    expect(text).toContain('aaaaaaaaaaaa: author ben@Bens-MacBook-Pro.local');
    expect(text).toContain(
      "git rebase --exec 'git commit --amend --reset-author --no-edit' origin/main"
    );
    expect(text).toContain('git push --force-with-lease');
  });
  it('uses a placeholder for an unexpected base ref and bounds a long list', () => {
    const { commit, git } = repository();
    const base = commit('base@example.com');
    const head = commit('ok@example.com');
    const event = eventFor(base, head, '--upload-pack=x y');
    expect(checkCommitEmails({ event, reader: { git: (args: string[]) => git(args) } }).base).toBe(
      '<base>'
    );
    const findings = Array.from({ length: 25 }, (_, i) => ({
      sha: i.toString(16).padStart(40, '0'),
      roles: [{ role: 'author' as const, email: 'a@h.local' }],
    }));
    const text = commitEmailReport({ checked: 25, base: '<base>', findings });
    expect(text.match(/: author a@h\.local/g)).toHaveLength(20);
    expect(text).toContain('... and 5 more');
  });
  it('reports a clean range without a fix', () => {
    const text = commitEmailReport({ checked: 2, base: 'origin/main', findings: [] });
    expect(text).toContain('Checked 2 commit(s); none');
    expect(text).not.toContain('rebase');
  });
});

describe('the command', () => {
  function run(env: Record<string, string>, event?: unknown) {
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-commit-email-run-'));
    roots.push(root);
    const eventPath = path.join(root, 'event.json');
    const summary = path.join(root, 'summary.md');
    if (event !== undefined) writeFileSync(eventPath, JSON.stringify(event));
    const result = spawnSync(process.execPath, [script], {
      env: {
        ...process.env,
        GITHUB_EVENT_PATH: eventPath,
        GITHUB_STEP_SUMMARY: summary,
        ...env,
      },
      encoding: 'utf8',
    });
    let written = '';
    try {
      written = readFileSync(summary, 'utf8');
    } catch {
      // The decision never depends on the summary file.
    }
    return { ...result, written };
  }
  const here = spawnSync('git', ['rev-parse', 'HEAD'], {
    cwd: path.dirname(script),
    encoding: 'utf8',
  }).stdout.trim();

  it('passes an empty range and writes the same text to stdout and the summary', () => {
    const result = run({ GITHUB_EVENT_NAME: 'pull_request' }, eventFor(here, here));
    expect(result.status).toBe(0);
    expect(result.stdout).toContain('Checked 0 commit(s)');
    expect(result.written).toBe(result.stdout);
  });
  it('fails closed without a pull request event or evidence', () => {
    for (const [env, event] of [
      [{ GITHUB_EVENT_NAME: 'merge_group' }, eventFor(here, here)],
      [{ GITHUB_EVENT_NAME: 'pull_request' }, undefined],
      [{ GITHUB_EVENT_NAME: 'pull_request' }, { pull_request: {} }],
    ] as const) {
      const result = run({ ...env }, event);
      expect(result.status).toBe(1);
      expect(result.stdout).toContain('evidence unavailable');
      expect(result.stdout).toContain('Rerun Code quality');
    }
  });
});
