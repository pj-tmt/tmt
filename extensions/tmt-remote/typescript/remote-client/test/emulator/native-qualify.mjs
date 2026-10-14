// Explicit host-emulator acceptance: ordinary native Auth/REST, checkout Colab vectors only.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { runInPrivateWorld } from '../../../../../../typescript/scripts/private-world.mjs';

const root = fileURLToPath(new URL('../../../../../../', import.meta.url));
const testName = 'firebase_auth::emulator_tests::native_auth_and_sealed_sync_match_the_colab_rules';
const artifacts = path.join(root, 'extensions/tmt-colab/contracts/vectors');
const cargo = [
  'test',
  '--locked',
  '-p',
  'tmt-remote',
  '--lib',
  testName,
  '--',
  '--ignored',
  '--exact',
];
function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: path.join(root, 'rust'),
    encoding: 'utf8',
    ...options,
  });
  assert.equal(result.error, undefined, `${command} did not start`);
  process.stdout.write(result.stdout ?? '');
  process.stderr.write(result.stderr ?? '');
  assert.equal(result.status, 0, `${command} failed (signal ${result.signal ?? 'none'})`);
  return result.stdout;
}
if (process.argv[2] === '--scenario') {
  assert.equal(process.env.GCLOUD_PROJECT, 'demo-tmt-colab');
  assert.equal(process.env.FIREBASE_AUTH_EMULATOR_HOST, '127.0.0.1:9099');
  assert.equal(process.env.FIRESTORE_EMULATOR_HOST, '127.0.0.1:8085');
  const env = {
    ...process.env,
    REMOTE_SYNC_AUTH_PORT: '9099',
    REMOTE_SYNC_FIRESTORE_PORT: '8085',
    REMOTE_SYNC_COLAB_ARTIFACTS: artifacts,
    REMOTE_SYNC_EXPIRES_AT: new Date(Date.now() + 86400000).toISOString().replace(/\.000Z$/, 'Z'),
  };
  const result = runInPrivateWorld({
    argv: ['cargo', ...cargo, '--nocapture'],
    base: env,
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
      if (result.status === 0)
        assert.match(result.stdout, /test result: ok\. 1 passed; 0 failed; 0 ignored;/);
      return result;
    },
  });
  assert.equal(result.status, 0, 'native emulator scenario or private-world cleanup failed');
} else {
  assert.equal(process.argv.length, 2, 'only the explicit scenario mode is supported');
  const start = performance.now();
  const listing = run('cargo', [...cargo, '--list']);
  assert.equal(
    listing.split('\n').filter((line) => line === `${testName}: test`).length,
    1,
    'select exactly one ignored native emulator test',
  );
  const hashes = {};
  for (const name of [
    'deploy-declaration-v1.json',
    'deploy-declaration-v1.firestore.rules',
    'deploy-declaration-v1.firestore.indexes.json',
  ]) {
    hashes[name] = createHash('sha256')
      .update(readFileSync(path.join(artifacts, name)))
      .digest('hex');
  }
  console.log(`native emulator checkout artifact hashes: ${JSON.stringify(hashes)}`);
  const world = mkdtempSync(path.join(tmpdir(), 'remote-native-emulator-'));
  try {
    writeFileSync(
      path.join(world, 'firebase.json'),
      JSON.stringify({
        firestore: {
          rules: path.join(artifacts, 'deploy-declaration-v1.firestore.rules'),
          indexes: path.join(artifacts, 'deploy-declaration-v1.firestore.indexes.json'),
        },
        emulators: {
          auth: { host: '127.0.0.1', port: 9099 },
          firestore: { host: '127.0.0.1', port: 8085 },
          hub: { host: '127.0.0.1', port: 4400 },
          ui: { enabled: false },
          singleProjectMode: true,
        },
      }),
    );
    const env = { ...process.env, GCLOUD_PROJECT: 'demo-tmt-colab', CI: 'true' };
    delete env.FIREBASE_TOKEN;
    delete env.GOOGLE_APPLICATION_CREDENTIALS;
    const script = fileURLToPath(import.meta.url);
    // This source path is fixed by the checkout, never caller-provided shell text.
    assert.ok(!/["`$\\\n\r]/.test(script), 'safe fixed runner path');
    run(
      'firebase',
      [
        'emulators:exec',
        '--only',
        'auth,firestore',
        '--project',
        'demo-tmt-colab',
        '--config',
        path.join(world, 'firebase.json'),
        '--non-interactive',
        `node "${script}" --scenario`,
      ],
      { cwd: world, env, stdio: 'inherit' },
    );
  } finally {
    rmSync(world, { recursive: true, force: true });
    console.log(`native emulator step wall time: ${Math.ceil(performance.now() - start)} ms`);
  }
}
