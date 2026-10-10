// Inert archive bytes and cargo-dist metadata; no archive extraction or native compilation.
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import {
  createReleaseRecord,
  releaseRecordBytes,
  RELEASE_RECORD,
} from '../../scripts/release-index.mjs';
export const RECORD_TARGETS = [
  'aarch64-apple-darwin',
  'aarch64-unknown-linux-musl',
  'x86_64-apple-darwin',
  'x86_64-unknown-linux-musl',
];
export function writeRecordFixture(directory: string, product = 'cli', tag = 'v5.0.0-alpha.9') {
  mkdirSync(directory, { recursive: true });
  const version = tag.slice(tag.indexOf('v') + 1);
  const archives = RECORD_TARGETS.map(
    (target) => `${product === 'cli' ? 'tmt-cli' : `tmt-${product}`}-${target}.tar.gz`
  );
  const artifacts = Object.fromEntries(
    RECORD_TARGETS.map((target, index) => {
      const name = archives[index];
      const bytes = Buffer.from(`inert archive ${target}`);
      writeFileSync(path.join(directory, name), bytes);
      return [
        name,
        {
          kind: 'executable-zip',
          name,
          target_triples: [target],
          checksums: { sha256: createHash('sha256').update(bytes).digest('hex') },
          assets: [
            product === 'cli' ? 'tmt' : `tmt-${product}`,
            'LICENSE',
            'NATIVE-INSTALL.md',
            'THIRD-PARTY-NOTICES.txt',
          ].map((path) => ({ path })),
        },
      ];
    })
  );
  writeFileSync(
    path.join(directory, 'dist-manifest.json'),
    JSON.stringify({
      announcement_tag: tag,
      artifacts,
      releases: [{ app_name: `tmt-${product}`, app_version: version, artifacts: archives }],
    })
  );
  const record = createReleaseRecord({
    product,
    tag,
    releaseId: 1,
    sourceSha: 'a'.repeat(40),
    directory,
  });
  writeFileSync(path.join(directory, RELEASE_RECORD), releaseRecordBytes(record));
  return {
    record,
    names: ['release-publication.json', 'dist-manifest.json', ...archives, RELEASE_RECORD],
    bytes: (name: string) => readFileSync(path.join(directory, name)),
  };
}
