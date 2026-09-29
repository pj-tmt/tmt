import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { gunzipSync } from 'node:zlib';
import * as tar from 'tar';

const executables = { cli: 'tmt', office: 'tmt-office', squad: 'tmt-squad' };
/** The agent-skills tree an extension archive carries under one directory. */
const skillsRoot = 'skills';
const skillFileLimit = 1024 * 1024;
const skillTreeFileLimit = 16 * 64;

function runtimeFiles(product = 'cli') {
  assert(Object.hasOwn(executables, product), 'Unknown native product');
  return [executables[product], 'LICENSE', 'NATIVE-INSTALL.md', 'THIRD-PARTY-NOTICES.txt'];
}
const compressedLimit = 64 * 1024 * 1024;
const expandedLimit = 128 * 1024 * 1024;

function archiveRootName(name) {
  assert(/^[a-zA-Z0-9][a-zA-Z0-9._-]*\.tar\.gz$/.test(name), 'Invalid native archive name');
  return name.slice(0, -'.tar.gz'.length);
}

export function readBoundedFile(file, limit) {
  const descriptor = fs.openSync(
    file,
    fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK
  );
  try {
    const stat = fs.fstatSync(descriptor);
    assert(stat.isFile() && stat.size <= limit, 'Artifact input must be a bounded regular file');
    const bytes = Buffer.alloc(stat.size + 1);
    let length = 0;
    while (length < bytes.length) {
      const count = fs.readSync(descriptor, bytes, length, bytes.length - length, null);
      if (!count) break;
      length += count;
    }
    assert.equal(length, stat.size, 'Artifact input changed while reading');
    return bytes.subarray(0, length);
  } finally {
    fs.closeSync(descriptor);
  }
}

/** Consume cargo-dist metadata; do not maintain a second checksum/version catalog. */
export function selectNativeArtifact(manifestFile, archiveFile, target, product = 'cli') {
  const requiredFiles = runtimeFiles(product);
  const manifest = JSON.parse(readBoundedFile(manifestFile, 4 * 1024 * 1024));
  const name = path.basename(archiveFile);
  archiveRootName(name);
  const artifact = manifest.artifacts?.[name];
  assert.equal(artifact?.kind, 'executable-zip', 'Manifest is missing the native archive');
  assert.equal(artifact.name, name, 'Manifest archive name mismatch');
  assert.deepEqual(artifact.target_triples, [target], 'Manifest target mismatch');
  assert(/^[a-f0-9]{64}$/.test(artifact.checksums?.sha256), 'Manifest requires SHA-256');
  const releases = manifest.releases?.filter((release) => release.artifacts.includes(name));
  assert.equal(releases?.length, 1, 'Archive must belong to exactly one release');
  assert.equal(releases[0].app_name, `tmt-${product}`, 'Archive must belong to the TMT release');
  const version = releases[0].app_version;
  assert(typeof version === 'string' && version.length > 0, 'Manifest requires a version');
  // cargo-dist declares an included directory as one asset named after it.
  const declared = product === 'squad' ? [skillsRoot] : [];
  assert.deepEqual(
    artifact.assets.map((asset) => asset.path).sort(),
    [...requiredFiles, ...declared].sort(),
    'Manifest must describe exactly the native runtime files'
  );
  return {
    name,
    version,
    target,
    sha256: artifact.checksums.sha256,
    ...(product === 'cli' ? {} : { product }),
    ...(declared.length > 0 ? { skills: true } : {}),
  };
}

/** Extract only verified regular files into an owned directory and always remove it. */
export async function withNativeArtifact(archiveFile, metadata, inspect) {
  const requiredFiles = runtimeFiles(metadata.product);
  const rootName = archiveRootName(metadata.name);
  const compressed = readBoundedFile(archiveFile, compressedLimit);
  assert.equal(
    createHash('sha256').update(compressed).digest('hex'),
    metadata.sha256,
    'Native archive checksum mismatch'
  );
  // Bound the whole stream, including metadata and padding, before tar parsing.
  const expanded = gunzipSync(compressed, { maxOutputLength: expandedLimit });
  const staging = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt native archive '));
  try {
    const snapshot = path.join(staging, 'snapshot.tar');
    fs.writeFileSync(snapshot, expanded, { flag: 'wx', mode: 0o600 });
    const expected = new Set(requiredFiles.map((name) => `${rootName}/${name}`));
    const tree = `${rootName}/${skillsRoot}`;
    const seen = new Set();
    let skillFiles = 0;
    tar.t({
      file: snapshot,
      sync: true,
      strict: true,
      onReadEntry(entry) {
        assert(!seen.has(entry.path), `Duplicate native archive entry: ${entry.path}`);
        seen.add(entry.path);
        if (entry.path === `${rootName}/` && entry.type === 'Directory') return;
        const inTree =
          metadata.skills && (entry.path === tree || entry.path.startsWith(`${tree}/`));
        if (inTree && entry.type === 'Directory') {
          assert.equal(entry.mode & 0o7000, 0, 'Skill directories must not set special bits');
          return;
        }
        if (inTree) {
          skillFiles += 1;
          assert(skillFiles <= skillTreeFileLimit, 'Native archive carries too many skill files');
          assert(entry.size <= skillFileLimit, `Skill file exceeds its bound: ${entry.path}`);
        }
        assert(
          expected.has(entry.path) || inTree,
          `Unexpected native archive entry: ${entry.path}`
        );
        assert.equal(entry.type, 'File', `Native archive entry must be regular: ${entry.path}`);
        assert.equal(entry.mode & 0o7000, 0, 'Native archive must not set special permission bits');
        assert(entry.size > 0, `Empty native archive entry: ${entry.path}`);
        if (entry.path === `${rootName}/${requiredFiles[0]}`) {
          assert(entry.mode & 0o111, 'Native executable lacks execute permission');
        }
      },
    });
    for (const entry of expected) assert(seen.has(entry), `Missing native archive entry: ${entry}`);
    assert(!metadata.skills || skillFiles > 0, 'Native archive declares skills but carries none');
    const extracted = path.join(staging, 'extracted');
    fs.mkdirSync(extracted);
    tar.x({ file: snapshot, cwd: extracted, sync: true, strict: true, preserveOwner: false });
    return await inspect(path.join(extracted, rootName));
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }
}
