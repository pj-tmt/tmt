// Explicit B4 proof: private real native owner + hosted reader, demo Auth/Firestore only.
// No Docker, Hosting deployment, billing, credential import or production-network fallback.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const app = path.join(root, 'extensions/tmt-colab/typescript/app');
const project = 'demo-tmt-colab-hosted';
const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', ...options });
  assert.equal(result.error, undefined, `${command} did not start`);
  assert.equal(result.status, 0, `${command} failed (signal ${result.signal ?? 'none'})`);
};
const freePort = () =>
  new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });

if (process.argv[2] === '--scenario') {
  assert.equal(process.argv.length, 4);
  assert.equal(process.env.GCLOUD_PROJECT, project);
  for (const name of ['FIREBASE_AUTH_EMULATOR_HOST', 'FIRESTORE_EMULATOR_HOST'])
    assert.match(process.env[name] ?? '', /^127\.0\.0\.1:[1-9][0-9]{0,4}$/);
  run(
    'corepack',
    [
      'pnpm@10.33.0',
      '--dir',
      'typescript',
      '--filter',
      '@tmt/colab-app',
      'exec',
      'playwright',
      'test',
      '--config',
      'playwright.hosted.config.ts',
    ],
    {
      env: { ...process.env, TMT_ACCEPTANCE_BIN_DIR: process.argv[3] },
    },
  );
} else {
  assert.equal(process.argv.length, 2);
  const java = spawnSync('java', ['-version'], { encoding: 'utf8' });
  assert.equal(java.status, 0, 'Java 21 must already be selected');
  assert.match(java.stderr, /version "21[."]/);
  const world = mkdtempSync(path.join(tmpdir(), 'colab-hosted-emulator-'));
  try {
    const home = path.join(world, 'home');
    mkdirSync(home);
    const toolEnv = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: path.join(home, 'config'),
      XDG_DATA_HOME: path.join(home, 'data'),
      XDG_STATE_HOME: path.join(home, 'state'),
      CLOUDSDK_CONFIG: path.join(home, 'gcloud'),
      GOOGLE_CLOUD_PROJECT: project,
      XDG_CACHE_HOME: path.join(home, '.cache'),
      FIREBASE_CLI_DISABLE_UPDATE_CHECK: '1',
    };
    delete toolEnv.FIREBASE_CONFIG;
    delete toolEnv.FIREBASE_TOKEN;
    delete toolEnv.GOOGLE_APPLICATION_CREDENTIALS;
    const version = spawnSync('firebase', ['--version'], { encoding: 'utf8', env: toolEnv });
    assert.equal(version.status, 0, 'firebase-tools must already be installed');
    assert.equal(version.stdout.trim(), '15.29.0', 'use pinned firebase-tools 15.29.0');
    const bins = path.join(world, 'bin');
    mkdirSync(path.join(bins, 'examples'), { recursive: true });
    const auth = `127.0.0.1:${await freePort()}`,
      firestore = `127.0.0.1:${await freePort()}`;
    const env = {
      ...process.env,
      CARGO_BUILD_JOBS: '2',
      TMT_COLAB_APP_DIR: path.join(app, 'dist'),
      TMT_COLAB_HOSTING_DIR: path.join(app, 'dist-hosted'),
    };
    for (const file of ['dist/index.html', 'dist-hosted/index.html'])
      assert.ok(existsSync(path.join(app, file)), 'build app + hosted assets before the proof');
    // Separate invocations preserve the ordinary Core's production feature set.
    run('cargo', ['build', '--offline', '--locked', '-p', 'tmt-cli', '-p', 'tmt-colab', '--bins'], {
      cwd: path.join(root, 'rust'),
      env,
    });
    const debug = path.join(root, 'rust/target/debug');
    for (const name of ['tmt', 'tmt-colab'])
      copyFileSync(path.join(debug, name), path.join(bins, name));
    run(
      'cargo',
      [
        'build',
        '--offline',
        '--locked',
        '-p',
        'tmt-remote',
        '--bin',
        'tmt-remote',
        '--features',
        'test-emulator',
      ],
      { cwd: path.join(root, 'rust'), env },
    );
    copyFileSync(path.join(debug, 'tmt-remote'), path.join(bins, 'remote-emulator'));
    run(
      'cargo',
      ['build', '--offline', '--locked', '-p', 'tmt-remote', '--example', 'compose_hosting'],
      { cwd: path.join(root, 'rust'), env },
    );
    copyFileSync(
      path.join(debug, 'examples/compose_hosting'),
      path.join(bins, 'examples/compose_hosting'),
    );
    // AcceptanceWorld otherwise passes only its private HOME/XDG/PATH. This test-only wrapper
    // carries the exact loopback endpoints to the default-off native emulator build.
    const binary = path.join(bins, 'remote-emulator');
    assert.ok(!/["`$\\\n\r]/.test(binary), 'safe fixed private path');
    writeFileSync(
      path.join(bins, 'tmt-remote'),
      `#!/bin/sh\nexport FIREBASE_AUTH_EMULATOR_HOST=${auth}\nexport FIRESTORE_EMULATOR_HOST=${firestore}\nexec "${binary}" "$@"\n`,
      { mode: 0o700 },
    );
    const cache = path.join(home, '.cache/firebase/emulators');
    mkdirSync(cache, { recursive: true });
    // Reuse only the pinned tools' cached emulator JAR, never any account/config file.
    const jar = 'cloud-firestore-emulator-v1.22.0.jar';
    const cached = path.join(process.env.HOME ?? '', '.cache/firebase/emulators', jar);
    assert.ok(existsSync(cached), 'cache the emulator JAR with pinned firebase-tools first');
    copyFileSync(cached, path.join(cache, jar));
    const config = path.join(world, 'firebase.json');
    const artifacts = path.join(root, 'extensions/tmt-colab/contracts/vectors');
    writeFileSync(
      config,
      JSON.stringify({
        firestore: {
          rules: path.join(artifacts, 'deploy-declaration-v1.firestore.rules'),
          indexes: path.join(artifacts, 'deploy-declaration-v1.firestore.indexes.json'),
        },
        emulators: {
          auth: { host: '127.0.0.1', port: Number(auth.split(':')[1]) },
          firestore: { host: '127.0.0.1', port: Number(firestore.split(':')[1]) },
          hub: { host: '127.0.0.1', port: await freePort() },
          ui: { enabled: false },
          singleProjectMode: true,
        },
      }),
    );
    const scenario = fileURLToPath(import.meta.url);
    assert.ok(!/["`$\\\n\r]/.test(scenario + bins), 'safe fixed command paths');
    const emulatorEnv = {
      ...toolEnv,
      GCLOUD_PROJECT: project,
      CI: 'true',
      FIREBASE_CLI_DISABLE_UPDATE_CHECK: '1',
    };
    emulatorEnv.PLAYWRIGHT_BROWSERS_PATH =
      process.env.PLAYWRIGHT_BROWSERS_PATH ??
      path.join(
        process.env.HOME ?? '',
        process.platform === 'darwin' ? 'Library/Caches/ms-playwright' : '.cache/ms-playwright',
      );
    delete emulatorEnv.FIREBASE_TOKEN;
    delete emulatorEnv.GOOGLE_APPLICATION_CREDENTIALS;
    // Corepack is a local, already prepared dependency; it need not download into the private HOME.
    emulatorEnv.COREPACK_HOME =
      process.env.COREPACK_HOME ?? path.join(process.env.HOME ?? '', '.cache/node/corepack');
    run(
      'firebase',
      [
        'emulators:exec',
        '--only',
        'auth,firestore',
        '--project',
        project,
        '--config',
        config,
        '--non-interactive',
        `node "${scenario}" --scenario "${bins}"`,
      ],
      { cwd: world, env: emulatorEnv },
    );
  } catch (error) {
    // These demo-emulator logs live in the private world, which must still be removed.
    for (const name of ['firebase-debug.log', 'firestore-debug.log']) {
      try {
        const source = path.join(world, name);
        if (existsSync(source)) {
          const destination = path.join(app, 'test-results/emulators');
          mkdirSync(destination, { recursive: true });
          copyFileSync(source, path.join(destination, name));
        }
      } catch (copyError) {
        console.error(`Could not preserve ${name}: ${String(copyError).slice(0, 200)}`);
      }
    }
    throw error;
  } finally {
    rmSync(world, { recursive: true, force: true });
  }
}
