import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { runPackedCommand } from './packed-command.mjs';
import { writeExecutable } from '../test/support/executable-fixture.mjs';

const APP_BYTES = 16 * 1024 * 1024;
const APP_FILES = 128;
const context = JSON.stringify({ owner: true, deviceId: 'packaging-proof', name: 'Packaging' });
const shellQuote = (value) => `'${value.replaceAll("'", "'\\''")}'`;

function expectedFiles(directory) {
  assert(fs.lstatSync(directory).isDirectory(), 'Expected app must be a real directory');
  const files = new Map();
  let total = 0;
  const add = (relative) => {
    const file = path.join(directory, relative);
    const stat = fs.lstatSync(file);
    total += stat.size;
    assert(stat.isFile() && stat.size > 0 && total <= APP_BYTES, 'Invalid expected app bytes');
    assert(files.size < APP_FILES, 'Too many expected app files');
    files.set(`/${relative}`, fs.readFileSync(file));
  };
  for (const name of fs.readdirSync(directory).sort()) {
    if (name === 'assets') {
      assert(
        fs.lstatSync(path.join(directory, name)).isDirectory(),
        'Expected assets must be real'
      );
      for (const asset of fs.readdirSync(path.join(directory, name)).sort()) {
        assert(/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/.test(asset), 'Unsafe expected asset name');
        add(`assets/${asset}`);
      }
    } else {
      assert(
        ['index.html', 'renderer.html', 'THIRD-PARTY-NOTICES.txt'].includes(name),
        'Unexpected app entry'
      );
      add(name);
    }
  }
  assert(
    files.has('/index.html') && files.has('/THIRD-PARTY-NOTICES.txt'),
    'Expected app is incomplete'
  );
  return files;
}

function requestAsset(socket, route, owner = true) {
  return new Promise((resolve, reject) => {
    const request = http.request(
      {
        socketPath: socket,
        path: route,
        headers: owner ? { 'tmt-device-context': context } : {},
      },
      (response) => {
        const chunks = [];
        let size = 0;
        response.on('data', (chunk) => {
          size += chunk.length;
          if (size > APP_BYTES) request.destroy(new Error('Colab asset exceeds its bound'));
          else chunks.push(chunk);
        });
        response.on('error', reject);
        response.on('end', () =>
          resolve({
            status: response.statusCode,
            headers: response.headers,
            bytes: Buffer.concat(chunks),
          })
        );
      }
    );
    // A total deadline also bounds a peer that sends one byte before each idle timeout.
    const timeout = setTimeout(
      () => request.destroy(new Error('Colab asset deadline exceeded')),
      3_000
    );
    request.once('close', () => clearTimeout(timeout));
    request.on('error', (error) =>
      reject(new Error(`Colab asset request failed (${route}): ${error.message}`, { cause: error }))
    );
    request.end();
  });
}

function waitReady(child) {
  return new Promise((resolve, reject) => {
    let stdout = '';
    let stderr = '';
    let done = false;
    const finish = (error, value) => {
      if (done) return;
      done = true;
      clearTimeout(timeout);
      if (error) reject(error);
      else resolve(value);
    };
    const timeout = setTimeout(() => finish(new Error('Colab readiness timed out')), 5_000);
    child.stdout.on('data', (chunk) => {
      if (done) return;
      stdout += chunk;
      if (stdout.length > 4096) return finish(new Error('Colab readiness exceeds its bound'));
      if (stdout.includes('\n')) {
        try {
          finish(null, JSON.parse(stdout));
        } catch (error) {
          finish(error);
        }
      }
    });
    child.stderr.on('data', (chunk) => {
      if (done) return;
      stderr += chunk;
      if (stderr.length > 4096) finish(new Error('Colab diagnostics exceed their bound'));
    });
    child.once('error', (error) => finish(error));
    // 'close' follows the stdio streams' end, so a fast startup failure's stderr is complete.
    child.once('close', (code, signal) =>
      finish(new Error(`Colab exited before readiness: ${code}/${signal}: ${stderr}`))
    );
  });
}

function exited(child) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Colab shutdown timed out')), 5_000);
    const finish = (code, signal) => {
      clearTimeout(timer);
      resolve({ code, signal });
    };
    if (child.exitCode !== null || child.signalCode !== null)
      finish(child.exitCode, child.signalCode);
    else child.once('exit', finish);
  });
}

