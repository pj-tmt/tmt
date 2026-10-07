// Compiled CLI schema transport; Core owns schema semantics and the export implementation.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { readBoundedFile, selectNativeArtifact } from './native-artifact-policy.mjs';
import { assertNativeTarget } from './native-runtime-proof.mjs';
import { runPackedCommand } from './packed-command.mjs';

export const CLI_SCHEMA_TARGETS = [
  'aarch64-apple-darwin',
  'aarch64-unknown-linux-musl',
  'x86_64-apple-darwin',
  'x86_64-unknown-linux-musl',
];
const MiB = 1024 * 1024;
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
const sha = (value) => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const cut = (value) => typeof value === 'string' && /^[a-f0-9]{40}$/.test(value);
const fields = (value, keys) => {
  assert(value && typeof value === 'object' && !Array.isArray(value), 'Schema object required');
  assert.deepEqual(Object.keys(value).sort(), [...keys].sort(), 'Unknown or missing schema field');
};

/** Exact compact exporter encoding also rejects duplicate keys without a second JSON parser. */
export function parseCompiledSchema(text) {
  assert(typeof text === 'string' && Buffer.byteLength(text) <= MiB, 'Schema output bound');
  const record = JSON.parse(text);
  assert.equal(text, `${JSON.stringify(record)}\n`, 'Expected one compact schema record/newline');
  fields(record, ['schema_version', 'product', 'source_sha', 'databases', 'source_files']);
  assert.equal(record.schema_version, 1, 'Unsupported application schema');
  assert.equal(record.product, 'cli', 'CLI schema only');
  assert(cut(record.source_sha), 'Invalid schema source SHA');
  assert(Array.isArray(record.databases) && record.databases.length === 1, 'CLI domain required');
  fields(record.databases[0], ['domain', 'version']);
  assert.equal(record.databases[0].domain, 'tmt-core-db', 'CLI domain mismatch');
  assert(
    Number.isInteger(record.databases[0].version) &&
      record.databases[0].version >= 0 &&
      record.databases[0].version <= 2_147_483_647,
    'Unknown compiled database version'
  );
  assert(
    Array.isArray(record.source_files) &&
      record.source_files.length > 0 &&
      record.source_files.length <= 64,
    'Compiled source closure bound'
  );
  let previous = '';
  for (const entry of record.source_files) {
    fields(entry, ['path', 'sha256']);
    assert(
      typeof entry.path === 'string' &&
        entry.path.length <= 256 &&
        entry.path
          .split('/')
          .every((part) => /^[A-Za-z0-9_.-]+$/.test(part) && !['.', '..'].includes(part)),
      'Invalid compiled source path'
    );
    assert(
      entry.path > previous && sha(entry.sha256),
      'Duplicate/unsorted source or invalid digest'
    );
    previous = entry.path;
  }
  return record;
}

/** Closed declared inputs, not Rust parsing, module tracing or SQL-derived versions. */
export function assertCompiledSource(record, snapshot, root) {
  assert.equal(snapshot.schema, 1, 'Version snapshot required');
  assert.equal(snapshot.product, 'cli', 'CLI version snapshot required');
  assert(
    cut(snapshot.cut) && snapshot.hashes && typeof snapshot.hashes === 'object',
    'Captured cut required'
  );
  assert.equal(record.source_sha, snapshot.cut, 'Schema differs from captured cut');
  const owner = 'rust/crates/tmt-adapters/src/storage/';
  const names = Object.keys(snapshot.hashes)
    .filter(
      (name) =>
        name === `${owner}migrations.rs` ||
        name === `${owner}migrations/host_names.rs` ||
        name.startsWith(`${owner}schema/`)
    )
    .sort();
  assert(
    names.includes(`${owner}migrations.rs`) && names.includes(`${owner}migrations/host_names.rs`),
    'Missing compiled closure owners'
  );
  assert(
    names.some((name) => name.startsWith(`${owner}schema/`)),
    'Missing compiled SQL input directory'
  );
  assert(
    names.every(
      (name) =>
        !name.startsWith(`${owner}schema/`) ||
        /^rust\/crates\/tmt-adapters\/src\/storage\/schema\/[A-Za-z0-9_.-]+\.sql$/.test(name)
    ),
    'Unknown compiled closure input'
  );
  assert.deepEqual(
    record.source_files,
    names.map((name) => ({ path: name, sha256: snapshot.hashes[name] })),
    'Incomplete or changed compiled source closure'
  );
  for (const entry of record.source_files) {
    assert.equal(
      digest(readBoundedFile(path.join(root, entry.path), 4 * MiB)),
      entry.sha256,
      'Prepared schema source changed beyond captured version-only inputs'
    );
  }
}

