import { execFileSync, spawnSync, type SpawnSyncReturns } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { availableParallelism, cpus, loadavg, tmpdir } from 'node:os';
import { performance, type EventLoopUtilization } from 'node:perf_hooks';
import { basename, join } from 'node:path';
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

// File-scoped wrappers also observe synchronous calls made by imported release tooling.
// They delegate unchanged; no process-wide builtin patch or production observer is installed.
const syncProcessObserver = vi.hoisted(() => ({
  observe: undefined as
    | (<T>(command: string, firstArg: string | null, execute: () => T) => T)
    | undefined,
}));
vi.mock('node:child_process', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:child_process')>();
  return {
    ...actual,
    spawnSync: (...args: Parameters<typeof actual.spawnSync>) =>
      syncProcessObserver.observe
        ? syncProcessObserver.observe(
            args[0],
            Array.isArray(args[1]) ? (args[1][0] ?? null) : null,
            () => actual.spawnSync(...args)
          )
        : actual.spawnSync(...args),
    execFileSync: (...args: Parameters<typeof actual.execFileSync>) =>
      syncProcessObserver.observe
        ? syncProcessObserver.observe(
            args[0],
            Array.isArray(args[1]) ? (args[1][0] ?? null) : null,
            () => actual.execFileSync(...args)
          )
        : actual.execFileSync(...args),
  };
});

function readLinuxPressure(
  read = (path: string) => readFileSync(path, 'utf8'),
  platform: NodeJS.Platform = process.platform
) {
  if (platform !== 'linux') return undefined;
  const counter = (value: string | undefined) => {
    if (!value || !/^\d+$/.test(value)) return null;
    const number = Number(value);
    return Number.isSafeInteger(number) ? number : null;
  };
  const pressure = (path: string) => {
    try {
      const source = read(path);
      const total = (kind: string) =>
        counter(source.match(new RegExp(`^${kind} .*\\btotal=(\\d+)(?:\\s|$)`, 'm'))?.[1]);
      return { someTotalUs: total('some'), fullTotalUs: total('full'), error: null };
    } catch (error) {
      return { someTotalUs: null, fullTotalUs: null, error: String(error).slice(0, 300) };
    }
  };
  const cpuStat = () => {
    try {
      const fields = read('/proc/stat')
        .match(/^cpu[ \t]+(.+)$/m)?.[1]
        .trim()
        .split(/\s+/);
      return { iowaitTicks: counter(fields?.[4]), stealTicks: counter(fields?.[7]), error: null };
    } catch (error) {
      return { iowaitTicks: null, stealTicks: null, error: String(error).slice(0, 300) };
    }
  };
  // Host-wide cumulative counters, not case attribution; /proc/stat units stay ticks (no HZ guess).
  return {
    cpu: pressure('/proc/pressure/cpu'),
    io: pressure('/proc/pressure/io'),
    cpuStat: cpuStat(),
  };
}

