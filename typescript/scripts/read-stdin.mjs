import { readSync } from 'node:fs';
import { performance } from 'node:perf_hooks';

// Read bytes first: pipe chunks can divide a UTF-8 code point. Inherited pipes
// can also be non-blocking; wait only within one deadline, never interpret
// EAGAIN as EOF or swallow descriptor errors.
export function readStdinUtf8({ maxWaitMs = 90_000 } = {}) {
  const buffer = Buffer.allocUnsafe(64 * 1024);
  const chunks = [];
  const deadline = performance.now() + maxWaitMs;
  const sleeper = new Int32Array(new SharedArrayBuffer(4));
  for (;;) {
    let count;
    try {
      count = readSync(0, buffer, 0, buffer.length, null);
    } catch (error) {
      if (error.code === 'EOF') break;
      if (error.code !== 'EAGAIN') throw error;
      const remaining = deadline - performance.now();
      if (remaining <= 0) throw error;
      Atomics.wait(sleeper, 0, 0, Math.min(1, remaining));
      continue;
    }
    if (count === 0) break;
    chunks.push(Buffer.from(buffer.subarray(0, count)));
  }
  return Buffer.concat(chunks).toString('utf8');
}
