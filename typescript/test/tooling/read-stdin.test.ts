import { spawnSync } from 'node:child_process';
import { closeSync, openSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { runWithNonblockingStdin } from '../support/nonblocking-stdin.js';

const reader = new URL('../../scripts/read-stdin.mjs', import.meta.url).href;
const read = `import {readStdinUtf8} from ${JSON.stringify(reader)};`;

describe('synchronous stdin reader', () => {
  it('waits for a real non-blocking producer and decodes UTF-8 once', () => {
    const input = '雪🧪'.repeat(100_000);
    const result = runWithNonblockingStdin(
      [
        '--input-type=module',
        '--eval',
        `${read}const text=readStdinUtf8();process.stdout.write(JSON.stringify({bytes:Buffer.byteLength(text),text:text.slice(-3)}));`,
      ],
      input
    );
    expect(result.ready).toBe(true);
    expect(result.status, result.stderr).toBe(0);
    expect(JSON.parse(result.stdout)).toEqual({ bytes: Buffer.byteLength(input), text: '雪🧪' });
  });

  it('rethrows EAGAIN when a real pipe remains open without data past the deadline', () => {
    const result = runWithNonblockingStdin(
      [
        '--input-type=module',
        '--eval',
        `${read}try{readStdinUtf8({maxWaitMs:20});process.exitCode=1;}catch(error){process.stdout.write(error.code);}`,
      ],
      null
    );
    expect(result.ready).toBe(true);
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toBe('EAGAIN');
  });

  it.each([
    { label: 'empty', input: '' },
    { label: 'unicode', input: '雪🧪'.repeat(20_000) },
  ])('ends at normal EOF without losing bytes ($label)', ({ input }) => {
    const result = spawnSync(
      process.execPath,
      ['--input-type=module', '--eval', `${read}process.stdout.write(readStdinUtf8());`],
      { input, encoding: 'utf8', timeout: 5000 }
    );
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toBe(input);
  });

  it('accepts the EOF error code without treating other errors as EOF', () => {
    const code = `import fs from 'node:fs';import {syncBuiltinESMExports} from 'node:module';
      const original=fs.readSync;fs.readSync=(...args)=>{const count=original(...args);if(count===0)throw Object.assign(new Error('end'),{code:'EOF'});return count;};syncBuiltinESMExports();
      const {readStdinUtf8}=await import(${JSON.stringify(reader)});process.stdout.write(readStdinUtf8());`;
    const result = spawnSync(process.execPath, ['--input-type=module', '--eval', code], {
      input: 'complete',
      encoding: 'utf8',
      timeout: 5000,
    });
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toBe('complete');
  });

  it('propagates an invalid descriptor', () => {
    const result = spawnSync(
      process.execPath,
      [
        '--input-type=module',
        '--eval',
        `import {closeSync} from 'node:fs';${read}closeSync(0);try{readStdinUtf8();process.exitCode=1;}catch(error){process.stdout.write(error.code);}`,
      ],
      { encoding: 'utf8', timeout: 5000 }
    );
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toBe('EBADF');
  });

  it('propagates a directory read error', () => {
    const descriptor = openSync(fileURLToPath(new URL('../', import.meta.url)), 'r');
    try {
      const result = spawnSync(
        process.execPath,
        [
          '--input-type=module',
          '--eval',
          `${read}try{readStdinUtf8();process.exitCode=1;}catch(error){process.stdout.write(error.code);}`,
        ],
        { stdio: [descriptor, 'pipe', 'pipe'], encoding: 'utf8', timeout: 5000 }
      );
      expect(result.status, result.stderr).toBe(0);
      expect(result.stdout).toBe('EISDIR');
    } finally {
      closeSync(descriptor);
    }
  });
});
