#!/usr/bin/env node
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { runPackedCommand } from './packed-command.mjs';
import { assertBenchmarkHelp } from '../test/support/performance-contract.mjs';

export function nativeHostTarget() {
  const architecture =
    process.arch === 'arm64' ? 'aarch64' : process.arch === 'x64' ? 'x86_64' : null;
  const platform = process.platform === 'darwin' ? 'apple-darwin' : 'unknown-linux-musl';
  assert(
    ['darwin', 'linux'].includes(process.platform) && architecture,
    'Unsupported verification host'
  );
  return `${architecture}-${platform}`;
}

export function assertNativeTarget(target, message) {
  assert.equal(target, nativeHostTarget(), message);
}

/** Validate the existing model-free driver declaration without tool or service lookup. */
export function assertHerdrCapabilities(raw, expectedVersion, subject) {
  assert.equal(typeof expectedVersion, 'string', 'Herdr proof requires its independent version');
  const answer = JSON.parse(raw);
  assert.equal(answer.ok?.name, 'herdr', `${subject} Herdr driver name mismatch`);
  assert.equal(answer.ok?.kind, 'host', `${subject} Herdr driver kind mismatch`);
  assert.equal(answer.ok?.version, expectedVersion, `${subject} Herdr driver version mismatch`);
  assert(answer.ok?.protocols?.includes(1), `${subject} Herdr driver protocol mismatch`);
}

function macOsTool(name, cwd, inspect = runPackedCommand) {
  // Resolve outside the product's isolated HOME to avoid Xcode shim diagnostics.
  const tool = inspect('/usr/bin/xcrun', ['--find', name], {
    cwd,
    env: process.env,
  }).trim();
  assert(tool, `macOS ${name} path is empty`);
  return tool;
}

/** Inspect bytes, not process.arch: Rosetta can execute an accidental arm64 binary. */
export function assertMacOsArchitecture(executable, target, options, inspect = runPackedCommand) {
  if (!target.endsWith('-apple-darwin')) return;
  const expected = {
    'x86_64-apple-darwin': 'x86_64',
    'aarch64-apple-darwin': 'arm64',
  }[target];
  assert(expected, 'Unsupported macOS verification target');
  const lipo = macOsTool('lipo', options.cwd, inspect);
  const actual = inspect(lipo, ['-archs', executable], options).trim();
  assert.equal(actual, expected, `Executable must contain exactly ${expected}: ${executable}`);
}

function verifyLinkage(executable, cwd, env, subject) {
  if (process.platform === 'darwin') {
    const otool = macOsTool('otool', cwd);
    let libraries;
    try {
      libraries = runPackedCommand(otool, ['-L', executable], {
        cwd,
        env,
      });
    } catch (error) {
      throw new Error(`${subject} linkage inspection failed`, { cause: error });
    }
    const dependencies = libraries.trim().split('\n').slice(1);
    assert(dependencies.length > 0, 'Mach-O dependency inventory is empty');
    for (const dependency of dependencies) {
      assert(
        /^\s+(\/usr\/lib\/|\/System\/Library\/Frameworks\/)/.test(dependency),
        `Native archive requires a non-system library: ${dependency}`
      );
    }
    return;
  }

  const readelf = ['/usr/bin/readelf', '/bin/readelf'].find((candidate) =>
    fs.existsSync(candidate)
  );
  assert(readelf, 'Linux runtime verification requires readelf');
  let headers;
  try {
    headers = runPackedCommand(readelf, ['--program-headers', executable], { cwd, env });
  } catch (error) {
    throw new Error(`${subject} linkage inspection failed`, { cause: error });
  }
  assert(!/\bINTERP\b/.test(headers), 'Musl archive must not require a dynamic interpreter');
  let dynamic;
  try {
    dynamic = runPackedCommand(readelf, ['--dynamic', executable], { cwd, env });
  } catch (error) {
    throw new Error(`${subject} linkage inspection failed`, { cause: error });
  }
  assert(!/\(NEEDED\)/.test(dynamic), 'Musl archive must not require shared libraries');
}

/**
 * Prove the runtime behavior shared by raw native binaries and extracted archives.
 * Archive inventory, checksums, notices and license checks remain in their callers.
 */
