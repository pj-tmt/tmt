// One slot-granted Auth+Firestore world, exact Colab bytes, no cloud credentials or Docker reset.
import { execFileSync, spawn } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, chmod, statfs } from 'node:fs/promises';
import { createWriteStream } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';
import source from './colab-source.json' with { type: 'json' };

const leaf = fileURLToPath(new URL('../..', import.meta.url));
const root = path.resolve(leaf, '../../../..');
const colabHead = source.head;
assert.equal(
  source.status,
  'accepted',
  'wait for Colab primary to accept the replacement source head',
);
const git = (...args) => execFileSync('git', ['-C', root, ...args], { encoding: 'utf8' }).trim();
const head = git('rev-parse', 'HEAD');
assert.equal(
  process.env.REMOTE_SYNC_INFRA_GRANTED,
  '1',
  'Infra must explicitly grant the slot for this head',
);
assert.equal(
  process.env.REMOTE_SYNC_HEAD,
  head,
  'slot and evidence must name the exact Remote head',
);
assert.equal(
  git('status', '--porcelain'),
  '',
  'qualifying source tree must be committed and clean',
);
const disk = await statfs('/private/tmp');
assert.ok(disk.bavail * disk.bsize >= 30 * 1024 ** 3, '30 GiB disk floor');
const evidence = await mkdtemp('/private/tmp/remote-2694-qualifying-');
const world = path.join(evidence, 'world'),
  artifacts = path.join(world, 'artifacts'),
  output = path.join(world, 'output');
await mkdir(artifacts, { recursive: true });
await mkdir(output);
await chmod(evidence, 0o700);
await chmod(world, 0o755);
await chmod(artifacts, 0o755);
await chmod(output, 0o777);
const show = (file) => execFileSync('git', ['-C', root, 'show', `${colabHead}:${file}`]);
const names = [
  'deploy-declaration-v1.json',
  'deploy-declaration-v1.firestore.rules',
  'deploy-declaration-v1.firestore.indexes.json',
];
const digests = {};
for (const name of names) {
  const bytes = show(`extensions/tmt-colab/contracts/vectors/${name}`);
  await writeFile(path.join(artifacts, name), bytes);
  digests[name] = createHash('sha256').update(bytes).digest('hex');
}
await writeFile(
  path.join(world, 'Dockerfile'),
  show('extensions/tmt-colab/rust/tmt-colab/tests/emulator/Dockerfile'),
);
await writeFile(
  path.join(world, 'firebase.json'),
  JSON.stringify(
    {
      firestore: {
        rules: '/world/artifacts/deploy-declaration-v1.firestore.rules',
        indexes: '/world/artifacts/deploy-declaration-v1.firestore.indexes.json',
      },
      emulators: {
        auth: { host: '0.0.0.0', port: 9099 },
        firestore: { host: '0.0.0.0', port: 8085 },
        hub: { host: '0.0.0.0', port: 4400 },
        ui: { enabled: false },
        singleProjectMode: true,
      },
    },
    null,
    2,
  ),
);
// Normal asset builds clear dist; make this private entry inside the qualifying run.
const privateBuild = execFileSync(
  process.execPath,
  [
    path.join(root, 'typescript/node_modules/vite-plus/bin/vp'),
    'build',
    leaf,
    '--config',
    path.join(leaf, 'test/emulator/vite.config.ts'),
  ],
  { cwd: leaf, encoding: 'utf8' },
);
await writeFile(path.join(evidence, 'private-build.log'), privateBuild);
const tag = `tmt-remote-1-2694-${head.slice(0, 8)}-${process.pid}`;
const imageList = () =>
  execFileSync('docker', ['images', '--no-trunc', '--format', '{{json .}}'], { encoding: 'utf8' });
await writeFile(path.join(evidence, 'images-before.jsonl'), imageList());
await writeFile(
  path.join(evidence, 'manifest.json'),
  JSON.stringify(
    { head, colabHead, tag, artifacts: digests, diskFreeBytes: disk.bavail * disk.bsize },
    null,
    2,
  ),
);
console.log(`Evidence: ${evidence}`);
const run = (binary, args, log, options = {}) =>
  new Promise((resolve, reject) => {
    const stream = createWriteStream(path.join(evidence, log));
    const child = spawn(binary, args, { cwd: leaf, ...options, stdio: ['ignore', 'pipe', 'pipe'] });
    child.stdout.pipe(stream, { end: false });
    child.stderr.pipe(stream, { end: false });
    child.once('error', reject);
    child.once('close', (code) => {
      stream.end();
      if (code === 0) resolve();
      else reject(Error(`${log}: exit ${code}`));
    });
  });
