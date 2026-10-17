import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import {
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
    expect(partitionCases([...cases].reverse(), 1, 2).map((test) => test.name)).toEqual(['a', 'c']);
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
  expect(consumers).not.toMatch(/rust-cache|cargo (build|test|nextest)|install-ci-rust/);
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
