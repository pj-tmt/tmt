import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const playwright = fileURLToPath(new URL('../node_modules/.bin/playwright', import.meta.url));

function listedTests(args) {
  const result = spawnSync(playwright, ['test', '--list', '--reporter=json', ...args], {
    cwd: root,
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
  });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  const report = JSON.parse(result.stdout);
  assert.deepEqual(report.errors, []);
  const identities = [];
  function visit(suites) {
    for (const suite of suites ?? []) {
      for (const spec of suite.specs ?? []) {
        identities.push(`${spec.file}:${spec.line}:${spec.column}:${spec.title}:${spec.id}`);
      }
      visit(suite.suites);
    }
  }
  visit(report.suites);
  return identities.sort();
}

const complete = listedTests([]);
const partitions = new Map([
  ['emulator', listedTests(['--config', 'playwright.emulator.config.ts'])],
  ['local 1/3', listedTests(['--config', 'playwright.local.config.ts', '--shard', '1/3'])],
  ['local 2/3', listedTests(['--config', 'playwright.local.config.ts', '--shard', '2/3'])],
  ['local 3/3', listedTests(['--config', 'playwright.local.config.ts', '--shard', '3/3'])],
  ...Array.from({ length: 8 }, (_, index) => {
    const shard = `${index + 1}/8`;
    return [
      `native ${shard}`,
      listedTests(['--config', 'playwright.native.config.ts', '--shard', shard]),
    ];
  }),
]);
const capacity = listedTests(['--config', 'playwright.capacity.config.ts']);
const visual = listedTests(['--config', 'playwright.visual.config.ts']);

assert.equal(complete.length, 130, 'Update the reviewed standard browser inventory.');
const partitioned = [...partitions.values()].flat();
assert.equal(new Set(partitioned).size, partitioned.length, 'Browser partitions overlap.');
assert.deepEqual(partitioned.sort(), complete, 'Browser partitions omit or add listed tests.');
assert.equal(capacity.length, 4, 'Update the reviewed opt-in capacity inventory.');
assert.equal(new Set(capacity).size, capacity.length, 'Capacity identities overlap.');
assert.equal(
  capacity.filter((identity) => complete.includes(identity)).length,
  0,
  'Standard and opt-in browser identities overlap.'
);
assert.deepEqual(
  [...complete, ...capacity].sort(),
  listedTests(['--config', 'playwright.all.config.ts']),
  'Required and opt-in inventories must retain full browser coverage.'
);
assert.equal(visual.length, 3, 'Update the reviewed opt-in visual inventory.');
assert.equal(new Set(visual).size, visual.length, 'Visual identities overlap.');
assert.equal(
  visual.filter((identity) => [...complete, ...capacity].includes(identity)).length,
  0,
  'Opt-in pixel baselines must not enter the standard or capacity suites.'
);

const decoration = [...partitions.entries()]
  .filter(([, tests]) =>
    tests.some((identity) => identity.startsWith('native-decoration.spec.ts:'))
  )
  .map(([name]) => name);
assert.deepEqual(decoration, ['emulator'], 'Native decoration must stay emulator-backed.');

for (const [name, tests] of partitions) process.stdout.write(`${name}: ${tests.length}\n`);
process.stdout.write(`complete: ${complete.length}\n`);
process.stdout.write(`capacity opt-in: ${capacity.length}\n`);
process.stdout.write(`visual opt-in: ${visual.length}\n`);
