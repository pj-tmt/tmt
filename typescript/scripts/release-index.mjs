// Release records bind GitHub's real release identity to the already verified cargo-dist bytes.
// Index branch publication belongs to a later, separately authorized writer (#2373).
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { readBoundedFile, selectNativeArtifact } from './native-artifact-policy.mjs';
import { archivePrefix, releasePolicy } from './native-release-policy.mjs';
import { versionOfTag } from './release-versions.mjs';

export const RELEASE_RECORD = 'tmt-release-record.json';
export const RECORD_LIMIT = 256 * 1024;
export const POINTER_LIMIT = 16 * 1024;
const RELEASE_BASE = 'https://github.com/pj-tmt/tmt/releases/download/';
const INDEX_BASE = 'https://raw.githubusercontent.com/pj-tmt/tmt/release-index/';
const TARGETS = [
  'aarch64-apple-darwin',
  'aarch64-unknown-linux-musl',
  'x86_64-apple-darwin',
  'x86_64-unknown-linux-musl',
];
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
const object = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

function identity(value) {
  assert(object(value), 'Release index requires an object');
  assert.equal(value.schemaVersion, 1, 'Unsupported release-index schema; rerun install.sh');
  const policy = releasePolicy(value.product);
  assert(typeof value.version === 'string', 'Release index requires a version');
  const parts = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-alpha\.(0|[1-9]\d*))?$/.exec(
    value.version
  );
  assert(
    parts &&
      parts
        .slice(1)
        .filter((part) => part !== undefined)
        .every((part) => Number.isSafeInteger(Number(part))),
    'Release index requires a canonical stable or alpha version'
  );
  assert.equal(
    value.tag,
    `${policy.tagPrefix}${value.version}`,
    'Release index tag/version mismatch'
  );
}

function asset(value, name, tag, limit) {
  assert(object(value), 'Release index requires an asset');
  assert.equal(value.name, name, 'Release index asset name mismatch');
  assert.equal(value.url, `${RELEASE_BASE}${tag}/${name}`, 'Release index asset URL mismatch');
  assert(
    Number.isSafeInteger(value.size) && value.size > 0 && value.size <= limit,
    'Release index asset size outside bounds'
  );
  assert(
    typeof value.sha256 === 'string' && /^[a-f0-9]{64}$/.test(value.sha256),
    'Release index requires SHA-256'
  );
}

/** Required v1 fields are validated; additive fields are intentionally ignored. */
export function validateReleaseRecord(record) {
  identity(record);
  assert(
    Number.isSafeInteger(record.releaseId) && record.releaseId > 0,
    'Release index requires a positive safe release ID'
  );
  assert(
    typeof record.sourceSha === 'string' && /^[a-f0-9]{40}$/.test(record.sourceSha),
    'Release index requires a source SHA'
  );
  asset(record.manifest, 'dist-manifest.json', record.tag, 4 * 1024 * 1024);
  assert(
    object(record.archives) && Object.keys(record.archives).length === 4,
    'Release index requires four native targets'
  );
  for (const [target, archive] of Object.entries(record.archives)) {
    assert(TARGETS.includes(target), 'Unsupported release-index target');
    asset(
      archive,
      `${archivePrefix(record.product)}-${target}.tar.gz`,
      record.tag,
      64 * 1024 * 1024
    );
  }
  return record;
}

function parse(bytes, limit) {
  assert(Buffer.byteLength(bytes) <= limit, 'Release index document exceeds its byte cap');
  return JSON.parse(bytes.toString());
}

export function parseReleaseRecord(bytes) {
  return validateReleaseRecord(parse(bytes, RECORD_LIMIT));
}

