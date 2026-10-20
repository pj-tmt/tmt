import { execFileSync, spawnSync, type SpawnSyncReturns } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { availableParallelism, cpus, loadavg, tmpdir } from 'node:os';
import { performance, type EventLoopUtilization } from 'node:perf_hooks';
import { basename, join } from 'node:path';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vite-plus/test';
import {
  createCutClient,
  cutExitCode,
  runReleaseCuts,
  type CutClient,
  type DraftRelease,
} from '../../scripts/release-cut-live.mjs';
import { planReleaseCuts, type CutMetadata } from '../../scripts/release-cut.mjs';
import { DECLARATION_SUPPORT } from '../../scripts/release-declaration.mjs';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import { checkMigration, releaseCommits } from '../../scripts/publication-gates.mjs';
import { readCargoWorkspace, type CargoWorkspace } from '../../scripts/cargo-workspace.mjs';
import { readReleaseSourceAtRef } from '../../scripts/release-source-at-ref.mjs';

import { writeReleaseWorkspace } from '../support/release-workspace-fixture.js';

const { productOfTag } = await import(
  new URL('../../scripts/native-release-policy.mjs', import.meta.url).href
);

vi.mock('../../scripts/cargo-workspace.mjs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../scripts/cargo-workspace.mjs')>();
  return { ...actual, readCargoWorkspace: vi.fn(actual.readCargoWorkspace) };
});

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
const inheritedCargoHome = process.env.CARGO_HOME;
const inheritedRustupHome = process.env.RUSTUP_HOME;
function privateCargoHome() {
  // The dependency-free workspace needs no registry and must not share other workers' locks.
  const home = mkdtempSync(join(tmpdir(), 'release-live-cargo-'));
  vi.stubEnv('CARGO_HOME', home);
  return {
    home,
    cleanup() {
      vi.unstubAllEnvs();
      rmSync(home, { recursive: true, force: true });
    },
  };
}
let caseCargoHome: ReturnType<typeof privateCargoHome>;
let diagnostics: ReturnType<typeof releaseCutDiagnostics>;
const fixtureWorkspaces = new Map<boolean, CargoWorkspace>();
beforeAll(async () => {
  const actual = await vi.importActual<typeof import('../../scripts/cargo-workspace.mjs')>(
    '../../scripts/cargo-workspace.mjs'
  );
  const cargoHome = privateCargoHome();
  try {
    // Lifecycle cases change Git/release state, never these two dependency-free Cargo graphs.
    for (const activated of [false, true]) {
      const root = mkdtempSync(join(tmpdir(), 'release-live-workspace-'));
      try {
        writeReleaseWorkspace(root, activated ? ['remote', 'colab'] : []);
        fixtureWorkspaces.set(activated, actual.readCargoWorkspace(root));
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    }
  } finally {
    cargoHome.cleanup();
  }
}, 60_000);
beforeEach(() => {
  vi.mocked(readCargoWorkspace).mockReset();
  caseCargoHome = privateCargoHome();
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
    try {
      caseCargoHome.cleanup();
    } finally {
      roots.splice(0).forEach((root) => rmSync(root, { recursive: true, force: true }));
    }
  }
});

describe('release cut private Cargo home', () => {
  it('sets an empty private home during the case without changing RUSTUP_HOME', () => {
    expect(process.env.CARGO_HOME).toBe(caseCargoHome.home);
    expect(process.env.CARGO_HOME).not.toBe(inheritedCargoHome);
    expect(readdirSync(caseCargoHome.home)).toEqual([]);
    expect(process.env.RUSTUP_HOME).toBe(inheritedRustupHome);
  });
  it('restores the inherited home and removes distinct empty homes between cases', () => {
    // Exercise the same hook lifetime without running Git, Cargo or a release case.
    const hookHome = caseCargoHome.home;
    caseCargoHome.cleanup();
    expect(process.env.CARGO_HOME).toBe(inheritedCargoHome);
    expect(existsSync(hookHome)).toBe(false);
    const homes: string[] = [];
    for (let i = 0; i < 2; i++) {
      const current = privateCargoHome();
      try {
        homes.push(current.home);
        expect(process.env.CARGO_HOME).toBe(current.home);
        expect(readdirSync(current.home)).toEqual([]);
        expect(process.env.RUSTUP_HOME).toBe(inheritedRustupHome);
      } finally {
        current.cleanup();
      }
      expect(process.env.CARGO_HOME).toBe(inheritedCargoHome);
      expect(existsSync(current.home)).toBe(false);
    }
    expect(new Set([hookHome, ...homes]).size).toBe(3);
  });
});

