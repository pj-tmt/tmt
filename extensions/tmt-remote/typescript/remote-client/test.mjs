import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const output = mkdtempSync(join(tmpdir(), 'tmt-codec-test-'));
try {
  for (const [command, args] of [
    ['tsc', ['--outDir', output]],
    ['python3', ['test/reference.py', '--check']],
    [process.execPath, ['--test', join(output, 'test/canonical-bytes.test.js')]],
  ]) {
    writeFileSync(join(output, 'package.json'), '{"type":"module"}\n');
    const result = spawnSync(command, args, { stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      process.exitCode = result.status ?? 1;
      break;
    }
  }
} finally {
  rmSync(output, { recursive: true, force: true });
}
