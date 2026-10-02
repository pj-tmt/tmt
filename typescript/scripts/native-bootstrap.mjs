import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import {
  readBoundedFile,
  selectNativeArtifact,
  withNativeArtifact,
} from './native-artifact-policy.mjs';

// This maps uname evidence, not a second release inventory. Only artifacts in
// the verified cargo-dist manifest are emitted into the generated installer.
const hosts = {
  'aarch64-apple-darwin': 'Darwin:arm64|Darwin:aarch64',
  'x86_64-apple-darwin': 'Darwin:x86_64',
  'aarch64-unknown-linux-musl': 'Linux:aarch64|Linux:arm64',
  'x86_64-unknown-linux-musl': 'Linux:x86_64',
};

/** Release tooling only: verify local artifacts before generating executable code. */
export async function generateNativeBootstrap(manifestFile, archiveDirectory, planFile) {
  const bytes = readBoundedFile(manifestFile, 4 * 1024 * 1024);
  const manifest = JSON.parse(bytes);
  const entries = Object.entries(manifest.artifacts ?? {}).filter(
    ([, artifact]) => artifact.kind === 'executable-zip'
  );
  assert(entries.length > 0 && entries.length <= 4, 'Require one to four native artifacts');
  // Release preparation must cover cargo-dist's complete configured plan. Local
  // single-target verification deliberately omits this optional release gate.
  if (planFile !== undefined) {
    const plan = JSON.parse(readBoundedFile(planFile, 4 * 1024 * 1024));
    const planned = Object.entries(plan.artifacts ?? {}).filter(
      ([, artifact]) => artifact.kind === 'executable-zip'
    );
    const inventory = (artifacts) =>
      artifacts.map(([name, artifact]) => [name, artifact.target_triples]).sort();
    assert.deepEqual(
      inventory(entries),
      inventory(planned),
      'Release must contain every planned native artifact'
    );
  }
  const seen = new Set();
  const cases = [];
  let version;
  for (const [name, artifact] of entries) {
    assert(/^[a-zA-Z0-9][a-zA-Z0-9._-]*\.tar\.gz$/.test(name), 'Unsafe native archive name');
    assert.equal(artifact.target_triples?.length, 1, 'Require one native target per archive');
    const target = artifact.target_triples[0];
    assert(Object.hasOwn(hosts, target), 'Unsupported bootstrap target');
    assert(!seen.has(target), 'Duplicate bootstrap target');
    seen.add(target);
    const archive = path.join(archiveDirectory, name);
    const metadata = selectNativeArtifact(manifestFile, archive, target, 'cli', { release: true });
    // Restrict code interpolation to URL-safe version tokens. Runtime semver and
    // forward-only pin policy remain in the native installer, not this generator.
    assert(
      /^\d+\.\d+\.\d+(?:-alpha(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(
        metadata.version
      ),
      'Unsafe or unsupported bootstrap version'
    );
    version ??= metadata.version;
    assert.equal(metadata.version, version, 'Bootstrap artifacts must share one version');
    await withNativeArtifact(archive, metadata, () => undefined);
    const size = readBoundedFile(archive, 64 * 1024 * 1024).length;
    cases.push(
      `    ${hosts[target]}) archive='${name}'; archive_hash='${metadata.sha256}'; archive_size=${size} ;;`
    );
  }
  assert(
    readBoundedFile(manifestFile, 4 * 1024 * 1024).equals(bytes),
    'Manifest changed during generation'
  );
  const template = fs.readFileSync(
    new URL('../../scripts/native-bootstrap.sh', import.meta.url),
    'utf8'
  );
  const replacements = {
    VERSION: version,
    CHANNEL: version.split('+')[0].includes('-') ? 'alpha' : 'stable',
    MANIFEST_HASH: createHash('sha256').update(bytes).digest('hex'),
    MANIFEST_SIZE: String(bytes.length),
    TARGET_CASES: cases.sort().join('\n'),
  };
  return template.replace(/@@([A-Z_]+)@@/g, (_, key) => {
    assert(Object.hasOwn(replacements, key), `Unknown bootstrap template field ${key}`);
    return replacements[key];
  });
}