// This observer belongs to this real-Git scenario; it never changes subprocess or test deadlines.
// CPU covers the process (including sibling threads); ELU covers this worker, including blocked calls.
function releaseCutDiagnostics(
  now: () => number,
  context: {
    loadavg: number[];
    cpuCount: number;
    availableParallelism: number;
    workerId: string | null;
    poolId: string | null;
    workerCount: number | null;
  },
  observations: {
    cpuUsage: () => NodeJS.CpuUsage;
    eventLoopUtilization: typeof performance.eventLoopUtilization;
    linuxPressure?: () => ReturnType<typeof readLinuxPressure> | undefined;
  } = {
    cpuUsage: () => process.cpuUsage(),
    eventLoopUtilization: (...args: Parameters<typeof performance.eventLoopUtilization>) =>
      performance.eventLoopUtilization(...args),
    linuxPressure: () => readLinuxPressure(),
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
  const slowSyncCalls: { command: string; firstArg: string | null; elapsedMs: number }[] = [];
  let syncCallCount = 0;
  let syncElapsedMs = 0;
  let phase:
    | {
        name: string;
        startedMs: number;
        cpuUsage: NodeJS.CpuUsage;
        eventLoopUtilization: EventLoopUtilization;
        linuxPressure: ReturnType<typeof readLinuxPressure> | undefined;
      }
    | undefined;
  const phaseReport = (
    current: NonNullable<typeof phase>,
    endedMs: number,
    cpuUsage: NodeJS.CpuUsage,
    eventLoopUtilization: EventLoopUtilization,
    linuxPressure: ReturnType<typeof readLinuxPressure> | undefined
  ) => {
    const user = (cpuUsage.user - current.cpuUsage.user) / 1_000;
    const system = (cpuUsage.system - current.cpuUsage.system) / 1_000;
    const elu = observations.eventLoopUtilization(
      eventLoopUtilization,
      current.eventLoopUtilization
    );
    return {
      name: current.name,
      elapsedMs: endedMs - current.startedMs,
      processCpuMs: { user, system, total: user + system },
      ...(current.linuxPressure || linuxPressure
        ? { linuxPressure: { before: current.linuxPressure ?? null, after: linuxPressure ?? null } }
        : {}),
      eventLoopUtilization: {
        idleMs: elu.idle,
        activeMs: elu.active,
        utilization: elu.utilization,
      },
    };
  };
  const phases: ReturnType<typeof phaseReport>[] = [];
  return {
    phase(name: string) {
      const startedMs = now();
      const cpuUsage = observations.cpuUsage();
      const eventLoopUtilization = observations.eventLoopUtilization();
      const linuxPressure = observations.linuxPressure?.();
      if (phase)
        phases.push(phaseReport(phase, startedMs, cpuUsage, eventLoopUtilization, linuxPressure));
      phase = { name, startedMs, cpuUsage, eventLoopUtilization, linuxPressure };
    },
    synchronous<T>(command: string, firstArg: string | null, execute: () => T): T {
      const startedMs = now();
      try {
        return execute();
      } finally {
        const elapsedMs = now() - startedMs;
        syncCallCount++;
        syncElapsedMs += elapsedMs;
        if (elapsedMs > 250)
          slowSyncCalls.push({
            command: basename(command).slice(0, 100),
            firstArg: firstArg?.slice(0, 300) ?? null,
            elapsedMs,
          });
      }
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
      const current = phase
        ? [
            phaseReport(
              phase,
              endedMs,
              observations.cpuUsage(),
              observations.eventLoopUtilization(),
              observations.linuxPressure?.()
            ),
          ]
        : [];
      return {
        elapsedMs,
        context,
        phases: [...phases, ...current],
        syncCallCount,
        syncElapsedMs,
        slowSyncCalls: [...slowSyncCalls],
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
    // The public test API has no pool-size getter; IDs and the static config are not a live count.
    workerCount: null,
  });
  syncProcessObserver.observe = (command, firstArg, execute) =>
    diagnostics.synchronous(command, firstArg, execute);
});
afterEach(({ task }) => {
  syncProcessObserver.observe = undefined;
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
  mkdirSync(join(root, 'extensions/ops'), { recursive: true });
  writeFileSync(
    join(root, '.github/components.json'),
    JSON.stringify({
      components: {
        cli: { owns: ['.'], excludes: ['extensions'], package: 'tmt-cli' },
        ops: { owns: ['extensions/ops'], package: 'tmt-ops' },
        private: { owns: ['shared'], release: false, releaseConsumers: ['ops'] },
        'driver-herdr': { owns: ['driver'], package: 'tmt-driver-herdr', release: false },
      },
    })
  );
  const fixtureMap = () => command(['show', 'HEAD:.github/components.json']);
  writeReleaseWorkspace(root);
  const previous = commit('chore: initial fixture');
  command(['tag', 'v5.0.0-alpha.48']);
  command(['tag', 'tmt-ops-v0.1.0-alpha.14']);
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
  writeFileSync(join(root, 'extensions/ops/feature.txt'), 'feature');
  const cut = commit('feat: shared feature');
  command(['update-ref', 'refs/remotes/origin/main', cut]);
  const releases: DraftRelease[] = ['v5.0.0-alpha.48', 'tmt-ops-v0.1.0-alpha.14'].map(
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
      ['ops', 'tmt-ops-v0.1.0-alpha.15'],
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
    for (const product of ['cli', 'ops']) {
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
      ['ops', 'created', 'tmt-ops-v0.1.0-alpha.15'],
    ]);
    expect(f.client.draft).toHaveBeenCalledTimes(2);
    expect(f.client.dispatch).toHaveBeenNthCalledWith(1, 'cli', 'v5.0.0-alpha.49');
    expect(f.client.dispatch).toHaveBeenNthCalledWith(2, 'ops', 'tmt-ops-v0.1.0-alpha.15');
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
      writeFileSync(join(f.root, 'extensions/ops/feature.txt'), 'new Ops work');
      const later = f.commit('feat: Ops follow-up');
      f.command(['update-ref', 'refs/remotes/origin/main', later]);
      f.state.cut = later;
      vi.mocked(f.client.main).mockReturnValue(later);
      const result = await runReleaseCuts({ ...f, live: true });
      expect(result.actions.find((action) => action.product === 'ops')).toMatchObject({
        status: 'created',
        product: 'ops',
        tag: 'tmt-ops-v0.1.0-alpha.16',
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
      expect(f.client.dispatch).toHaveBeenCalledWith('ops', 'tmt-ops-v0.1.0-alpha.16');
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
      { product: 'ops', status: 'already-cut', reason: expect.any(String) },
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
    expect(f.client.dispatch).toHaveBeenCalledWith('ops', 'tmt-ops-v0.1.0-alpha.15');
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
    workerCount: 2,
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
  it('records exact CPU and worker ELU deltas for closed and unfinished slow-case phases', () => {
    let elapsedMs = 0;
    let cpuUsage = { user: 1_000, system: 2_000 };
    let elu = { idle: 100, active: 200, utilization: 2 / 3 };
    const observations = {
      cpuUsage: () => cpuUsage,
      eventLoopUtilization: vi.fn((end?: EventLoopUtilization, start?: EventLoopUtilization) => {
        if (!end || !start) return elu;
        const idle = end.idle - start.idle;
        const active = end.active - start.active;
        return { idle, active, utilization: active / (idle + active) };
      }),
    };
    const trace = releaseCutDiagnostics(() => elapsedMs, context, observations);
    trace.phase('fixture');
    elapsedMs = 200;
    cpuUsage = { user: 126_000, system: 52_000 };
    elu = { idle: 125, active: 375, utilization: 0.75 };
    trace.phase('coordinator');
    elapsedMs = 6_200;
    cpuUsage = { user: 1_376_000, system: 302_000 };
    elu = { idle: 4_625, active: 1_875, utilization: 1_875 / 6_500 };
    const report = trace.report();
    expect(report).toEqual({
      elapsedMs: 6_200,
      context,
      phases: [
        {
          name: 'fixture',
          elapsedMs: 200,
          processCpuMs: { user: 125, system: 50, total: 175 },
          eventLoopUtilization: { idleMs: 25, activeMs: 175, utilization: 0.875 },
        },
        {
          name: 'coordinator',
          elapsedMs: 6_000,
          processCpuMs: { user: 1_250, system: 250, total: 1_500 },
          eventLoopUtilization: { idleMs: 4_500, activeMs: 1_500, utilization: 0.25 },
        },
      ],
      syncCallCount: 0,
      syncElapsedMs: 0,
      slowSyncCalls: [],
      gitCallCount: 0,
      gitElapsedMs: 0,
      slowestGitCalls: [],
    });
    expect(observations.eventLoopUtilization).toHaveBeenLastCalledWith(elu, {
      idle: 125,
      active: 375,
      utilization: 0.75,
    });
    // Reporting does not reset the unfinished phase or reuse the prior delta as a baseline.
    expect(trace.report()).toEqual(report);
    const unknownWorkers = releaseCutDiagnostics(
      () => elapsedMs,
      { ...context, workerCount: null },
      observations
    );
    elapsedMs += 5_001;
    expect(unknownWorkers.report()?.context.workerCount).toBeNull();
  });
  it('records literal Linux pressure and CPU-stat snapshots at closed and unfinished boundaries', () => {
    let elapsedMs = 0;
    let generation = 0;
    const sources = [
      [
        'some avg10=0.10 avg60=0.20 avg300=0.30 total=100\nfull avg10=0.00 total=5\n',
        'cpu 1 2 3 4 50 6 7 80 9 10\n',
      ],
      [
        'some avg10=0.10 avg60=0.20 avg300=0.30 total=160\nfull avg10=0.00 total=9\n',
        'cpu 1 2 3 4 70 6 7 110 9 10\n',
      ],
      [
        'some avg10=0.10 avg60=0.20 avg300=0.30 total=200\nfull avg10=0.00 total=12\n',
        'cpu 1 2 3 4 90 6 7 150 9 10\n',
      ],
    ];
    const reader = vi.fn((path: string) => sources[generation][path === '/proc/stat' ? 1 : 0]);
    const observations = {
      cpuUsage: () => ({ user: 0, system: 0 }),
      eventLoopUtilization: () => ({ idle: 0, active: 0, utilization: 0 }),
      linuxPressure: () => readLinuxPressure(reader, 'linux'),
    };
    const trace = releaseCutDiagnostics(() => elapsedMs, context, observations);
    trace.phase('fixture');
    generation = 1;
    elapsedMs = 200;
    trace.phase('coordinator');
    generation = 2;
    elapsedMs = 6_200;
    const snapshot = (
      someTotalUs: number,
      fullTotalUs: number,
      iowaitTicks: number,
      stealTicks: number
    ) => ({
      cpu: { someTotalUs, fullTotalUs, error: null },
      io: { someTotalUs, fullTotalUs, error: null },
      cpuStat: { iowaitTicks, stealTicks, error: null },
    });
    expect(trace.report()?.phases.map((phase) => phase.linuxPressure)).toEqual([
      { before: snapshot(100, 5, 50, 80), after: snapshot(160, 9, 70, 110) },
      { before: snapshot(160, 9, 70, 110), after: snapshot(200, 12, 90, 150) },
    ]);
    expect(reader.mock.calls.map(([path]) => path)).toEqual([
      '/proc/pressure/cpu',
      '/proc/pressure/io',
      '/proc/stat',
      '/proc/pressure/cpu',
      '/proc/pressure/io',
      '/proc/stat',
      '/proc/pressure/cpu',
      '/proc/pressure/io',
      '/proc/stat',
    ]);
    const offLinux = releaseCutDiagnostics(() => elapsedMs, context, {
      ...observations,
      linuxPressure: () => readLinuxPressure(reader, 'darwin'),
    });
    offLinux.phase('fixture');
    elapsedMs += 5_001;
    expect(offLinux.report()?.phases[0]).not.toHaveProperty('linuxPressure');
    expect(reader).toHaveBeenCalledTimes(9);
  });
  it('keeps unavailable and malformed Linux readings unknown rather than zero', () => {
    const failure = new Error('unavailable');
    expect(
      readLinuxPressure(() => {
        throw failure;
      }, 'linux')
    ).toEqual({
      cpu: { someTotalUs: null, fullTotalUs: null, error: 'Error: unavailable' },
      io: { someTotalUs: null, fullTotalUs: null, error: 'Error: unavailable' },
      cpuStat: { iowaitTicks: null, stealTicks: null, error: 'Error: unavailable' },
    });
    expect(
      readLinuxPressure(() => 'some avg10=1 total=9007199254740992\ncpu 1 2\n', 'linux')
    ).toEqual({
      cpu: { someTotalUs: null, fullTotalUs: null, error: null },
      io: { someTotalUs: null, fullTotalUs: null, error: null },
      cpuStat: { iowaitTicks: null, stealTicks: null, error: null },
    });
  });
  it('retains aggregate synchronous command wall time and exact results/errors through both API wrappers', async () => {
    const { runPackedCommand } = await vi.importActual<{
      runPackedCommand: (
        executable: string,
        args: string[],
        options: { cwd: string; env: NodeJS.ProcessEnv; isolateProcessGroup: boolean }
      ) => string;
    }>('../../scripts/packed-command.mjs');
    let elapsedMs = 0;
    const trace = releaseCutDiagnostics(() => elapsedMs, context);
    const failure = new Error('exact child failure');
    let calls = 0;
    const observed: [string, string | null][] = [];
    syncProcessObserver.observe = (command, firstArg, execute) =>
      trace.synchronous(command, firstArg, () => {
        observed.push([command, firstArg]);
        calls++;
        elapsedMs += calls * 10;
        if (calls === 1) return success as ReturnType<typeof execute>;
        if (calls === 2) return 'exact exec output' as ReturnType<typeof execute>;
        if (calls === 3) return success as ReturnType<typeof execute>;
        throw failure;
      });
    // The fake observer returns/throws without executing these command callbacks.
    expect(spawnSync('unused', ['spawn-first'], { encoding: 'utf8' })).toBe(success);
    expect(execFileSync('unused', ['exec-first'], { encoding: 'utf8' })).toBe('exact exec output');
    expect(runPackedCommand('unused', [], { cwd: '/', env: {}, isolateProcessGroup: false })).toBe(
      'exact output'
    );
    let caught: unknown;
    try {
      execFileSync('unused');
    } catch (error) {
      caught = error;
    }
    expect(caught).toBe(failure);
    expect(calls).toBe(4);
    expect(observed).toEqual([
      ['unused', 'spawn-first'],
      ['unused', 'exec-first'],
      ['unused', null],
      ['unused', null],
    ]);
    elapsedMs = 6_000;
    expect(trace.report()).toMatchObject({
      syncCallCount: 4,
      syncElapsedMs: 100,
      gitCallCount: 0,
      gitElapsedMs: 0,
    });
  });
  it('records every synchronous call over 250ms with bounded identity only in a slow case', () => {
    let elapsedMs = 0;
    const trace = releaseCutDiagnostics(() => elapsedMs, context);
    const failure = new Error('same failure');
    expect(
      trace.synchronous('/fixture/bin/git', 'status', () => {
        elapsedMs += 250;
        return success;
      })
    ).toBe(success);
    expect(
      trace.synchronous('/fixture/bin/node', 'x'.repeat(400), () => {
        elapsedMs += 251;
        return success;
      })
    ).toBe(success);
    let caught: unknown;
    try {
      trace.synchronous('/fixture/bin/' + 'c'.repeat(120), null, () => {
        elapsedMs += 300;
        throw failure;
      });
    } catch (error) {
      caught = error;
    }
    expect(caught).toBe(failure);
    elapsedMs = 5_000;
    expect(trace.report()).toBeNull();
    elapsedMs = 5_001;
    expect(trace.report()).toMatchObject({
      syncCallCount: 3,
      syncElapsedMs: 801,
      slowSyncCalls: [
        { command: 'node', firstArg: 'x'.repeat(300), elapsedMs: 251 },
        { command: 'c'.repeat(100), firstArg: null, elapsedMs: 300 },
      ],
    });
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
