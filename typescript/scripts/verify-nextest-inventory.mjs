import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { runPackedCommand } from './packed-command.mjs';

const identity = (parts) => JSON.stringify(parts);

/** Cargo's libtest listing is authoritative for names and the ignored flag. */
export function parseLibtestList(text) {
  const names = text
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      assert.ok(line.endsWith(': test'), `Unexpected libtest listing: ${line}`);
      return line.slice(0, -6);
    });
  assert.equal(new Set(names).size, names.length, 'Duplicate libtest name');
  return names.sort();
}

export function cargoInventory(artifacts, list) {
  const suites = [];
  for (const artifact of artifacts) {
    if (artifact.reason !== 'compiler-artifact' || !artifact.profile.test || !artifact.executable)
      continue;
    assert.equal(artifact.target.kind.length, 1, 'Ambiguous Cargo target kind');
    const names = parseLibtestList(list(artifact.executable, false));
    const ignored = new Set(parseLibtestList(list(artifact.executable, true)));
    assert.ok(
      [...ignored].every((name) => names.includes(name)),
      'Unknown ignored Cargo test'
    );
    suites.push({
      id: [artifact.package_id, artifact.target.kind[0], artifact.target.name],
      tests: names.map((name) => ({ name, ignored: ignored.has(name) })),
    });
  }
  assert.ok(suites.length > 0, 'Cargo selected no test harnesses');
  return suites;
}

function nextestInventory(document) {
  const suites = Object.values(document['rust-suites']).map((suite) => {
    assert.equal(suite.status, 'listed', 'Nextest skipped a test harness');
    return {
      id: [suite['package-id'], suite.kind, suite['binary-name']],
      tests: Object.entries(suite.testcases).map(([name, test]) => {
        assert.equal(test.kind, 'test', 'Unsupported nextest testcase kind');
        assert.equal(typeof test.ignored, 'boolean', 'Missing nextest ignored flag');
        const filter = test['filter-match'];
        assert.ok(['matches', 'mismatch'].includes(filter.status), 'Missing nextest filter status');
        return { name, ignored: test.ignored, selected: filter.status === 'matches', filter };
      }),
    };
  });
  assert.equal(
    suites.reduce((sum, suite) => sum + suite.tests.length, 0),
    document['test-count']
  );
  return suites;
}

function index(suites) {
  const harnesses = new Set();
  const tests = new Map();
  for (const suite of suites) {
    const harness = identity(suite.id);
    assert.ok(!harnesses.has(harness), `Duplicate harness: ${harness}`);
    harnesses.add(harness);
    for (const test of suite.tests) {
      const key = identity([...suite.id, test.name]);
      assert.ok(!tests.has(key), `Duplicate test: ${key}`);
      tests.set(key, test);
    }
  }
  return { harnesses: [...harnesses].sort(), tests };
}

/** Compare every harness/name/ignored flag, then prove a disjoint selected union. */
export function verifyNextestInventory(cargo, full, partitions) {
  assert.equal(partitions.length, 2, 'Expected exactly two partitions');
  const expected = index(cargo);
  assert.ok(expected.tests.size > 0, 'Cargo selected no tests');
  const all = index(nextestInventory(full));
  const compare = (actual) => {
    assert.deepEqual(actual.harnesses, expected.harnesses, 'Harness selection changed');
    assert.deepEqual(
      [...actual.tests.keys()].sort(),
      [...expected.tests.keys()].sort(),
      'Test selection changed'
    );
    for (const [key, test] of expected.tests)
      assert.equal(actual.tests.get(key).ignored, test.ignored, `Ignored policy changed: ${key}`);
  };
  compare(all);
  for (const [key, test] of all.tests) {
    assert.equal(test.selected, !test.ignored, `Default selection changed: ${key}`);
    if (test.ignored) assert.equal(test.filter.reason, 'ignored', `Unexpected exclusion: ${key}`);
  }
  const selected = new Set();
  const partitionCounts = [];
  for (const partition of partitions) {
    const shard = index(nextestInventory(partition));
    compare(shard);
    let count = 0;
    for (const [key, test] of shard.tests) {
      if (!test.selected) continue;
      assert.ok(!test.ignored, `Ignored test selected: ${key}`);
      assert.ok(!selected.has(key), `Partition overlap: ${key}`);
      selected.add(key);
      count++;
    }
    assert.ok(count > 0, 'Empty test partition');
    partitionCounts.push(count);
  }
  const runnable = [...expected.tests]
    .filter(([, test]) => !test.ignored)
    .map(([key]) => key)
    .sort();
  assert.deepEqual([...selected].sort(), runnable, 'Partition union changed');
  return {
    harnesses: expected.harnesses,
    tests: [...expected.tests].map(([id, test]) => ({ id: JSON.parse(id), ignored: test.ignored })),
    runnable: runnable.length,
    ignored: expected.tests.size - runnable.length,
    partitionCounts,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [artifactsPath, fullPath, firstPath, secondPath, outputPath] = process.argv.slice(2);
  assert.ok(outputPath, 'Expected Cargo messages, nextest full/shard1/shard2 JSON and output path');
  const artifacts = readFileSync(artifactsPath, 'utf8')
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line));
  const cargo = cargoInventory(artifacts, (binary, ignored) =>
    runPackedCommand(binary, [...(ignored ? ['--ignored'] : []), '--list', '--format', 'terse'], {
      cwd: process.cwd(),
      env: process.env,
    })
  );
  const read = (file) => JSON.parse(readFileSync(file, 'utf8'));
  const proof = verifyNextestInventory(cargo, read(fullPath), [read(firstPath), read(secondPath)]);
  writeFileSync(outputPath, `${JSON.stringify(proof, null, 2)}\n`);
  console.log(
    `Exact Cargo/nextest parity: ${proof.runnable} runnable, ${proof.ignored} ignored, ${proof.harnesses.length} harnesses; partitions ${proof.partitionCounts.join(' + ')}`
  );
}
