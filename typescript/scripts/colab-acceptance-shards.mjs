import { readFileSync, readdirSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const ACCEPTANCE_DIRECTORY = new URL(
  '../../extensions/tmt-colab/typescript/app/acceptance/',
  import.meta.url
);
const WEIGHTS = new URL('shard-weights.json', ACCEPTANCE_DIRECTORY);

/** Shards of the real-binary acceptance run. The workflow declares one job per shard, so this is fixed. */
export const ACCEPTANCE_SHARDS = 2;

export function loadShardWeights(text = readFileSync(WEIGHTS, 'utf8')) {
  const weights = JSON.parse(text);
  const positive = (value) => Number.isFinite(value) && value > 0;
  if (
    !positive(weights.defaultSeconds) ||
    typeof weights.files !== 'object' ||
    weights.files === null ||
    !Object.values(weights.files).every(positive)
  ) {
    throw new Error('The acceptance shard weights must be positive numbers.');
  }
  return weights;
}

/** The acceptance spec file names, sorted. Playwright runs only files directly in this directory. */
export function listAcceptanceFiles(directory = ACCEPTANCE_DIRECTORY) {
  return readdirSync(directory)
    .filter((name) => /^[a-z0-9-]+\.spec\.ts$/.test(name))
    .sort();
}

/**
 * Splits whole spec files over `count` shards, largest first onto the currently lightest shard
 * (ties go to the lowest index), so the result is deterministic. Playwright's own `--shard` cuts
 * the alphabetical list by test count, which put the heavy `page-*` specs together in one shard
 * (11 and 21 minutes for two shards, #2827). Each shard's files are sorted by name. Every file
 * lands in exactly one shard, whatever the weights.
 */
export function splitAcceptanceFiles(files, count, weights = loadShardWeights()) {
  if (!Number.isInteger(count) || count < 1)
    throw new Error('The shard count must be a positive integer.');
  const seconds = (file) => weights.files[file] ?? weights.defaultSeconds;
  const shards = Array.from({ length: count }, () => ({ load: 0, files: [] }));
  const ordered = [...files].sort((a, b) => seconds(b) - seconds(a) || (a < b ? -1 : 1));
  for (const file of ordered) {
    const lightest = shards.reduce((best, shard) => (shard.load < best.load ? shard : best));
    lightest.files.push(file);
    lightest.load += seconds(file);
  }
  return shards.map((shard) => ({ files: shard.files.sort(), seconds: shard.load }));
}

/** The Playwright file filters (paths under the app directory) of shard `index` (1-based) of `count`. */
export function acceptanceShardFiles(index, count, files = listAcceptanceFiles()) {
  if (!Number.isInteger(index) || index < 1 || index > count)
    throw new Error('The shard index must be between 1 and the shard count.');
  return splitAcceptanceFiles(files, count)
    .at(index - 1)
    .files.map((file) => `acceptance/${file}`);
}

// `node scripts/colab-acceptance-shards.mjs INDEX COUNT` prints one filter per line for the workflow.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [index, count] = process.argv.slice(2).map(Number);
  process.stdout.write(acceptanceShardFiles(index, count).join('\n') + '\n');
}
