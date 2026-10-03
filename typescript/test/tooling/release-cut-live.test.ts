import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import {
  createCutClient,
  runReleaseCuts,
  type CutClient,
  type DraftRelease,
} from '../../scripts/release-cut-live.mjs';
import type { CutMetadata } from '../../scripts/release-cut.mjs';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import { checkMigration, releaseCommits } from '../../scripts/publication-gates.mjs';

const roots: string[] = [];
afterEach(() => roots.splice(0).forEach((root) => rmSync(root, { recursive: true, force: true })));

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'release-live-test-'));
  roots.push(root);
  const command = (args: string[]) => {
    const result = spawnSync('git', args, { cwd: root, encoding: 'utf8', timeout: 10_000 });
    if (result.status !== 0) throw new Error(result.stderr);
    return result.stdout.trimEnd();
  };
  const commit = (message: string) => {
    command(['add', '.']);
    command([
      '-c',
      'user.name=Release fixture',
      '-c',
      'user.email=fixture@example.test',
      '-c',
      'commit.gpgsign=false',
      'commit',
      '-m',
      `${message}\n\nCo-authored-by: Codex <codex@openai.com>`,
    ]);
    return command(['rev-parse', 'HEAD']);
  };
  command(['init', '-q', '-b', 'main']);
  mkdirSync(join(root, '.github'));
  mkdirSync(join(root, 'extensions/squad'), { recursive: true });
  writeFileSync(
    join(root, '.github/components.json'),
    JSON.stringify({
      components: {
        cli: { owns: ['.'], excludes: ['extensions'], package: 'tmt-cli' },
        squad: { owns: ['extensions/squad'], package: 'tmt-squad' },
        private: { owns: ['shared'], release: false, releaseConsumers: ['squad'] },
        'driver-herdr': { owns: ['driver'], package: 'tmt-driver-herdr', release: false },
      },
    })
  );
  const previous = commit('chore: initial fixture');
  command(['tag', 'v5.0.0-alpha.48']);
  command(['tag', 'tmt-squad-v0.1.0-alpha.14']);
  writeFileSync(join(root, 'cli.txt'), 'feature');
  writeFileSync(join(root, 'extensions/squad/feature.txt'), 'feature');
  const cut = commit('feat: shared feature');
  command(['update-ref', 'refs/remotes/origin/main', cut]);
  const releases: DraftRelease[] = ['v5.0.0-alpha.48', 'tmt-squad-v0.1.0-alpha.14'].map(
    (tag_name, index) => ({
      id: index + 1,
      tag_name,
      draft: false,
      target_commitish: previous,
      assets: [],
    })
  );
  const state: CutMetadata & { releases: DraftRelease[] } = {
    schema: 1,
    repository: 'pj-tmt/tmt',
    cut,
    draftVisibility: 'trusted',
    capturedAt: '2026-10-03T12:00:00Z',
    releases,
    runs: [],
  };
  const client: CutClient = {
    main: vi.fn(() => cut),
    metadata: vi.fn(() => structuredClone(state)),
    release: vi.fn((id: number) => structuredClone(releases.find((release) => release.id === id)!)),
    tagged: vi.fn(() => false),
    draft: vi.fn(({ tag, cut, body }) => {
      const release = {
        id: releases.length + 1,
        draft: true,
        tag_name: tag,
        target_commitish: cut,
        body,
        assets: [],
      };
      releases.push(release);
      return structuredClone(release);
    }),
    dispatch: vi.fn(),
  };
  const git = vi.fn((args: string[]) => (args[0] === 'fetch' ? '' : command(args)));
  return { client, git, state, releases, cut, previous, commit, root, command };
}