function requestOfficeControl(receipt, route) {
  return new Promise((resolve, reject) => {
    const request = http.request(
      {
        host: '127.0.0.1',
        port: receipt.port,
        path: route,
        method: 'POST',
        headers: {
          Host: `127.0.0.1:${receipt.port}`,
          Authorization: `Bearer ${receipt.controlToken}`,
          'X-TMT-Office-Nonce': receipt.nonce,
          'Content-Length': '0',
        },
        timeout: 2_000,
      },
      (response) => {
        response.resume();
        response.on('end', () => resolve(response.statusCode));
      }
    );
    request.on('timeout', () => request.destroy(new Error('Office control request timed out')));
    request.on('error', reject);
    request.end();
  });
}

function waitForOfficeReady(child) {
  return new Promise((resolve, reject) => {
    let stdout = '';
    let stderr = '';
    let done = false;
    let timeout;
    const finish = (callback) => {
      if (done) return;
      done = true;
      clearTimeout(timeout);
      try {
        callback();
      } catch (error) {
        reject(error);
      }
    };
    timeout = setTimeout(
      () => finish(() => reject(new Error('Office service readiness timed out'))),
      5_000
    );
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
      if (stdout.length > 4_096) {
        finish(() => reject(new Error('Office readiness output is too large')));
        return;
      }
      if (stdout.includes('\n')) {
        finish(() => {
          assert.equal(stdout, 'TMT-OFFICE-SERVICE/1 READY\n');
          resolve();
        });
      }
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
      if (stderr.length > 4_096)
        finish(() => reject(new Error('Office readiness diagnostics are too large')));
    });
    child.once('exit', (status, signal) =>
      finish(() =>
        reject(new Error(`Office service exited before readiness: ${status}/${signal}: ${stderr}`))
      )
    );
    child.once('error', (error) => finish(() => reject(error)));
  });
}

function waitForExit(child) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('Office service shutdown timed out')), 5_000);
    child.once('exit', (status, signal) => {
      clearTimeout(timeout);
      try {
        assert.equal(signal, null, `Office service terminated: ${signal}`);
        assert.equal(status, 0, 'Office service shutdown failed');
        resolve();
      } catch (error) {
        reject(error);
      }
    });
  });
}

async function terminateOwnedChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolve) => child.once('exit', resolve));
  try {
    process.kill(-child.pid, 'SIGKILL');
  } catch (error) {
    if (error.code !== 'ESRCH') throw error;
  }
  await Promise.race([
    exited,
    new Promise((_, reject) =>
      setTimeout(() => reject(new Error('Office service cleanup timed out')), 5_000)
    ),
  ]);
}

