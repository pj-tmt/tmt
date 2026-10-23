import { readFileSync, readdirSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const ACCEPTANCE_DIRECTORY = new URL(
  '../../extensions/tmt-colab/typescript/app/acceptance/',
  import.meta.url
);
const WEIGHTS = new URL('shard-weights.json', ACCEPTANCE_DIRECTORY);
const ACCEPTANCE_PATTERN = '^[a-z0-9-]+\\.spec\\.ts$';
const ACCEPTANCE = {
  directory: ACCEPTANCE_DIRECTORY,
  weightsFile: WEIGHTS,
  specPattern: ACCEPTANCE_PATTERN,
  prefix: 'acceptance',
};
const COMPONENT_DIRECTORY = new URL(
  '../../extensions/tmt-colab/typescript/app/e2e/',
  import.meta.url
);
const COMPONENT = {
  directory: COMPONENT_DIRECTORY,
  weightsFile: new URL('shard-weights.json', COMPONENT_DIRECTORY),
  specPattern: ACCEPTANCE_PATTERN,
  prefix: 'e2e',
};

/** Shards of the real-binary acceptance run. The workflow declares one job per shard, so this is fixed. */
export const ACCEPTANCE_SHARDS = 2;
export const COMPONENT_SHARDS = 7;

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

/** Whole files directly in a suite directory, sorted; specPattern is a regex source without flags. */
export function listSpecFiles(directory, specPattern) {
  const pattern = new RegExp(specPattern);
  return readdirSync(directory)
    .filter((name) => pattern.test(name))
    .sort();
}

/** The acceptance spec file names, sorted. Playwright runs only files directly in this directory. */
export function listAcceptanceFiles(directory = ACCEPTANCE_DIRECTORY) {
  return listSpecFiles(directory, ACCEPTANCE_PATTERN);
}

/**
 * Splits whole spec files over `count` shards, largest first onto the currently lightest shard
 * (ties go to the lowest index), so the result is deterministic. Playwright's own `--shard` cuts
 * the alphabetical list by test count, which put the heavy `page-*` specs together in one shard
 * (11 and 21 minutes for two shards, #2827). Each shard's files are sorted by name. Every file
 * lands in exactly one shard, whatever the weights.
 */
export function splitSpecFiles(files, count, weights) {
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

export function splitAcceptanceFiles(files, count, weights = loadShardWeights()) {
  return splitSpecFiles(files, count, weights);
}

/** Playwright filters under the app directory, using the caller's discovery and measured weights. */
export function specShardFiles(
  index,
  count,
  suite,
  files = listSpecFiles(suite.directory, suite.specPattern)
) {
  if (!Number.isInteger(index) || index < 1 || index > count)
    throw new Error('The shard index must be between 1 and the shard count.');
  if (
    !/^[a-z0-9-]+$/.test(suite.prefix) ||
    !files.every((file) => /^[a-z0-9-]+\.(?:spec|test)\.[cm]?[jt]sx?$/.test(file))
  )
    throw new Error('Shard filters need plain suite and spec file names.');
  return splitSpecFiles(files, count, loadShardWeights(readFileSync(suite.weightsFile, 'utf8')))
    .at(index - 1)
    .files.map((file) => `${suite.prefix}/${file}`);
}

/** The Playwright file filters (paths under the app directory) of shard `index` (1-based) of `count`. */
export function acceptanceShardFiles(index, count, files = listAcceptanceFiles()) {
  return specShardFiles(index, count, ACCEPTANCE, files);
}

export function listComponentFiles() {
  return listSpecFiles(COMPONENT.directory, COMPONENT.specPattern);
}

export function loadComponentWeights(text = readFileSync(COMPONENT.weightsFile, 'utf8')) {
  const weights = loadShardWeights(text);
  if (!Number.isFinite(weights.jobOverheadSeconds) || weights.jobOverheadSeconds < 0)
    throw new Error('Component job overhead must be a nonnegative number.');
  if (!Number.isFinite(weights.modelErrorSeconds) || weights.modelErrorSeconds < 0)
    throw new Error('Component model error must be a nonnegative number.');
  return weights;
}

export function componentShardFiles(index, count, files = listComponentFiles()) {
  return specShardFiles(index, count, COMPONENT, files);
}

// The optional third argument selects component specs; the existing acceptance command is unchanged.
// Only plain suite/spec names reach the filters, so splitting this line on spaces is exact.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [index, count, suite] = process.argv.slice(2);
  if (suite !== undefined && suite !== 'component')
    throw new Error('The suite must be component or omitted for acceptance.');
  const shardFiles = suite === 'component' ? componentShardFiles : acceptanceShardFiles;
  process.stdout.write(shardFiles(Number(index), Number(count)).join(' ') + '\n');
}