function fixture(activateExtensions = false, allProducts = false, releasableDriver = false) {
  const workspace = fixtureWorkspaces.get(activateExtensions);
  if (!workspace) throw new Error('Release lifecycle fixture workspace was not read.');
  vi.mocked(readCargoWorkspace).mockReturnValue(workspace);
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
  writeReleaseWorkspace(root, allProducts ? ['driver-herdr'] : []);
  const previous = commit('chore: initial fixture');
  command(['tag', 'v5.0.0-alpha.48']);
  command(['tag', 'tmt-ops-v0.1.0-alpha.14']);
  const registry = JSON.parse(fixtureMap());
  registry.components.ops.requiresCliSha = previous;
  writeFileSync(join(root, '.github/components.json'), JSON.stringify(registry));
  if (activateExtensions) {
    for (const product of ['remote', 'colab', ...(allProducts ? ['digest'] : [])]) {
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
    if (allProducts) {
      const driver = registry.components['driver-herdr'];
      delete registry.components['driver-herdr'];
      registry.components['driver-herdr'] = {
        ...driver,
        release: true,
        bootstrapSha: previous,
        initialVersion: '0.1.0-alpha.1',
      };
    }
    writeFileSync(join(root, '.github/components.json'), JSON.stringify(registry));
    writeReleaseWorkspace(root, [
      'remote',
      'colab',
      ...(allProducts ? ['digest', 'driver-herdr'] : []),
    ]);
  }
  if (releasableDriver) {
    mkdirSync(join(root, 'driver'), { recursive: true });
    writeFileSync(join(root, 'driver/feature.txt'), 'feature');
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
  // Map the fixed repository support commit into this fixture's minimal Git graph.
  const git = vi.fn((args: string[]) =>
    args[0] === 'fetch'
      ? ''
      : command(args.map((arg) => (arg === DECLARATION_SUPPORT ? previous : arg)))
  );
  return { client, git, state, releases, cut, previous, commit, root, command };
}

it('reads the exact release-source ref through real Cargo metadata', async () => {
  const f = fixture(true);
  const actual = await vi.importActual<typeof import('../../scripts/cargo-workspace.mjs')>(
    '../../scripts/cargo-workspace.mjs'
  );
  vi.mocked(readCargoWorkspace).mockImplementation(actual.readCargoWorkspace);
  const source = readReleaseSourceAtRef(f.cut, { root: f.root, warm: true });
  expect(source.map).toEqual(
    parseComponentMap(f.command(['show', `${f.cut}:.github/components.json`]), {
      historical: true,
    })
  );
  expect(
    source.workspace.packages.map(({ name, version, manifest }) => [name, version, manifest]).sort()
  ).toEqual([
    ['tmt-cli', '5.0.0-dev', 'rust/crates/tmt-cli/Cargo.toml'],
    ['tmt-colab', '0.1.0-dev', 'extensions/tmt-colab/rust/tmt-colab/Cargo.toml'],
    ['tmt-ops', '0.1.0-dev', 'extensions/ops/Cargo.toml'],
    ['tmt-remote', '0.1.0-dev', 'extensions/tmt-remote/rust/tmt-remote/Cargo.toml'],
  ]);
  expect(readCargoWorkspace).toHaveBeenCalledExactlyOnceWith(expect.any(String));
  // A real metadata read can encounter cold/contended I/O; lifecycle cases keep their 10 s budget.
}, 30_000);

// Fake REST preserves the real planner/Git/source fixtures; no release credentials or writes.
function cutTransport(
  f: ReturnType<typeof fixture>,
  onRequest?: (path: string, args: string[], options: { timeoutMs: number }) => void
) {
  return vi.fn((_command: string, args: string[], options: { timeoutMs: number }) => {
    const path = args[1].replace('repos/pj-tmt/tmt/', '');
    onRequest?.(path, args, options);
    const field = (name: string) =>
      args.find((arg) => arg.startsWith(`${name}=`))!.slice(name.length + 1);
    if (path === 'git/ref/heads/main') return JSON.stringify({ object: { sha: f.cut } });
    if (path.startsWith('releases?')) return `HTTP/2.0 200 OK\n\n${JSON.stringify(f.releases)}`;
    if (path.startsWith('git/matching-refs/tags/')) return '[]';
    if (path === 'releases')
      return JSON.stringify(
        f.client.draft({
          tag: field('tag_name'),
          cut: field('target_commitish'),
          body: field('body'),
          product: field('tag_name').startsWith('v') ? 'cli' : field('tag_name').split('-')[1],
        })
      );
    if (/^releases\/[0-9]+$/.test(path))
      return JSON.stringify(f.client.release(Number(path.split('/')[1])));
    if (path === 'actions/workflows/native-release.yml/dispatches') {
      f.client.dispatch(field('inputs[product]'), field('inputs[tag]'));
      return '';
    }
    throw new Error(`Unexpected cut request: ${path}`);
  });
}

describe('#4098 bounded cut and dispatch uncertainty', () => {
  it.each([
    'zero',
    'multiple',
    'truncated',
    'tag',
    'event',
    'branch',
    'workflow',
    'before',
    'before-fractional',
    'after',
    'draft-id',
    'draft-tag',
    'draft-target',
    'draft-body',
    'duplicate-id',
  ])('keeps dispatch uncertain for %s evidence', (control) => {
    const cut = 'a'.repeat(40);
    const row = { product: 'cli', cut, status: 'proposed', tag: 'v5.0.0-alpha.99' };
    const startedAt =
      Date.parse('2026-10-20T10:00:00Z') + (control === 'before-fractional' ? 500 : 0);
    const finishedAt = startedAt + 1000;
    const draft = {
      id: 17,
      draft: true,
      tag_name: row.tag,
      target_commitish: cut,
      body: 'exact body',
    };
    const run = {
      id: 101,
      event: 'workflow_dispatch',
      display_title: `Native release: ${row.tag}`,
      head_branch: 'main',
      path: '.github/workflows/native-release.yml',
      created_at: new Date(startedAt).toISOString(),
      html_url: 'https://github.com/pj-tmt/tmt/actions/runs/101',
    };
    if (control === 'tag') run.display_title += '-wrong';
    if (control === 'event') run.event = 'push';
    if (control === 'branch') run.head_branch = 'other';
    if (control === 'workflow') run.path = '.github/workflows/other.yml';
    if (control === 'before-fractional') run.created_at = new Date(startedAt - 500).toISOString();
    if (control === 'before') run.created_at = new Date(startedAt - 1000).toISOString();
    if (control === 'after') run.created_at = new Date(finishedAt + 1000).toISOString();
    if (control === 'draft-id') draft.id += 1;
    if (control === 'draft-tag') draft.tag_name += '-wrong';
    if (control === 'draft-target') draft.target_commitish = 'b'.repeat(40);
    if (control === 'draft-body') draft.body += '-wrong';
    const runs =
      control === 'zero'
        ? []
        : control === 'multiple'
          ? [run, { ...run, id: 102, html_url: 'https://github.com/pj-tmt/tmt/actions/runs/102' }]
          : control === 'duplicate-id'
            ? [run, run]
            : [run];
    const execute = vi.fn((_command: string, args: string[]) =>
      JSON.stringify(
        args[1].includes('/releases/')
          ? draft
          : { total_count: control === 'truncated' ? 101 : runs.length, workflow_runs: runs }
      )
    );
    const client = createCutClient(
      { repository: 'pj-tmt/tmt', token: 'fixture-token', ref: 'refs/heads/main' },
      execute
    );
    expect(() =>
      client.reconcileDispatch!({ id: 17, row, body: 'exact body', startedAt, finishedAt })
    ).toThrow();
    expect(
      execute.mock.calls.every(([, args]) => args[args.indexOf('--method') + 1] === 'GET')
    ).toBe(true);
    expect(execute.mock.calls.length).toBeLessThanOrEqual(2);
    expect(
      cutExitCode({
        actions: [{ product: 'cli', status: 'dispatch-uncertain', reconciled: false }],
      })
    ).toBe(1);
  });

  it('reserves only three reconciliation reads and never exceeds 180 total calls', () => {
    const cut = 'a'.repeat(40);
    const row = { product: 'cli', cut, status: 'proposed', tag: 'v5.0.0-alpha.99' };
    const stamp = Date.parse('2026-10-20T10:00:00Z');
    const input = { id: 17, row, body: 'exact', startedAt: stamp, finishedAt: stamp + 1000 };
    const execute = vi.fn((_command: string, args: string[]) => {
      if (args[1].includes('/git/ref/')) return JSON.stringify({ object: { sha: cut } });
      if (args[1].includes('/releases/'))
        return JSON.stringify({
          id: 17,
          draft: true,
          tag_name: row.tag,
          target_commitish: cut,
          body: 'exact',
        });
      return JSON.stringify({
        total_count: 1,
        workflow_runs: [
          {
            id: 101,
            event: 'workflow_dispatch',
            head_branch: 'main',
            path: '.github/workflows/native-release.yml',
            display_title: `Native release: ${row.tag}`,
            created_at: new Date(stamp).toISOString(),
            html_url: 'https://github.com/pj-tmt/tmt/actions/runs/101',
          },
        ],
      });
    });
    const client = createCutClient(
      { repository: 'pj-tmt/tmt', token: 'fixture-token', ref: 'refs/heads/main' },
      execute
    );
    for (let i = 0; i < 177; i++) client.main();
    expect(() => client.main()).toThrow('normal; 177/180');
    expect(client.reconcileDispatch!(input)).toMatchObject({ runId: 101 });
    expect(() => client.reconcileDispatch!(input)).toThrow('reconciliation; 180/180');
    expect(execute).toHaveBeenCalledTimes(180);
  });

  it('keeps the final 30 seconds read-only and closes the shared deadline', () => {
    const base = Date.now();
    let clock = base;
    const now = vi.spyOn(Date, 'now').mockImplementation(() => clock);
    const execute = vi.fn(() => JSON.stringify({ object: { sha: 'a'.repeat(40) } }));
    try {
      const client = createCutClient(
        { repository: 'pj-tmt/tmt', token: 'fixture-token', ref: 'refs/heads/main' },
        execute
      );
      clock = base + 330000;
      expect(() => client.main()).toThrow('normal');
      clock = base + 360000;
      expect(() =>
        client.reconcileDispatch!({
          id: 1,
          row: { product: 'cli', status: 'proposed', cut: 'a'.repeat(40), tag: 'v5.0.0-alpha.99' },
          body: 'exact',
          startedAt: base,
          finishedAt: base + 1,
        })
      ).toThrow('reconciliation');
      expect(execute).not.toHaveBeenCalled();
    } finally {
      now.mockRestore();
    }
  });

  it('preserves full extension declarations and emits every row outcome with command timings', async () => {
    const f = fixture(true);
    const actual = await vi.importActual<typeof import('../../scripts/cargo-workspace.mjs')>(
      '../../scripts/cargo-workspace.mjs'
    );
    vi.mocked(readCargoWorkspace).mockImplementation(actual.readCargoWorkspace);
    const { map, workspace } = readReleaseSourceAtRef(f.cut, { root: f.root });
    const full = await planReleaseCuts({
      metadata: f.state,
      map,
      workspace,
      git: f.git,
      date: '2026-10-03',
    });
    for (const row of full.components)
      expect(
        await planReleaseCuts({
          metadata: f.state,
          map,
          workspace,
          git: f.git,
          date: '2026-10-03',
          product: row.product,
        })
      ).toEqual({ ...full, components: [row] });
    expect(full.components.find((row) => row.product === 'remote')?.declaration).toBeTruthy();
    const events: Record<string, unknown>[] = [];
    vi.mocked(f.client.dispatch).mockImplementationOnce(() => {
      throw new Error('known HTTP rejection');
    });
    const result = await runReleaseCuts({
      ...f,
      live: true,
      observe: (entry) => events.push(entry),
    });
    expect(cutExitCode(result)).toBe(1);
    const outcomes = events.filter((entry) => entry.kind === 'row');
    expect(outcomes).toHaveLength(result.actions.length);
    expect(outcomes[0]).toMatchObject({
      status: 'failed',
      stage: 'dispatch',
      reason: 'known HTTP rejection',
    });
    expect(
      events.some((entry) => entry.kind === 'git' && typeof entry.elapsedMs === 'number')
    ).toBe(true);
    expect(
      events.some(
        (entry) =>
          entry.kind === 'source-command' &&
          entry.program === 'cargo' &&
          entry.operation === 'metadata'
      )
    ).toBe(true);
    expect(events.some((entry) => entry.kind === 'phase' && entry.stage === 'initial-plan')).toBe(
      true
    );
    expect(JSON.stringify(events)).not.toContain('GH_TOKEN');
  });

  it('finishes six proposed rows at 67 + 6x28 seconds inside one deadline/request cap', async () => {
    const f = fixture(true, true, true);
    const actual = await vi.importActual<typeof import('../../scripts/cargo-workspace.mjs')>(
      '../../scripts/cargo-workspace.mjs'
    );
    vi.mocked(readCargoWorkspace).mockReturnValue(actual.readCargoWorkspace(f.root));
    let clock = Date.parse('2026-10-20T10:00:00Z');
    const started = clock;
    const now = vi.spyOn(Date, 'now').mockImplementation(() => clock);
    try {
      const execute = cutTransport(f, (path, _args, options) => {
        if (path === 'git/ref/heads/main' || path.startsWith('releases?')) return;
        const cost = 5600; // Five independently sub-10-second REST calls per row.
        clock += Math.min(cost, options.timeoutMs);
        if (options.timeoutMs < cost) throw new Error('spawnSync gh ETIMEDOUT');
      });
      const client = createCutClient(
        { repository: 'pj-tmt/tmt', token: 'fixture-token', ref: 'refs/heads/main' },
        execute
      );
      const git = (args: string[]) => {
        if (args[0] === 'fetch') clock += 67000;
        return f.git(args);
      };
      const result = await runReleaseCuts({ ...f, client, git, live: true });
      expect(
        result.actions.filter((row) => row.status === 'failed').map((row) => row.reason)
      ).toEqual([]);
      expect(result.actions.map((row) => row.status)).toEqual(Array(6).fill('created'));
      expect(clock - started).toBe(235000);
      expect(execute).toHaveBeenCalledTimes(1 + 3 + 6 * 5);
      expect(execute.mock.calls.length).toBeLessThanOrEqual(180);
      expect(f.client.draft).toHaveBeenCalledTimes(6);
      expect(f.client.dispatch).toHaveBeenCalledTimes(6);
      expect(
        f.git.mock.calls.filter(([args]) => args[0] === 'tag' && args[1] === '--list')
      ).toHaveLength(12);
      expect(cutExitCode(result)).toBe(0);
    } finally {
      now.mockRestore();
    }
  });

  it('reads back a timed-out successful dispatch without another POST and marks it reconciled', async () => {
    const f = fixture();
    let clock = Date.parse('2026-10-20T10:00:00Z');
    const now = vi.spyOn(Date, 'now').mockImplementation(() => clock);
    const runs: Record<string, unknown>[] = [];
    const transport = cutTransport(f);
    const execute = vi.fn((command: string, args: string[], options: { timeoutMs: number }) => {
      const path = args[1].replace('repos/pj-tmt/tmt/', '');
      if (path.startsWith('actions/workflows/native-release.yml/runs?'))
        return JSON.stringify({ total_count: runs.length, workflow_runs: runs });
      const result = transport(command, args, options);
      if (path === 'actions/workflows/native-release.yml/dispatches' && runs.length === 0) {
        const tag = args
          .find((arg) => arg.startsWith('inputs[tag]='))!
          .slice('inputs[tag]='.length);
        runs.push({
          id: 101,
          event: 'workflow_dispatch',
          display_title: `Native release: ${tag}`,
          head_branch: 'main',
          path: '.github/workflows/native-release.yml',
          created_at: new Date(clock).toISOString(),
          html_url: 'https://github.com/pj-tmt/tmt/actions/runs/101',
        });
        clock += 1000;
        throw new Error('spawnSync gh ETIMEDOUT');
      }
      return result;
    });
    try {
      const client = createCutClient(
        { repository: 'pj-tmt/tmt', token: 'fixture-token', ref: 'refs/heads/main' },
        execute
      );
      const result = await runReleaseCuts({ ...f, client, live: true });
      expect(result.actions[0]).toMatchObject({
        status: 'dispatch-uncertain',
        reconciled: true,
        runId: 101,
      });
      expect(result.actions[1].status).toBe('created');
      expect(cutExitCode(result)).toBe(0);
      expect(f.client.draft).toHaveBeenCalledTimes(2);
      expect(f.client.dispatch).toHaveBeenCalledTimes(2);
      expect(f.client.release).toHaveBeenCalledTimes(3);
    } finally {
      now.mockRestore();
    }
  });
});

describe('live release cut lifecycle', () => {
  it.each([695, 3000])(
    'uses one P+2 catalogue with %i records for four proposed products',
    async (count) => {
      const f = fixture(true);
      while (f.releases.length < count)
        f.releases.push({
          id: 100000 + f.releases.length,
          tag_name: `history-${f.releases.length}`,
          draft: false,
          target_commitish: f.previous,
          assets: [],
        });
      const execute = vi.fn((_command: string, args: string[]) => {
        const path = args[1].replace('repos/pj-tmt/tmt/', '');
        const field = (name: string) =>
          args.find((arg) => arg.startsWith(`${name}=`))!.slice(name.length + 1);
        if (path === 'git/ref/heads/main') return JSON.stringify({ object: { sha: f.cut } });
        if (path.startsWith('releases?')) {
          const page = Number(new URL(`https://fixture.invalid/${path}`).searchParams.get('page'));
          const pages = Math.ceil(f.releases.length / 100);
          return `HTTP/2.0 200 OK\nLink: <https://api.github.com/repos/pj-tmt/tmt/releases?per_page=100&page=${pages}>; rel="last"\n\n${JSON.stringify(f.releases.slice((page - 1) * 100, page * 100))}`;
        }
        if (path.startsWith('git/matching-refs/tags/')) return '[]';
        if (path === 'releases')
          return JSON.stringify(
            f.client.draft({
              tag: field('tag_name'),
              cut: field('target_commitish'),
              body: field('body'),
              product: field('tag_name').startsWith('v') ? 'cli' : field('tag_name').split('-')[1],
            })
          );
        if (new RegExp('^releases/[0-9]+$').test(path))
          return JSON.stringify(f.client.release(Number(path.split('/')[1])));
        if (path === 'actions/workflows/native-release.yml/dispatches') {
          f.client.dispatch(field('inputs[product]'), field('inputs[tag]'));
          return '';
        }
        throw new Error(`Unexpected cut request: ${path}`);
      });
      const client = createCutClient(
        { repository: 'pj-tmt/tmt', token: 'fixture-token', ref: 'refs/heads/main' },
        execute
      );
      const metadata = vi.spyOn(client, 'metadata');
      const result = await runReleaseCuts({ ...f, client, live: true });
      expect(result.actions.map((row) => row.status)).toEqual([
        'created',
        'created',
        'created',
        'created',
      ]);
      expect(metadata).toHaveBeenCalledTimes(1);
      const catalogueGets = Math.ceil(count / 100) + 2;
      expect(execute.mock.calls.filter(([, args]) => args[1].includes('/releases?'))).toHaveLength(
        catalogueGets
      );
      expect(execute).toHaveBeenCalledTimes(catalogueGets + 1 + 4 * 5);
      expect(f.client.release).toHaveBeenCalledTimes(4);
      expect(f.client.dispatch).toHaveBeenCalledTimes(4);
    }
  );

  it('captures one snapshot for every row and retains exact post-create verification', async () => {
    const f = fixture(true);
    const captured = structuredClone(f.state);
    vi.mocked(f.client.metadata)
      .mockImplementationOnce(() => captured)
      .mockImplementation(() => {
        throw new Error('Second catalogue read');
      });
    const result = await runReleaseCuts({ ...f, live: true });
    expect(result.actions.map((row) => row.status)).toEqual([
      'created',
      'created',
      'created',
      'created',
    ]);
    expect(f.client.metadata).toHaveBeenCalledTimes(1);
    expect(f.client.release).toHaveBeenCalledTimes(4);
    expect(f.client.dispatch).toHaveBeenCalledTimes(4);
    expect(captured.releases).toEqual(f.releases); // Returned creations advance the captured allocation view.
  });

  it('preserves an old held tagless high-version reservation from the one complete snapshot', async () => {
    const f = fixture();
    f.releases.push({
      id: 999,
      tag_name: 'v5.0.0-alpha.900',
      draft: true,
      target_commitish: f.previous,
      assets: [{ name: 'publication-held.json' }],
    });
    const held = structuredClone(f.releases.at(-1));
    const result = await runReleaseCuts({ ...f, live: true });
    expect(result.actions[0]).toMatchObject({ status: 'created', tag: 'v5.0.0-alpha.901' });
    expect(f.releases.find((release) => release.id === 999)).toEqual(held);
    expect(f.client.metadata).toHaveBeenCalledTimes(1);
    expect(f.command(['tag', '--list', 'v5.0.0-alpha.900'])).toBe('');
  });

  it('bounds catalogue reads and REST work for five proposed products and a sixth unchanged row', async () => {
    const f = fixture(true, true);
    const actual = await vi.importActual<typeof import('../../scripts/cargo-workspace.mjs')>(
      '../../scripts/cargo-workspace.mjs'
    );
    vi.mocked(readCargoWorkspace).mockReturnValue(actual.readCargoWorkspace(f.root));
    // One nine-page catalogue; returned allocations do not trigger another catalogue read.
    while (f.releases.length < 900)
      f.releases.push({
        id: 1000 + f.releases.length,
        draft: false,
        tag_name: `history-${f.releases.length}`,
        target_commitish: f.previous,
        assets: [],
      });
    const execute = vi.fn((_executable: string, args: string[]) => {
      const path = args[1].replace('repos/pj-tmt/tmt/', '');
      const method = args[args.indexOf('--method') + 1];
      const field = (name: string) =>
        args.find((arg) => arg.startsWith(`${name}=`))!.slice(name.length + 1);
      if (path === 'git/ref/heads/main') return JSON.stringify({ object: { sha: f.cut } });
      if (path.startsWith('releases?')) {
        const page = Number(new URL(`https://fixture.invalid/${path}`).searchParams.get('page'));
        const lastPage = Math.ceil(f.releases.length / 100);
        return `HTTP/2.0 200 OK\nLink: <https://api.github.com/repos/pj-tmt/tmt/releases?per_page=100&page=${lastPage}>; rel="last"\n\n${JSON.stringify(f.releases.slice((page - 1) * 100, page * 100))}`;
      }
      if (path.startsWith('git/matching-refs/tags/')) return '[]';
      if (path === 'releases' && method === 'POST') {
        const tag = field('tag_name');
        return JSON.stringify(
          f.client.draft({
            tag,
            cut: field('target_commitish'),
            body: field('body'),
            product: productOfTag(tag)!,
          })
        );
      }
      if (/^releases\/\d+$/.test(path))
        return JSON.stringify(f.client.release(Number(path.split('/')[1])));
      if (path === 'actions/workflows/native-release.yml/dispatches') {
        f.client.dispatch(field('inputs[product]'), field('inputs[tag]'));
        return '';
      }
      throw new Error(`Unexpected cut REST call: ${path}`);
    });
    const client = createCutClient(
      { repository: 'pj-tmt/tmt', token: 'fixture-token', ref: 'refs/heads/main' },
      execute
    );
    const metadata = vi.spyOn(client, 'metadata');
    const result = await runReleaseCuts({ ...f, client, live: true });
    expect(result.components).toHaveLength(6);
    expect(result.actions.map((row) => [row.product, row.status])).toEqual([
      ['cli', 'created'],
      ['ops', 'created'],
      ['remote', 'created'],
      ['colab', 'created'],
      ['digest', 'created'],
      ['driver-herdr', 'no-releasable-commits'],
    ]);
    expect(f.client.draft).toHaveBeenCalledTimes(5);
    expect(f.client.dispatch).toHaveBeenCalledTimes(5);
    expect(metadata).toHaveBeenCalledTimes(1); // One initial snapshot for all proposed rows.
    expect(execute.mock.calls.filter(([, args]) => args[1].includes('/releases?'))).toHaveLength(
      11
    );
    expect(execute).toHaveBeenCalledTimes(37);
    expect(execute.mock.calls.length).toBeLessThan(180);
    expect(f.git.mock.calls.filter(([args]) => args[0] === 'for-each-ref')).toHaveLength(5);
    expect(
      f.git.mock.calls.filter(([args]) => args[0] === 'tag' && args[1] === '--contains')
    ).toHaveLength(10);
  });

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
  it('holds a first product cut until a published CLI contains its registration and declaration support', async () => {
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
    expect(f.client.metadata).toHaveBeenCalledTimes(1); // Blocked and proposed rows share the captured snapshot.
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
        reason: expect.stringContaining('minimum-cli-unpublished'),
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
    const first = await runReleaseCuts({ ...f, live: true });
    expect(first.actions[0]).toMatchObject({ status: 'dispatch-uncertain', reconciled: false });
    expect(cutExitCode(first)).toBe(1);
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
