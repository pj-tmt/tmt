import { accessSync, constants, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { writeExecutable } from './executable-fixture.mjs';

/** Publish the explicitly built native fixture; never compile or use a host fallback in tests. */
export function colabFixtureBinary(root: string): string {
  const source =
    process.env.TMT_TEST_COLAB_FIXTURE ??
    fileURLToPath(
      new URL('../../../rust/target/debug/examples/colab-runtime-fixture', import.meta.url)
    );
  assert(path.isAbsolute(source), 'TMT_TEST_COLAB_FIXTURE must be absolute');
  assert(
    statSync(source).isFile(),
    'Build the tmt-test-support colab-runtime-fixture example first'
  );
  accessSync(source, constants.X_OK);
  const executable = path.join(root, 'colab-fixture');
  writeExecutable(executable, readFileSync(source), 0o700);
  return executable;
}
