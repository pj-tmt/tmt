import { readFileSync } from 'node:fs';
import path from 'node:path';
import { createHash, webcrypto } from 'node:crypto';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import {
  decodePropCatalogPage,
  decodePropInstallReceipt,
  propDocumentDigest,
} from './prop-catalog-contract.js';
import { BUILTIN_DIGEST, WORKSHOP_DIGEST } from './prop-contract.js';

afterEach(() => vi.unstubAllGlobals());
it.each([
  ['builtin-props-v1.tmtprop.json', BUILTIN_DIGEST],
  ['workshop-furniture-v2.tmtprop.json', WORKSHOP_DIGEST],
])('matches the immutable native identity of %s and hashes exact bytes', async (file, digest) => {
  vi.stubGlobal('crypto', webcrypto);
  const document = readFileSync(path.resolve(process.cwd(), '../../../contracts', file), 'utf8');
  await expect(propDocumentDigest({ expectedRevision: 0, document })).resolves.toBe(digest);
  const changed = `${document}\n`;
  const bytes = Buffer.from(changed);
  const length = Buffer.alloc(8);
  length.writeBigUInt64BE(BigInt(bytes.length));
  const version = JSON.parse(document).formatVersion;
  const expected = createHash('sha256')
    .update(`TMT-OFFICE-PROP-PACK-V${version}\0`)
    .update(length)
    .update(bytes)
    .digest('hex');
  await expect(propDocumentDigest({ expectedRevision: 0, document: changed })).resolves.toBe(
    `sha256:${expected}`
  );
  expect(`sha256:${expected}`).not.toBe(digest);
});

it('admits bounded metadata pages but rejects duplicate, crossed and malformed entries', () => {
  const value = {
    revision: 2,
    entries: [{ digest: WORKSHOP_DIGEST, label: 'Workshop' }],
    excluded: [],
    nextCursor: null,
  };
  expect(decodePropCatalogPage(value)).toEqual(value);
  for (const changed of [
    { ...value, revision: -1 },
    { ...value, entries: [...value.entries, ...value.entries] },
    { ...value, excluded: [{ digest: WORKSHOP_DIGEST, reason: 'oversized' }] },
    { ...value, nextCursor: '../cursor' },
    { ...value, entries: Array(21).fill(value.entries[0]) },
    { ...value, path: 'file' },
  ])
    expect(() => decodePropCatalogPage(changed)).toThrow();
  expect(
    decodePropInstallReceipt({ revision: 2, digest: WORKSHOP_DIGEST, changed: false })
  ).toEqual({ revision: 2, digest: WORKSHOP_DIGEST, changed: false });
  expect(() =>
    decodePropInstallReceipt({ revision: 2, digest: 'file:///pack', changed: true })
  ).toThrow();
});