describe('live release cut lifecycle', () => {
  it('cuts exactly the owner-selected product and explicit version while keeping publication gates', async () => {
    const f = fixture();
    const result = await runReleaseCuts({
      ...f,
      live: true,
      product: 'cli',
      version: '5.1.0-alpha.0',
    });
    expect(result.actions).toEqual([
      { product: 'cli', status: 'created', tag: 'v5.1.0-alpha.0', cut: f.cut },
    ]);
    expect(f.client.draft).toHaveBeenCalledTimes(1);
    expect(f.client.dispatch).toHaveBeenCalledExactlyOnceWith('cli');
  });
  it('refuses version selection without a product and selection of parked products before mutation', async () => {
    const f = fixture();
    await expect(runReleaseCuts({ ...f, live: true, version: '6.0.0' })).rejects.toThrow(
      'exactly one product'
    );
    await expect(
      runReleaseCuts({ ...f, live: true, product: 'driver-herdr', version: '1.0.0' })
    ).rejects.toThrow('unreleased');
    expect(f.client.draft).not.toHaveBeenCalled();
    expect(f.client.dispatch).not.toHaveBeenCalled();
  });
  it('does not resume an older draft as a substitute for an explicit owner-selected cut', async () => {
    const f = fixture();
    f.releases.push({
      id: 999,
      draft: true,
      tag_name: 'v5.0.0-alpha.49',
      target_commitish: f.cut,
      assets: [],
    });
    const result = await runReleaseCuts({
      ...f,
      live: true,
      product: 'cli',
      version: '6.0.0-alpha.0',
    });
    expect(result.actions).toEqual([
      {
        product: 'cli',
        status: 'in-flight',
        reason: 'Explicit cut waits for the existing component draft.',
      },
    ]);
    expect(f.client.draft).not.toHaveBeenCalled();
    expect(f.client.dispatch).not.toHaveBeenCalled();
  });
  it('holds private-leaf and nested breaking changes for both released-root and additive consumer', () => {
    const f = fixture();
    mkdirSync(join(f.root, 'shared'));
    writeFileSync(join(f.root, 'shared/style.rs'), 'changed contract');
    const cut = f.commit(
      'chore: aggregate\n\nBEGIN_NESTED_COMMIT\nfeat(style)!: incompatible palette\nEND_NESTED_COMMIT'
    );
    const map = parseComponentMap(f.command(['show', `${cut}:.github/components.json`]));
    for (const product of ['cli', 'squad']) {
      const commits = releaseCommits({ from: f.previous, to: cut, product, map }, f.git);
      const result = checkMigration({
        files: [],
        counts: {},
        previous: { tag: 'previous', counts: {} },
        commits,
        alpha: true,
      });
      expect(result.ok).toBe(false);
      expect(result.reason).toContain(cut.slice(0, 8));
      expect(commits.find((c) => c.sha === cut)?.breaking).toBe(true);
      const control = checkMigration({
        files: [],
        counts: {},
        previous: { tag: 'previous', counts: {} },
        commits: commits.filter((c) => c.sha !== cut),
        alpha: true,
      });
      expect(control.ok).toBe(true);
    }
  });
  it('creates exactly one tagless draft per released component at X, with exact cut notes, then dispatches', async () => {
    const f = fixture();
    const result = await runReleaseCuts({ ...f, live: true });
    expect(result.actions.map((a) => [a.product, a.status, a.tag])).toEqual([
      ['cli', 'created', 'v5.0.0-alpha.49'],
      ['squad', 'created', 'tmt-squad-v0.1.0-alpha.15'],
    ]);
    expect(f.client.draft).toHaveBeenCalledTimes(2);
    expect(f.client.dispatch).toHaveBeenNthCalledWith(1, 'cli');
    expect(f.client.dispatch).toHaveBeenNthCalledWith(2, 'squad');
    for (const draft of f.releases.filter((r) => r.draft)) {
      expect(draft.target_commitish).toBe(f.cut);
      expect(draft.body).toContain(`Release cut: ${f.cut}`);
      expect([...draft.body!.matchAll(/\/commit\/([a-f0-9]{40})\)/g)].map((m) => m[1])).toEqual([
        f.cut,
      ]);
      expect(f.command(['tag', '--list', draft.tag_name])).toBe('');
    }
  });
  it('dry-run has no draft or dispatch mutation', async () => {
    const f = fixture();
    const result = await runReleaseCuts(f);
    expect(result.actions.map((a) => a.status)).toEqual(['would-create', 'would-create']);
    expect(f.client.draft).not.toHaveBeenCalled();
    expect(f.client.dispatch).not.toHaveBeenCalled();
  });
  it('keeps X and excludes later commits when main advances during acquisition', async () => {
    const f = fixture();
    writeFileSync(join(f.root, 'later.txt'), 'later');
    const later = f.commit('fix: later merge');
    f.command(['update-ref', 'refs/remotes/origin/main', later]);
    await runReleaseCuts({ ...f, live: true });
    const draft = f.releases.find((r) => r.draft)!;
    expect(draft.target_commitish).toBe(f.cut);
    expect(draft.body).not.toContain(later);
  });
  it('recovers dispatch failure from the existing draft without creating another version', async () => {
    const f = fixture();
    vi.mocked(f.client.dispatch).mockImplementationOnce(() => {
      throw new Error('uncertain dispatch');
    });
    const first = await runReleaseCuts({ ...f, live: true });
    expect(first.actions[0]).toMatchObject({ status: 'failed', reason: 'uncertain dispatch' });
    expect(first.actions[1].status).toBe('created');
    const before = f.releases.filter((r) => r.draft);
    vi.mocked(f.client.draft).mockClear();
    const second = await runReleaseCuts({ ...f, live: true });
    expect(second.actions.map((a) => a.status)).toEqual(['resumed', 'resumed']);
    expect(f.client.draft).not.toHaveBeenCalled();
    expect(f.releases.filter((r) => r.draft)).toEqual(before);
  });
  it('never retries an uncertain create inside the run, while another component can progress', async () => {
    const f = fixture();
    vi.mocked(f.client.draft).mockImplementationOnce(() => {
      throw new Error('uncertain create');
    });
    const result = await runReleaseCuts({ ...f, live: true });
    expect(result.actions.map((a) => a.status)).toEqual(['failed', 'created']);
    expect(f.client.draft).toHaveBeenCalledTimes(2);
    expect(f.client.dispatch).toHaveBeenCalledTimes(1);
    expect(f.client.dispatch).toHaveBeenCalledWith('squad');
  });
  it.each(['publication-held.json', 'verification-failed.json'])(
    'never automatically retries a draft with %s',
    async (name) => {
      const f = fixture();
      f.releases.push({
        id: 3,
        draft: true,
        tag_name: 'v5.0.0-alpha.49',
        target_commitish: f.cut,
        assets: [
          { name },
          ...(name === 'publication-held.json' ? [{ name: 'release-publication.json' }] : []),
        ],
      });
      const result = await runReleaseCuts({ ...f, live: true });
      expect(result.actions[0].status).toBe('parked');
      expect(f.client.dispatch).not.toHaveBeenCalledWith('cli');
      expect(vi.mocked(f.client.draft).mock.calls.every(([args]) => args.product !== 'cli')).toBe(
        true
      );
    }
  );
  it('an active CLI run blocks CLI only; an unknown active run blocks every product', async () => {
    const f = fixture();
    f.state.runs!.push({ id: 1, status: 'in_progress', display_title: 'Native release: cli' });
    const first = await runReleaseCuts({ ...f, live: true });
    expect(first.actions.map((a) => a.status)).toEqual(['blocked', 'created']);
    vi.mocked(f.client.draft).mockClear();
    f.state.runs!.push({ id: 2, status: 'queued', display_title: 'ambiguous run' });
    await runReleaseCuts({ ...f, live: true });
    expect(f.client.draft).not.toHaveBeenCalled();
  });
  it.each(['tag', 'target', 'notes', 'published'])(
    'a changed %s after create fails before dispatch',
    async (change) => {
      const f = fixture();
      vi.mocked(f.client.release).mockImplementation((id) => {
        const release = structuredClone(f.releases.find((r) => r.id === id)!);
        if (change === 'tag') release.tag_name = 'v5.0.0-alpha.999';
        if (change === 'target') release.target_commitish = f.previous;
        if (change === 'notes') release.body = 'different notes';
        if (change === 'published') release.draft = false;
        return release;
      });
      const result = await runReleaseCuts({ ...f, live: true });
      expect(result.actions.every((a) => a.status === 'failed')).toBe(true);
      expect(f.client.dispatch).not.toHaveBeenCalled();
    }
  );
  it('draft visibility failure fails before any mutation', async () => {
    const f = fixture();
    f.state.draftVisibility = 'unknown';
    await expect(runReleaseCuts({ ...f, live: true })).rejects.toThrow('Draft visibility');
    expect(f.client.draft).not.toHaveBeenCalled();
  });
});

