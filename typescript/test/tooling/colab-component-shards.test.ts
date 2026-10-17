import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { expect, it } from 'vite-plus/test';
import {
  listSpecFiles,
  specShardFiles,
  splitSpecFiles,
} from '../../scripts/colab-acceptance-shards.mjs';

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

it('measures four contiguous shards without changing coverage or execution limits', () => {
  const workflow = readFileSync(
    new URL('../../../.github/workflows/colab-browser.yml', import.meta.url),
    'utf8'
  );
  const app = workflow.split('\n  colab-app:\n')[1].split('\n  colab-hosted:\n')[0];
  expect(app).toContain('shard: [1, 2, 3, 4]');
  expect(app).toContain('--shard=${{ matrix.shard }}/4 --reporter=dot,json');
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
