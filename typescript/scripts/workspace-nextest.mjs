import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  mkdirSync,
  readFileSync,
  writeFileSync,
  realpathSync,
  lstatSync,
  existsSync,
  readdirSync,
} from 'node:fs';
import { createHash } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import { runInPrivateWorld } from './private-world.mjs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const clean = (text) => text.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '');

/** Libtest decorates should-panic names in --list, while nextest uses the actual name. */
export function libtestCases(text) {
  const names = clean(text)
    .split('\n')
    .flatMap((line) => {
      const match = /^(.*): test$/.exec(line);
      return match ? [match[1].replace(/ - should panic$/, '')] : [];
    });
  assert.equal(new Set(names).size, names.length, 'duplicate libtest case');
  return names.sort();
}

/** Compare an independently discovered Cargo binary closure and libtest cases. */
export function compareInventories(cargo, nextest) {
  const suites = Object.values(nextest['rust-suites']);
  assert.ok(suites.length > 0, 'empty nextest inventory');
  const binaries = new Map(cargo.map((suite) => [suite.executable, suite]));
  assert.equal(binaries.size, cargo.length, 'duplicate Cargo executable');
  assert.equal(suites.length, binaries.size, 'Cargo/nextest binary count differs');
  const cases = [];
  for (const suite of suites) {
    const original = binaries.get(suite['binary-path']);
    assert.ok(original, `nextest binary absent from Cargo: ${suite['binary-path']}`);
    binaries.delete(suite['binary-path']);
    const names = Object.keys(suite.testcases).sort();
    assert.deepEqual(names, original.tests, `${suite['binary-id']}: case identities differ`);
    const ignored = names.filter((name) => suite.testcases[name].ignored);
    assert.deepEqual(ignored, original.ignored, `${suite['binary-id']}: ignored set differs`);
    for (const name of names) {
      const match = suite.testcases[name]['filter-match'];
      assert.ok(
        match.status === 'matches' ||
          (suite.testcases[name].ignored &&
            match.status === 'mismatch' &&
            match.reason === 'ignored'),
        `${suite['binary-id']}: ${name}: filtered test`
      );
      cases.push({ binary: suite['binary-id'], name, ignored: ignored.includes(name) });
    }
  }
  assert.equal(binaries.size, 0, 'Cargo binary omitted by nextest');
  return cases;
}

/** A green summary alone cannot hide a filtered, missing or repeated case. */
export function verifyExecution(cases, log) {
  const observed = clean(log)
    .split('\n')
    .flatMap((line) => {
      const match = /PASS\s+\[\s*[\d.]+s\]\s+\(\s*\d+\/\d+\)\s+(\S+)\s+(.+)$/.exec(line);
      return match ? [JSON.stringify([match[1], match[2]])] : [];
    });
  assert.equal(new Set(observed).size, observed.length, 'repeated nextest PASS');
  const expected = cases
    .filter((test) => !test.ignored)
    .map((test) => JSON.stringify([test.binary, test.name]));
  assert.deepEqual(observed.sort(), expected.sort(), 'nextest executed case identities differ');
  return { passed: observed.length, ignored: cases.length - observed.length, total: cases.length };
}

export const NEXTEST_VERSION = '0.9.148';
export const TOOLCHAIN = '1.97.0';
const key = (test) => JSON.stringify([test.binary, test.name]);
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

function admittedCases(cases) {
  assert.ok(Array.isArray(cases) && cases.length > 0 && cases.length <= 100_000, 'case bound');
  for (const test of cases)
    assert.ok(
      typeof test.binary === 'string' &&
        test.binary.length > 0 &&
        test.binary.length < 1024 &&
        typeof test.name === 'string' &&
        test.name.length > 0 &&
        test.name.length < 8192 &&
        typeof test.ignored === 'boolean',
      'invalid case identity'
    );
  assert.equal(new Set(cases.map(key)).size, cases.length, 'duplicate case identity');
  assert.ok(
    cases.some((test) => !test.ignored),
    'no active cases'
  );
  return cases;
}