function snapshotDigest(snapshot) {
  return digest(
    JSON.stringify({
      cut: snapshot.cut,
      product: snapshot.product,
      tag: snapshot.tag,
      version: snapshot.version,
      hashes: Object.entries(snapshot.hashes).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
    })
  );
}

/** Execute only on an already matching, unprivileged prepare/verification host. */
export function exportCompiledSchema(
  { executable, target, snapshot, root },
  execute = runPackedCommand
) {
  assertNativeTarget(target, 'Schema export requires a matching native host');
  assert.equal(snapshot.product, 'cli', 'CLI preparation required');
  assert(cut(snapshot.cut), 'Captured preparation SHA required');
  const binarySHA = digest(readBoundedFile(executable, 128 * MiB));
  const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-schema-export-'));
  const home = path.join(sandbox, 'home');
  const cwd = path.join(sandbox, 'cwd');
  const emptyPath = path.join(sandbox, 'path');
  [home, cwd, emptyPath].forEach((directory) => fs.mkdirSync(directory));
  const env = {
    HOME: home,
    XDG_CONFIG_HOME: path.join(home, 'config'),
    PATH: emptyPath,
    LANG: 'C',
    TMPDIR: sandbox,
  };
  let success = false;
  try {
    let output;
    let commandError;
    try {
      output = execute(
        path.resolve(executable),
        ['__native-schema', '--source-sha', snapshot.cut, '--json'],
        { cwd, env, timeoutMs: 10_000 }
      );
    } catch (error) {
      commandError = new Error(
        'Prepared CLI schema exporter is unavailable or failed; no legacy preparation fallback',
        { cause: error }
      );
    }
    try {
      assert.deepEqual(
        fs.readdirSync(sandbox).sort(),
        ['cwd', 'home', 'path'],
        'Schema exporter wrote state'
      );
      for (const directory of [home, cwd, emptyPath])
        assert.equal(fs.readdirSync(directory).length, 0, 'Schema exporter wrote state');
    } catch (error) {
      if (commandError)
        throw new AggregateError([commandError, error], 'Schema exporter failed and wrote state');
      throw error;
    }
    if (commandError) throw commandError;
    const record = parseCompiledSchema(output);
    assertCompiledSource(record, snapshot, root);
    assert.equal(
      digest(readBoundedFile(executable, 128 * MiB)),
      binarySHA,
      'Prepared schema executable changed during export'
    );
    success = true;
    return { record, output_sha256: digest(output), binary_sha256: binarySHA };
  } finally {
    if (success) fs.rmSync(sandbox, { recursive: true });
    // Red evidence remains owned and visible; this check is not an OS security sandbox.
    else process.stderr.write(`Schema export evidence retained: ${sandbox}\n`);
  }
}

export function captureSchema(
  { executable, archive, target, snapshot, root },
  execute = runPackedCommand
) {
  const exported = exportCompiledSchema({ executable, target, snapshot, root }, execute);
  return {
    schema_version: 1,
    target,
    source_snapshot_sha256: snapshotDigest(snapshot),
    archive_sha256: digest(readBoundedFile(archive, 64 * MiB)),
    ...exported,
  };
}

export function readSchemaEvidence(file) {
  const text = readBoundedFile(file, MiB).toString('utf8');
  const record = JSON.parse(text);
  assert.equal(text, `${JSON.stringify(record)}\n`, 'Target evidence encoding changed');
  return record;
}

/** cargo-dist has merged first; only this new field is added to its final metadata. */
export function attachApplicationSchema({ manifest, evidence, snapshot, root, directory }) {
  assert.deepEqual(
    JSON.parse(readBoundedFile(path.join(directory, 'dist-manifest.json'), 4 * MiB)),
    manifest,
    'Assembled manifest differs from the selected cargo-dist bytes'
  );
  assert(
    !Object.hasOwn(manifest, 'tmt_application_schema'),
    'Schema field already exists before assembly'
  );
  assert(Array.isArray(evidence) && evidence.length === 4, 'Four target schema records required');
  assert.deepEqual(
    evidence.map((entry) => entry.target).sort(),
    CLI_SCHEMA_TARGETS,
    'Missing or duplicate schema target'
  );
  for (const entry of evidence) {
    fields(entry, [
      'schema_version',
      'target',
      'source_snapshot_sha256',
      'archive_sha256',
      'record',
      'output_sha256',
      'binary_sha256',
    ]);
    assert.equal(entry.schema_version, 1, 'Unsupported target schema evidence');
    assert.equal(
      entry.source_snapshot_sha256,
      snapshotDigest(snapshot),
      'Different preparation source snapshot'
    );
    assert(
      sha(entry.binary_sha256) && sha(entry.archive_sha256),
      'Missing prepared binary/archive digest'
    );
    const text = `${JSON.stringify(entry.record)}\n`;
    const record = parseCompiledSchema(text);
    assert.equal(entry.output_sha256, digest(text), 'Compiled output digest mismatch');
    assertCompiledSource(record, snapshot, root);
    assert.deepEqual(record, evidence[0].record, 'Target schema disagreement');
    const archive = path.join(directory, `tmt-cli-${entry.target}.tar.gz`);
    const metadata = selectNativeArtifact(
      path.join(directory, 'dist-manifest.json'),
      archive,
      entry.target,
      'cli',
      { release: true }
    );
    assert.equal(
      metadata.sha256,
      entry.archive_sha256,
      'Final manifest/archive schema provenance mismatch'
    );
    assert.equal(
      digest(readBoundedFile(archive, 64 * MiB)),
      entry.archive_sha256,
      'Prepared archive changed'
    );
  }
  const updated = { ...manifest, tmt_application_schema: evidence[0].record };
  assert(
    Buffer.byteLength(`${JSON.stringify(updated, null, 2)}\n`) <= 4 * MiB,
    'Final manifest bound'
  );
  return updated;
}

