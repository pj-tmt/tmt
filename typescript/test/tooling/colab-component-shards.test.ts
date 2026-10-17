import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { expect, it } from 'vite-plus/test';
import {
  COMPONENT_SHARDS,
  componentShardFiles,
  listComponentFiles,
  loadComponentWeights,
  listSpecFiles,
  specShardFiles,
  splitSpecFiles,
} from '../../scripts/colab-acceptance-shards.mjs';

const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);

it('uses the caller directory, pattern, weights and prefix without an acceptance-suite fallback', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'colab-shards-'));
  try {
    for (const name of ['a.spec.ts', 'b.spec.ts', 'c.spec.ts', 'helper.ts'])
      writeFileSync(path.join(directory, name), '');
    const weights = { defaultSeconds: 2, files: { 'a.spec.ts': 9, 'b.spec.ts': 5 } };
    const weightsFile = path.join(directory, 'weights.json');
    writeFileSync(weightsFile, JSON.stringify(weights));
    const suite = {
      directory: pathToFileURL(`${directory}/`),
      weightsFile: pathToFileURL(weightsFile),
      specPattern: '^[a-z0-9-]+\\.spec\\.ts$',
      prefix: 'e2e',
    };
    const files = listSpecFiles(suite.directory, suite.specPattern);
    expect(files).toEqual(['a.spec.ts', 'b.spec.ts', 'c.spec.ts']);
    expect(splitSpecFiles(files, 2, weights)).toEqual([
      { files: ['a.spec.ts'], seconds: 9 },
      { files: ['b.spec.ts', 'c.spec.ts'], seconds: 7 },
    ]);
    const filters = [specShardFiles(1, 2, suite), specShardFiles(2, 2, suite)];
    expect(filters.flat().sort()).toEqual(files.map((file) => `e2e/${file}`));
    expect(() => specShardFiles(1, 2, { ...suite, prefix: '../e2e' })).toThrow(/plain/);
    expect(() => specShardFiles(1, 2, suite, ['../a.spec.ts'])).toThrow(/plain/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

it('runs the measured whole-file split without changing coverage or execution limits', () => {
  const workflow = readFileSync(
    new URL('../../../.github/workflows/colab-browser.yml', import.meta.url),
    'utf8'
  );
  const app = workflow.split('\n  colab-app:\n')[1].split('\n  colab-hosted:\n')[0];
  const matrix = app
    .match(/shard: \[([\d, ]+)\]/)?.[1]
    .split(',')
    .map(Number);
  expect(matrix).toEqual(Array.from({ length: COMPONENT_SHARDS }, (_, index) => index + 1));
  expect(app).toContain(
    `shard_files="$(node scripts/colab-acceptance-shards.mjs \${{ matrix.shard }} ${COMPONENT_SHARDS} component)"`
  );
  expect(app).toContain(
    `name: Colab app component specs (Chromium, shard \${{ matrix.shard }}/${COMPONENT_SHARDS})`
  );
  expect(app).not.toContain('--shard=');
  expect(app).toContain('read -r -a files <<< "$shard_files"');
  expect(app).toContain('if [ "${#files[@]}" -eq 0 ]; then');
  expect(app).toContain('test:browser "${files[@]}" --reporter=dot,json');
  expect(app).toContain(
    'PLAYWRIGHT_JSON_OUTPUT_FILE: ${{ runner.temp }}/colab-app-results/results.json'
  );
  expect(app).toContain('timeout-minutes: 20');
  const config = readFileSync(
    new URL('../../../extensions/tmt-colab/typescript/app/playwright.config.ts', import.meta.url),
    'utf8'
  );
  expect(config).toContain('workers: 1');
  expect(config).toContain('retries: 0');
  expect(config).toContain('timeout: 30000');
});

it('puts every component file in exactly one shard and retains unknown files with the default weight', () => {
  const files = listComponentFiles();
  const weights = loadComponentWeights();
  expect(files.length).toBeGreaterThan(50);
  for (const count of [1, 2, 3, COMPONENT_SHARDS, 7]) {
    const shards = splitSpecFiles(files, count, weights);
    expect(shards.flatMap((shard) => shard.files).sort()).toEqual(files);
    expect(splitSpecFiles([...files].reverse(), count, weights)).toEqual(shards);
    const filters = Array.from({ length: count }, (_, index) =>
      componentShardFiles(index + 1, count)
    );
    expect(filters.flat().sort()).toEqual(files.map((file) => `e2e/${file}`));
  }
  const withNew = splitSpecFiles([...files, 'brand-new.spec.ts'], COMPONENT_SHARDS, weights);
  expect(withNew.flatMap((shard) => shard.files).sort()).toEqual(
    [...files, 'brand-new.spec.ts'].sort()
  );
  const total = withNew.reduce((sum, shard) => sum + shard.seconds, 0);
  expect(total).toBe(
    files.reduce(
      (sum, file) => sum + (weights.files[file] ?? weights.defaultSeconds),
      weights.defaultSeconds
    )
  );
  expect(Object.keys(weights.files).filter((file) => !files.includes(file))).toEqual([]);
});

it('discovers the same component spec files as tracked Playwright inputs, without nested omissions', () => {
  const directory = 'extensions/tmt-colab/typescript/app/e2e';
  const tracked = runPackedCommand('git', ['ls-files', '-z', '--', directory], {
    cwd: fileURLToPath(new URL('../../../', import.meta.url)),
    env: process.env,
  })
    .split('\0')
    .filter((file: string) => /\.(?:spec|test)\.[cm]?[jt]sx?$/.test(file));
  expect(tracked.map((file: string) => path.relative(directory, file)).sort()).toEqual(
    listComponentFiles()
  );
});

it('uses the smallest shard count predicted within twelve minutes including measured job overhead', () => {
  const weights = loadComponentWeights();
  const wallSeconds = (count: number) =>
    Math.max(
      ...splitSpecFiles(listComponentFiles(), count, weights).map((shard) => shard.seconds)
    ) + weights.jobOverheadSeconds;
  expect(wallSeconds(COMPONENT_SHARDS)).toBeLessThanOrEqual(12 * 60);
  for (let count = 1; count < COMPONENT_SHARDS; count++)
    expect(wallSeconds(count)).toBeGreaterThan(12 * 60);
  expect(() => loadComponentWeights('{"defaultSeconds":1,"files":{}}')).toThrow(/overhead/);
});
