import fs from 'node:fs';
import Database from 'better-sqlite3';

/**
 * A stored document of the Ops state database (`cron`, or one room's `checklist`), or
 * undefined when it is not stored yet. The file exists before the first Store creation has
 * committed its `documents` migration, and a writer may hold the lock meanwhile, so a missing
 * table or a busy database also means "not yet" for a poller. Every other error is real.
 */
export function storedDocument(
  database: string,
  kind: 'cron' | 'checklist',
  key = ''
): Buffer | undefined {
  if (!fs.existsSync(database)) return undefined;
  let db: Database.Database | undefined;
  try {
    db = new Database(database, { readonly: true, timeout: 200 });
    const row = db
      .prepare('SELECT body FROM documents WHERE kind = ? AND key = ?')
      .get(kind, key) as { body: Buffer } | undefined;
    return row?.body;
  } catch (error) {
    if (notStoredYet(error)) return undefined;
    throw error;
  } finally {
    db?.close();
  }
}

function notStoredYet(error: unknown): boolean {
  if (!(error instanceof Database.SqliteError)) return false;
  return (
    error.code === 'SQLITE_BUSY' ||
    error.code === 'SQLITE_LOCKED' ||
    error.message === 'no such table: documents'
  );
}
