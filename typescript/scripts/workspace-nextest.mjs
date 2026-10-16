import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync, realpathSync } from 'node:fs';
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
      assert.equal(suite.testcases[name]['filter-match'].status, 'matches', 'filtered test');
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

function command(program, args) {
  const result = spawnSync(program, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${program} ${args.join(' ')} failed: ${result.stderr}`);
  return result.stdout;
}

function inventory(directory) {
  mkdirSync(directory, { recursive: true });
  const build = command('cargo', [
    'test',
    '--locked',
    '--workspace',
    '--no-run',
    '--message-format=json',
  ]);
  writeFileSync(path.join(directory, 'cargo-build.jsonl'), build);
  const executables = build
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line))
    .filter((item) => item.reason === 'compiler-artifact' && item.profile.test && item.executable)
    .map((item) => item.executable);
  const cargo = executables.map((executable) => ({
    executable,
    tests: libtestCases(command(executable, ['--list', '--format', 'terse'])),
    ignored: libtestCases(command(executable, ['--ignored', '--list', '--format', 'terse'])),
  }));
  const nextest = command('cargo', [
    'nextest',
    'list',
    '--locked',
    '--workspace',
    '--message-format',
    'json',
  ]);
  writeFileSync(path.join(directory, 'cargo-inventory.json'), JSON.stringify(cargo));
  writeFileSync(path.join(directory, 'nextest-inventory.json'), nextest);
  const cases = compareInventories(cargo, JSON.parse(nextest));
  writeFileSync(path.join(directory, 'cases.json'), JSON.stringify(cases));
  console.log(`Cargo/nextest inventory equal: ${cases.length} cases in ${cargo.length} binaries`);
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [mode, directory] = process.argv.slice(2);
  assert.ok(
    directory && ['inventory', 'verify'].includes(mode),
    'usage: workspace-nextest.mjs inventory|verify <evidence-directory>'
  );
  if (mode === 'inventory') inventory(directory);
  else {
    const result = verifyExecution(
      JSON.parse(readFileSync(path.join(directory, 'cases.json'), 'utf8')),
      readFileSync(path.join(directory, 'nextest.log'), 'utf8')
    );
    writeFileSync(path.join(directory, 'result.json'), JSON.stringify(result));
    console.log(JSON.stringify(result));
  }
}
