import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import { storedDocument } from '../support/ops-state.js';

const directories: string[] = [];
function database(name = 'ops.db'): string {
  const directory = mkdtempSync(path.join(tmpdir(), 'ops-state-support-'));
  directories.push(directory);
  return path.join(directory, name);
}
function create(file: string, statements: string): void {
  const db = new Database(file);
  try {
    db.exec(statements);
  } finally {
    db.close();
  }
}

afterEach(() => {
  for (const directory of directories.splice(0)) rmSync(directory, { recursive: true });
});

describe('storedDocument', () => {
  it('reads as not stored yet until the first Store has committed its documents table', () => {
    const missing = database();
    expect(storedDocument(missing, 'cron')).toBeUndefined();

    // The file appears before the migration commits.
    const empty = database();
    writeFileSync(empty, '');
    expect(storedDocument(empty, 'cron')).toBeUndefined();

    const unrelated = database();
    create(unrelated, 'CREATE TABLE other(id INTEGER)');
    expect(storedDocument(unrelated, 'checklist', 'room')).toBeUndefined();
  });

  it('returns the stored bytes and nothing for another kind or key', () => {
    const file = database();
    create(
      file,
      `CREATE TABLE documents(kind TEXT NOT NULL, key TEXT NOT NULL, body BLOB NOT NULL, PRIMARY KEY(kind, key));
       INSERT INTO documents VALUES('cron', '', x'7b7d'), ('checklist', 'room-1', x'5b5d');`
    );
    expect(storedDocument(file, 'cron')?.toString('utf8')).toBe('{}');
    expect(storedDocument(file, 'checklist', 'room-1')?.toString('utf8')).toBe('[]');
    expect(storedDocument(file, 'checklist', 'room-2')).toBeUndefined();
  });

  it('reads as not stored yet while a writer holds the database, and rethrows every other error', () => {
    const file = database();
    create(file, 'CREATE TABLE documents(kind TEXT, key TEXT, body BLOB)');
    const writer = new Database(file);
    try {
      writer.exec('BEGIN EXCLUSIVE');
      expect(storedDocument(file, 'cron')).toBeUndefined();
    } finally {
      writer.close();
    }

    const garbage = database();
    writeFileSync(garbage, 'this is not a SQLite database, and it is long enough to be read');
    expect(() => storedDocument(garbage, 'cron')).toThrow(/not a database/);
  });
});
