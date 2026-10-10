import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import {
  createReleaseRecord,
  parseChannelPointer,
  parseReleaseRecord,
  releaseRecordBytes,
  verifyReleaseRecord,
  POINTER_LIMIT,
  RECORD_LIMIT,
} from '../../scripts/release-index.mjs';
import { writeRecordFixture, RECORD_TARGETS } from '../support/release-record-fixture.js';
const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});
const fixture = (product = 'cli', tag = 'v5.0.0-alpha.9') => {
  const directory = mkdtempSync(path.join(tmpdir(), 'release-record-'));
  roots.push(directory);
  const { record } = writeRecordFixture(directory, product, tag);
  return {
    directory,
    record,
    input: { directory, product, tag, releaseId: 1, sourceSha: 'a'.repeat(40) },
  };
};
const sha256 = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');

describe('release-index v1', () => {
  it.each([
    ['cli', 'v5.0.0-alpha.9'],
    ['ops', 'tmt-ops-v0.1.0-alpha.2'],
    ['colab', 'tmt-colab-v0.1.0'],
  ])('binds %s record identity to each actual manifest/archive byte', (product, tag) => {
    const { directory, record, input } = fixture(product, tag);
    const manifest = readFileSync(path.join(directory, 'dist-manifest.json'));
    expect(record).toEqual({
      schemaVersion: 1,
      product,
      version: tag.slice(tag.indexOf('v') + 1),
      tag,
      releaseId: 1,
      sourceSha: 'a'.repeat(40),
      manifest: {
        name: 'dist-manifest.json',
        url: `https://github.com/pj-tmt/tmt/releases/download/${tag}/dist-manifest.json`,
        size: manifest.length,
        sha256: sha256(manifest),
      },
      archives: Object.fromEntries(
        RECORD_TARGETS.map((target) => {
          const name = `${product === 'cli' ? 'tmt-cli' : `tmt-${product}`}-${target}.tar.gz`;
          const bytes = Buffer.from(`inert archive ${target}`);
          return [
            target,
            {
              name,
              url: `https://github.com/pj-tmt/tmt/releases/download/${tag}/${name}`,
              size: bytes.length,
              sha256: sha256(bytes),
            },
          ];
        })
      ),
    });
    expect(verifyReleaseRecord({ ...input, recordBytes: releaseRecordBytes(record) })).toEqual(
      record
    );
    expect(releaseRecordBytes(createReleaseRecord(input))).toEqual(releaseRecordBytes(record));
  });

  it('accepts additive fields without permitting required fields to disagree', () => {
    const { record, input } = fixture();
    const extended = { ...record, later: true, manifest: { ...record.manifest, later: true } };
    expect(verifyReleaseRecord({ ...input, recordBytes: JSON.stringify(extended) })).toEqual(
      extended
    );
    expect(() =>
      verifyReleaseRecord({ ...input, releaseId: 2, recordBytes: JSON.stringify(record) })
    ).toThrow('releaseId mismatch');
    expect(() =>
      verifyReleaseRecord({
        ...input,
        sourceSha: 'b'.repeat(40),
        recordBytes: JSON.stringify(record),
      })
    ).toThrow('sourceSha mismatch');
  });

  it.each([
    ['unsupported schema', { schemaVersion: 2 }, 'Unsupported release-index schema'],
    ['zero release ID', { releaseId: 0 }, 'positive safe release ID'],
    ['unsafe release ID', { releaseId: Number.MAX_SAFE_INTEGER + 1 }, 'positive safe release ID'],
    ['source SHA', { sourceSha: 'main' }, 'source SHA'],
    ['foreign product', { product: 'other' }, 'Unknown native product'],
    ['tag', { tag: 'v5.0.0-alpha.8' }, 'tag/version mismatch'],
    ['leading zero', { version: '05.0.0-alpha.9' }, 'canonical stable or alpha version'],
    ['beta', { version: '5.0.0-beta.9' }, 'canonical stable or alpha version'],
    ['target count', { archives: {} }, 'four native targets'],
  ])('refuses %s', (_name, change, error) => {
    const { record } = fixture();
    expect(() => parseReleaseRecord(JSON.stringify({ ...record, ...change }))).toThrow(error);
  });

  it.each([
    ['url', 'https://example.com/manifest', 'URL mismatch'],
    ['name', '../manifest', 'name mismatch'],
    ['size', 0, 'size outside bounds'],
    ['size', 4 * 1024 * 1024 + 1, 'size outside bounds'],
    ['sha256', 'short', 'requires SHA-256'],
  ])('refuses manifest %s=%s', (key, value, error) => {
    const { record } = fixture();
    expect(() =>
      parseReleaseRecord(
        JSON.stringify({ ...record, manifest: { ...record.manifest, [key]: value } })
      )
    ).toThrow(error);
  });

  it('refuses target aliases, cross-target assets, changed bytes and manifest checksums', () => {
    const { record, input, directory } = fixture();
    const first = RECORD_TARGETS[0];
    const archives = { ...record.archives };
    delete archives[first];
    archives['aarch64-linux'] = record.archives[first];
    expect(() => parseReleaseRecord(JSON.stringify({ ...record, archives }))).toThrow(
      'Unsupported release-index target'
    );
    expect(() =>
      parseReleaseRecord(
        JSON.stringify({
          ...record,
          archives: { ...record.archives, [first]: record.archives[RECORD_TARGETS[1]] },
        })
      )
    ).toThrow('name mismatch');
    writeFileSync(path.join(directory, record.archives[first].name), 'changed');
    expect(() => createReleaseRecord(input)).toThrow('archive checksum mismatch');
    const fresh = fixture();
    const wrong = {
      ...fresh.record,
      manifest: { ...fresh.record.manifest, sha256: 'b'.repeat(64) },
    };
    expect(() =>
      verifyReleaseRecord({ ...fresh.input, recordBytes: JSON.stringify(wrong) })
    ).toThrow('asset sha256 mismatch');
  });

  it('validates only the canonical pointer locations and binds version/channel/record limits', () => {
    const { record } = fixture();
    const pointer = {
      schemaVersion: 1,
      product: 'cli',
      channel: 'alpha',
      version: record.version,
      tag: record.tag,
      record: {
        url: `https://github.com/pj-tmt/tmt/releases/download/${record.tag}/tmt-release-record.json`,
        size: 100,
        sha256: 'a'.repeat(64),
      },
    };
    expect(parseChannelPointer(JSON.stringify(pointer))).toEqual(pointer);
    const backfill = {
      ...pointer,
      record: {
        ...pointer.record,
        url: `https://raw.githubusercontent.com/pj-tmt/tmt/release-index/records/${record.tag}.json`,
      },
    };
    expect(parseChannelPointer(JSON.stringify(backfill))).toEqual(backfill);
    expect(() => parseChannelPointer(JSON.stringify({ ...pointer, channel: 'stable' }))).toThrow(
      'channel/version mismatch'
    );
    expect(() =>
      parseChannelPointer(
        JSON.stringify({
          ...pointer,
          record: { ...pointer.record, url: 'https://api.github.com/record' },
        })
      )
    ).toThrow('record URL mismatch');
    expect(() =>
      parseChannelPointer(
        JSON.stringify({ ...pointer, record: { ...pointer.record, size: RECORD_LIMIT + 1 } })
      )
    ).toThrow('record size outside bounds');
    expect(() => parseChannelPointer(' '.repeat(POINTER_LIMIT + 1))).toThrow('byte cap');
    expect(() => parseReleaseRecord(' '.repeat(RECORD_LIMIT + 1))).toThrow('byte cap');
    expect(() => parseReleaseRecord('{')).toThrow();
  });
});
