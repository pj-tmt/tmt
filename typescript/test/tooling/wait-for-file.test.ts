import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { waitForFileContent } from '../e2e/wait-for-file.js';

let root: string;

beforeAll(() => {
  root = mkdtempSync(path.join(os.tmpdir(), 'wait-for-file-'));
});

afterAll(() => rmSync(root, { recursive: true, force: true }));

function target(name: string): string {
  return path.join(root, name);
}

describe('waitForFileContent', () => {
  it('returns content that is already there', async () => {
    const file = target('present');
    writeFileSync(file, '130');
    expect(await waitForFileContent(file)).toBe('130');
  });

  it('does not treat an existing but empty file as ready, the shell-redirect race', async () => {
    const file = target('created-before-written');
    writeFileSync(file, '');
    setTimeout(() => writeFileSync(file, '143'), 60);
    expect(await waitForFileContent(file, { timeoutMs: 800 })).toBe('143');
  });

  it('waits for a file that does not exist yet', async () => {
    const file = target('appears-later');
    setTimeout(() => writeFileSync(file, '{"child":1}'), 60);
    expect(await waitForFileContent(file, { timeoutMs: 800 })).toBe('{"child":1}');
  });

  it('times out with the described condition when the file stays empty', async () => {
    const file = target('stays-empty');
    writeFileSync(file, '');
    await expect(
      waitForFileContent(file, { timeoutMs: 100, description: 'foreground exit status' })
    ).rejects.toThrow('Timed out waiting for foreground exit status.');
  });

  it('times out when the file never appears, naming the file by default', async () => {
    const file = target('never-appears');
    await expect(waitForFileContent(file, { timeoutMs: 100 })).rejects.toThrow(
      `Timed out waiting for content in ${file}.`
    );
  });

  it('does not accept content that arrives after the bound', async () => {
    const file = target('too-late');
    const written = new Promise<void>((resolve) =>
      setTimeout(() => {
        writeFileSync(file, 'late');
        resolve();
      }, 150)
    );
    await expect(waitForFileContent(file, { timeoutMs: 50 })).rejects.toThrow('Timed out waiting');
    await written;
    expect(await waitForFileContent(file)).toBe('late');
  });

  it('reports a real read failure instead of waiting it out', async () => {
    const directory = target('a-directory');
    mkdirSync(directory);
    await expect(waitForFileContent(directory, { timeoutMs: 5_000 })).rejects.toThrow(/EISDIR/);
  });
});
