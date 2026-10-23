import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const fixture = fileURLToPath(new URL('./nonblocking-stdin.py', import.meta.url));
// Observe actual OS errors without synthesizing EAGAIN or changing read behavior.
const observe = `
import fs from 'node:fs';
import {syncBuiltinESMExports} from 'node:module';
const read = fs.readSync;
let announced = false;
fs.readSync = (...args) => {
  try { return read(...args); }
  catch (error) {
    if (args[0] === 0 && error.code === 'EAGAIN' && !announced) {
      announced = true;
      fs.writeSync(2, 'stdin-eagain\\n');
    }
    throw error;
  }
};
syncBuiltinESMExports();
`;

export function runWithNonblockingStdin(args: string[], input: string | null) {
  const child = spawnSync('python3', [fixture], {
    input: JSON.stringify({
      argv: [
        process.execPath,
        '--import',
        `data:text/javascript,${encodeURIComponent(observe)}`,
        ...args,
      ],
      input,
    }),
    encoding: 'utf8',
    timeout: 28_000,
    maxBuffer: 1024 * 1024,
    env: { ...process.env, GITHUB_OUTPUT: '', GITHUB_STEP_SUMMARY: '' },
  });
  if (child.error || child.status !== 0) {
    throw new Error(`Non-blocking stdin fixture failed: ${child.error?.message ?? child.stderr}`);
  }
  return JSON.parse(child.stdout) as {
    ready: boolean;
    status: number;
    stdout: string;
    stderr: string;
  };
}
