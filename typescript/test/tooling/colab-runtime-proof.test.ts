import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vite-plus/test';
import { verifyColabApp } from '../../scripts/colab-runtime-proof.mjs';
import { colabFixtureBinary } from '../support/colab-runtime-fixture.js';

const { nativeHostTarget, verifyNativeRuntime } = await import(
  pathToFileURL(fileURLToPath(new URL('../../scripts/native-runtime-proof.mjs', import.meta.url)))
    .href
);
let root: string;
let app: string;
const notices = 'Rust attribution\nTiny app attribution\n';
beforeAll(() => {
  root = mkdtempSync('/tmp/colab-verifier-fixture-');
  app = path.join(root, 'expected-app');
  mkdirSync(path.join(app, 'assets'), { recursive: true });
  writeFileSync(
    path.join(app, 'index.html'),
    '<!doctype html><script src="./assets/app.js"></script><link href="./assets/app.css" rel="stylesheet">tiny embedded app\n'
  );
  writeFileSync(path.join(app, 'assets/app.js'), "console.log('embedded fixture');\n");
  writeFileSync(path.join(app, 'assets/app.css'), 'body { color: blue; }\n');
  writeFileSync(path.join(app, 'THIRD-PARTY-NOTICES.txt'), 'Tiny app attribution\n');
  colabFixtureBinary(root);
}, 30_000);
afterAll(() => rmSync(root, { recursive: true, force: true }));

const proof = (variant = 'valid', combined = notices) =>
  verifyColabApp({
    executable: path.join(root, 'colab-fixture'),
    args: ['--fixture-variant', variant],
    expectedApp: app,
    notices: combined,
    version: '0.1.0-alpha.1',
  });

describe('relocated native Colab app proof', () => {
  it('excuses an exiting-group denial only after confirming group absence', async () => {
    const kill = process.kill.bind(process);
    let group = 0;
    let denied = false;
    let absent = false;
    const signal = vi.spyOn(process, 'kill').mockImplementation((pid, operation) => {
      if (operation === 'SIGTERM') group = pid;
      if (pid === group && !denied && (operation === 0 || operation === 'SIGKILL')) {
        denied = true;
        throw Object.assign(new Error('Simulated exiting-group denial'), { code: 'EPERM' });
      }
      try {
        return kill(pid, operation);
      } catch (error) {
        if (pid === group && operation === 0 && (error as NodeJS.ErrnoException).code === 'ESRCH')
          absent = true;
        throw error;
      }
    });
    try {
      await proof();
      expect(denied).toBe(true);
      expect(absent).toBe(true);
    } finally {
      signal.mockRestore();
    }
  });

  it('fails cleanup and retains state when exiting-group absence cannot be confirmed', async () => {
    const kill = process.kill.bind(process);
    const directories = vi.spyOn(fs, 'mkdtempSync');
    let group = 0;
    const signal = vi.spyOn(process, 'kill').mockImplementation((pid, operation) => {
      if (operation === 'SIGTERM') group = pid;
      if (pid === group && (operation === 0 || operation === 'SIGKILL'))
        throw Object.assign(new Error('Simulated inspection denial'), { code: 'EPERM' });
      return kill(pid, operation);
    });
    let state = '';
    try {
      const attempt = proof();
      state = directories.mock.results[0].value;
      await expect(attempt).rejects.toThrow('process group absence was not confirmed');
      expect(fs.existsSync(state)).toBe(true);
    } finally {
      signal.mockRestore();
      directories.mockRestore();
      if (state) rmSync(state, { recursive: true, force: true });
    }
  });

  it('runs a native fixture with exact embedded HTML, assets and notices through the shared archive proof', async () => {
    await verifyNativeRuntime({
      executable: path.join(root, 'colab-fixture'),
      target: nativeHostTarget(),
      version: '0.1.0-alpha.1',
      product: 'colab',
      colabApp: app,
      notices,
      subject: 'Colab fixture archive',
    });
    // No source app path is passed to the child; the independent input only belongs to the verifier.
    expect(readFileSync(path.join(app, 'assets/app.js'), 'utf8')).toContain('embedded fixture');
    await verifyColabApp({
      executable: path.join(root, 'colab-fixture'),
      notices,
      version: '0.1.0-alpha.1',
    });
  });

  it.each([
    ['PLACEHOLDER', 'placeholder or incomplete app'],
    ['CORRUPT_ASSET', 'embedded bytes differ: /assets/app.js'],
    ['STARTUP_FAILURE', 'COLAB_APP_UNAVAILABLE'],
    ['LEAK_SOCKET', 'did not clean up its socket'],
  ])('fails the %s mutation after the positive control passes', async (variant, message) => {
    await proof();
    await expect(proof(variant)).rejects.toThrow(message);
  });

  it.each(['Rust attribution\n', 'Tiny app attribution\n'])(
    'rejects incomplete combined notices: %s',
    async (combined) => {
      await expect(proof('valid', combined)).rejects.toThrow(
        'combined notices omit Rust or frontend'
      );
    }
  );
});
