import { describe, expect, it } from 'vite-plus/test';
import { isRepository, resolveRepository } from '../../scripts/github-repository.mjs';

describe('one repository source (#5139)', () => {
  it('keeps an explicit flag ahead of the environment without discovery', () => {
    expect(
      resolveRepository({
        repo: 'flag/project',
        env: { GITHUB_REPOSITORY: 'env/project' },
        execute: () => {
          throw new Error('unexpected discovery');
        },
      })
    ).toBe('flag/project');
  });
  it('uses GITHUB_REPOSITORY without querying gh or git', () => {
    expect(
      resolveRepository({
        env: { GITHUB_REPOSITORY: 'tmt-ninja/tmt' },
        execute: () => {
          throw new Error('unexpected discovery');
        },
      })
    ).toBe('tmt-ninja/tmt');
  });
  it('uses the canonical gh repository so a stale origin need not change', () => {
    const calls: unknown[] = [];
    expect(
      resolveRepository({
        env: {},
        cwd: '/fixture',
        execute: (exe, args, options) => {
          calls.push([exe, args]);
          expect(options).toMatchObject({
            cwd: '/fixture',
            env: {},
            timeoutMs: 10000,
            maxBuffer: 4096,
          });
          return 'tmt-ninja/tmt\n';
        },
      })
    ).toBe('tmt-ninja/tmt');
    expect(calls).toEqual([
      ['gh', ['repo', 'view', '--json', 'nameWithOwner', '--jq', '.nameWithOwner']],
    ]);
  });
  it.each([
    'https://github.com/fork/project.git',
    'git@github.com:fork/project.git',
    'ssh://git@github.com/fork/project.git',
    'https://github.com/fork/project/',
  ])('falls back to the bounded origin read (%s)', (remote) => {
    const calls: unknown[] = [];
    expect(
      resolveRepository({
        env: {},
        execute: (exe, args, options) => {
          calls.push([exe, args]);
          if (exe === 'gh') throw new Error('unavailable');
          expect(options.timeoutMs).toBe(3000);
          return remote;
        },
      })
    ).toBe('fork/project');
    expect(calls).toHaveLength(2);
    expect(calls[1]).toEqual(['git', ['remote', 'get-url', 'origin']]);
  });
  it.each([
    '',
    '../repo',
    'owner/..',
    'owner/repo/extra',
    'owner/repo\n',
    'owner/repo?token=secret',
  ])('refuses malformed explicit/environment values without fallback (%j)', (repo) => {
    expect(isRepository(repo)).toBe(false);
    for (const options of [{ repo, env: {} }, { env: { GITHUB_REPOSITORY: repo } }])
      expect(() =>
        resolveRepository({
          ...options,
          execute: () => {
            throw new Error('must not discover');
          },
        })
      ).toThrow('OWNER/REPO');
  });
  it.each([
    'https://example.org/owner/repo',
    'https://token@github.com/owner/repo',
    'file:///tmp/repo',
    'https://github.com/owner/repo?token=secret',
  ])('rejects an unsupported or credential-bearing remote without echoing it', (remote) => {
    expect(() =>
      resolveRepository({
        env: {},
        execute: (exe) => {
          if (exe === 'gh') throw new Error('gh secret');
          return remote;
        },
      })
    ).toThrow('origin must be a GitHub repository');
  });
  it('fails closed on malformed gh evidence or unavailable discovery without leaking command errors', () => {
    expect(() => resolveRepository({ env: {}, execute: () => 'invalid' })).toThrow('OWNER/REPO');
    expect(() =>
      resolveRepository({
        env: {},
        execute: () => {
          throw new Error('secret');
        },
      })
    ).toThrow('cannot discover GitHub repository');
  });
});

it('routes the default gate reader through the environment-selected repository', async () => {
  const { ghReader } = await import('../../scripts/ci-gate-time.mjs');
  const calls: string[][] = [];
  const read = ghReader({
    env: { GITHUB_REPOSITORY: 'tmt-ninja/tmt' },
    execute: (_exe, args) => {
      calls.push(args);
      return '{}';
    },
  });
  read('actions/runs/1/jobs');
  expect(calls).toEqual([['api', '--method', 'GET', 'repos/tmt-ninja/tmt/actions/runs/1/jobs']]);
  expect(read.requests()).toBe(1);
});
