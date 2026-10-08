import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import {
  FULL_E2E_SHARDS,
  RETIRED_E2E_FILES,
  e2eShardFiles,
  listE2eFiles,
  loadShardWeights,
  splitE2eFiles,
} from '../../scripts/e2e-shards.mjs';

const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);
const repository = fileURLToPath(new URL('../../../', import.meta.url));

describe('Docker E2E shards', () => {
  it('retires only the two Office companion scenarios', () => {
    expect(RETIRED_E2E_FILES).toEqual(['office-command.e2e.test.ts', 'office-consent.e2e.test.ts']);
    expect(RETIRED_E2E_FILES.every((file) => files.includes(file))).toBe(true);
  });

  const files = listE2eFiles();
  const weights = loadShardWeights();
  const sorted = (list: readonly string[]) => [...list].sort();

  it('puts every scenario file in exactly one shard, for any shard count', () => {
    expect(files.length).toBeGreaterThan(40);
    for (const count of [1, 2, 3, 5]) {
      const shards = splitE2eFiles(files, count, weights);
      expect(shards, `count ${count}`).toHaveLength(count);
      expect(sorted(shards.flatMap((shard) => shard.files)), `count ${count}`).toEqual(files);
    }
    // The split CI actually runs.
    const [first, second] = e2eShardFiles('full', [], files);
    expect(sorted([...first, ...second])).toEqual(
      files.filter((file) => !RETIRED_E2E_FILES.includes(file))
    );
    expect([...first, ...second].filter((file) => RETIRED_E2E_FILES.includes(file))).toEqual([]);
    expect(FULL_E2E_SHARDS).toBe(2);
  });

  it('finds every scenario file the E2E suite would run, with none hidden in a subdirectory', () => {
    const tracked = runPackedCommand('git', ['ls-files', '-z', '--', 'typescript/test/e2e'], {
      cwd: repository,
      env: process.env,
    })
      .split('\0')
      .filter((file: string) => file.endsWith('.e2e.test.ts'));
    expect(
      tracked.map((file: string) => path.relative('typescript/test/e2e', file)).sort()
    ).toEqual(files);
  });

  it('is deterministic and keeps each shard sorted by name', () => {
    const once = splitE2eFiles(files, 2, weights);
    expect(splitE2eFiles([...files].reverse(), 2, weights)).toEqual(once);
    for (const shard of once) expect(shard.files).toEqual(sorted(shard.files));
  });

  it('balances the current inventory, counting the adapter tests against shard 1', () => {
    const [first, second] = splitE2eFiles(files, 2, weights);
    const fileSeconds = (shard: { files: string[] }) =>
      shard.files.reduce((sum, file) => sum + (weights.files[file] ?? weights.defaultSeconds), 0);
    expect(first.seconds).toBe(fileSeconds(first) + weights.adapterTestsSeconds);
    expect(second.seconds).toBe(fileSeconds(second));
    expect(Math.abs(first.seconds - second.seconds)).toBeLessThanOrEqual(10);
    expect(
      Math.max(first.seconds, second.seconds) / Math.min(first.seconds, second.seconds)
    ).toBeLessThan(1.1);
  });

  it('gives a file with no weight the default weight and still places it', () => {
    const withNew = [...files, 'brand-new.e2e.test.ts'];
    const shards = splitE2eFiles(withNew, 2, weights);
    expect(sorted(shards.flatMap((shard) => shard.files))).toEqual(sorted(withNew));
    const total = shards.reduce((sum, shard) => sum + shard.seconds, 0);
    const known = files.reduce(
      (sum, file) => sum + (weights.files[file] ?? weights.defaultSeconds),
      0
    );
    expect(total).toBe(known + weights.defaultSeconds + weights.adapterTestsSeconds);
  });

  it('keeps the weight file honest: no weight names a missing file, and every weight is positive', () => {
    for (const [file, seconds] of Object.entries(weights.files)) {
      expect(files, `${file} has a weight but does not exist`).toContain(file);
      expect(seconds).toBeGreaterThan(0);
    }
    expect(() =>
      loadShardWeights('{"adapterTestsSeconds":1,"defaultSeconds":1,"files":{"a":0}}')
    ).toThrow('positive numbers');
    expect(() =>
      loadShardWeights('{"adapterTestsSeconds":0,"defaultSeconds":1,"files":{}}')
    ).toThrow('positive numbers');
    expect(() => loadShardWeights('{"adapterTestsSeconds":1,"defaultSeconds":1}')).toThrow(
      'positive numbers'
    );
    const text = readFileSync(
      path.join(repository, 'typescript/test/e2e/shard-weights.json'),
      'utf8'
    );
    expect(loadShardWeights(text)).toEqual(weights);
  });

  it.each([0, -1, 1.5, Number.NaN, Infinity])('rejects the shard count %s', (count) => {
    expect(() => splitE2eFiles(files, count, weights)).toThrow('positive integer');
  });

  it('runs one shard for a scoped component, none when nothing native is selected', () => {
    expect(e2eShardFiles('ops', ['ops.e2e.test.ts'], files)).toEqual([['ops.e2e.test.ts'], []]);
    expect(e2eShardFiles('none', [], files)).toEqual([[], []]);
    expect(e2eShardFiles('', [], files)).toEqual([[], []]);
  });
});
