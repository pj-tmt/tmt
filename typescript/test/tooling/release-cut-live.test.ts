import { spawnSync, type SpawnSyncReturns } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { availableParallelism, cpus, loadavg, tmpdir } from 'node:os';
import { performance } from 'node:perf_hooks';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vite-plus/test';
import {
  createCutClient,
  runReleaseCuts,
  type CutClient,
  type DraftRelease,
} from '../../scripts/release-cut-live.mjs';
import type { CutMetadata } from '../../scripts/release-cut.mjs';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import { checkMigration, releaseCommits } from '../../scripts/publication-gates.mjs';

import { writeReleaseWorkspace } from '../support/release-workspace-fixture.js';

// This observer belongs to this real-Git scenario; it never changes subprocess or test deadlines.
function releaseCutDiagnostics(
  now: () => number,
  context: {
    loadavg: number[];
    cpuCount: number;
    availableParallelism: number;
    workerId: string | null;
    poolId: string | null;
  }
) {
  const startedMs = now();
  const gitCalls: {
    argv: string;
    elapsedMs: number;
    status: number | null;
    signal: string | null;
    error: string | null;
  }[] = [];
  const phases: { name: string; elapsedMs: number }[] = [];
  let phase: { name: string; startedMs: number } | undefined;
  const finishPhase = (endedMs: number) => {
    if (phase) phases.push({ name: phase.name, elapsedMs: endedMs - phase.startedMs });
  };
  return {
    phase(name: string) {
      const startedMs = now();
      finishPhase(startedMs);
      phase = { name, startedMs };
    },
    git(args: string[], execute: () => SpawnSyncReturns<string>) {
      const startedMs = now();
      let result: SpawnSyncReturns<string> | undefined;
      let failure: unknown;
      try {
        result = execute();
        return result;
      } catch (error) {
        failure = error;
        throw error;
      } finally {
        gitCalls.push({
          argv: args.join(' ').slice(0, 300),
          elapsedMs: now() - startedMs,
          status: result?.status ?? null,
          signal: result?.signal ?? null,
          error:
            result?.error?.message.slice(0, 300) ??
            (failure === undefined ? null : String(failure).slice(0, 300)),
        });
      }
    },
    report() {
      const endedMs = now();
      const elapsedMs = endedMs - startedMs;
      if (elapsedMs <= 5_000) return null;
      // Include an unfinished phase when the await/assertion fails; do not add a timer.
      const current = phase ? [{ name: phase.name, elapsedMs: endedMs - phase.startedMs }] : [];
      return {
        elapsedMs,
        context,
        phases: [...phases, ...current],
        gitCallCount: gitCalls.length,
        gitElapsedMs: gitCalls.reduce((total, call) => total + call.elapsedMs, 0),
        slowestGitCalls: [...gitCalls].sort((a, b) => b.elapsedMs - a.elapsedMs).slice(0, 12),
      };
    },
  };
}

const roots: string[] = [];
let diagnostics: ReturnType<typeof releaseCutDiagnostics>;
beforeEach(() => {
  diagnostics = releaseCutDiagnostics(() => performance.now(), {
    loadavg: loadavg(),
    cpuCount: cpus().length,
    availableParallelism: availableParallelism(),
    workerId: process.env.VITEST_WORKER_ID ?? null,
    poolId: process.env.VITEST_POOL_ID ?? null,
  });
});
afterEach(({ task }) => {
  try {
    const report = diagnostics.report();
    if (report)
      console.error(
        'Release cut case diagnostics:',
        JSON.stringify({ test: task.name, ...report })
      );
  } catch {
    // A best-effort diagnostic must not replace the case failure or prevent its existing cleanup.
  } finally {
    roots.splice(0).forEach((root) => rmSync(root, { recursive: true, force: true }));
  }
});

