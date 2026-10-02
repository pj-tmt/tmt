import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import { withSandbox } from '../support/cli-process.js';
import {
  initializeHistoricalDatabase,
  seedStoragePrefix,
  seedStorageReference,
  storageSnapshot,
} from './storage-fixture.js';

const manifest = JSON.parse(
  readFileSync(new URL('../fixtures/storage-history/manifest.json', import.meta.url), 'utf8')
) as {
  fixtures: Record<
    string,
    {
      databaseSha256: string;
      snapshotSha256: string;
      schemaVersion: number;
      rowCounts: Record<string, number>;
    }
  >;
};
const hash = (value: Buffer | string) => createHash('sha256').update(value).digest('hex');

describe('frozen historical database evidence', () => {
  it('retains every historical input and independent reference', () => {
    const expected = ['empty-8'];
    for (let version = 0; version <= 8; version++) {
      expected.push(`prefix-${version}`, `reference-${version}`);
    }
    expect(Object.keys(manifest.fixtures).sort()).toEqual(expected.sort());
    const archives = readdirSync(new URL('../fixtures/storage-history/', import.meta.url))
      .filter((name) => name.endsWith('.db.gz'))
      .sort();
    expect(archives).toEqual(expected.map((name) => `${name}.db.gz`).sort());
  });

  it.each(Object.entries(manifest.fixtures))(
    'preserves %s bytes and SQL evidence',
    async (name, evidence) => {
      await withSandbox(async (sandbox) => {
        if (name === 'empty-8') initializeHistoricalDatabase(sandbox.database);
        else if (name.startsWith('prefix-'))
          seedStoragePrefix(sandbox.database, Number(name.slice(7)));
        else if (name.startsWith('reference-'))
          seedStorageReference(sandbox.database, Number(name.slice(10)));
        else throw new Error(`Unknown historical fixture: ${name}`);
        expect(hash(readFileSync(sandbox.database))).toBe(evidence.databaseSha256);
        const snapshot = storageSnapshot(sandbox.database);
        expect(hash(JSON.stringify(snapshot))).toBe(evidence.snapshotSha256);
        expect(snapshot.migrations).toHaveLength(evidence.schemaVersion);
        expect(
          Object.fromEntries(snapshot.tables.map((table) => [table.name, table.rows.length]))
        ).toEqual(evidence.rowCounts);
        if (process.platform !== 'win32')
          expect(statSync(sandbox.database).mode & 0o777).toBe(0o600);
      });
    }
  );

  it('rejects invalid versions and refuses to overwrite an existing database', async () => {
    await withSandbox(async (sandbox) => {
      for (const version of [-1, 9, 1.5, NaN]) {
        expect(() => seedStoragePrefix(sandbox.database, version)).toThrow(/between 0 and 8/);
        expect(() => seedStorageReference(sandbox.database, version)).toThrow(/between 0 and 8/);
      }
      initializeHistoricalDatabase(sandbox.database);
      const before = readFileSync(sandbox.database);
      expect(() => seedStoragePrefix(sandbox.database, 0)).toThrow(/EEXIST/);
      expect(readFileSync(sandbox.database)).toEqual(before);
    });
  });
});