async function cleanupChild(child) {
  if (!child?.pid) return;
  const stopped = exited(child);
  let signalError;
  try {
    // Do not signal a group after observing its absence, including graceful exit.
    process.kill(-child.pid, 0);
    process.kill(-child.pid, 'SIGKILL');
  } catch (error) {
    if (error.code !== 'ESRCH') signalError = error;
  }
  await stopped;
  const deadline = performance.now() + 3_000;
  for (;;) {
    try {
      process.kill(-child.pid, 0);
    } catch (error) {
      if (error.code === 'ESRCH') {
        // Darwin can deny a signal to an exiting group. Direct exit alone is
        // insufficient: only this subsequent absence observation excuses EPERM.
        if (signalError && signalError.code !== 'EPERM') throw signalError;
        return;
      }
      if (error.code !== 'EPERM') throw error;
      signalError ??= error;
    }
    if (performance.now() >= deadline)
      throw new Error('Colab process group absence was not confirmed', { cause: signalError });
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

/** One installed-app proof shared by archive verification and public-install smoke. */
export async function verifyColabApp({
  executable,
  args = [],
  version,
  expectedApp,
  notices,
  tmtExecutable,
}) {
  assert.equal(typeof notices, 'string', 'Colab proof requires combined archive notices');
  assert(notices.length > 0, 'Colab combined notices are empty');
  const expected = expectedApp ? expectedFiles(expectedApp) : null;
  // Short paths stay below the Unix socket bound on macOS, even with a long TMPDIR.
  const root = fs.mkdtempSync(path.join(fs.realpathSync('/tmp'), 'tmt-colab-proof-'));
  let child;
  try {
    const runtime = path.join(root, 'runtime');
    const home = path.join(root, 'home');
    const state = path.join(root, 'state');
    const cwd = path.join(root, 'outside-checkout');
    const emptyPath = path.join(root, 'empty-path');
    for (const directory of [runtime, home, state, cwd, emptyPath])
      fs.mkdirSync(directory, { mode: 0o700 });
    const binary = path.join(runtime, 'tmt-colab');
    writeExecutable(binary, fs.readFileSync(executable), 0o700);
    if (!tmtExecutable) {
      // Archive proof injects only storage-root discovery; public smoke uses its real installed CLI.
      tmtExecutable = path.join(root, 'core-storage-fixture');
      writeExecutable(
        tmtExecutable,
        `#!/bin/sh\n[ "$1" = api ] || exit 2\nprintf '%s' ${shellQuote(JSON.stringify({ dataRoot: state }))}\n`,
        0o700
      );
    }
    const env = {
      HOME: home,
      TMUX_TEAM_HOME: state,
      TMPDIR: root,
      PATH: emptyPath,
      LANG: 'C',
      TMT_EXECUTABLE: tmtExecutable,
    };
    assert.equal(
      runPackedCommand(binary, [...args, '--version'], { cwd, env }),
      `colab ${version}\n`,
      'Colab version mismatch'
    );
    const socket = path.join(state, 'colab', 'door.sock');
    child = spawn(binary, [...args, 'serve', '--json'], {
      cwd,
      env,
      detached: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const ready = await waitReady(child);
    assert.equal(ready.socket, socket, 'Colab reported a socket outside isolated state');
    assert.equal(ready.state, 'mounted', 'Colab did not mount');
    const index = await requestAsset(socket, '/');
    assert.equal(index.status, 200, 'Colab index did not serve');
    assert.match(index.headers['content-type'] ?? '', /^text\/html/);
    const html = index.bytes.toString('utf8');
    const assets = [
      ...html.matchAll(/(?:src|href)=["']\.\/assets\/([a-zA-Z0-9][a-zA-Z0-9._-]{0,127})["']/g),
    ].map((match) => `/assets/${match[1]}`);
    assert(
      assets.some((route) => route.endsWith('.js')) &&
        assets.some((route) => route.endsWith('.css')),
      'Colab served a placeholder or incomplete app'
    );
    if (expected)
      assert(
        index.bytes.equals(expected.get('/index.html')),
        'Colab embedded index differs from the build'
      );
    const routes = expected
      ? [...expected.keys()]
      : [...new Set(assets), '/THIRD-PARTY-NOTICES.txt'];
    assert(routes.length <= APP_FILES, 'Colab app inventory exceeds its bound');
    let total = 0;
    for (const route of routes) {
      const response = await requestAsset(socket, route);
      assert.equal(response.status, 200, `Colab asset unavailable: ${route}`);
      total += response.bytes.length;
      assert(
        response.bytes.length > 0 && total <= APP_BYTES,
        'Colab app bytes are empty or oversized'
      );
      if (expected)
        assert(response.bytes.equals(expected.get(route)), `Colab embedded bytes differ: ${route}`);
      if (route.endsWith('.js'))
        assert.match(response.headers['content-type'] ?? '', /^text\/javascript/);
      if (route.endsWith('.css'))
        assert.match(response.headers['content-type'] ?? '', /^text\/css/);
      if (route === '/THIRD-PARTY-NOTICES.txt') {
        const frontend = response.bytes.toString('utf8');
        assert(
          notices.endsWith(frontend) && notices.length > frontend.length,
          'Colab combined notices omit Rust or frontend attribution'
        );
      }
    }
    assert.equal(
      (await requestAsset(socket, assets[0], false)).status,
      403,
      'Colab assets must remain owner-only'
    );
    assert.equal(
      (await requestAsset(socket, '/assets/missing-packaging-proof.js')).status,
      404,
      'Colab must reject absent assets'
    );
    const stopped = exited(child);
    process.kill(-child.pid, 'SIGTERM');
    const result = await stopped;
    assert.equal(result.code, 0, 'Colab shutdown failed');
    assert.equal(result.signal, null, 'Colab shutdown was not graceful');
    assert(!fs.existsSync(socket), 'Colab did not clean up its socket');
    assert.deepEqual(
      fs.readdirSync(runtime),
      ['tmt-colab'],
      'Colab runtime must not require sibling app files'
    );
    assert.deepEqual(fs.readdirSync(cwd), [], 'Colab must not create checkout files');
  } finally {
    await cleanupChild(child);
    fs.rmSync(root, { recursive: true, force: true });
  }
}