/** Slice the independent active inventory, rather than trusting a matching count. */
export function partitionCases(cases, index, total) {
  admittedCases(cases);
  assert.ok(Number.isInteger(total) && total >= 2 && total <= 3, 'partition count');
  assert.ok(Number.isInteger(index) && index >= 1 && index <= total, 'partition index');
  return cases
    .filter((test) => !test.ignored)
    .sort((a, b) =>
      a.binary < b.binary
        ? -1
        : a.binary > b.binary
          ? 1
          : a.name < b.name
            ? -1
            : a.name > b.name
              ? 1
              : 0
    )
    .filter((_, position) => position % total === index - 1);
}

/** Prove the pinned nextest slice selects exactly the pre-registered identities. */
export function admitPartition(cases, inventory, index, total) {
  const all = new Map(admittedCases(cases).map((test) => [key(test), test]));
  const selected = [];
  for (const suite of Object.values(inventory['rust-suites'])) {
    for (const [name, test] of Object.entries(suite.testcases)) {
      const identity = { binary: suite['binary-id'], name };
      const original = all.get(key(identity));
      assert.ok(original, 'partition contains unknown case');
      all.delete(key(identity));
      assert.equal(test.ignored, original.ignored, 'partition ignored set differs');
      const match = test['filter-match'];
      if (match.status === 'matches') {
        assert.ok(!test.ignored, 'partition selected ignored case');
        selected.push(original);
      } else
        assert.ok(
          match.status === 'mismatch' &&
            (match.reason === 'partition' || (test.ignored && match.reason === 'ignored')),
          'unexpected partition filter'
        );
    }
  }
  assert.equal(all.size, 0, 'partition omitted inventory case');
  const expected = partitionCases(cases, index, total);
  assert.deepEqual(selected.map(key).sort(), expected.map(key).sort(), 'slice assignment differs');
  return expected;
}

/** Admit bytes and producer identity before nextest can decode or execute anything. */
export function admitArchive(manifest, bytes, expectedIdentity) {
  assert.equal(manifest.schema, 1, 'manifest schema');
  assert.deepEqual(manifest.identity, expectedIdentity, 'consumer identity differs');
  assert.ok(bytes.length > 0 && bytes.length <= 1024 ** 3, 'archive byte bound');
  assert.equal(bytes.length, manifest.archive.bytes, 'archive size differs');
  assert.equal(sha256(bytes), manifest.archive.sha256, 'archive hash differs');
  partitionCases(manifest.cases, 1, manifest.partitions);
  return manifest;
}

/** Missing/skipped/failed/foreign consumers can never satisfy the caller's original gate. */
export function aggregatePartitions(manifest, results, states) {
  assert.equal(manifest.schema, 1, 'manifest schema');
  assert.deepEqual(
    states,
    { producer: 'success', consumers: 'success' },
    'workspace worker failed or skipped'
  );
  const cases = admittedCases(manifest.cases);
  partitionCases(cases, 1, manifest.partitions);
  assert.equal(results.length, manifest.partitions, 'missing/extra partition result');
  const seen = new Set();
  const union = [];
  for (const result of results) {
    assert.equal(result.schema, 1, 'result schema');
    assert.deepEqual(result.identity, manifest.identity, 'foreign partition identity');
    assert.deepEqual(result.archive, manifest.archive, 'foreign partition archive');
    assert.equal(result.partitions, manifest.partitions, 'result partition count');
    assert.ok(!seen.has(result.index), 'duplicate partition result');
    seen.add(result.index);
    assert.equal(result.exit, 0, 'partition execution failed');
    assert.equal(result.worldRemoved, true, 'partition world retained');
    const expected = partitionCases(cases, result.index, manifest.partitions);
    assert.deepEqual(result.cases, expected, 'partition result identities differ');
    verifyExecution(expected, result.log);
    union.push(...result.cases);
  }
  assert.equal(new Set(union.map(key)).size, union.length, 'overlapping partition cases');
  assert.deepEqual(
    union.map(key).sort(),
    cases
      .filter((test) => !test.ignored)
      .map(key)
      .sort(),
    'partition union differs'
  );
  return { passed: union.length, ignored: cases.length - union.length, total: cases.length };
}

