import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';

const roots: string[] = [];
const writer = fileURLToPath(new URL('../support/executable-fixture.mjs', import.meta.url));
function temporaryRoot(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-executable-fixture-'));
  roots.push(root);
  return root;
}

afterEach(() => {
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe('executable fixture publication', () => {
  it('can execute complete fixture bytes as soon as publication returns', () => {
    const file = path.join(temporaryRoot(), 'fixture with spaces');
    writeExecutable(file, '#!/bin/sh\nprintf "fixture:%s" "$1"\n', 0o700);
    expect(execFileSync(file, ['argument with spaces'], { encoding: 'utf8' })).toBe(
      'fixture:argument with spaces'
    );
    expect(fs.statSync(file).mode & 0o777).toBe(0o700);
  });

  it('preserves binary bytes and deliberately non-executable modes', () => {
    const file = path.join(temporaryRoot(), 'binary');
    const bytes = Uint8Array.from([0, 255, 128, 10]);
    writeExecutable(file, bytes, 0o644);
    expect(fs.readFileSync(file)).toEqual(Buffer.from(bytes));
    expect(fs.statSync(file).mode & 0o777).toBe(0o644);
  });

  it('replaces an inode held open for writing without exposing that descriptor to execution', () => {
    const file = path.join(temporaryRoot(), 'replaced');
    writeExecutable(file, '#!/bin/sh\nprintf old\n');
    const old = fs.openSync(file, 'r+');
    try {
      const previous = fs.fstatSync(old);
      writeExecutable(file, '#!/bin/sh\nprintf new\n');
      expect(fs.statSync(file).ino).not.toBe(previous.ino);
      expect(execFileSync(file, { encoding: 'utf8' })).toBe('new');
      expect(fs.readFileSync(old, 'utf8')).toBe('#!/bin/sh\nprintf old\n');
    } finally {
      fs.closeSync(old);
    }
  });

  it('closes the actual staging descriptor before chmod and publication', () => {
    const root = temporaryRoot();
    const preload = path.join(root, 'observe.mjs');
    writeExecutable(
      preload,
      `import fs from 'node:fs';
import assert from 'node:assert/strict';
let descriptor;
const open = fs.openSync;
fs.openSync = (...args) => {
  const fd = open(...args);
  if (String(args[0]).endsWith('/payload')) descriptor = fd;
  return fd;
};
for (const method of ['chmodSync', 'renameSync']) {
  const original = fs[method];
  fs[method] = (...args) => {
    assert.notEqual(descriptor, undefined);
    assert.throws(() => fs.fstatSync(descriptor), { code: 'EBADF' });
    return original(...args);
  };
}
`,
      0o644
    );
    const file = path.join(root, 'closed');
    execFileSync(process.execPath, ['--import', preload, writer, '--write', file, String(0o755)], {
      input: '#!/bin/sh\nprintf closed\n',
    });
    expect(execFileSync(file, { encoding: 'utf8' })).toBe('closed');
    expect(fs.readdirSync(root).sort()).toEqual(['closed', 'observe.mjs']);
  });

  it('keeps the previous executable and removes staging bytes when fsync fails', () => {
    const root = temporaryRoot();
    const file = path.join(root, 'retained');
    writeExecutable(file, '#!/bin/sh\nprintf retained\n');
    const preload = path.join(root, 'fail-sync.mjs');
    writeExecutable(
      preload,
      `import fs from 'node:fs';
fs.fsyncSync = () => { throw new Error('fixture fsync failed'); };
`,
      0o644
    );
    expect(() =>
      execFileSync(
        process.execPath,
        ['--import', preload, writer, '--write', file, String(0o755)],
        {
          input: '#!/bin/sh\nprintf replacement\n',
          stdio: ['pipe', 'pipe', 'pipe'],
        }
      )
    ).toThrow(/fixture fsync failed/);
    expect(execFileSync(file, { encoding: 'utf8' })).toBe('retained');
    expect(fs.readdirSync(root).sort()).toEqual(['fail-sync.mjs', 'retained']);
  });

  it('propagates publication failure without leaving a staging directory', () => {
    const root = temporaryRoot();
    const directory = path.join(root, 'directory');
    fs.mkdirSync(directory);
    expect(() => writeExecutable(directory, '#!/bin/sh\n')).toThrow();
    expect(fs.readdirSync(root)).toEqual(['directory']);
    expect(fs.statSync(directory).isDirectory()).toBe(true);
  });
});
