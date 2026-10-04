import Database from 'better-sqlite3';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { gunzipSync } from 'node:zlib';

// Independent process-test oracle; do not derive it from the runtime under test.
export const EXPECTED_NATIVE_SCHEMA_VERSION = 47;

export const FIXTURE_IDENTITY_ID = 'identity-known';
export const FIXTURE_MISSING_IDENTITY_ID = 'identity-missing';
export const FIXTURE_ATTEMPT_COUNT = 205;
export const FIXTURE_PREPARED_AT_MS = 1_700_000_000_000;
export const FIXTURE_SATURATION_AT_MS = Number.MAX_SAFE_INTEGER - 1;
export const FIXTURE_FINAL_REQUEST_ID = 'request-final';
export const FIXTURE_FINAL_BODY = 'fixture final response';

export interface StorageSnapshot {
  readonly migrations: readonly MigrationSnapshot[];
  readonly tables: readonly TableSnapshot[];
}

interface MigrationSnapshot {
  readonly version: number;
  readonly name: string;
}

interface TableSnapshot {
  readonly name: string;
  readonly columns: readonly Record<string, unknown>[];
  readonly indexes: readonly IndexSnapshot[];
  readonly foreignKeys: readonly Record<string, unknown>[];
  readonly rows: readonly Record<string, unknown>[];
}

interface IndexSnapshot {
  readonly name: string;
  readonly columns: readonly Record<string, unknown>[];
  readonly unique: number;
  readonly origin: string;
  readonly partial: number;
}

const PRIMARY_KEY_ORDER: Record<string, readonly string[]> = {
  _migrations: ['version'],
  bindings: ['id'],
  identities: ['id'],
  office_dispatch_operations: ['operation_id'],
  office_meeting_rooms: ['room_id'],
  office_meeting_members: ['room_id', 'identity_id'],
  identity_preambles: ['identity_id'],
  preamble_counters: ['identity_id'],
  request_attempts: ['attempt_id'],
  request_attention_identities: ['identity_id'],
  request_responses: ['request_id'],
  role_profiles: ['identity_id'],
};

function fixtureVersion(version: number): number {
  if (!Number.isInteger(version) || version < 0 || version > 8) {
    throw new Error('Fixture migration version must be between 0 and 8.');
  }
  return version;
}

function copyFixture(file: string, name: string): void {
  const archive = new URL(`../fixtures/storage-history/${name}.db.gz`, import.meta.url);
  const bytes = gunzipSync(readFileSync(archive));
  mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  writeFileSync(file, bytes, { flag: 'wx', mode: 0o600 });
}

/** Copy a closed historical database; never run the implementation under test to seed it. */
export function seedStoragePrefix(file: string, version: number): void {
  copyFixture(file, `prefix-${fixtureVersion(version)}`);
}

/** Copy the historical TypeScript upgrade result for an independently seeded prefix. */
export function seedStorageReference(file: string, version: number): void {
  copyFixture(file, `reference-${fixtureVersion(version)}`);
}

/** Keep the public reply migration scenario on an empty historical schema 8 database. */
export function initializeHistoricalDatabase(file: string): void {
  copyFixture(file, 'empty-8');
}

function quoteIdentifier(identifier: string): string {
  return `"${identifier.replaceAll('"', '""')}"`;
}

function orderedRows(
  database: Database.Database,
  table: string,
  columns: readonly Record<string, unknown>[]
): Record<string, unknown>[] {
  const order = PRIMARY_KEY_ORDER[table] ?? columns.map((column) => String(column.name));
  const orderBy = order.map((column) => quoteIdentifier(column)).join(', ');
  const rows = database
    .prepare(`SELECT * FROM ${quoteIdentifier(table)} ORDER BY ${orderBy}`)
    .all() as Record<string, unknown>[];
  return rows.map((row) => {
    if (table !== '_migrations') return row;
    const { applied_at: _appliedAt, ...migration } = row;
    return migration;
  });
}

/** Read SQLite structure and rows without comparing sqlite_schema SQL text. */
export function storageSnapshot(file: string): StorageSnapshot {
  const database = new Database(file, { readonly: true });
  try {
    const migrations = database
      .prepare('SELECT version, name FROM _migrations ORDER BY version')
      .all() as MigrationSnapshot[];
    const tables: TableSnapshot[] = [];
    const tableNames = database
      .prepare(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT GLOB 'sqlite_*' ORDER BY name"
      )
      .all() as Array<{ name: string }>;
    for (const { name: table } of tableNames) {
      const columns = database.pragma(`table_info(${quoteIdentifier(table)})`) as Record<
        string,
        unknown
      >[];
      const indexRows = database.pragma(`index_list(${quoteIdentifier(table)})`) as Array<{
        name: string;
        unique: number;
        origin: string;
        partial: number;
      }>;
      const indexes = indexRows
        .map((index) => ({
          name: index.name,
          unique: index.unique,
          origin: index.origin,
          partial: index.partial,
          columns: database.pragma(`index_xinfo(${quoteIdentifier(index.name)})`) as Record<
            string,
            unknown
          >[],
        }))
        .sort((left, right) => left.name.localeCompare(right.name));
      const foreignKeys = database.pragma(`foreign_key_list(${quoteIdentifier(table)})`) as Record<
        string,
        unknown
      >[];
      tables.push({
        name: table,
        columns,
        indexes,
        foreignKeys,
        rows: orderedRows(database, table, columns),
      });
    }
    return { migrations, tables };
  } finally {
    database.close();
  }
}
