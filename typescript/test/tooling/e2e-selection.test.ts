import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';
import { e2eShardFiles, listE2eFiles, RETIRED_E2E_FILES } from '../../scripts/e2e-shards.mjs';

const typescript = fileURLToPath(new URL('../../', import.meta.url));
const dockerfile = readFileSync(path.join(typescript, 'test/e2e/Dockerfile'), 'utf8');

let root: string;
beforeAll(() => {
  root = mkdtempSync(path.join(os.tmpdir(), 'e2e-selection-'));
  writeExecutable(
    path.join(root, 'pnpm'),
    '#!/usr/bin/env node\nprocess.stdout.write(JSON.stringify(process.argv.slice(2)));\n',
    0o755
  );
});
afterAll(() => rmSync(root, { recursive: true, force: true }));

/**
 * The arguments the Docker image's real `CMD` hands to `pnpm` for a `TMT_E2E_FILES` value: the
 * command runs as written, with a fake `pnpm` that prints its arguments and the workspace
 * directory pointed at this checkout's `typescript` directory.
 */
function pnpmArguments(files: string): string[] {
  const command = /^CMD (\[.*\])$/m.exec(dockerfile)?.[1];
  expect(command, 'the CMD of test/e2e/Dockerfile').toBeDefined();
  const [shell, flag, script] = JSON.parse(command as string) as string[];
  expect(script).toContain('cd /workspace/typescript');
  const result = spawnSync(shell, [flag, script.replace('/workspace/typescript', typescript)], {
    encoding: 'utf8',
    env: {
      PATH: `${root}${path.delimiter}${process.env.PATH}`,
      TMT_E2E_ADAPTER_TESTS: '0',
      TMT_E2E_FILES: files,
    },
    timeout: 30_000,
  });
  expect(result.status, result.stderr).toBe(0);
  return JSON.parse(result.stdout) as string[];
}

/** The filters after the fixed `exec vp test run --config <config>` prefix. */
function filtersOf(args: string[]): string[] {
  expect(args.slice(0, 6)).toEqual([
    'exec',
    'vp',
    'test',
    'run',
    '--config',
    'test/e2e/vitest.config.ts',
  ]);
  return args.slice(6);
}

/**
 * The files vitest itself selects for each list of positional filters, through its own file
 * selection (no test runs, so no container): a child process, because vitest's node API should
 * not start inside a vitest worker.
 */
function selectedByVitest(filterLists: string[][]): string[][] {
  const script = `
    import { createVitest } from 'vite-plus/test/node';
    const lists = JSON.parse(process.argv[1]);
    const vitest = await createVitest('test', {
      config: 'test/e2e/vitest.config.ts', run: true, watch: false, reporters: [],
    });
    const out = [];
    for (const filters of lists) {
      const specs = await vitest.globTestSpecifications(filters);
      out.push(specs.map((spec) => spec.moduleId)
        .map((file) => file.slice(process.cwd().length + 1)).sort());
    }
    await vitest.close();
    process.stdout.write(JSON.stringify(out));
  `;
  const result = spawnSync(
    process.execPath,
    ['--input-type=module', '-e', script, JSON.stringify(filterLists)],
    { cwd: typescript, encoding: 'utf8', timeout: 120_000 }
  );
  expect(result.status, result.stderr).toBe(0);
  return JSON.parse(result.stdout) as string[][];
}

describe('what the Docker E2E shards select', () => {
  const [first, second] = e2eShardFiles('full', []);
  const anchored = (files: string[]) => files.map((file) => `test/e2e/${file}`).sort();
  let selections: { shardFilters: string[][]; single: string[]; bare: string[] };
  let selected: string[][];
  beforeAll(() => {
    selections = {
      shardFilters: [first, second].map((files) => filtersOf(pnpmArguments(files.join(' ')))),
      single: filtersOf(pnpmArguments('routing.e2e.test.ts')),
      // The substring rule itself: the old command line passed the bare name.
      bare: ['routing.e2e.test.ts'],
    };
    selected = selectedByVitest([...selections.shardFilters, selections.single, selections.bare]);
  });

  it('hands vitest one anchored path per configured file, and no filter for an empty list', () => {
    expect(selections.shardFilters[0]).toEqual(first.map((file) => `test/e2e/${file}`));
    expect(selections.shardFilters[1]).toEqual(second.map((file) => `test/e2e/${file}`));
    expect(selections.single).toEqual(['test/e2e/routing.e2e.test.ts']);
    expect(pnpmArguments('')).toEqual([
      'exec',
      'vp',
      'test',
      'run',
      '--config',
      'test/e2e/vitest.config.ts',
    ]);
  });

  it('selects exactly the configured files in each shard, none twice and none of another shard', () => {
    expect(selected[0]).toEqual(anchored(first));
    expect(selected[1]).toEqual(anchored(second));
    expect(new Set([...selected[0], ...selected[1]]).size).toBe(
      selected[0].length + selected[1].length
    );
    expect([...selected[0], ...selected[1]].sort()).toEqual(
      anchored(listE2eFiles().filter((file) => !RETIRED_E2E_FILES.includes(file)))
    );
  });

  it('selects one file for a name that is a substring of others', () => {
    expect(selected[2]).toEqual(['test/e2e/routing.e2e.test.ts']);
  });

  it('is needed: vitest matches a bare file name by substring', () => {
    // If this ever stops holding, the anchoring above is no longer needed, but harmless.
    expect(selected[3]).toEqual([
      'test/e2e/check-routing.e2e.test.ts',
      'test/e2e/routing.e2e.test.ts',
      'test/e2e/session-routing.e2e.test.ts',
    ]);
  });
});
