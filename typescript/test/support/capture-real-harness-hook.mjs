// Diagnostic fixture only: Core requested the pinned client's synthetic hook shape (#4610).
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export async function forwardCapturedHook(executable, args, directory, input) {
  let bytes;
  if (args[0] === '__hook' && args[1] === 'codex') {
    const chunks = [];
    let size = 0;
    for await (const chunk of input) {
      const buffer = Buffer.from(chunk);
      size += buffer.length;
      assert(size <= 65536, 'HARNESS_HOOK_CAPTURE_BYTES: more than 65536 bytes');
      chunks.push(buffer);
    }
    bytes = Buffer.concat(chunks);
    mkdirSync(directory, { recursive: true, mode: 0o700 });
    let stored = false;
    for (let slot = 0; slot < 8; slot++) {
      try {
        writeFileSync(path.join(directory, `${slot}.json`), bytes, { flag: 'wx', mode: 0o600 });
        stored = true;
        break;
      } catch (error) {
        if (error.code !== 'EEXIST') throw error;
      }
    }
    assert(stored, 'HARNESS_HOOK_CAPTURE_COUNT: more than eight hook invocations');
  }
  const result = spawnSync(executable, args, {
    ...(bytes === undefined
      ? { stdio: 'inherit' }
      : { input: bytes, stdio: ['pipe', 'inherit', 'inherit'] }),
    timeout: 2000,
    killSignal: 'SIGKILL',
  });
  if (result.error) throw result.error;
  assert(result.status !== null, `HARNESS_HOOK_CHILD: ${result.signal}`);
  return result.status;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert(
    process.platform === 'linux' &&
      existsSync('/.dockerenv') &&
      process.env.TMT_REAL_HARNESS === '1',
    'HARNESS_CONTAINER_REQUIRED: synthetic hook capture is container-only'
  );
  const [executable, directory, ...args] = process.argv.slice(2);
  assert(
    path.isAbsolute(executable ?? '') && path.isAbsolute(directory ?? ''),
    'HARNESS_HOOK_CAPTURE_PATH'
  );
  const timer =
    args[0] === '__hook' && args[1] === 'codex'
      ? setTimeout(
          () => process.stdin.destroy(new Error('HARNESS_HOOK_CAPTURE_EOF: stdin deadline')),
          1000
        )
      : undefined;
  try {
    process.exitCode = await forwardCapturedHook(executable, args, directory, process.stdin);
  } finally {
    clearTimeout(timer);
  }
}