describe('live cut REST boundary', () => {
  it('pins structured draft fields to X, uses REST dispatch and keeps tokens in the environment', () => {
    const execute = vi.fn(
      (
        _executable: string,
        _args: string[],
        _options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
      ) => '{}'
    );
    const client = createCutClient(
      { repository: 'pj-tmt/tmt', token: 'fixture-secret', ref: 'refs/heads/main' },
      execute
    );
    client.draft({
      product: 'cli',
      tag: 'v5.0.0-alpha.49',
      cut: 'a'.repeat(40),
      body: 'notes\nline',
    });
    client.dispatch('cli');
    expect(execute.mock.calls[0][1]).toContain('target_commitish=' + 'a'.repeat(40));
    expect(execute.mock.calls[0][1]).toContain('draft=true');
    expect(execute.mock.calls[0][1]).toContain('body=notes\nline');
    expect(execute.mock.calls[1][1]).toContain(
      'repos/pj-tmt/tmt/actions/workflows/native-release.yml/dispatches'
    );
    for (const [executable, args, options] of execute.mock.calls) {
      expect(executable).toBe('gh');
      expect(args).not.toContain('fixture-secret');
      expect(args).not.toContain('graphql');
      expect(options.env.GH_TOKEN).toBe('fixture-secret');
    }
  });
  it('refuses mutations outside main, with a valid mutation control', () => {
    const execute = vi.fn(
      (
        _executable: string,
        _args: string[],
        _options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
      ) => '{}'
    );
    createCutClient(
      { repository: 'pj-tmt/tmt', token: 'token', ref: 'refs/heads/main' },
      execute
    ).dispatch('cli');
    expect(execute).toHaveBeenCalledTimes(1);
    expect(() =>
      createCutClient(
        { repository: 'pj-tmt/tmt', token: 'token', ref: 'refs/heads/feature' },
        execute
      ).dispatch('cli')
    ).toThrow('main-only');
    expect(execute).toHaveBeenCalledTimes(1);
  });
});
