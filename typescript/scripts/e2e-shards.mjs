import { readFileSync, readdirSync } from 'node:fs';

const E2E_DIRECTORY = new URL('../test/e2e/', import.meta.url);
const WEIGHTS = new URL('../test/e2e/shard-weights.json', import.meta.url);

/** Shards of the full Docker E2E run. The workflow declares one job per shard, so this is fixed. */
export const FULL_E2E_SHARDS = 2;

export function loadShardWeights(text = readFileSync(WEIGHTS, 'utf8')) {
  const weights = JSON.parse(text);
  const positive = (value) => Number.isFinite(value) && value > 0;
  if (
    !positive(weights.adapterTestsSeconds) ||
    !positive(weights.defaultSeconds) ||
    typeof weights.files !== 'object' ||
    weights.files === null ||
    !Object.values(weights.files).every(positive)
  ) {
    throw new Error('The E2E shard weights must be positive numbers.');
  }
  return weights;
}

/** The E2E scenario file names, sorted. */
export function listE2eFiles(directory = E2E_DIRECTORY) {
  return readdirSync(directory)
    .filter((name) => name.endsWith('.e2e.test.ts'))
    .sort();
}

/**
 * Splits whole files over `count` shards, largest first onto the currently lightest
 * shard (ties go to the lowest index), so the result is deterministic. Shard 1 also
 * runs the Rust adapter tests, which count as its starting load. Each shard's files
 * are sorted by name. Every file lands in exactly one shard, whatever the weights.
 */
export function splitE2eFiles(files, count, weights = loadShardWeights()) {
  if (!Number.isInteger(count) || count < 1)
    throw new Error('The shard count must be a positive integer.');
  const seconds = (file) => weights.files[file] ?? weights.defaultSeconds;
  const shards = Array.from({ length: count }, (_, index) => ({
    load: index === 0 ? weights.adapterTestsSeconds : 0,
    files: [],
  }));
  const ordered = [...files].sort((a, b) => seconds(b) - seconds(a) || (a < b ? -1 : 1));
  for (const file of ordered) {
    const lightest = shards.reduce((best, shard) => (shard.load < best.load ? shard : best));
    lightest.files.push(file);
    lightest.load += seconds(file);
  }
  return shards.map((shard) => ({ files: shard.files.sort(), seconds: shard.load }));
}

/**
 * The E2E files each of the two Docker E2E jobs runs for a native scope. `full`
 * splits every scenario file; a scoped component runs only its own file list in the
 * first shard (the second is skipped); `none` runs nothing.
 */
export function e2eShardFiles(scope, scopedFiles, files = listE2eFiles()) {
  if (scope === 'full') {
    const [first, second] = splitE2eFiles(files, FULL_E2E_SHARDS);
    return [first.files, second.files];
  }
  if (scope === 'none' || scope === '') return [[], []];
  return [[...scopedFiles], []];
}
