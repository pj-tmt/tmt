import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import {
  mkdtempSync,
  readFileSync,
  rmSync,
  existsSync,
  mkdirSync,
  symlinkSync,
  chmodSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { runInPrivateWorld } from '../../scripts/private-world.mjs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { describe, expect, it } from 'vite-plus/test';
import {
  exampleExecutables,
  runtimeArchiveConfig,
  verifyRuntimeExecutables,
  consumerEnvironment,
  admitMachineIdentity,
  TOOLCHAIN,
  admitArchive,
  admitPartition,
  aggregatePartitions,
  partitionCases,
  NEXTEST_VERSION,
  type WorkspaceManifest,
  type PartitionResult,
  type TestCase,
  compareInventories,
  libtestCases,
  verifyExecution,
} from '../../scripts/workspace-nextest.mjs';

const cargo = () => [
  { executable: '/target/tests', tests: ['active', 'ignored'], ignored: ['ignored'] },
];
const nextest = () => ({
  'rust-suites': {
    suite: {
      'binary-path': '/target/tests',
      'binary-id': 'owner::suite',
      testcases: {
        active: { ignored: false, 'filter-match': { status: 'matches' } },
        ignored: { ignored: true, 'filter-match': { status: 'matches' } },
      },
    },
  },
});

describe('workspace nextest equivalence evidence', () => {
  it('reads libtest names, including panic decoration, and rejects duplicate identities', () => {
    expect(
      libtestCases('active: test\npanic - should panic: test\n2 tests, 0 benchmarks\n')
    ).toEqual(['active', 'panic']);
    expect(() => libtestCases('a: test\na: test\n')).toThrow('duplicate libtest case');
  });

  it('matches the independent binary, case and ignored inventories, including empty binaries', () => {
    const cases = compareInventories(cargo(), nextest());
    expect(cases).toEqual([
      { binary: 'owner::suite', name: 'active', ignored: false },
      { binary: 'owner::suite', name: 'ignored', ignored: true },
    ]);
    const empty = nextest();
    empty['rust-suites'].suite.testcases =
      {} as (typeof empty)['rust-suites']['suite']['testcases'];
    expect(
      compareInventories([{ executable: '/target/tests', tests: [], ignored: [] }], empty)
    ).toEqual([]);
  });

  it.each(['binary', 'case', 'ignored', 'filter', 'empty'] as const)(
    'refuses a %s mismatch',
    (kind) => {
      const input = nextest();
      if (kind === 'binary') input['rust-suites'].suite['binary-path'] = '/other';
      if (kind === 'case')
        delete (
          input['rust-suites'].suite.testcases as Partial<
            (typeof input)['rust-suites']['suite']['testcases']
          >
        ).active;
      if (kind === 'ignored') input['rust-suites'].suite.testcases.active.ignored = true;
      if (kind === 'filter')
        input['rust-suites'].suite.testcases.active['filter-match'].status = 'mismatch';
      if (kind === 'empty') input['rust-suites'] = {} as (typeof input)['rust-suites'];
      expect(() => compareInventories(cargo(), input)).toThrow();
    }
  );

  it('admits the default nextest ignored reason only for independently ignored cases', () => {
    const input = nextest();
    input['rust-suites'].suite.testcases.ignored['filter-match'] = {
      status: 'mismatch',
      reason: 'ignored',
    } as { status: string };
    expect(compareInventories(cargo(), input)).toHaveLength(2);
    input['rust-suites'].suite.testcases.active['filter-match'] = {
      status: 'mismatch',
      reason: 'ignored',
    } as { status: string };
    expect(() => compareInventories(cargo(), input)).toThrow('filtered test');
    input['rust-suites'].suite.testcases.active['filter-match'] = { status: 'matches' };
    input['rust-suites'].suite.testcases.ignored['filter-match'] = {
      status: 'mismatch',
      reason: 'expression',
    } as { status: string };
    expect(() => compareInventories(cargo(), input)).toThrow('filtered test');
  });

  it('pins real PASS identities, not a green summary or a matching count', () => {
    const cases = compareInventories(cargo(), nextest());
    const pass = '        PASS [  0.123s] (1/1) owner::suite active\n';
    expect(verifyExecution(cases, pass)).toEqual({ passed: 1, ignored: 1, total: 2 });
    for (const output of [
      '',
      'Summary: 1 test passed',
      pass.replace('active', 'other'),
      pass + pass,
      pass.replace('PASS', 'FAIL'),
      pass + pass.replace('active', 'ignored'),
    ]) {
      expect(() => verifyExecution(cases, output)).toThrow();
    }
  });
});

describe('compiled workspace partitions', () => {
  const cases = [
    { binary: 'suite', name: 'a', ignored: false },
    { binary: 'suite', name: 'b', ignored: false },
    { binary: 'suite', name: 'c', ignored: false },
    { binary: 'suite', name: 'd', ignored: false },
    { binary: 'suite', name: 'ignored', ignored: true },
  ];
  const identity = { source: 'a'.repeat(40), run: '123', attempt: '1' };
  const bytes = Buffer.from('compiled test closure');
  const manifest = (): WorkspaceManifest => ({
    schema: 1,
    identity,
    partitions: 2,
    cases,
    runtimeFiles: [],
    archive: { bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') },
  });
  const log = (tests: TestCase[]) =>
    tests.map((test) => `PASS [ 0.001s] (1/2) ${test.binary} ${test.name}`).join('\n');
  const results = (): PartitionResult[] =>
    [1, 2].map((index) => {
      const selected = partitionCases(cases, index, 2);
      return {
        schema: 1,
        identity,
        archive: manifest().archive,
        index,
        partitions: 2,
        exit: 0,
        worldRemoved: true,
        cases: selected,
        log: log(selected),
      };
    });
  const inventory = (index: number) => ({
    'rust-suites': {
      suite: {
        'binary-path': '/target/test',
        'binary-id': 'suite',
        testcases: Object.fromEntries(
          cases.map((test) => [
            test.name,
            {
              ignored: test.ignored,
              'filter-match': test.ignored
                ? { status: 'mismatch', reason: 'ignored' }
                : partitionCases(cases, index, 2).some((selected) => selected.name === test.name)
                  ? { status: 'matches' }
                  : { status: 'mismatch', reason: 'partition' },
            },
          ])
        ),
      },
    },
  });
  const success = { producer: 'success', consumers: 'success' };

  it('uses a stable case slice and refuses invalid or duplicate independent identities', () => {
    expect(partitionCases([...cases].reverse(), 1, 2).map((test) => test.name)).toEqual(['d', 'b']);
    expect(partitionCases(cases, 2, 2).map((test) => test.name)).toEqual(['b', 'd']);
    expect(partitionCases(cases, 3, 3).map((test) => test.name)).toEqual(['c']);
    for (const [index, total] of [
      [0, 2],
      [3, 2],
      [1, 0],
      [1, 4],
      [1, 2.5],
    ])
      expect(() => partitionCases(cases, index, total)).toThrow();
    expect(() => partitionCases([...cases, cases[0]], 1, 2)).toThrow('duplicate case');
    expect(() => partitionCases([], 1, 2)).toThrow('case bound');
    expect(() =>
      partitionCases(
        cases.filter((test) => test.ignored),
        1,
        2
      )
    ).toThrow('no active cases');
  });

  it('admits pinned-tool partition listings only when the exact selected identities match', () => {
    expect(admitPartition(cases, inventory(1), 1, 2)).toEqual(partitionCases(cases, 1, 2));
    expect(() => admitPartition(cases, inventory(2), 1, 2)).toThrow('slice assignment differs');
    const filtered = inventory(1);
    filtered['rust-suites'].suite.testcases.b['filter-match'] = {
      status: 'mismatch',
      reason: 'expression',
    };
    expect(() => admitPartition(cases, filtered, 1, 2)).toThrow('unexpected partition filter');
    const missing = inventory(1);
    delete missing['rust-suites'].suite.testcases.b;
    expect(() => admitPartition(cases, missing, 1, 2)).toThrow('omitted inventory case');
    const unknown = inventory(1);
    unknown['rust-suites'].suite.testcases.other = unknown['rust-suites'].suite.testcases.a;
    expect(() => admitPartition(cases, unknown, 1, 2)).toThrow('unknown case');
    const ignored = inventory(1);
    ignored['rust-suites'].suite.testcases.ignored.ignored = false;
    expect(() => admitPartition(cases, ignored, 1, 2)).toThrow('ignored set differs');
  });

  it('binds source/run/attempt/toolchain/platform and exact archive bytes before decoding', () => {
    expect(admitArchive(manifest(), bytes, identity)).toEqual(manifest());
    expect(() => admitArchive({ ...manifest(), schema: 2 }, bytes, identity)).toThrow('schema');
    for (const field of ['source', 'run', 'attempt', 'toolchain', 'platform'])
      expect(() => admitArchive(manifest(), bytes, { ...identity, [field]: 'other' })).toThrow(
        'identity differs'
      );
    expect(() =>
      admitArchive(manifest(), Buffer.concat([bytes, Buffer.from('!')]), identity)
    ).toThrow('size differs');
    expect(() => admitArchive(manifest(), Buffer.alloc(bytes.length), identity)).toThrow(
      'hash differs'
    );
  });

  it('proves every active independent case exactly once and retains the ignored obligation', () => {
    expect(aggregatePartitions(manifest(), results(), success)).toEqual({
      passed: 4,
      ignored: 1,
      total: 5,
    });
  });

  it.each(['producer', 'consumers'] as const)('refuses any non-success %s worker', (worker) => {
    for (const state of ['failure', 'cancelled', 'skipped', '', 'unknown'])
      expect(() =>
        aggregatePartitions(manifest(), results(), { ...success, [worker]: state })
      ).toThrow('worker failed or skipped');
  });

  it.each([
    'missing',
    'extra',
    'duplicate',
    'failed',
    'retained',
    'foreign',
    'archive',
    'schema',
    'partition',
    'case',
    'pass',
    'repeat',
  ])('refuses %s result evidence without replacing identity proof with counts', (kind) => {
    const input = results();
    if (kind === 'missing') input.pop();
    if (kind === 'extra') input.push(input[0]);
    if (kind === 'duplicate') input[1] = input[0];
    if (kind === 'failed') input[0].exit = 1;
    if (kind === 'retained') input[0].worldRemoved = false;
    if (kind === 'foreign') input[0].identity = { ...identity, attempt: '2' };
    if (kind === 'archive') input[0].archive = { ...manifest().archive, sha256: 'b'.repeat(64) };
    if (kind === 'schema') input[0].schema = 2;
    if (kind === 'partition') input[0].partitions = 3;
    if (kind === 'case') input[0].cases = partitionCases(cases, 2, 2);
    if (kind === 'pass') input[0].log = input[0].log.replace(' a', ' z');
    if (kind === 'repeat') input[0].log += '\n' + input[0].log;
    expect(() => aggregatePartitions(manifest(), input, success)).toThrow();
  });
});

it('captures timing and the original failure in a real bounded child phase', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'workspace-phase-'));
  try {
    const script = fileURLToPath(new URL('../../scripts/workspace-nextest.mjs', import.meta.url));
    const run = (code: string) =>
      spawnSync(
        process.execPath,
        [script, 'phase', root, 'fixture', process.execPath, '-e', code],
        { encoding: 'utf8' }
      );
    expect(run('console.log("complete")').status).toBe(0);
    const red = run('console.error("original phase failure"); process.exit(7)');
    expect(red.status).not.toBe(0);
    expect(readFileSync(path.join(root, 'fixture.log'), 'utf8')).toContain(
      'original phase failure'
    );
    const phases = JSON.parse(readFileSync(path.join(root, 'phases.json'), 'utf8'));
    expect(phases.map((phase: { exit: number }) => phase.exit)).toEqual([0, 7]);
    expect(phases.every((phase: { worldRemoved: boolean }) => phase.worldRemoved)).toBe(true);
    for (const phase of phases) expect(existsSync(phase.world)).toBe(false);
    expect(phases.every((phase: { seconds: number }) => phase.seconds >= 0)).toBe(true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

it.each([0, 7])('forwards complete large phase output to parent pipes with exit %s', (exit) => {
  const root = mkdtempSync(path.join(tmpdir(), 'workspace-phase-output-'));
  try {
    const stdoutTail =
      '\nFAIL [ 0.001s] suite broken\npanic: stdout failure detail\nSummary: 1 failed\nstdout tail\n';
    const stderrTail =
      '\nFAIL [ 0.001s] suite broken\npanic: stderr failure detail\nSummary: 1 failed\nstderr tail\n';
    const stdout = 'stdout begin\n' + 'o'.repeat(256 * 1024) + stdoutTail;
    const stderr = 'stderr begin\n' + 'e'.repeat(256 * 1024) + stderrTail;
    const code = `const { writeFileSync } = require('node:fs');
writeFileSync(1, 'stdout begin\\n' + 'o'.repeat(256 * 1024) + ${JSON.stringify(stdoutTail)});
writeFileSync(2, 'stderr begin\\n' + 'e'.repeat(256 * 1024) + ${JSON.stringify(stderrTail)});
process.exitCode = ${exit};`;
    const result = spawnSync(
      process.execPath,
      [
        fileURLToPath(new URL('../../scripts/workspace-nextest.mjs', import.meta.url)),
        'phase',
        root,
        'fixture',
        process.execPath,
        '-e',
        code,
      ],
      { encoding: 'utf8', maxBuffer: 2 * 1024 * 1024, timeout: 10_000 }
    );
    expect(result.error).toBeUndefined();
    expect(result.signal).toBeNull();
    expect(result.status).toBe(exit === 0 ? 0 : 1);
    expect(result.stdout.startsWith(stdout)).toBe(true);
    expect(result.stderr.startsWith(stderr)).toBe(true);
    expect(readFileSync(path.join(root, 'fixture.log'), 'utf8').startsWith(stdout + stderr)).toBe(
      true
    );
    const phases = JSON.parse(readFileSync(path.join(root, 'phases.json'), 'utf8'));
    expect(phases).toHaveLength(1);
    expect(phases[0].exit).toBe(exit);
    expect(phases[0].worldRemoved).toBe(true);
    expect(existsSync(phases[0].world)).toBe(false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

it('keeps the workflow compile-once, VM-isolated and fail-closed under its original caller gate', () => {
  const read = (file: string) => readFileSync(new URL(`../../../${file}`, import.meta.url), 'utf8');
  const ci = read('.github/workflows/ci.yml');
  const caller = ci.split('\n  native-workspace-tests:\n')[1].split('\n  native-process-tests:')[0];
  expect(caller).toContain('name: Native Rust workspace tests');
  expect(caller).toContain('needs: changes');
  expect(caller).toContain('uses: ./.github/workflows/workspace-partitions.yml');
  expect(caller).toContain('partitions: 2');
  expect(caller).toContain('save-cache: ${{ github.ref');
  const workflow = read('.github/workflows/workspace-partitions.yml');
  const producer = workflow.split('\n  producer:\n')[1].split('\n  consumers:')[0];
  const consumers = workflow.split('\n  consumers:\n')[1].split('\n  aggregate:')[0];
  const aggregate = workflow.split('\n  aggregate:\n')[1];
  expect(workflow).not.toMatch(
    /pull_request:|workflow_dispatch:|schedule:|self-hosted|secrets:|write/
  );
  expect(workflow.match(/runs-on: ubuntu-latest/g)).toHaveLength(3);
  expect(workflow).toContain('contents: read');
  expect(producer).toContain('workspace-nextest.mjs inventory');
  expect(producer).toContain('cargo test --locked --workspace --doc');
  expect(producer).toContain('cargo build --locked --workspace');
  expect(producer).toContain('native-qualify.mjs');
  expect(producer).toContain('cloud-sync-emulator.mjs');
  expect(consumers).toContain('needs: producer');
  expect(consumers).toContain('fail-fast: false');
  expect(consumers).toContain('fromJSON(needs.producer.outputs.matrix');
  expect(consumers).toContain('workspace-nextest.mjs consume');
  expect(consumers).not.toMatch(/rust-cache|cargo (build|test|nextest)/);
  expect(consumers).toContain(
    'sh scripts/install-ci-rust.sh 1.97.0 --profile minimal --component rustfmt,clippy'
  );
  expect(aggregate).toContain('needs: [producer, consumers]');
  expect(aggregate).toContain('if: always()');
  expect(aggregate).toContain('test "$PRODUCER_RESULT" = success');
  expect(aggregate).toContain('test "$CONSUMERS_RESULT" = success');
  expect(aggregate).toContain('workspace-nextest.mjs aggregate');
  const setup = read('.github/actions/setup-nextest/action.yml');
  expect(setup).toContain(`cargo-nextest-${NEXTEST_VERSION}`);
  expect(setup).toContain('223ba936714cc861a9cdedad5a8eb664dfdd69d6fbdf6cea6aa8184f523290dc');
  expect(setup).toContain('sha256sum --check');
});

it('keeps Ops on common setup plus Cargo/architecture while full scope owns the partition path', () => {
  const workflow = readFileSync(
    new URL('../../../.github/workflows/workspace-partitions.yml', import.meta.url),
    'utf8'
  );
  const producer = workflow.split('\n  producer:\n')[1].split('\n  consumers:')[0];
  const steps = producer.split('\n      - ').slice(1);
  const common = [
    'name: Check out repository',
    'uses: actions/setup-node@v4',
    'name: Record the pinned Node version',
    'name: Admit scope and partition count',
    'name: Install pinned Rust toolchains',
    'name: Cache Rust dependencies',
  ];
  const full = [
    'Install pinned nextest',
    'Test and build workspace',
    'Archive the verified compilation inventory',
    'Build workspace development binaries',
    'Upload compiled closure',
  ];
  const fullSync = [
    'Select hosted Temurin 21 for native sync acceptance',
    'Install pinned native sync emulator tools',
    'Prove native Auth and sealed sync against Colab Rules',
    'Prove Colab cloud sync between real serves and Colab Rules',
  ];
  const guards = new Map<string, string | undefined>([
    ...common.map((name) => [name, undefined] as const),
    ...full.map((name) => [`name: ${name}`, "inputs.scope == 'full'"] as const),
    ...fullSync.map(
      (name) => [`name: ${name}`, "inputs.scope == 'full' && inputs.native-sync"] as const
    ),
    [
      'name: Test workspace documentation separately',
      "always() && inputs.scope == 'full' && steps.toolchain.outcome == 'success'",
    ],
    [
      'name: Preserve independent inventory and phase evidence',
      "always() && inputs.scope == 'full'",
    ],
    ['name: Test Ops and architecture', "inputs.scope == 'ops'"],
  ]);
  expect(steps).toHaveLength(guards.size);
  for (const step of steps) {
    const name = step.split('\n')[0];
    expect(guards.has(name), name).toBe(true);
    expect(step.match(/^        if: (.+)$/m)?.[1], name).toBe(guards.get(name));
    if (name === 'name: Test Ops and architecture') {
      expect(step).toContain('cargo test --locked -p tmt-ops');
      expect(step).toContain('cargo test --locked -p tmt-cli --test architecture');
    }
  }
  const consumers = workflow.split('\n  consumers:\n')[1].split('\n  aggregate:')[0];
  expect(consumers).toContain("    if: inputs.scope == 'full'\n");
  const aggregate = workflow.split('\n  aggregate:\n')[1];
  expect(aggregate).toContain('if: always()');
  expect(aggregate).toContain('test "$PRODUCER_RESULT" = success');
  expect(aggregate).toContain('if [ "$SCOPE" = full ]; then test "$CONSUMERS_RESULT" = success');
  expect(aggregate).toContain('elif [ "$SCOPE" = ops ]; then test "$CONSUMERS_RESULT" = skipped');
});

it('admits observed compiler versions while collectors validate only their own run context', () => {
  const context = { source: 'a'.repeat(40), run: '123', attempt: '1', toolchain: TOOLCHAIN };
  const machine = {
    ...context,
    cargo: '/owned/toolchain/bin/cargo',
    cargoVersion: `cargo ${TOOLCHAIN} (fixture)`,
    rustcVersion: `rustc ${TOOLCHAIN} (fixture)`,
  };
  expect(admitMachineIdentity(machine, context)).toEqual(machine);
  for (const field of ['cargoVersion', 'rustcVersion'])
    expect(() => admitMachineIdentity({ ...machine, [field]: 'older compiler' }, context)).toThrow(
      'version differs'
    );
  expect(() => admitMachineIdentity({ ...machine, cargo: 'cargo' }, context)).toThrow('absolute');
  expect(() => admitMachineIdentity(machine, { ...context, attempt: '2' })).toThrow(
    'run identity differs'
  );
  expect(() => admitMachineIdentity({ ...machine, toolchain: 'other' }, context)).toThrow(
    'run identity differs'
  );
});

it('gives the isolated consumer a real Cargo executable without changing its parent environment', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'consumer-cargo-'));
  try {
    const cargo = path.join(root, 'cargo');
    writeExecutable(cargo, '#!/bin/sh\nprintf "%s\\n" "cargo fixture version"\n', 0o755);
    const base = { ...process.env, CARGO: '/missing/parent/cargo' };
    const env = consumerEnvironment(cargo, base);
    expect(base.CARGO).toBe('/missing/parent/cargo');
    expect(env.CARGO).toBe(cargo);
    let stdout = '';
    const result = runInPrivateWorld({
      argv: [
        process.execPath,
        '-e',
        `
        const assert = require('node:assert/strict');
        const {spawnSync} = require('node:child_process');
        const compiledCargo = process.argv[1];
        assert.equal(process.env.CARGO, compiledCargo, 'runtime Cargo missing');
        const versions = [compiledCargo, process.env.CARGO].map(cargo => {
          const child = spawnSync(cargo, ['--version'], {encoding:'utf8'});
          assert.equal(child.status, 0);
          return child.stdout.trim();
        });
        console.log(JSON.stringify({cargo:process.env.CARGO, versions}));
      `,
        cargo,
      ],
      base: env,
      spawn: (program, args, options) => {
        const child = spawnSync(program, args, {
          ...options,
          stdio: 'pipe',
          encoding: 'utf8',
          timeout: 10_000,
        });
        stdout = child.stdout;
        return child;
      },
      log: () => {},
    });
    expect(result.status).toBe(0);
    expect(JSON.parse(stdout)).toEqual({
      cargo,
      versions: ['cargo fixture version', 'cargo fixture version'],
    });
    expect(existsSync(result.world.root)).toBe(false);
    expect(() => consumerEnvironment('cargo', base)).toThrow('absolute');
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

// Contiguous six-active-case excerpt from producer artifact 12084071416, run 38987924215.
// Original active offset 1930 is even. Pinned nextest orders bin/tmt before tmt-cli-style;
// raw string sorting reverses those suites and changes every selected identity here.
it('preserves the real nextest 0.9.148 slice order across binary-id punctuation', () => {
  const listing = {
    'rust-suites': {
      'tmt-cli::bin/tmt': {
        'binary-id': 'tmt-cli::bin/tmt',
        'binary-path': '/home/runner/work/tmt/tmt/rust/target/debug/deps/tmt-b07081d2529c7f72',
        testcases: {
          'workspace_restore_command::tests::recorded_board_arguments_remain_literal_and_use_this_executable':
            {
              kind: 'test',
              ignored: false,
              'filter-match': {
                status: 'matches',
              },
            },
          'workspace_restore_command::tests::revival_uses_ordinary_resume_without_replaying_snapshot_coordinates_or_retry':
            {
              kind: 'test',
              ignored: false,
              'filter-match': {
                status: 'mismatch',
                reason: 'partition',
              },
            },
          'workspace_restore_command::tests::stale_missing_renamed_and_replaced_identities_never_get_a_launch_command':
            {
              kind: 'test',
              ignored: false,
              'filter-match': {
                status: 'matches',
              },
            },
        },
      },
      'tmt-cli-style': {
        'binary-id': 'tmt-cli-style',
        'binary-path':
          '/home/runner/work/tmt/tmt/rust/target/debug/deps/tmt_cli_style-cc027419151f5401',
        testcases: {
          'audit::hidden_tests::a_child_of_a_hidden_command_is_hidden_too': {
            kind: 'test',
            ignored: false,
            'filter-match': {
              status: 'mismatch',
              reason: 'partition',
            },
          },
          'audit::hidden_tests::a_double_underscore_command_must_be_hidden': {
            kind: 'test',
            ignored: false,
            'filter-match': {
              status: 'matches',
            },
          },
          'audit::hidden_tests::a_grammar_whose_hidden_commands_are_all_listed_is_clean': {
            kind: 'test',
            ignored: false,
            'filter-match': {
              status: 'mismatch',
              reason: 'partition',
            },
          },
        },
      },
    },
  };
  const cargo = [
    {
      executable: '/home/runner/work/tmt/tmt/rust/target/debug/deps/tmt-b07081d2529c7f72',
      tests: [
        'workspace_restore_command::tests::recorded_board_arguments_remain_literal_and_use_this_executable',
        'workspace_restore_command::tests::revival_uses_ordinary_resume_without_replaying_snapshot_coordinates_or_retry',
        'workspace_restore_command::tests::stale_missing_renamed_and_replaced_identities_never_get_a_launch_command',
      ],
      ignored: [],
    },
    {
      executable: '/home/runner/work/tmt/tmt/rust/target/debug/deps/tmt_cli_style-cc027419151f5401',
      tests: [
        'audit::hidden_tests::a_child_of_a_hidden_command_is_hidden_too',
        'audit::hidden_tests::a_double_underscore_command_must_be_hidden',
        'audit::hidden_tests::a_grammar_whose_hidden_commands_are_all_listed_is_clean',
      ],
      ignored: [],
    },
  ];
  const full = structuredClone(listing);
  for (const suite of Object.values(full['rust-suites']))
    for (const test of Object.values(suite.testcases)) test['filter-match'] = { status: 'matches' };
  const cases = compareInventories(cargo, full);
  const selected = admitPartition(cases, listing, 1, 2);
  expect(selected.map((test) => [test.binary, test.name])).toEqual([
    [
      'tmt-cli::bin/tmt',
      'workspace_restore_command::tests::recorded_board_arguments_remain_literal_and_use_this_executable',
    ],
    [
      'tmt-cli::bin/tmt',
      'workspace_restore_command::tests::stale_missing_renamed_and_replaced_identities_never_get_a_launch_command',
    ],
    ['tmt-cli-style', 'audit::hidden_tests::a_double_underscore_command_must_be_hidden'],
  ]);
  expect(partitionCases(cases, 2, 2)).toEqual(cases.filter((_, i) => i % 2 === 1));
  const sorted = [...cases].sort((a, b) =>
    a.binary < b.binary
      ? -1
      : a.binary > b.binary
        ? 1
        : a.name < b.name
          ? -1
          : a.name > b.name
            ? 1
            : 0
  );
  expect(() => admitPartition(sorted, listing, 1, 2)).toThrow('slice assignment differs');
});

// Cargo JSON from producer 117283454313: fixture_worker is an example, not a nextest bin-exe.
it('carries Cargo-built example workers and refuses missing, foreign or non-executable runtime inputs', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'archive-runtime-'));
  try {
    const target = path.join(root, 'target');
    expect(() => verifyRuntimeExecutables([], target)).not.toThrow();
    const file = path.join(target, 'debug/examples/fixture_worker');
    mkdirSync(path.dirname(file), { recursive: true });
    writeExecutable(file, '#!/bin/sh\nexit 0\n', 0o755);
    const example = {
      reason: 'compiler-artifact',
      profile: { test: false },
      target: { kind: ['example'] },
      executable: file,
    };
    const files = exampleExecutables(
      [example, { ...example, profile: { test: true } }, { ...example, target: { kind: ['bin'] } }],
      target
    );
    expect(files).toEqual(['debug/examples/fixture_worker']);
    expect(runtimeArchiveConfig(files)).toContain(
      '{ path = "debug/examples/fixture_worker", relative-to = "target", on-missing = "error" }'
    );
    expect(() => verifyRuntimeExecutables(files, target)).not.toThrow();
    chmodSync(file, 0o644);
    expect(() => verifyRuntimeExecutables(files, target)).toThrow('runtime executable');
    rmSync(file);
    expect(() => verifyRuntimeExecutables(files, target)).toThrow();
    const outside = path.join(root, 'outside');
    writeExecutable(outside, '#!/bin/sh\nexit 0\n', 0o755);
    symlinkSync(outside, file);
    expect(() => verifyRuntimeExecutables(files, target)).toThrow('runtime executable');
    for (const bad of [
      '../outside',
      '/absolute',
      'debug/../outside',
      'debug/./worker',
      'debug/worker\n',
    ])
      expect(() => runtimeArchiveConfig([bad])).toThrow('runtime executable escape');
    expect(() => exampleExecutables([{ ...example, executable: outside }], target)).toThrow(
      'escape'
    );
    expect(() => runtimeArchiveConfig([...files, ...files])).toThrow('duplicate');
    const source = readFileSync(
      new URL('../../scripts/workspace-nextest.mjs', import.meta.url),
      'utf8'
    );
    expect(source).toContain('writeFileSync(config, runtimeArchiveConfig(runtimeFiles))');
    expect(source).toContain("'--config-file',\n    config,\n    'archive'");
    expect(source).toContain(
      "verifyRuntimeExecutables(manifest.runtimeFiles, path.resolve('target'))"
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