function fixture(activateExtensions = false) {
  const root = mkdtempSync(join(tmpdir(), 'release-live-test-'));
  roots.push(root);
  const command = (args: string[]) => {
    const result = diagnostics.git(args, () =>
      spawnSync('git', args, { cwd: root, encoding: 'utf8', timeout: 10_000 })
    );
    if (result.status !== 0) throw new Error(result.stderr, { cause: { status: result.status } });
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
  const fixtureMap = () => command(['show', 'HEAD:.github/components.json']);
  writeReleaseWorkspace(root);
  const previous = commit('chore: initial fixture');
  command(['tag', 'v5.0.0-alpha.48']);
  command(['tag', 'tmt-squad-v0.1.0-alpha.14']);
  if (activateExtensions) {
    const registry = JSON.parse(fixtureMap());
    for (const product of ['remote', 'colab']) {
      registry.components[`tmt-${product}`] = {
        package: `tmt-${product}`,
        owns: [`extensions/tmt-${product}`],
        bootstrapSha: previous,
        initialVersion: '0.1.0-alpha.1',
        requiresCliSha: previous,
      };
    }
    registry.components['colab-app'] = {
      owns: ['extensions/tmt-colab/typescript/app'],
      release: false,
      releaseConsumers: ['tmt-colab'],
    };
    registry.components['remote-client'] = {
      owns: ['extensions/tmt-remote/typescript/remote-client'],
      release: false,
      releaseConsumers: ['tmt-remote'],
    };
    writeFileSync(join(root, '.github/components.json'), JSON.stringify(registry));
    writeReleaseWorkspace(root, ['remote', 'colab']);
  }
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
  it('automatically allocates both approved first alphas using prefixed component identities', async () => {
    diagnostics.phase('fixture');
    const f = fixture(true);
    diagnostics.phase('coordinator');
    const result = await runReleaseCuts({ ...f, live: true });
    diagnostics.phase('assertions');
    for (const product of ['remote', 'colab']) {
      expect(result.actions).toContainEqual({
        product,
        status: 'created',
        tag: `tmt-${product}-v0.1.0-alpha.1`,
        cut: f.cut,
      });
      expect(f.client.dispatch).toHaveBeenCalledWith(product, `tmt-${product}-v0.1.0-alpha.1`);
    }
    expect(
      result.components
        .filter((row) => ['remote', 'colab'].includes(row.product))
        .every((row) => row.previous === f.previous)
    ).toBe(true);
    expect(f.command(['tag', '--list', 'tmt-remote-v*'])).toBe('');
    expect(f.command(['tag', '--list', 'tmt-colab-v*'])).toBe('');
  });
  it('holds a first product cut until the newest published CLI contains its registration', async () => {
    const f = fixture(true);
    const registry = JSON.parse(f.command(['show', 'HEAD:.github/components.json']));
    for (const product of ['remote', 'colab'])
      registry.components[`tmt-${product}`].requiresCliSha = f.cut;
    writeFileSync(join(f.root, '.github/components.json'), JSON.stringify(registry));
    const later = f.commit('feat: require installed product registration');
    f.command(['update-ref', 'refs/remotes/origin/main', later]);
    vi.mocked(f.client.main).mockReturnValue(later);
    f.state.cut = later;
    const blocked = await runReleaseCuts({ ...f, live: true });
    for (const [product, tag] of [
      ['cli', 'v5.0.0-alpha.49'],
      ['squad', 'tmt-squad-v0.1.0-alpha.15'],
    ]) {
      expect(blocked.actions).toContainEqual({ product, status: 'created', tag, cut: later });
      expect(f.client.dispatch).toHaveBeenCalledWith(product, tag);
    }
    expect(blocked.actions.some((action) => action.status === 'failed')).toBe(false);
    for (const product of ['remote', 'colab'])
      expect(blocked.actions).toContainEqual({
        product,
        status: 'blocked',
        reason: expect.stringContaining('predates registration'),
      });
    for (const product of ['remote', 'colab']) {
      expect(f.client.dispatch).not.toHaveBeenCalledWith(product, expect.anything());
      expect(f.releases.some((release) => release.tag_name.startsWith(`tmt-${product}-v`))).toBe(
        false
      );
    }
    f.command(['tag', 'v5.0.0-alpha.49', f.cut]);
    f.releases.push({
      id: 99,
      tag_name: 'v5.0.0-alpha.49',
      draft: false,
      target_commitish: f.cut,
      assets: [],
    });
    const admitted = await runReleaseCuts({ ...f, live: true });
    for (const product of ['remote', 'colab'])
      expect(admitted.actions).toContainEqual({
        product,
        status: 'created',
        tag: `tmt-${product}-v0.1.0-alpha.1`,
        cut: later,
      });
  });
  it.each(['remote', 'colab'])(
    'selects %s explicitly and advances from its published first cut without reusing the seed',
    async (product) => {
      const f = fixture(true);
      const first = await runReleaseCuts({ ...f, live: true, product });
      const tag = `tmt-${product}-v0.1.0-alpha.1`;
      expect(first.actions).toEqual([{ product, status: 'created', tag, cut: f.cut }]);
      f.releases.find((release) => release.tag_name === tag)!.draft = false;
      f.command(['tag', tag]);
      writeFileSync(join(f.root, `extensions/tmt-${product}/feature.txt`), 'next feature');
      const later = f.commit('feat: product follow-up');
      f.command(['update-ref', 'refs/remotes/origin/main', later]);
      vi.mocked(f.client.main).mockReturnValue(later);
      f.state.cut = later;
      const next = await runReleaseCuts({ ...f, live: true, product });
      expect(next.actions).toEqual([
        { product, status: 'created', tag: `tmt-${product}-v0.1.0-alpha.2`, cut: later },
      ]);
      expect(next.components.find((row) => row.product === product)?.previousTag).toBe(tag);
    }
  );
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
    expect(f.client.dispatch).toHaveBeenCalledExactlyOnceWith('cli', 'v5.1.0-alpha.0');
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
    expect(f.client.dispatch).toHaveBeenNthCalledWith(1, 'cli', 'v5.0.0-alpha.49');
    expect(f.client.dispatch).toHaveBeenNthCalledWith(2, 'squad', 'tmt-squad-v0.1.0-alpha.15');
    for (const draft of f.releases.filter((r) => r.draft)) {
      expect(draft.target_commitish).toBe(f.cut);
      expect(draft.body).toContain(`Release cut: ${f.cut}`);
      expect([...draft.body!.matchAll(/\/commit\/([a-f0-9]{40})\)/g)].map((m) => m[1])).toEqual([
        f.cut,
      ]);
      expect(f.command(['tag', '--list', draft.tag_name])).toBe('');
    }
  });
  it('keeps the published ancestor range after an unpublished failed cut, with an independent tag', async () => {
    const f = fixture();
    await runReleaseCuts({ ...f, live: true });
    const failed = f.releases.find((release) => release.tag_name === 'v5.0.0-alpha.49')!;
    failed.assets = [{ name: 'verification-failed.json' }];
    writeFileSync(join(f.root, 'cli.txt'), 'later fix');
    const later = f.commit('fix: next independent cut');
    f.command(['update-ref', 'refs/remotes/origin/main', later]);
    f.state.cut = later;
    vi.mocked(f.client.main).mockReturnValue(later);
    const result = await runReleaseCuts({ ...f, live: true });
    expect(result.actions[0]).toMatchObject({
      status: 'created',
      tag: 'v5.0.0-alpha.50',
      cut: later,
    });
    const next = f.releases.find((release) => release.tag_name === 'v5.0.0-alpha.50')!;
    expect(
      [...next.body!.matchAll(/\/commit\/([a-f0-9]{40})/g)].map((match) => match[1]).sort()
    ).toEqual([later, f.cut].sort());
    expect(failed.draft).toBe(true);
    expect(failed.assets).toEqual([{ name: 'verification-failed.json' }]);
  });
  it('dry-run has no draft or dispatch mutation', async () => {
    const f = fixture();
    const result = await runReleaseCuts(f);
    expect(result.actions.map((a) => a.status)).toEqual(['would-create', 'would-create']);
    expect(f.client.draft).not.toHaveBeenCalled();
    expect(f.client.dispatch).not.toHaveBeenCalled();
  });
  it.each(['failed', 'in-flight', 'published', 'held'])(
    '%s cut controls new-work eligibility after unrelated main movement',
    async (state) => {
      const f = fixture();
      await runReleaseCuts({ ...f, live: true });
      const previousCut = f.releases.find((release) => release.tag_name === 'v5.0.0-alpha.49')!;
      if (state === 'failed') previousCut.assets = [{ name: 'verification-failed.json' }];
      if (state === 'held') previousCut.assets = [{ name: 'publication-held.json' }];
      if (state === 'published') {
        previousCut.draft = false;
        f.command(['tag', previousCut.tag_name, f.cut]);
      }
      const unchanged = structuredClone(previousCut);
      vi.mocked(f.client.draft).mockClear();
      vi.mocked(f.client.dispatch).mockClear();
      writeFileSync(join(f.root, 'extensions/squad/feature.txt'), 'new Squad work');
      const later = f.commit('feat: Squad follow-up');
      f.command(['update-ref', 'refs/remotes/origin/main', later]);
      f.state.cut = later;
      vi.mocked(f.client.main).mockReturnValue(later);
      const result = await runReleaseCuts({ ...f, live: true });
      expect(result.actions.find((action) => action.product === 'squad')).toMatchObject({
        status: 'created',
        product: 'squad',
        tag: 'tmt-squad-v0.1.0-alpha.16',
      });
      expect(result.actions.find((action) => action.product === 'cli')).toMatchObject(
        state === 'failed'
          ? { status: 'created', tag: 'v5.0.0-alpha.50', cut: later }
          : { status: 'no-releasable-commits' }
      );
      if (state === 'failed') {
        const replacement = f.releases.find((release) => release.tag_name === 'v5.0.0-alpha.50')!;
        expect(
          [...replacement.body!.matchAll(/\/commit\/([a-f0-9]{40})/g)].map((m) => m[1])
        ).toEqual([f.cut]);
        expect(f.client.dispatch).toHaveBeenCalledWith('cli', replacement.tag_name);
      }
      expect(f.client.draft).toHaveBeenCalledTimes(state === 'failed' ? 2 : 1);
      expect(f.client.dispatch).toHaveBeenCalledTimes(state === 'failed' ? 2 : 1);
      expect(f.client.dispatch).toHaveBeenCalledWith('squad', 'tmt-squad-v0.1.0-alpha.16');
      expect(previousCut).toEqual(unchanged);
    }
  );
  it('replaces a verification-failed cut at the same X with the next number, without retrying it', async () => {
    const f = fixture();
    await runReleaseCuts({ ...f, live: true });
    const failed = f.releases.find((release) => release.tag_name === 'v5.0.0-alpha.49')!;
    failed.assets = [{ name: 'verification-failed.json' }];
    const unchanged = structuredClone(failed);
    vi.mocked(f.client.draft).mockClear();
    vi.mocked(f.client.dispatch).mockClear();
    const result = await runReleaseCuts({ ...f, live: true });
    expect(result.actions).toEqual([
      { product: 'cli', status: 'created', tag: 'v5.0.0-alpha.50', cut: f.cut },
      { product: 'squad', status: 'already-cut', reason: expect.any(String) },
    ]);
    expect(f.client.draft).toHaveBeenCalledTimes(1);
    expect(f.client.dispatch).toHaveBeenCalledExactlyOnceWith('cli', 'v5.0.0-alpha.50');
    const replacement = f.releases.find((release) => release.tag_name === 'v5.0.0-alpha.50')!;
    expect(replacement.body).toBe(failed.body!.replaceAll('5.0.0-alpha.49', '5.0.0-alpha.50'));
    expect(failed).toEqual(unchanged);
    expect(f.command(['tag', '--list', replacement.tag_name])).toBe('');
  });
  it('does not allocate again for non-releasable changes after an unpublished cut', async () => {
    const f = fixture();
    await runReleaseCuts({ ...f, live: true });
    vi.mocked(f.client.draft).mockClear();
    vi.mocked(f.client.dispatch).mockClear();
    writeFileSync(join(f.root, 'cli.txt'), 'internal cleanup');
    const later = f.commit('chore: internal cleanup');
    f.command(['update-ref', 'refs/remotes/origin/main', later]);
    f.state.cut = later;
    vi.mocked(f.client.main).mockReturnValue(later);
    const result = await runReleaseCuts({ ...f, live: true });
    expect(result.actions.map((action) => action.status)).toEqual([
      'no-releasable-commits',
      'no-releasable-commits',
    ]);
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
  it('does not create a duplicate cut at the same X after an uncertain dispatch', async () => {
    const f = fixture();
    vi.mocked(f.client.dispatch).mockImplementationOnce(() => {
      throw new Error('uncertain dispatch');
    });
    await runReleaseCuts({ ...f, live: true });
    vi.mocked(f.client.draft).mockClear();
    const second = await runReleaseCuts({ ...f, live: true });
    expect(second.actions.map((a) => a.status)).toEqual(['already-cut', 'already-cut']);
    expect(f.client.draft).not.toHaveBeenCalled();
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
    expect(f.client.dispatch).toHaveBeenCalledWith('squad', 'tmt-squad-v0.1.0-alpha.15');
  });
  it.each(['publication-held.json', 'verification-failed.json'])(
    'a prior draft with %s never blocks a later cut or gets retried',
    async (name) => {
      const f = fixture();
      f.releases.push({
        id: 999,
        draft: true,
        tag_name: 'v5.0.0-alpha.49',
        target_commitish: f.previous,
        assets: [{ name }],
      });
      const result = await runReleaseCuts({ ...f, live: true });
      expect(result.actions[0]).toMatchObject({ status: 'created', tag: 'v5.0.0-alpha.50' });
      expect(f.client.dispatch).toHaveBeenCalledWith('cli', 'v5.0.0-alpha.50');
      expect(f.releases.find((r) => r.id === 999)?.assets).toEqual([{ name }]);
    }
  );
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
    client.dispatch('cli', 'v5.0.0-alpha.49');
    expect(execute.mock.calls[0][1]).toContain('target_commitish=' + 'a'.repeat(40));
    expect(execute.mock.calls[0][1]).toContain('draft=true');
    expect(execute.mock.calls[0][1]).toContain('body=notes\nline');
    expect(execute.mock.calls[1][1]).toContain(
      'repos/pj-tmt/tmt/actions/workflows/native-release.yml/dispatches'
    );
    expect(execute.mock.calls[1][1]).toContain('inputs[tag]=v5.0.0-alpha.49');
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
    ).dispatch('cli', 'v5.0.0-alpha.49');
    expect(execute).toHaveBeenCalledTimes(1);
    expect(() =>
      createCutClient(
        { repository: 'pj-tmt/tmt', token: 'token', ref: 'refs/heads/feature' },
        execute
      ).dispatch('cli', 'v5.0.0-alpha.49')
    ).toThrow('main-only');
    expect(execute).toHaveBeenCalledTimes(1);
  });
});