export async function verifyNativeRuntime({
  executable,
  target,
  version,
  skill,
  inboxSkill,
  officeSkill,
  opsSkill,
  squadSkill,
  colabSkill,
  profileContent,
  subject,
  product = 'cli',
  herdrDriver,
  herdrVersion,
  colabApp,
  notices,
  matchingHostMessage = `${subject} requires a matching native host`,
}) {
  assert(fs.statSync(executable).isFile(), `${subject} must be a regular file`);
  fs.accessSync(executable, fs.constants.X_OK);
  assertNativeTarget(target, matchingHostMessage);
  assert(version.length > 0, 'Native runtime version must not be empty');

  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tmt installed proof's ")));
  try {
    const home = path.join(root, 'home');
    const cwd = path.join(root, 'outside checkout');
    const xdg = path.join(root, 'config');
    const emptyPath = path.join(root, 'empty-path');
    for (const directory of [home, cwd, emptyPath]) fs.mkdirSync(directory);
    // An allowlist avoids ambient provider, tmux, loader and runtime overrides.
    const env = { HOME: home, XDG_CONFIG_HOME: xdg, PATH: emptyPath, LANG: 'C', TMPDIR: root };
    assertMacOsArchitecture(executable, target, { cwd, env });
    verifyLinkage(executable, cwd, env, subject);

    const run = (args) => runPackedCommand(executable, args, { cwd, env });
    assert(
      ['cli', 'office', 'ops', 'squad', 'driver-herdr', 'remote', 'colab'].includes(product),
      'Unknown native runtime product'
    );
    const proveHerdr = (driver, expectedVersion) => {
      assert.equal(
        typeof expectedVersion,
        'string',
        'Herdr proof requires its independent version'
      );
      assertHerdrCapabilities(
        runPackedCommand(driver, ['__tmt-driver', '1', 'capabilities'], { cwd, env }),
        expectedVersion,
        subject
      );
    };
    if (product === 'driver-herdr') {
      proveHerdr(executable, version);
      assert(!fs.existsSync(xdg), 'Driver probe must not initialize config state');
      assert.deepEqual(fs.readdirSync(home), [], 'Driver probe must not create home state');
      assert.deepEqual(fs.readdirSync(cwd), [], 'Driver probe must not create workspace state');
      return;
    }
    if (product === 'remote') {
      assert.equal(run(['--version']).trim(), `remote ${version}`, `${subject} version mismatch`);
      const help = run(['--help']);
      for (const command of ['serve', 'pair', 'devices'])
        assert(new RegExp(`^ {2}${command} +`, 'm').test(help), `${subject} missing ${command}`);
      assert(!fs.existsSync(xdg), 'Remote probes must not initialize config state');
      assert.deepEqual(fs.readdirSync(home), [], 'Remote probes must not create home state');
      assert.deepEqual(fs.readdirSync(cwd), [], 'Remote probes must not create workspace state');
      return;
    }
    if (product === 'colab') {
      assert.equal(typeof colabSkill, 'string', 'Colab runtime proof requires its skill');
      assert.equal(run(['skill']), colabSkill, `${subject} embedded skill mismatch`);
      assert(!fs.existsSync(xdg), 'Colab skill probe must not initialize config state');
      assert.deepEqual(fs.readdirSync(home), [], 'Colab skill probe must not create home state');
      assert.deepEqual(
        fs.readdirSync(cwd),
        [],
        'Colab skill probe must not create workspace state'
      );
      assert(colabApp, 'Colab archive proof requires independent expected app bytes');
      // Other products retain the minimal verifier image's existing dependency closure.
      const { verifyColabApp } = await import('./colab-runtime-proof.mjs');
      await verifyColabApp({ executable, version, expectedApp: colabApp, notices });
      return;
    }
    // Both current Ops and published predecessor Squad archives use these read-only probes.
    if (product === 'ops' || product === 'squad') {
      const productSkill = product === 'ops' ? opsSkill : squadSkill;
      const name = product === 'ops' ? 'Ops' : 'Squad';
      assert.equal(typeof productSkill, 'string', `${name} runtime proof requires its skill`);
      assert.equal(run(['--version']), `${product} ${version}\n`, `${subject} version mismatch`);
      assert.equal(run(['skill', 'show']), productSkill, `${subject} embedded skill mismatch`);
      assert(!fs.existsSync(xdg), `${name} proof must not initialize config state`);
      assert.deepEqual(fs.readdirSync(home), [], `${name} proof must not create home state`);
      assert.deepEqual(fs.readdirSync(cwd), [], `${name} proof must not create workspace state`);
      return;
    }
    if (product === 'office') {
      assert.equal(
        run(['__tmt-office', '1', 'probe']),
        `TMT-OFFICE/1\n${version}\n`,
        `${subject} handshake mismatch`
      );
      assert.equal(
        run(['__tmt-office-service', '1', 'asset-probe']),
        'TMT-OFFICE-LOCAL/1\n',
        `${subject} embedded local Office proof mismatch`
      );
      assert(!fs.existsSync(xdg), 'Office probe must not initialize config state');
      assert.deepEqual(fs.readdirSync(home), [], 'Office probe must not create home state');
      assert.deepEqual(fs.readdirSync(cwd), [], 'Office probe must not create workspace state');
      const child = spawn(executable, ['__tmt-office-service', '1', 'serve'], {
        cwd,
        env: { ...env, TMT_OFFICE_SERVICE_PORT: '0' },
        detached: true,
        stdio: ['ignore', 'pipe', 'pipe'],
      });
      try {
        await waitForOfficeReady(child);
        const receiptPath = path.join(xdg, 'tmux-team', 'office', 'runtime', 'service-v1.json');
        const receipt = JSON.parse(fs.readFileSync(receiptPath, 'utf8'));
        assert.equal(receipt.runningVersion, version, 'Office service version mismatch');
        assert.equal(
          await requestOfficeControl(receipt, '/control/v1/health'),
          200,
          'Office service health authentication failed'
        );
        const exited = waitForExit(child);
        assert.equal(
          await requestOfficeControl(receipt, '/control/v1/stop'),
          200,
          'Office service stop authentication failed'
        );
        await exited;
        assert(!fs.existsSync(receiptPath), 'Office service receipt was not cleaned up');
      } finally {
        await terminateOwnedChild(child);
      }
      return;
    }
    if (herdrDriver !== undefined) {
      // The companion remains independently versioned while #1084 keeps it in CLI archives.
      assertMacOsArchitecture(herdrDriver, target, { cwd, env });
      verifyLinkage(herdrDriver, cwd, env, `${subject} Herdr driver`);
      proveHerdr(herdrDriver, herdrVersion);
    }
    assert.equal(typeof skill, 'string', 'CLI runtime proof requires the canonical skill');
    assert.equal(typeof inboxSkill, 'string', 'CLI runtime proof requires the inbox skill');
    assert.equal(typeof officeSkill, 'string', 'CLI runtime proof requires the Office skill');
    const json = (args) => JSON.parse(run([...args, '--json']));
    const globalRoot = path.join(xdg, 'tmux-team');
    assert.equal(run(['--version']).trim(), version, `${subject} version mismatch`);
    assertBenchmarkHelp(run(['--help']));
    assert.equal(run(['learn', '--skill']), skill, `${subject} embedded skill mismatch`);
    assert.equal(
      run(['learn', '--skill', 'tmt-office']),
      officeSkill,
      `${subject} embedded Office skill mismatch`
    );
    assert(!fs.existsSync(xdg), 'Read-only runtime commands must not initialize config state');
    const customRoot = path.join(cwd, 'custom skills');
    fs.mkdirSync(customRoot);
    const targetRoot = fs.realpathSync(customRoot);
    const targetPath = path.join(targetRoot, 'tmux-team');
    const inboxTargetPath = path.join(targetRoot, 'tmt-inbox');
    const officeTargetPath = path.join(targetRoot, 'tmt-office');
    const installed = [
      { skill: 'tmux-team', target: targetPath, content: skill },
      { skill: 'tmt-inbox', target: inboxTargetPath, content: inboxSkill },
    ];
    assert.deepEqual(json(['install', '--dir', customRoot]), {
      installed: installed.map(({ skill: name, target }) => ({
        skill: name,
        target,
        changed: true,
      })),
    });
    assert(
      !fs.existsSync(officeTargetPath),
      'Core skill installation must not expose optional Office guidance'
    );
    const managedAssets = path.join(fs.realpathSync(globalRoot), 'skill-assets');
    const sources = installed.map(({ skill: name, target, content }) => {
      assert(fs.lstatSync(target).isSymbolicLink(), `${name} must be a managed link`);
      assert.equal(fs.readFileSync(path.join(target, 'SKILL.md'), 'utf8'), content);
      const source = fs.realpathSync(target);
      assert(
        source.startsWith(`${managedAssets}${path.sep}`),
        `${name} must use the isolated managed asset store`
      );
      assert.equal(path.basename(source), name, `${name} source name mismatch`);
      assert.match(path.basename(path.dirname(source)), /^[0-9a-f]{64}$/, 'Bundle digest mismatch');
      return source;
    });
    assert.equal(
      path.dirname(sources[0]),
      path.dirname(sources[1]),
      'Installed skills must share one verified bundle digest'
    );
    assert.deepEqual(
      JSON.parse(fs.readFileSync(path.join(globalRoot, 'skill-installations.json'))),
      {
        version: 1,
        targets: [inboxTargetPath, targetPath],
      }
    );
    assert.deepEqual(json(['install', '--dir', customRoot]), {
      installed: installed.map(({ skill: name, target }) => ({
        skill: name,
        target,
        changed: false,
      })),
    });
    assert.deepEqual(json(['__native-refresh-skills']), {
      refreshed: [
        { target: inboxTargetPath, changed: false },
        { target: targetPath, changed: false },
      ],
      skipped: [],
      conflicts: [],
    });
    assert.deepEqual(
      installed.map(({ target }) => fs.realpathSync(target)),
      sources,
      'Repeat install and refresh must preserve exact bundle ownership'
    );
    const database = path.join(globalRoot, 'tmux-team.db');
    assert(!fs.existsSync(database), 'Skill installation must not initialize SQLite');
    const created = json(['identity', 'create', 'artifact-proof']);
    assert.equal(created.created, true);
    assert.equal(created.identity.name, 'artifact-proof');
    assert.equal(created.identity.lifetime, 'saved');
    const shown = json(['identity', 'show', 'artifact-proof']);
    assert.deepEqual(shown.identity, created.identity);
    const profile = json(['role', '--identity', 'artifact-proof', 'set', profileContent]);
    assert.equal(profile.role.content, profileContent);
    assert.deepEqual(json(['role', '--identity', 'artifact-proof', 'show']), profile);
    const descriptor = fs.openSync(database, 'r');
    try {
      const header = Buffer.alloc(16);
      assert.equal(fs.readSync(descriptor, header, 0, 16, 0), 16);
      assert.equal(header.toString(), 'SQLite format 3\0');
    } finally {
      fs.closeSync(descriptor);
    }
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}