/** Pointer validation is inert; PR A has no index writer or client transport. */
export function parseChannelPointer(bytes) {
  const pointer = parse(bytes, POINTER_LIMIT);
  identity(pointer);
  assert.equal(
    pointer.channel,
    pointer.version.includes('-alpha.') ? 'alpha' : 'stable',
    'Release index channel/version mismatch'
  );
  assert(object(pointer.record), 'Release index pointer requires a record');
  assert(
    [
      `${RELEASE_BASE}${pointer.tag}/${RELEASE_RECORD}`,
      `${INDEX_BASE}records/${pointer.tag}.json`,
    ].includes(pointer.record.url),
    'Release index record URL mismatch'
  );
  assert(
    Number.isSafeInteger(pointer.record.size) &&
      pointer.record.size > 0 &&
      pointer.record.size <= RECORD_LIMIT,
    'Release index record size outside bounds'
  );
  assert(
    typeof pointer.record.sha256 === 'string' && /^[a-f0-9]{64}$/.test(pointer.record.sha256),
    'Release index requires record SHA-256'
  );
  return pointer;
}

/** Reuses the bounded native manifest/selection owner; does not extract or execute archives. */
export function createReleaseRecord({ product, tag, releaseId, sourceSha, directory }) {
  const manifestFile = path.join(directory, 'dist-manifest.json');
  const bytes = readBoundedFile(manifestFile, 4 * 1024 * 1024);
  const manifest = JSON.parse(bytes);
  assert.equal(manifest.announcement_tag, tag, 'Release record manifest tag mismatch');
  const describe = (name, content) => ({
    name,
    url: `${RELEASE_BASE}${tag}/${name}`,
    size: content.length,
    sha256: digest(content),
  });
  const archives = {};
  for (const [name, entry] of Object.entries(manifest.artifacts ?? {})) {
    if (entry.kind !== 'executable-zip') continue;
    assert.equal(entry.target_triples?.length, 1, 'Release record requires one target per archive');
    const target = entry.target_triples[0];
    assert(TARGETS.includes(target), 'Unsupported release-index target');
    assert.equal(
      name,
      `${archivePrefix(product)}-${target}.tar.gz`,
      'Release record archive name mismatch'
    );
    assert(!Object.hasOwn(archives, target), 'Release record has duplicate target');
    const file = path.join(directory, name);
    const metadata = selectNativeArtifact(manifestFile, file, target, product);
    assert.equal(
      metadata.version,
      versionOfTag(tag, product),
      'Release record manifest version mismatch'
    );
    const archive = describe(name, readBoundedFile(file, 64 * 1024 * 1024));
    assert.equal(archive.sha256, metadata.sha256, 'Release record archive checksum mismatch');
    archives[target] = archive;
  }
  assert(
    readBoundedFile(manifestFile, 4 * 1024 * 1024).equals(bytes),
    'Release record manifest changed while reading'
  );
  return validateReleaseRecord({
    schemaVersion: 1,
    product,
    version: versionOfTag(tag, product),
    tag,
    releaseId,
    sourceSha,
    manifest: describe('dist-manifest.json', bytes),
    archives: Object.fromEntries(Object.entries(archives).sort(([a], [b]) => a.localeCompare(b))),
  });
}

export function releaseRecordBytes(record) {
  validateReleaseRecord(record);
  const bytes = Buffer.from(`${JSON.stringify(record)}\n`);
  assert(bytes.length <= RECORD_LIMIT, 'Release index document exceeds its byte cap');
  return bytes;
}

/** Compare every required field with fresh verified local bytes; additive fields remain allowed. */
export function verifyReleaseRecord({ recordBytes, ...input }) {
  const actual = parseReleaseRecord(recordBytes);
  const expected = createReleaseRecord(input);
  for (const key of ['schemaVersion', 'product', 'version', 'tag', 'releaseId', 'sourceSha'])
    assert.deepEqual(actual[key], expected[key], `Release record ${key} mismatch`);
  const compare = (actualAsset, expectedAsset) => {
    for (const key of ['name', 'url', 'size', 'sha256'])
      assert.deepEqual(
        actualAsset[key],
        expectedAsset[key],
        `Release record asset ${key} mismatch`
      );
  };
  compare(actual.manifest, expected.manifest);
  assert.deepEqual(
    Object.keys(actual.archives).sort(),
    Object.keys(expected.archives).sort(),
    'Release record target mismatch'
  );
  for (const target of Object.keys(expected.archives))
    compare(actual.archives[target], expected.archives[target]);
  return actual;
}