// Injected time/results only: these controls never start Git/Cargo or wait for a slow case.
describe('release cut slow-case diagnostics', () => {
  const context = {
    loadavg: [2, 3, 4],
    cpuCount: 8,
    availableParallelism: 6,
    workerId: '2',
    poolId: '1',
  };
  const success: SpawnSyncReturns<string> = {
    pid: 1,
    output: [],
    stdout: 'exact output',
    stderr: '',
    status: 0,
    signal: null,
  };
  it('is silent through exactly five seconds and includes start concurrency context above it', () => {
    let elapsedMs = 0;
    const trace = releaseCutDiagnostics(() => elapsedMs, context);
    elapsedMs = 4_999;
    expect(trace.report()).toBeNull();
    elapsedMs = 5_000;
    expect(trace.report()).toBeNull();
    elapsedMs = 5_001;
    expect(trace.report()).toMatchObject({ elapsedMs: 5_001, context, gitCallCount: 0 });
  });
  it('keeps only the slowest twelve bounded argv records while retaining complete totals', () => {
    let elapsedMs = 0;
    const trace = releaseCutDiagnostics(() => elapsedMs, context);
    for (let i = 1; i <= 15; i++) {
      expect(
        trace.git(['show', 'x'.repeat(400)], () => {
          elapsedMs += i;
          return success;
        })
      ).toBe(success);
    }
    elapsedMs = 6_000;
    const report = trace.report()!;
    expect(report.gitCallCount).toBe(15);
    expect(report.gitElapsedMs).toBe(120);
    expect(report.slowestGitCalls.map((call) => call.elapsedMs)).toEqual([
      15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4,
    ]);
    expect(report.slowestGitCalls.every((call) => call.argv.length === 300)).toBe(true);
  });
  it('preserves nonzero status and the exact thrown error while observing an unfinished phase', () => {
    let elapsedMs = 0;
    const trace = releaseCutDiagnostics(() => elapsedMs, context);
    trace.phase('fixture');
    elapsedMs = 20;
    trace.phase('coordinator');
    const rejected = { ...success, status: 1 };
    expect(trace.git(['merge-base'], () => rejected)).toBe(rejected);
    const failure = new Error('spawn refused');
    let caught: unknown;
    try {
      trace.git(['log'], () => {
        elapsedMs += 30;
        throw failure;
      });
    } catch (error) {
      caught = error;
    }
    expect(caught).toBe(failure);
    elapsedMs = 6_000;
    expect(trace.report()).toMatchObject({
      phases: [
        { name: 'fixture', elapsedMs: 20 },
        { name: 'coordinator', elapsedMs: 5_980 },
      ],
      gitCallCount: 2,
      gitElapsedMs: 30,
      slowestGitCalls: [
        { argv: 'log', elapsedMs: 30, status: null, signal: null, error: 'Error: spawn refused' },
        { argv: 'merge-base', elapsedMs: 0, status: 1, signal: null, error: null },
      ],
    });
  });
  it('retains timeout status, signal and bounded spawn error without changing the result', () => {
    let elapsedMs = 0;
    const trace = releaseCutDiagnostics(() => elapsedMs, context);
    const timedOut: SpawnSyncReturns<string> = {
      ...success,
      status: null,
      signal: 'SIGTERM',
      error: new Error('timeout '.repeat(100)),
    };
    expect(
      trace.git(['commit'], () => {
        elapsedMs = 10_000;
        return timedOut;
      })
    ).toBe(timedOut);
    const report = trace.report()!;
    expect(report.slowestGitCalls[0]).toMatchObject({
      status: null,
      signal: 'SIGTERM',
      elapsedMs: 10_000,
    });
    expect(report.slowestGitCalls[0].error).toHaveLength(300);
  });
});