function readJSON(file) {
  const stat = lstatSync(file);
  assert.ok(
    stat.isFile() && !stat.isSymbolicLink() && stat.size <= 64 * 1024 ** 2,
    'JSON file bound/type'
  );
  return JSON.parse(readFileSync(file, 'utf8'));
}
function writeJSON(file, data) {
  writeFileSync(file, JSON.stringify(data) + '\n');
}

function runIdentity() {
  const source = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  assert.equal(source.status, 0, 'source identity unavailable');
  assert.equal(source.stdout.trim(), process.env.GITHUB_SHA, 'checkout differs from event');
  assert.match(process.env.GITHUB_RUN_ID ?? '', /^[1-9][0-9]{0,15}$/);
  assert.match(process.env.GITHUB_RUN_ATTEMPT ?? '', /^[1-9][0-9]{0,5}$/);
  assert.equal(process.platform, 'linux', 'consumer platform');
  assert.equal(process.arch, 'x64', 'consumer architecture');
  assert.equal(process.env.CARGO_PROFILE_DEV_DEBUG, '0', 'dev debug environment');
  assert.equal(process.env.CARGO_INCREMENTAL, '0', 'incremental environment');
  return {
    source: source.stdout.trim(),
    run: process.env.GITHUB_RUN_ID,
    attempt: process.env.GITHUB_RUN_ATTEMPT,
    toolchain: TOOLCHAIN,
    nextest: NEXTEST_VERSION,
    platform: 'linux-x64',
    debug: '0',
    incremental: '0',
    workspace: realpathSync('.'),
  };
}

/** The native metadata tests use both runtime CARGO and a producer-compiled Cargo path. */
export function consumerEnvironment(cargo, base = process.env) {
  assert.ok(
    typeof cargo === 'string' && path.isAbsolute(cargo),
    'Cargo executable must be absolute'
  );
  return { ...base, CARGO: cargo };
}

/** Collectors validate run provenance; only actual execution machines supply compiler evidence. */
export function admitMachineIdentity(identity, expectedRun) {
  const { cargo, cargoVersion, rustcVersion, ...run } = identity;
  assert.deepEqual(run, expectedRun, 'run identity differs');
  consumerEnvironment(cargo, {});
  assert.ok(
    typeof cargoVersion === 'string' && cargoVersion.startsWith(`cargo ${TOOLCHAIN} `),
    'Cargo version differs'
  );
  assert.ok(
    typeof rustcVersion === 'string' && rustcVersion.startsWith(`rustc ${TOOLCHAIN} `),
    'rustc version differs'
  );
  return identity;
}

function executionIdentity() {
  const read = (program, args) => {
    const result = spawnSync(program, args, { encoding: 'utf8', timeout: 30_000, maxBuffer: 4096 });
    if (result.error) throw result.error;
    assert.equal(result.status, 0, `${program} ${args.join(' ')} identity failed`);
    return result.stdout.trim();
  };
  const cargo = realpathSync(read('rustup', ['which', 'cargo']));
  const context = runIdentity();
  return admitMachineIdentity(
    {
      ...context,
      cargo,
      cargoVersion: read(cargo, ['--version']),
      rustcVersion: read(path.join(path.dirname(cargo), 'rustc'), ['--version']),
    },
    context
  );
}

