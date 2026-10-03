import { afterEach, expect, it, vi } from 'vite-plus/test';
import { execFileSync } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
  readlinkSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  readReleaseSourceAtRef,
  withReleaseSourceAtRef,
} from '../../scripts/release-source-at-ref.mjs';

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});
function repository() {
  const root = mkdtempSync(join(tmpdir(), 'tmt-source-ref-test-'));
  roots.push(root);
  const git = (...args: string[]) =>
    execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
  git('init', '--quiet');
  git('config', 'user.name', 'Release fixture');
  git('config', 'user.email', 'fixture@example.test');
  git('config', 'commit.gpgsign', 'false');
  mkdirSync(join(root, 'rust'));
  writeFileSync(join(root, 'rust/Cargo.toml'), '[workspace]\n# first cut\n');
  writeFileSync(join(root, 'rust/Cargo.lock'), '# first lock\n');
  symlinkSync('Cargo.toml', join(root, 'rust/manifest-link'));
  const commit = (message: string) => {
    git('add', '.');
    git('commit', '--quiet', '-m', message, '-m', 'Co-authored-by: Codex <codex@openai.com>');
    return git('rev-parse', 'HEAD');
  };
  const sha = commit('First fixture cut');
  return { root, git, sha, commit };
}
it('reads the captured ref independently of current HEAD and dirty checkout bytes, then removes it', () => {
  const f = repository();
  writeFileSync(join(f.root, 'rust/Cargo.toml'), '[workspace]\n# later cut\n');
  f.commit('Later fixture cut');
  writeFileSync(join(f.root, 'rust/Cargo.toml'), 'uncommitted current bytes');
  let exported = '';
  const value = withReleaseSourceAtRef(
    f.sha,
    (root) => {
      exported = root;
      expect(readlinkSync(join(root, 'rust/manifest-link'))).toBe('Cargo.toml');
      return [
        readFileSync(join(root, 'rust/Cargo.toml'), 'utf8'),
        readFileSync(join(root, 'rust/Cargo.lock'), 'utf8'),
      ];
    },
    { root: f.root }
  );
  expect(value).toEqual(['[workspace]\n# first cut\n', '# first lock\n']);
  expect(existsSync(exported)).toBe(false);
  expect(readFileSync(join(f.root, 'rust/Cargo.toml'), 'utf8')).toBe('uncommitted current bytes');
});
it('cleans up when an offline metadata reader fails rather than returning incomplete evidence', () => {
  const f = repository();
  let exported = '';
  expect(() =>
    withReleaseSourceAtRef(
      f.sha,
      (root) => {
        exported = root;
        throw new Error('Offline lock resolution incomplete');
      },
      { root: f.root }
    )
  ).toThrow('Offline lock resolution incomplete');
  expect(existsSync(exported)).toBe(false);
});
it('rejects an export-ignore omission instead of silently shrinking release attribution', () => {
  const f = repository();
  writeFileSync(join(f.root, '.gitattributes'), 'rust/Cargo.lock export-ignore\n');
  const sha = f.commit('Omitted lock fixture');
  const read = vi.fn();
  expect(() => withReleaseSourceAtRef(sha, read, { root: f.root })).toThrow(
    'Tracked release source omitted from export: rust/Cargo.lock'
  );
  expect(read).not.toHaveBeenCalled();
});
it('rejects mutable or option-shaped refs before any command executes', () => {
  const execute = vi.fn();
  for (const sha of ['main', 'v5.0.0-alpha.1', '--worktree-attributes'])
    expect(() => withReleaseSourceAtRef(sha, () => undefined, { execute })).toThrow(
      'exact commit SHA'
    );
  expect(execute).not.toHaveBeenCalled();
});

it('uses only bounded pinned locked acquisition and cleans up when the cache cannot be warmed', () => {
  const f = repository();
  let exported = '';
  const execute = vi.fn(
    (command: string, args: string[], options: { cwd: string; timeoutMs: number }) => {
      if (command === 'cargo') {
        exported = options.cwd;
        expect(args).toEqual([
          '+1.97.0',
          'fetch',
          '--quiet',
          '--locked',
          '--manifest-path',
          join(exported, 'rust/Cargo.toml'),
        ]);
        expect(options.timeoutMs).toBe(120_000);
        throw new Error('Locked cache unavailable');
      }
      return execFileSync(command, args, { cwd: options.cwd, encoding: 'utf8' });
    }
  );
  expect(() => readReleaseSourceAtRef(f.sha, { root: f.root, warm: true, execute })).toThrow(
    'Locked cache unavailable'
  );
  expect(existsSync(exported)).toBe(false);
  expect(execute.mock.calls.filter(([command]) => command === 'cargo')).toHaveLength(1);
});
