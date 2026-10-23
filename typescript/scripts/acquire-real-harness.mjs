import { createHash } from 'node:crypto';
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import { Readable, Transform } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

export function validateHarnessPins(pins) {
  const packages = pins?.packages;
  const expected = [
    ['@anthropic-ai/claude-code', '2.1.285'],
    ['@anthropic-ai/claude-code-linux-x64', '2.1.285'],
    ['@openai/codex', '0.160.0'],
    ['@openai/codex-linux-x64', '0.160.0-linux-x64'],
  ];
  if (
    pins?.schemaVersion !== 1 ||
    packages?.length !== 4 ||
    pins.versions?.claude !== '2.1.285 (Claude Code)' ||
    pins.versions?.codex !== 'codex-cli 0.160.0'
  )
    throw new Error('HARNESS_PINS: unsupported manifest');
  for (const [index, [directory, version]] of expected.entries()) {
    const item = packages[index];
    const name = directory === '@openai/codex-linux-x64' ? '@openai/codex' : directory;
    const basename = name.split('/')[1];
    if (
      item?.installDirectory !== directory ||
      item.name !== name ||
      item.version !== version ||
      item.url !== `https://registry.npmjs.org/${name}/-/${basename}-${version}.tgz` ||
      !/^sha512-[A-Za-z0-9+/]{86}==$/.test(item.integrity)
    )
      throw new Error(`HARNESS_PINS: invalid package ${index}`);
  }
  return pins;
}

/** The injectable HTTP port lets unit tests use bytes, never real provider downloads. */
export async function downloadHarnessArchive(
  item,
  archive,
  { fetchImpl = fetch, maxBytes = 256 * 1024 * 1024 } = {}
) {
  if (fetchImpl === fetch && !existsSync('/.dockerenv'))
    throw new Error('HARNESS_CONTAINER_REQUIRED: provider downloads run only in the image build');
  if (!Number.isSafeInteger(maxBytes) || maxBytes < 1 || maxBytes > 256 * 1024 * 1024)
    throw new Error('HARNESS_DOWNLOAD_LIMIT: invalid byte bound');
  const signal = AbortSignal.timeout(90000);
  const response = await fetchImpl(item.url, { signal, redirect: 'error' });
  if (!response.ok || !response.body) throw new Error(`HARNESS_DOWNLOAD: HTTP ${response.status}`);
  const hash = createHash('sha512');
  let bytes = 0;
  const bounded = new Transform({
    transform(chunk, _encoding, callback) {
      bytes += chunk.length;
      if (bytes > maxBytes)
        return callback(new Error('HARNESS_DOWNLOAD_LIMIT: archive exceeds byte bound'));
      hash.update(chunk);
      callback(null, chunk);
    },
  });
  try {
    await pipeline(
      Readable.fromWeb(response.body),
      bounded,
      createWriteStream(archive, { mode: 0o600 }),
      { signal }
    );
    if (`sha512-${hash.digest('base64')}` !== item.integrity)
      throw new Error('HARNESS_ARCHIVE_INTEGRITY: pinned archive differs');
  } catch (error) {
    rmSync(archive, { force: true });
    throw error;
  }
  return { archiveBytes: bytes };
}

async function acquire(manifest, target) {
  // This entry point cannot fetch or install provider payloads on a developer host.
  if (!existsSync('/.dockerenv'))
    throw new Error('HARNESS_CONTAINER_REQUIRED: downloads run only inside the image build');
  const pins = validateHarnessPins(JSON.parse(readFileSync(manifest, 'utf8')));
  mkdirSync(target, { recursive: true, mode: 0o755 });
  const records = [];
  for (const [index, item] of pins.packages.entries()) {
    const archive = path.join(target, `archive-${index}.tgz`);
    const { archiveBytes } = await downloadHarnessArchive(item, archive);
    const destination = path.join(target, 'node_modules', item.installDirectory);
    mkdirSync(destination, { recursive: true, mode: 0o755 });
    const unpack = spawnSync('tar', ['-xzf', archive, '--strip-components=1', '-C', destination], {
      encoding: 'utf8',
      timeout: 30000,
      maxBuffer: 65536,
    });
    if (unpack.error || unpack.status !== 0)
      throw new Error(
        `HARNESS_UNPACK: package ${index}: ${unpack.error?.message ?? unpack.stderr}`
      );
    rmSync(archive);
    records.push({
      name: item.name,
      version: item.version,
      archiveBytes,
      integrity: item.integrity,
    });
  }
  writeFileSync(
    path.join(target, 'acquisition.json'),
    JSON.stringify({ packages: records }, null, 2) + '\n'
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 4) throw new Error('Usage: acquire-real-harness.mjs MANIFEST TARGET');
  await acquire(process.argv[2], process.argv[3]);
}