/** Opt-in for new preparation only; historical schema-less archives retain their readers. */
export function verifyApplicationSchema(
  { manifestBytes, executable, archive, evidence, target, snapshot, root },
  execute = runPackedCommand
) {
  assert(Buffer.isBuffer(manifestBytes) && manifestBytes.length <= 4 * MiB, 'Final manifest bound');
  const manifest = JSON.parse(manifestBytes.toString('utf8'));
  assert.equal(
    manifestBytes.toString('utf8'),
    `${JSON.stringify(manifest, null, 2)}\n`,
    'Final schema manifest encoding changed'
  );
  assert(
    Object.hasOwn(manifest, 'tmt_application_schema'),
    'Final manifest lost application schema'
  );
  const expected = parseCompiledSchema(`${JSON.stringify(manifest.tmt_application_schema)}\n`);
  assertCompiledSource(expected, snapshot, root);
  fields(evidence, [
    'schema_version',
    'target',
    'source_snapshot_sha256',
    'archive_sha256',
    'record',
    'output_sha256',
    'binary_sha256',
  ]);
  assert.equal(evidence.schema_version, 1);
  assert.equal(evidence.target, target, 'Wrong target schema evidence');
  assert.equal(
    evidence.source_snapshot_sha256,
    snapshotDigest(snapshot),
    'Different captured source'
  );
  assert.deepEqual(evidence.record, expected, 'Manifest schema differs from target capture');
  assert.equal(
    evidence.archive_sha256,
    digest(readBoundedFile(archive, 64 * MiB)),
    'Archived payload differs from target capture'
  );
  const actual = exportCompiledSchema({ executable, target, snapshot, root }, execute);
  assert.deepEqual(actual.record, expected, 'Archived CLI schema differs from final manifest');
  assert.deepEqual(evidence.record, actual.record, 'Archived schema differs from target capture');
  assert.equal(
    evidence.binary_sha256,
    actual.binary_sha256,
    'Archived binary differs from target capture'
  );
  assert.equal(
    evidence.output_sha256,
    actual.output_sha256,
    'Archived output differs from target capture'
  );
  return { manifest_sha256: digest(manifestBytes), ...actual };
}

function main([command, root, snapshotFile, targetOrDirectory, executable, archive, output]) {
  const snapshot = JSON.parse(readBoundedFile(snapshotFile, 4 * MiB));
  if (command === 'capture') {
    assert(
      root && targetOrDirectory && executable && archive && output,
      'Schema capture arguments required'
    );
    const evidence = captureSchema({
      root,
      snapshot,
      target: targetOrDirectory,
      executable,
      archive,
    });
    fs.writeFileSync(output, `${JSON.stringify(evidence)}\n`, { flag: 'wx' });
  } else if (command === 'assemble') {
    assert(
      root && targetOrDirectory && !executable && !archive && !output,
      'Schema assemble arguments required'
    );
    const directory = targetOrDirectory;
    const file = path.join(directory, 'dist-manifest.json');
    const manifest = JSON.parse(readBoundedFile(file, 4 * MiB));
    const evidence = CLI_SCHEMA_TARGETS.map((target) =>
      readSchemaEvidence(path.join(directory, `${target}-application-schema.json`))
    );
    const updated = attachApplicationSchema({ manifest, evidence, snapshot, root, directory });
    fs.writeFileSync(file, `${JSON.stringify(updated, null, 2)}\n`);
  } else
    throw new Error(
      'Usage: native-application-schema.mjs capture|assemble <source-root> <version-snapshot> <target|bundle-dir> [binary archive output]'
    );
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  main(process.argv.slice(2));
