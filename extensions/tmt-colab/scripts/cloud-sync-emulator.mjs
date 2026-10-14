// Explicit host-emulator proof of Colab's native cloud sync: the real `tmt-remote` (its
// private `test-emulator` build, which routes only to loopback emulators of a demo project),
// real Colab serves, and the Auth and Firestore emulators loaded with Colab's own composed
// Rules. The scenarios live in the ignored `object_channel::cloud_sync::proof` tests.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { runInPrivateWorld } from '../../../typescript/scripts/private-world.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const artifacts = path.join(root, 'extensions/tmt-colab/contracts/vectors');
const project = 'demo-tmt-colab-sync';
const proofs = 'object_channel::cloud_sync::proof::';
const scenarios = 3;
const test = [
  'test',
  '--locked',
  '-p',
  'tmt-colab',
  '--lib',
  proofs,
  '--',
  '--ignored',
  '--test-threads=1',
];
const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, { cwd: path.join(root, 'rust'), stdio: 'inherit', ...options });
  assert.equal(result.error, undefined, `${command} did not start`);
  assert.equal(result.status, 0, `${command} ${args.slice(0, 2).join(' ')} failed`);
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
  // Inside `firebase emulators:exec`: the emulators run and publish their hosts.
  assert.equal(process.env.GCLOUD_PROJECT, project);
  assert.match(process.env.FIREBASE_AUTH_EMULATOR_HOST ?? '', /^127\.0\.0\.1:\d+$/);
  assert.match(process.env.FIRESTORE_EMULATOR_HOST ?? '', /^127\.0\.0\.1:\d+$/);
  const result = runInPrivateWorld({
    argv: ['cargo', ...test],
    base: { ...process.env, TMT_PROOF_PROJECT: project, TMT_PROOF_BIN_DIR: process.argv[3] },
    enforce: true,
    spawn(command, args, options) {
      const result = spawnSync(command, args, {
        ...options,
        cwd: path.join(root, 'rust'),
        encoding: 'utf8',
        stdio: 'pipe',
      });
      process.stdout.write(result.stdout ?? '');
      process.stderr.write(result.stderr ?? '');
      // Both scenarios must have run: a filter that selects nothing is not a pass.
      if (result.status === 0)
        assert.match(
          result.stdout,
          new RegExp(`test result: ok\\. ${scenarios} passed; 0 failed; 0 ignored;`)
        );
      return result;
    },
  });
  assert.equal(result.status, 0, 'the cloud sync proof or private-world cleanup failed');
} else {
  assert.equal(process.argv.length, 2, 'only the explicit scenario mode is supported');
  const start = performance.now();
  const world = mkdtempSync(path.join(tmpdir(), 'colab-sync-emulator-'));
  try {
    // The ordinary executables first, then Remote's proof build, copied apart because both
    // are written to the same target directory.
    const bins = path.join(world, 'bin');
    mkdirSync(bins);
    const target = process.env.CARGO_TARGET_DIR ?? path.join(root, 'rust/target');
    const built = (name) => path.join(path.resolve(root, target), 'debug', name);
    run('cargo', ['build', '--locked', '-p', 'tmt-cli', '-p', 'tmt-colab', '--bins']);
    for (const name of ['tmt', 'tmt-colab']) copyFileSync(built(name), path.join(bins, name));
    run('cargo', [
      'build',
      '--locked',
      '-p',
      'tmt-remote',
      '--bin',
      'tmt-remote',
      '--features',
      'test-emulator',
    ]);
    copyFileSync(built('tmt-remote'), path.join(bins, 'tmt-remote'));
    // Listing first: the proof must be exactly its scenarios, selected by name.
    const listing = spawnSync('cargo', [...test.slice(0, -1), '--list'], {
      cwd: path.join(root, 'rust'),
      encoding: 'utf8',
    });
    assert.equal(listing.status, 0, 'cargo could not list the proof scenarios');
    assert.equal(
      listing.stdout.split('\n').filter((line) => line.startsWith(proofs) && line.endsWith(': test')).length,
      scenarios,
      'select exactly the ignored cloud sync scenarios',
    );
    writeFileSync(
      path.join(world, 'firebase.json'),
      JSON.stringify({
        firestore: {
          rules: path.join(artifacts, 'deploy-declaration-v1.firestore.rules'),
          indexes: path.join(artifacts, 'deploy-declaration-v1.firestore.indexes.json'),
        },
        emulators: {
          auth: { host: '127.0.0.1', port: await freePort() },
          firestore: { host: '127.0.0.1', port: await freePort() },
          hub: { host: '127.0.0.1', port: await freePort() },
          ui: { enabled: false },
          singleProjectMode: true,
        },
      }),
    );
    const env = { ...process.env, GCLOUD_PROJECT: project, CI: 'true' };
    delete env.FIREBASE_TOKEN;
    delete env.GOOGLE_APPLICATION_CREDENTIALS;
    const script = fileURLToPath(import.meta.url);
    // These paths are fixed by the checkout and the temporary directory, never caller text.
    assert.ok(!/["`$\\\n\r]/.test(script + bins), 'safe fixed paths');
    run(
      'firebase',
      [
        'emulators:exec',
        '--only',
        'auth,firestore',
        '--project',
        project,
        '--config',
        path.join(world, 'firebase.json'),
        '--non-interactive',
        `node "${script}" --scenario "${bins}"`,
      ],
      { cwd: world, env },
    );
  } finally {
    rmSync(world, { recursive: true, force: true });
    console.log(`cloud sync emulator proof wall time: ${Math.ceil(performance.now() - start)} ms`);
  }
}
