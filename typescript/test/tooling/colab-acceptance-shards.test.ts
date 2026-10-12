import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import {
  ACCEPTANCE_SHARDS,
  acceptanceShardFiles,
  listAcceptanceFiles,
  loadShardWeights,
  splitAcceptanceFiles,
} from '../../scripts/colab-acceptance-shards.mjs';

const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);
const repository = fileURLToPath(new URL('../../../', import.meta.url));
const acceptance = 'extensions/tmt-colab/typescript/app/acceptance';

describe('Colab acceptance shards', () => {
  const files = listAcceptanceFiles();
  const weights = loadShardWeights();
  const sorted = (list: readonly string[]) => [...list].sort();

  it('puts every spec file in exactly one shard, for any shard count', () => {
    expect(files.length).toBeGreaterThan(30);
    for (const count of [1, 2, 3, 5]) {
      const shards = splitAcceptanceFiles(files, count, weights);
      expect(shards, `count ${count}`).toHaveLength(count);
      expect(sorted(shards.flatMap((shard) => shard.files)), `count ${count}`).toEqual(files);
    }
    // The split CI actually runs, as the filters Playwright takes.
    const filters = Array.from({ length: ACCEPTANCE_SHARDS }, (_, index) =>
      acceptanceShardFiles(index + 1, ACCEPTANCE_SHARDS)
    );
    expect(sorted(filters.flat())).toEqual(files.map((file) => `acceptance/${file}`));
    expect(ACCEPTANCE_SHARDS).toBe(2);
  });

  it('finds every spec file Playwright would run, with none hidden in a subdirectory', () => {
    const tracked = runPackedCommand('git', ['ls-files', '-z', '--', acceptance], {
      cwd: repository,
      env: process.env,
    })
      .split('\0')
      .filter((file: string) => file.endsWith('.spec.ts'));
    expect(tracked.map((file: string) => path.relative(acceptance, file)).sort()).toEqual(files);
  });

  it('is deterministic and keeps each shard sorted by name', () => {
    const once = splitAcceptanceFiles(files, 2, weights);
    expect(splitAcceptanceFiles([...files].reverse(), 2, weights)).toEqual(once);
    for (const shard of once) expect(shard.files).toEqual(sorted(shard.files));
  });

  it('balances the measured weights, which Playwright by-count sharding did not', () => {
    for (const [count, bound] of [
      [2, 1.05],
      [3, 1.5],
    ] as const) {
      const seconds = splitAcceptanceFiles(files, count, weights).map((shard) => shard.seconds);
      expect(Math.max(...seconds) / Math.min(...seconds), `count ${count}`).toBeLessThan(bound);
    }
    // The heaviest spec (a 2 MiB page) bounds what any shard count can reach.
    const heaviest = Math.max(...Object.values<number>(weights.files));
    const [first, second] = splitAcceptanceFiles(files, 2, weights);
    expect(Math.max(first.seconds, second.seconds)).toBeLessThan(1000);
    expect(heaviest).toBeGreaterThan(400);
  });

  it('gives a file with no weight the default weight and still places it', () => {
    const withNew = [...files, 'brand-new.spec.ts'];
    const shards = splitAcceptanceFiles(withNew, 2, weights);
    expect(sorted(shards.flatMap((shard) => shard.files))).toEqual(sorted(withNew));
    const total = shards.reduce((sum, shard) => sum + shard.seconds, 0);
    const known = files.reduce(
      (sum, file) => sum + (weights.files[file] ?? weights.defaultSeconds),
      0
    );
    expect(total).toBe(known + weights.defaultSeconds);
  });

  it('keeps no weight for a file that no longer exists and refuses unusable weights', () => {
    expect(Object.keys(weights.files).filter((file) => !files.includes(file))).toEqual([]);
    expect(() => loadShardWeights('{"defaultSeconds":0,"files":{}}')).toThrow(/positive/);
    expect(() => loadShardWeights('{"defaultSeconds":5,"files":{"a.spec.ts":-1}}')).toThrow(
      /positive/
    );
    expect(() => splitAcceptanceFiles(files, 0, weights)).toThrow(/positive integer/);
    expect(() => acceptanceShardFiles(3, 2)).toThrow(/between 1 and/);
  });

  it('is what the workflow runs, and its command line carries no unquoted path', () => {
    const workflow = readFileSync(
      path.join(repository, '.github/workflows/colab-browser.yml'),
      'utf8'
    );
    // The listing is assigned first under `set -e`, so a failing script fails the step, and an empty
    // list is refused: with no filters Playwright would run the whole suite in every shard.
    expect(workflow).toContain('set -euo pipefail\n');
    expect(workflow).toContain(
      'shard_files="$(node scripts/colab-acceptance-shards.mjs ${{ matrix.shard }} 2)"'
    );
    expect(workflow).toContain('read -r -a files <<< "$shard_files"');
    expect(workflow).toContain('if [ "${#files[@]}" -eq 0 ]; then');
    expect(files.every((file) => /^[a-z0-9-]+\.spec\.ts$/.test(file))).toBe(true);
  });
});