/** Every effect is bounded and its timing/result survives a failing phase. */
function command(directory, phase, program, args, isolated = false, base = process.env) {
  const started = new Date().toISOString();
  const clock = performance.now();
  let result;
  const diagnostics = [];
  const spawn = (executable, argv, options = {}) => {
    result = spawnSync(executable, argv, {
      env: options.env ?? base,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 ** 2,
      timeout: 18 * 60_000,
    });
    return result;
  };
  const world = isolated
    ? runInPrivateWorld({
        argv: [program, ...args],
        base,
        spawn,
        log: (line) => diagnostics.push(line),
      })
    : undefined;
  if (!world) spawn(program, args);
  else result.status = world.status;
  const worldRemoved = world ? !existsSync(world.world.root) : undefined;
  const seconds = (performance.now() - clock) / 1000;
  writeFileSync(
    path.join(directory, `${phase}.log`),
    `${result.stdout ?? ''}${result.stderr ?? ''}${diagnostics.join('\n')}`
  );
  const file = path.join(directory, 'phases.json');
  const phases = existsSync(file) ? readJSON(file) : [];
  phases.push({
    phase,
    started,
    completed: new Date().toISOString(),
    seconds,
    exit: result.status,
    signal: result.signal,
    world: world?.world.root,
    worldRemoved,
    leaks: world?.leaks,
  });
  writeJSON(file, phases);
  process.stdout.write(result.stdout ?? '');
  process.stderr.write((result.stderr ?? '') + diagnostics.join('\n'));
  if (world) assert.equal(worldRemoved, true, 'private world retained');
  console.log(`#3284 phase=${phase} seconds=${seconds.toFixed(3)} exit=${result.status}`);
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${program} ${args.join(' ')} failed`);
  return result.stdout;
}

function inventory(directory, total) {
  const run = (phase, program, args) => command(directory, phase, program, args);
  const build = run('compile-inventory', 'cargo', [
    'test',
    '--locked',
    '--workspace',
    '--no-run',
    '--message-format=json',
  ]);
  const executables = build
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line))
    .filter((item) => item.reason === 'compiler-artifact' && item.profile.test && item.executable)
    .map((item) => item.executable);
  assert.ok(executables.length > 0 && executables.length <= 1000, 'Cargo executable bound');
  const cargo = executables.map((executable, index) => ({
    executable,
    tests: libtestCases(run(`libtest-${index}`, executable, ['--list', '--format', 'terse'])),
    ignored: libtestCases(
      run(`libtest-ignored-${index}`, executable, ['--ignored', '--list', '--format', 'terse'])
    ),
  }));
  writeJSON(path.join(directory, 'cargo-inventory.json'), cargo);
  const nextest = JSON.parse(
    run('nextest-inventory', 'cargo-nextest', [
      'nextest',
      'list',
      '--locked',
      '--workspace',
      '--message-format',
      'json',
    ])
  );
  writeJSON(path.join(directory, 'nextest-inventory.json'), nextest);
  const cases = compareInventories(cargo, nextest);
  admittedCases(cases);
  writeJSON(path.join(directory, 'cases.json'), cases);
  for (let index = 1; index <= total; index++) {
    const selected = JSON.parse(
      run(`partition-list-${index}`, 'cargo-nextest', [
        'nextest',
        'list',
        '--locked',
        '--workspace',
        '--partition',
        `slice:${index}/${total}`,
        '--message-format',
        'json',
      ])
    );
    admitPartition(cases, selected, index, total);
  }
  // The archive carries these integration executables as well as test binaries.
  const binaries = Object.values(nextest['rust-build-meta']['non-test-binaries']).flat();
  for (const binary of binaries) {
    assert.equal(binary.kind, 'bin-exe', 'unknown non-test binary kind');
    const file = path.resolve('target', binary.path);
    assert.ok(file.startsWith(path.resolve('target') + path.sep), 'non-test binary escape');
    assert.ok(lstatSync(file).isFile(), 'missing runtime executable');
  }
  console.log(`Cargo/nextest inventory equal: ${cases.length} cases in ${cargo.length} binaries`);
}

function archive(directory, total) {
  const archive = path.join(directory, 'workspace.tar.zst');
  command(directory, 'archive', 'cargo-nextest', [
    'nextest',
    'archive',
    '--locked',
    '--workspace',
    '--archive-file',
    archive,
  ]);
  const stat = lstatSync(archive);
  assert.ok(stat.isFile() && stat.size > 0 && stat.size <= 1024 ** 3, 'archive byte bound');
  const cases = admittedCases(readJSON(path.join(directory, 'cases.json')));
  partitionCases(cases, 1, total);
  writeJSON(path.join(directory, 'manifest.json'), {
    schema: 1,
    identity: executionIdentity(),
    partitions: total,
    cases,
    archive: { bytes: stat.size, sha256: sha256(readFileSync(archive)) },
  });
}

function consume(directory, index) {
  const manifest = readJSON(path.join(directory, 'manifest.json'));
  const machine = executionIdentity();
  assert.deepEqual(manifest.identity, machine, 'consumer identity differs');
  const expected = partitionCases(manifest.cases, index, manifest.partitions);
  const archive = path.join(directory, 'workspace.tar.zst');
  const stat = lstatSync(archive);
  assert.ok(
    stat.isFile() && !stat.isSymbolicLink() && stat.size <= 1024 ** 3,
    'archive byte bound/type'
  );
  admitArchive(manifest, readFileSync(archive), machine);
  assert.ok(!existsSync('target'), 'consumer target must start absent');
  // nextest owns decoding; extraction into this existing workspace preserves compiled absolute paths.
  const inventory = JSON.parse(
    command(directory, 'downloaded-extract-list', 'cargo-nextest', [
      'nextest',
      'list',
      '--archive-file',
      archive,
      '--extract-to',
      realpathSync('.'),
      '--message-format',
      'json',
      '--partition',
      `slice:${index}/${manifest.partitions}`,
    ])
  );
  assert.equal(
    inventory['rust-build-meta']['target-directory'],
    path.resolve('target'),
    'extracted target differs'
  );
  admitPartition(manifest.cases, inventory, index, manifest.partitions);
  for (const suite of Object.values(inventory['rust-suites']))
    assert.ok(lstatSync(suite['binary-path']).isFile(), 'missing extracted test executable');
  for (const binary of Object.values(inventory['rust-build-meta']['non-test-binaries']).flat())
    assert.ok(
      lstatSync(path.join('target', binary.path)).isFile(),
      'missing extracted runtime executable'
    );
  const metadata = [
    '--binaries-metadata',
    path.resolve('target/nextest/binaries-metadata.json'),
    '--cargo-metadata',
    path.resolve('target/nextest/cargo-metadata.json'),
  ];
  command(
    directory,
    'nextest-execution',
    'cargo-nextest',
    [
      'nextest',
      'run',
      ...metadata,
      '--no-fail-fast',
      '--test-threads',
      '4',
      '--partition',
      `slice:${index}/${manifest.partitions}`,
    ],
    true,
    consumerEnvironment(machine.cargo)
  );
  const phases = readJSON(path.join(directory, 'phases.json'));
  const worldRemoved = phases.at(-1).worldRemoved;
  assert.equal(worldRemoved, true, 'partition world retained');
  const log = readFileSync(path.join(directory, 'nextest-execution.log'), 'utf8');
  verifyExecution(expected, log);
  writeJSON(path.join(directory, 'result.json'), {
    schema: 1,
    identity: manifest.identity,
    archive: manifest.archive,
    index,
    partitions: manifest.partitions,
    exit: 0,
    worldRemoved,
    cases: expected,
    log,
  });
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [mode, location, value] = process.argv.slice(2);
  assert.ok(
    location,
    'usage: workspace-nextest.mjs inventory|archive|consume|aggregate <directory> [partition]'
  );
  const directory = path.resolve(location);
  mkdirSync(directory, { recursive: true });
  if (mode === 'inventory') inventory(directory, Number(value));
  else if (mode === 'archive') archive(directory, Number(value));
  else if (mode === 'consume') consume(directory, Number(value));
  else if (mode === 'phase') {
    assert.match(value ?? '', /^[a-z][a-z0-9-]{0,63}$/);
    const [program, ...args] = process.argv.slice(5);
    assert.ok(program, 'phase program missing');
    command(directory, value, program, args, true);
  } else if (mode === 'aggregate') {
    const manifest = readJSON(path.join(directory, 'manifest.json'));
    admitMachineIdentity(manifest.identity, runIdentity());
    const states = {
      producer: process.env.PRODUCER_RESULT,
      consumers: process.env.CONSUMERS_RESULT,
    };
    const names = Array.from(
      { length: manifest.partitions },
      (_, index) =>
        `workspace-result-${index + 1}-${manifest.identity.run}-${manifest.identity.attempt}`
    );
    assert.deepEqual(
      readdirSync(directory)
        .filter((name) => name.startsWith('workspace-result-'))
        .sort(),
      [...names].sort(),
      'missing/extra partition artifact'
    );
    const results = names.map((name) => readJSON(path.join(directory, name, 'result.json')));
    const summary = aggregatePartitions(manifest, results, states);
    writeJSON(path.join(directory, 'summary.json'), summary);
    console.log(JSON.stringify(summary));
  } else assert.fail('unknown workspace-nextest mode');
}