let emulator;
try {
  await run(
    'docker',
    ['build', '--tag', tag, '--file', path.join(world, 'Dockerfile'), world],
    'image-build.log',
  );
  const log = createWriteStream(path.join(evidence, 'emulators.log'));
  emulator = spawn(
    'docker',
    [
      'run',
      '--rm',
      '--init',
      '--name',
      tag,
      '-p',
      '127.0.0.1:8085:8085',
      '-p',
      '127.0.0.1:9099:9099',
      '-v',
      `${world}:/world:ro`,
      '-v',
      `${output}:/out:rw`,
      '-w',
      '/out',
      tag,
      'firebase',
      'emulators:start',
      '--only',
      'auth,firestore',
      '--project',
      'demo-tmt-colab',
      '--config',
      '/world/firebase.json',
      '--non-interactive',
    ],
    { stdio: ['ignore', 'pipe', 'pipe'] },
  );
  await new Promise((resolve, reject) => {
    let text = '';
    const timer = setTimeout(() => reject(Error('bounded emulator readiness expired')), 60000);
    const data = (chunk) => {
      log.write(chunk);
      text = (text + chunk.toString()).slice(-8192);
      if (text.includes('All emulators ready')) {
        clearTimeout(timer);
        resolve();
      }
    };
    emulator.stdout.on('data', data);
    emulator.stderr.on('data', data);
    emulator.once('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    emulator.once('close', (code) => {
      clearTimeout(timer);
      log.end();
      reject(Error(`emulator exited ${code}`));
    });
  });
  await run(
    process.execPath,
    [
      path.join(leaf, 'node_modules/@playwright/test/cli.js'),
      'test',
      '--config',
      'test/emulator/playwright.config.ts',
    ],
    'browser.log',
    {
      env: {
        ...process.env,
        REMOTE_SYNC_COLAB_HEAD: colabHead,
        REMOTE_SYNC_COLAB_ARTIFACTS: artifacts,
        REMOTE_SYNC_AUTH_PORT: '9099',
        REMOTE_SYNC_FIRESTORE_PORT: '8085',
      },
    },
  );
  await writeFile(
    path.join(evidence, 'result.json'),
    JSON.stringify({ head, colabHead, result: 'pass', cases: 1 }),
  );
} finally {
  const absent = (kind) => {
    try {
      execFileSync('docker', [kind, 'inspect', tag], { stdio: 'pipe', timeout: 15000 });
      return false;
    } catch (error) {
      if (error.status === 1 && /No such (container|image|object)/i.test(String(error.stderr)))
        return true;
      throw error; // Unreachable Docker is not evidence of successful cleanup.
    }
  };
  const containerCommand = (args) => {
    try {
      execFileSync('docker', args, { stdio: 'pipe', timeout: 20000 });
    } catch (error) {
      if (!absent('container')) throw error; // --rm may win the race between checks.
    }
  };
  if (emulator && !absent('container')) {
    containerCommand(['stop', '--time', '10', tag]);
    // The --rm runner may still be removing its container after stop acknowledges exit.
    if (!absent('container')) {
      containerCommand(['wait', tag]);
      if (!absent('container')) containerCommand(['container', 'rm', '-f', tag]);
    }
    assert.ok(absent('container'), `owned container remains: ${tag}`);
  }
  const removeImage = () => {
    try {
      execFileSync('docker', ['image', 'rm', tag], { stdio: 'pipe', timeout: 15000 });
    } catch (error) {
      console.error(`Image cleanup attempt failed for ${tag}: ${String(error.stderr ?? error)}`);
    }
  };
  if (!absent('image')) {
    removeImage();
    if (!absent('image')) removeImage(); // One retry after container teardown, never a loop.
    assert.ok(absent('image'), `owned image remains after cleanup retry: ${tag}`);
  }
  await writeFile(path.join(evidence, 'images-after.jsonl'), imageList());
  console.log(`released: ${evidence}; compare images-before/after for any first-time base pull`);
}
