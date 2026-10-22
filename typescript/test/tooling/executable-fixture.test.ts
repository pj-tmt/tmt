import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Worker } from 'node:worker_threads';
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

  it('hands the bytes over as files and leaves nothing behind', () => {
    const root = temporaryRoot();
    const file = path.join(root, 'tool');
    const bytes = Buffer.alloc(5 * 1024 * 1024, 7);
    writeExecutable(file, bytes, 0o755);
    expect(fs.readFileSync(file).equals(bytes)).toBe(true);
    writeExecutable(file, '');
    expect(fs.readFileSync(file)).toHaveLength(0);
    expect(fs.readdirSync(root)).toEqual(['tool']);
  });

  it('reports a writer that cannot start and still removes its working files', () => {
    const root = temporaryRoot();
    expect(() => writeExecutable(path.join(root, 'tool'), 'x', 0o7777 + 1)).toThrow(
      /Usage: executable-fixture\.mjs --write FILE MODE/
    );
    expect(fs.readdirSync(root)).toEqual([]);
  });

  it('kills a writer that does not finish and says what it was holding', () => {
    const root = temporaryRoot();
    const stuck = path.join(root, 'stuck.mjs');
    // Reports its pid, then waits for ever like a writer that never sees EOF.
    fs.writeFileSync(stuck, `setInterval(() => {}, 1000);\n`);
    const file = path.join(root, 'never-written');
    let error: Error | undefined;
    try {
      writeExecutable(file, 'twelve bytes', 0o755, { timeoutMs: 400, writer: stuck });
    } catch (caught) {
      error = caught as Error;
    }
    expect(error?.message).toMatch(/could not publish .*never-written \(12 bytes, mode 755\)/);
    expect(error?.message).toMatch(/did not finish within 400 ms and was killed/);
    const pid = /pid (\d+)\)/.exec(error?.message ?? '')?.[1];
    expect(pid).toBeDefined();
    // The report was taken while the writer was alive: its ps line names it, and lsof lists its files.
    expect(error?.message).toMatch(new RegExp(`ps:\\n[^\\n]*\\n\\s*${pid}\\s`));
    expect(error?.message).toMatch(/lsof \(first 60 lines\):/);
    expect(() => process.kill(Number(pid), 0)).toThrow(/ESRCH/);
    expect(fs.existsSync(file)).toBe(false);
    expect(fs.readdirSync(root).sort()).toEqual(['stuck.mjs']);
  });

  it('does not stall when sibling threads spawn long-lived children at the same time', async () => {
    const root = temporaryRoot();
    const worker = `
      const { workerData, parentPort } = require('node:worker_threads');
      const { spawn } = require('node:child_process');
      const path = require('node:path');
      (async () => {
        const { writeExecutable } = await import(workerData.module);
        for (let n = 0; n < workerData.loops; n++) {
          spawn('/bin/sleep', ['2'], { stdio: 'ignore' }).unref();
          writeExecutable(path.join(workerData.dir, 'f' + n), Buffer.alloc(65536, n), 0o755, { timeoutMs: 20000 });
        }
        parentPort.postMessage('done');
      })();`;
    const threads = Array.from({ length: 4 }, (_, i) => {
      const dir = path.join(root, `t${i}`);
      fs.mkdirSync(dir);
      return new Promise<void>((resolve, reject) => {
        const thread = new Worker(worker, {
          eval: true,
          workerData: {
            dir,
            loops: 25,
            module: new URL('../support/executable-fixture.mjs', import.meta.url).href,
          },
        });
        thread.once('message', () => resolve());
        thread.once('error', reject);
      });
    });
    await Promise.all(threads);
    for (let i = 0; i < 4; i++) expect(fs.readdirSync(path.join(root, `t${i}`))).toHaveLength(25);
  }, 60_000);

  it('propagates publication failure without leaving a staging directory', () => {
    const root = temporaryRoot();
    const directory = path.join(root, 'directory');
    fs.mkdirSync(directory);
    expect(() => writeExecutable(directory, '#!/bin/sh\n')).toThrow();
    expect(fs.readdirSync(root)).toEqual(['directory']);
    expect(fs.statSync(directory).isDirectory()).toBe(true);
  });
});
