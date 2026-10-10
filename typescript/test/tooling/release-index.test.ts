import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import {
  channelPointer,
  updateReleaseIndex,
  ghIndexApi,
  type IndexApi,
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

function indexFixture() {
  const documents = new Map<string, Buffer>();
  let number = 1;
  let pending: { path: string; bytes: Buffer }[] = [];
  const calls: { tip: string; paths: string[] }[] = [];
  const api: IndexApi = {
    readTip: () => ({ sha: String(number).padStart(40, '0'), tree: 'b'.repeat(40) }),
    readFile: (_tip, file, limit) => {
      const bytes = documents.get(file) ?? null;
      if (bytes && bytes.length > limit) throw new Error('oversize file');
      return bytes;
    },
    createCommit: (tip, entries) => {
      calls.push({ tip: tip.sha, paths: entries.map(({ path }) => path) });
      pending = entries;
      return 'c'.repeat(40);
    },
    advance: () => {
      number += 1;
      for (const { path, bytes } of pending) documents.set(path, bytes);
      return 'updated';
    },
  };
  return { api, documents, calls };
}

describe('protected release-index updates', () => {
  it('binds exact record bytes and updates only the selected channel, idempotently', () => {
    const { record } = fixture();
    const recordBytes = releaseRecordBytes(record);
    const { api, documents, calls } = indexFixture();
    documents.set('channels/ops/alpha.json', Buffer.from('untouched'));
    const expected = {
      schemaVersion: 1,
      product: 'cli',
      channel: 'alpha',
      version: '5.0.0-alpha.9',
      tag: 'v5.0.0-alpha.9',
      record: {
        url: 'https://github.com/pj-tmt/tmt/releases/download/v5.0.0-alpha.9/tmt-release-record.json',
        size: recordBytes.length,
        sha256: sha256(recordBytes),
      },
    };
    expect(channelPointer(recordBytes)).toEqual(expected);
    expect(updateReleaseIndex({ api, recordBytes })).toMatchObject({ changed: true, attempts: 1 });
    expect(JSON.parse(documents.get('channels/cli/alpha.json')!.toString())).toEqual(expected);
    expect(documents.get('channels/ops/alpha.json')!.toString()).toBe('untouched');
    expect(calls).toEqual([{ tip: '0'.repeat(39) + '1', paths: ['channels/cli/alpha.json'] }]);
    expect(updateReleaseIndex({ api, recordBytes }).changed).toBe(false);
    const older = { ...record, version: '5.0.0-alpha.8', tag: 'v5.0.0-alpha.8' };
    const olderPointer = {
      ...expected,
      version: older.version,
      tag: older.tag,
      record: { ...expected.record, url: expected.record.url.replace('alpha.9', 'alpha.8') },
    };
    // Lower versions are no-ops even if their valid record bytes differ.
    const bytes = recordBytes.toString().replaceAll('alpha.9', 'alpha.8');
    expect(updateReleaseIndex({ api, recordBytes: bytes }).changed).toBe(false);
    expect(parseChannelPointer(JSON.stringify(olderPointer)).version).toBe(older.version);
    expect(calls).toHaveLength(1);
    expect(
      updateReleaseIndex({
        api,
        recordBytes: recordBytes.toString().replaceAll('alpha.9', 'alpha.10'),
      }).changed
    ).toBe(true);
    expect(parseChannelPointer(documents.get('channels/cli/alpha.json')!).version).toBe(
      '5.0.0-alpha.10'
    );
    expect(calls).toHaveLength(2);
  });

  it('rejects malformed pointers, wrong identity, equal-version conflicts and oversized reads before effects', () => {
    const recordBytes = releaseRecordBytes(fixture().record);
    for (const pointer of [
      Buffer.from('bad'),
      Buffer.from(JSON.stringify({ ...channelPointer(recordBytes), product: 'ops' })),
      Buffer.from(
        JSON.stringify({
          ...channelPointer(recordBytes),
          record: { ...channelPointer(recordBytes).record, sha256: 'f'.repeat(64) },
        })
      ),
      Buffer.alloc(POINTER_LIMIT + 1),
    ]) {
      const { api, documents, calls } = indexFixture();
      documents.set('channels/cli/alpha.json', pointer);
      expect(() => updateReleaseIndex({ api, recordBytes })).toThrow();
      expect(calls).toEqual([]);
    }
  });

  it('re-applies races against a new tip preserving concurrent products, and fails after exactly five', () => {
    const recordBytes = releaseRecordBytes(fixture().record);
    const { api, documents, calls } = indexFixture();
    const advance = api.advance;
    let attempts = 0;
    api.advance = (commit) => {
      attempts += 1;
      if (attempts < 5) {
        documents.set('channels/ops/alpha.json', Buffer.from(String(attempts)));
        return 'race';
      }
      return advance(commit);
    };
    const readTip = api.readTip;
    api.readTip = () => ({ ...readTip(), sha: String(attempts + 1).padStart(40, '0') });
    expect(updateReleaseIndex({ api, recordBytes })).toMatchObject({ changed: true, attempts: 5 });
    expect(calls.map(({ tip }) => tip)).toEqual(
      [1, 2, 3, 4, 5].map((n) => String(n).padStart(40, '0'))
    );
    expect(documents.get('channels/ops/alpha.json')!.toString()).toBe('4');
    api.advance = () => 'race';
    documents.delete('channels/cli/alpha.json');
    const before = calls.length;
    expect(() => updateReleaseIndex({ api, recordBytes })).toThrow('5 attempts');
    expect(calls.length - before).toBe(5);
  });

  it('accepts a concurrent higher version without overwriting and refuses unexpected failures/readback', () => {
    const recordBytes = releaseRecordBytes(fixture().record);
    const { api, documents, calls } = indexFixture();
    api.advance = () => {
      documents.set(
        'channels/cli/alpha.json',
        Buffer.from(
          JSON.stringify(channelPointer(recordBytes.toString().replaceAll('alpha.9', 'alpha.10')))
        )
      );
      return 'race';
    };
    expect(updateReleaseIndex({ api, recordBytes })).toMatchObject({ changed: false, attempts: 2 });
    expect(calls).toHaveLength(1);
    documents.clear();
    api.advance = () => {
      throw new Error('permission denied');
    };
    expect(() => updateReleaseIndex({ api, recordBytes })).toThrow('permission denied');
    expect(calls).toHaveLength(2);
    api.advance = () => 'updated';
    expect(() => updateReleaseIndex({ api, recordBytes })).toThrow('readback failed');
  });

  it('requires exact post-write pointer bytes unless a validated newer pointer won the race', () => {
    const recordBytes = releaseRecordBytes(fixture().record);
    const { api, documents } = indexFixture();
    const advance = api.advance;
    api.advance = (commit) => {
      const result = advance(commit);
      documents.set(
        'channels/cli/alpha.json',
        Buffer.from(JSON.stringify(channelPointer(recordBytes)))
      );
      return result;
    };
    expect(() => updateReleaseIndex({ api, recordBytes })).toThrow('readback bytes failed');
  });

  it('writes the historical record and pointer atomically, retaining immutable record bytes', () => {
    const recordBytes = releaseRecordBytes(fixture().record);
    const { api, documents, calls } = indexFixture();
    expect(updateReleaseIndex({ api, recordBytes, bootstrap: true }).changed).toBe(true);
    expect(calls[0].paths).toEqual(['channels/cli/alpha.json', 'records/v5.0.0-alpha.9.json']);
    expect(documents.get('records/v5.0.0-alpha.9.json')).toEqual(recordBytes);
    expect(channelPointer(recordBytes, { bootstrap: true }).record.url).toBe(
      'https://raw.githubusercontent.com/pj-tmt/tmt/release-index/records/v5.0.0-alpha.9.json'
    );
    expect(updateReleaseIndex({ api, recordBytes, bootstrap: true }).changed).toBe(false);
    documents.delete('records/v5.0.0-alpha.9.json');
    expect(() => updateReleaseIndex({ api, recordBytes, bootstrap: true })).toThrow('no record');
    documents.set('records/v5.0.0-alpha.9.json', Buffer.from('conflict'));
    expect(() => updateReleaseIndex({ api, recordBytes, bootstrap: true })).toThrow('immutable');
  });
});

function http(status: number, data: unknown) {
  return {
    status: status < 300 ? 0 : 1,
    stdout: `HTTP/2.0 ${status} Status\r\ncontent-type: application/json\r\n\r\n${JSON.stringify(data)}`,
    stderr: '',
  };
}

describe('release-index Git API boundary', () => {
  it('uses captured parent/base tree, base64 bytes and force:false without credentials in argv', () => {
    const calls: { args: readonly string[]; input: any }[] = [];
    const sha = 'a'.repeat(40);
    const api = ghIndexApi({
      repository: 'pj-tmt/tmt',
      spawn: (_command, args, options) => {
        const input = JSON.parse((options as { input?: string }).input ?? 'null');
        calls.push({ args, input });
        if (args[1].endsWith('git/ref/heads/release-index'))
          return http(200, { object: { type: 'commit', sha } });
        if (args[1].includes('git/commits/') && !input) return http(200, { tree: { sha } });
        if (args[1].includes('contents/')) return http(404, { message: 'Not Found' });
        return http(input?.force === false ? 200 : 201, { sha });
      },
    });
    const tip = api.readTip();
    expect(tip).toEqual({ sha, tree: sha });
    expect(api.readFile(tip.sha, 'channels/cli/alpha.json', POINTER_LIMIT)).toBeNull();
    const commit = api.createCommit(tip, [
      { path: 'channels/cli/alpha.json', bytes: Buffer.from('actual bytes') },
    ]);
    expect(api.advance(commit)).toBe('updated');
    expect(calls.map((c) => c.input).filter(Boolean)).toEqual([
      { encoding: 'base64', content: Buffer.from('actual bytes').toString('base64') },
      {
        base_tree: sha,
        tree: [{ path: 'channels/cli/alpha.json', mode: '100644', type: 'blob', sha }],
      },
      {
        message: 'Advance verified release index: channels/cli/alpha.json',
        tree: sha,
        parents: [sha],
      },
      { sha, force: false },
    ]);
    expect(calls.every((c) => c.args.includes('--include'))).toBe(true);
  });

  it.each([
    [422, 'Update is not a fast forward', 'race'],
    [409, 'Update is not a fast forward', 'race'],
    [409, 'Conflict', null],
    [403, 'Forbidden', null],
    [422, 'Validation Failed', null],
    [500, 'Server Error', null],
  ])('only exact non-fast-forward is retryable (%s %s)', (status, message, outcome) => {
    const api = ghIndexApi({
      repository: 'pj-tmt/tmt',
      spawn: () => http(status as number, { message }),
    });
    if (outcome) expect(api.advance('a'.repeat(40))).toBe(outcome);
    else expect(() => api.advance('a'.repeat(40))).toThrow('update failed');
  });

  it('fails on a missing/malformed branch, oversize/invalid file or transport error', () => {
    for (const response of [
      http(404, { message: 'Not Found' }),
      http(200, { object: { type: 'tag', sha: 'a'.repeat(40) } }),
      http(200, { object: { type: 'commit', sha: 'bad' } }),
    ]) {
      expect(() =>
        ghIndexApi({ repository: 'pj-tmt/tmt', spawn: () => response }).readTip()
      ).toThrow();
    }
    for (const data of [
      { type: 'file', encoding: 'base64', size: POINTER_LIMIT + 1, content: '' },
      { type: 'file', encoding: 'base64', size: 1, content: Buffer.from('xx').toString('base64') },
      { type: 'symlink', encoding: 'base64', size: 1, content: 'eA==' },
    ]) {
      expect(() =>
        ghIndexApi({ repository: 'pj-tmt/tmt', spawn: () => http(200, data) }).readFile(
          'a'.repeat(40),
          'channels/cli/alpha.json',
          POINTER_LIMIT
        )
      ).toThrow();
    }
    const error = new Error('network');
    expect(() =>
      ghIndexApi({
        repository: 'pj-tmt/tmt',
        spawn: () => ({ error, status: null, stdout: '', stderr: '' }),
      }).readTip()
    ).toThrow(error);
    expect(() => ghIndexApi({ repository: 'other/repo' })).toThrow('repository mismatch');
  });
});
