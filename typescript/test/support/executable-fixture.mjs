import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const writer = fileURLToPath(import.meta.url);

/** Publish fixture bytes from a process that cannot leak a writable fd to test children. */
export function writeExecutable(file, contents, mode = 0o755) {
  execFileSync(process.execPath, [writer, '--write', file, String(mode)], {
    input: contents,
    stdio: ['pipe', 'pipe', 'pipe'],
  });
}

function publish(file, contents, mode) {
  const staging = fs.mkdtempSync(path.join(path.dirname(file), '.tmt-executable-'));
  const staged = path.join(staging, 'payload');
  try {
    const fd = fs.openSync(staged, 'wx', 0o600);
    try {
      fs.writeFileSync(fd, contents);
      fs.fsyncSync(fd);
    } finally {
      fs.closeSync(fd);
    }
    fs.chmodSync(staged, mode);
    fs.renameSync(staged, file);
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }
}

// Synthetic shell installers use this same writer with fixture bytes on stdin.
if (process.argv[1] === writer) {
  const [, , command, file, rawMode] = process.argv;
  const mode = Number(rawMode);
  if (command !== '--write' || !file || !Number.isInteger(mode) || mode < 0 || mode > 0o7777) {
    throw new Error('Usage: executable-fixture.mjs --write FILE MODE');
  }
  publish(file, fs.readFileSync(0), mode);
}
