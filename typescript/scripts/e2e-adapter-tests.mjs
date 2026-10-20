// Built-in-only: copied alone into the final image, after every compile/install layer.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { performance } from 'node:perf_hooks';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// Core/Infra-approved pinned-procps slice. Real-tmux library cases remain ignored;
// real tmux coverage belongs to the Docker scenarios, not this library slice.
export const PINNED_ADAPTER_TESTS = Object.freeze([
  'process::process_info::tests::stopped_and_exited_owned_children_keep_batch_parity',
  'process::ancestry::tests::a_real_chain_is_the_same_natively_and_through_ps',
]);

export function runAdapterTests({
  binary = '/opt/tmt-tests/adapter-tests',
  execute = spawnSync,
  output = (stream, text) => writeFileSync(stream, text),
} = {}) {
  const started = performance.now();
  const run = (args, visible) => {
    const result = execute(binary, args, {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      timeout: 30_000,
      maxBuffer: 1024 * 1024,
    });
    if (visible || result.status !== 0 || result.error || result.signal) {
      if (result.stdout) output(1, result.stdout);
      if (result.stderr) output(2, result.stderr);
    }
    if (result.error) throw result.error;
    assert.equal(result.signal, null, `Adapter test command terminated: ${result.signal}`);
    assert.equal(result.status, 0, `Adapter test command failed: ${result.status}`);
    return result.stdout;
  };
  const inventory = [...run(['--list', '--format', 'terse'], false).matchAll(/^(.+): test$/gm)].map(
    (match) => match[1]
  );
  for (const name of PINNED_ADAPTER_TESTS)
    assert.equal(
      inventory.filter((candidate) => candidate === name).length,
      1,
      `Expected exactly one listed adapter test: ${name}`
    );
  for (const name of PINNED_ADAPTER_TESTS) {
    // Include-ignored applies ONLY to this exact approved name, never the inventory.
    // The current two names are active; adding another case needs owner review.
    const text = run(['--exact', '--include-ignored', '--format', 'pretty', name], true);
    const summaries = [
      ...text.matchAll(/^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;/gm),
    ];
    assert.ok(
      summaries.length === 1 &&
        summaries[0][1] === '1' &&
        summaries[0][2] === '0' &&
        summaries[0][3] === '0',
      `Adapter test did not run exactly once: ${name}`
    );
  }
  const seconds = (performance.now() - started) / 1000;
  output(
    1,
    `Adapter environment execution seconds: ${seconds.toFixed(6)}; cases: ${PINNED_ADAPTER_TESTS.length}\n`
  );
  return { tests: PINNED_ADAPTER_TESTS.length, seconds };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    assert.equal(process.argv.length, 2, 'Usage: e2e-adapter-tests.mjs');
    runAdapterTests();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
