import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import childProcess from 'node:child_process';
import http from 'node:http';
import { syncBuiltinESMExports } from 'node:module';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vite-plus/test';
import { appEntries, expectedFiles, verifyColabApp } from '../../scripts/colab-runtime-proof.mjs';
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
  writeFileSync(path.join(app, 'renderer.html'), '<!doctype html>tiny renderer\n');
  writeFileSync(path.join(app, 'reader.html'), '<!doctype html>tiny reader\n');
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
  it('serves headers sent only after the native fixture accepts the connection', async () => {
    const spawn = childProcess.spawn;
    const requestAsset = http.request;
    let accepted = false;
    let released = false;
    let diagnostic = '';
    let releaseRequest: (() => void) | undefined;
    const launch = vi.spyOn(childProcess, 'spawn').mockImplementation((...args) => {
      const child = spawn(...args);
      child.stderr?.on('data', (chunk) => {
        diagnostic += chunk;
        if (!diagnostic.includes('COLAB_FIXTURE_REQUEST_ACCEPTED')) return;
        accepted = true;
        releaseRequest?.();
      });
      return child;
    });
    let held = false;
    const request = vi.spyOn(http, 'request').mockImplementation((...args) => {
      const client = requestAsset(...args);
      if (held) return client;
      held = true;
      const end = client.end.bind(client);
      vi.spyOn(client, 'end').mockImplementation(() => {
        releaseRequest = () => {
          if (released) return;
          released = true;
          end();
        };
        if (accepted) releaseRequest();
        return client;
      });
      return client;
    });
    syncBuiltinESMExports();
    try {
      await proof('REQUEST_BARRIER');
      expect(accepted).toBe(true);
      expect(released).toBe(true);
    } finally {
      launch.mockRestore();
      request.mockRestore();
      syncBuiltinESMExports();
    }
  });

  it('classifies an immediate startup failure only after stderr drains past exit', async () => {
    const spawn = childProcess.spawn;
    const directories = vi.spyOn(fs, 'mkdtempSync');
    let diagnostic = '';
    let diagnosticAtExit: string | undefined;
    const launch = vi.spyOn(childProcess, 'spawn').mockImplementation((...args) => {
      const child = spawn(...args);
      const stderr = child.stderr;
      if (!stderr) throw new Error('Startup regression requires piped stderr');
      const on = stderr.on.bind(stderr);
      // Hold the real diagnostic until exit. Node resumes stdio after emitting
      // exit and emits close only after it drains; no sleeps or invented events.
      vi.spyOn(stderr, 'on').mockImplementation((event, listener) => {
        const stream = on(event, listener);
        if (event === 'data') stderr.pause();
        return stream;
      });
      stderr.on('data', (chunk) => (diagnostic += chunk));
      child.once('exit', () => (diagnosticAtExit = diagnostic));
      return child;
    });
    syncBuiltinESMExports();
    try {
      await expect(proof('STARTUP_FAILURE')).rejects.toThrow(
        'Colab exited before readiness: 1/null: COLAB_APP_UNAVAILABLE\n'
      );
      expect(diagnosticAtExit).toBe('');
      expect(diagnostic).toBe('COLAB_APP_UNAVAILABLE\n');
      expect(fs.existsSync(directories.mock.results[0].value)).toBe(false);
    } finally {
      launch.mockRestore();
      directories.mockRestore();
      syncBuiltinESMExports();
    }
  });

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

  it('rejects different embedded skill bytes before the Colab app proof runs', async () => {
    await expect(
      verifyNativeRuntime({
        executable: path.join(root, 'colab-fixture'),
        target: nativeHostTarget(),
        version: '0.1.0-alpha.1',
        product: 'colab',
        colabSkill: 'Different skill bytes\n',
        colabApp: app,
        notices,
        subject: 'Colab fixture archive',
      })
    ).rejects.toThrow('embedded skill mismatch');
  });

  it('runs a native fixture with exact embedded HTML, assets and notices through the shared archive proof', async () => {
    await verifyNativeRuntime({
      executable: path.join(root, 'colab-fixture'),
      target: nativeHostTarget(),
      version: '0.1.0-alpha.1',
      product: 'colab',
      colabSkill: 'Colab fixture skill\n',
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

describe('declared app entries', () => {
  it('reads the one declaration the native inventory compiles in', () => {
    expect(appEntries()).toEqual({
      html: ['index.html', 'renderer.html', 'reader.html'],
      text: ['THIRD-PARTY-NOTICES.txt'],
    });
    expect(() => appEntries('html index.html extra\n')).toThrow('Invalid app entry declaration');
    expect(() => appEntries('binary index.html\n')).toThrow('Invalid app entry declaration');
    expect(() => appEntries('# only a comment\n')).toThrow('declares no entries');
  });

  it('accepts every declared entry (the alpha.4 reader.html regression) and nothing else', () => {
    const inventory = (directory: string) => [...expectedFiles(directory).keys()].sort();
    expect(inventory(app)).toEqual([
      '/THIRD-PARTY-NOTICES.txt',
      '/assets/app.css',
      '/assets/app.js',
      '/index.html',
      '/reader.html',
      '/renderer.html',
    ]);
    const copy = (name: string) => {
      const directory = path.join(root, name);
      fs.cpSync(app, directory, { recursive: true });
      return directory;
    };
    const stray = copy('stray-app');
    writeFileSync(path.join(stray, 'other.html'), 'x');
    expect(() => expectedFiles(stray)).toThrow('Unexpected app entry: other.html');
    for (const missing of [
      'index.html',
      'renderer.html',
      'reader.html',
      'THIRD-PARTY-NOTICES.txt',
    ]) {
      const incomplete = copy(`missing-${missing}`);
      rmSync(path.join(incomplete, missing));
      expect(() => expectedFiles(incomplete), missing).toThrow('Expected app is incomplete');
    }
  });
});
