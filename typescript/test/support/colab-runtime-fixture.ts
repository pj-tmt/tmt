import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

/** Compile a test-only native server with known embedded bytes and one optional defect. */
export function colabFixtureBinary(root: string, variant = 'valid'): string {
  assert(
    ['valid', 'PLACEHOLDER', 'CORRUPT_ASSET', 'STARTUP_FAILURE', 'LEAK_SOCKET'].includes(variant)
  );
  const executable = path.join(root, `colab-${variant}`);
  const source = fileURLToPath(new URL('../fixtures/colab-runtime.c', import.meta.url));
  const result = spawnSync(
    'cc',
    [
      '-std=c11',
      '-Wall',
      '-Wextra',
      '-Werror',
      ...(process.platform === 'linux' ? ['-static'] : []),
      ...(variant === 'valid' ? [] : [`-D${variant}`]),
      source,
      '-o',
      executable,
    ],
    { encoding: 'utf8', timeout: 15_000 }
  );
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return executable;
}
