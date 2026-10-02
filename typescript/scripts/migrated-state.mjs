// What the upgrade proof asks of a candidate release that opens state the previous release wrote:
// the previous release writes records through commands that need no tmux, and after the candidate
// opens that state (which applies its pending migrations) the database must hold exactly the
// candidate's migrations, pass SQLite's own checks and still hold every record. Only ids, counts
// and SQL invariants are compared: the shape of command output changes from alpha to alpha.
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { parseComponentMap } from './ci-scope.mjs';
import { countMigrations } from './publication-gates.mjs';

const ROOT = new URL('../../', import.meta.url);

/** The migrations the CLI's source lists, read as the publication gate reads them. */
export function expectedMigrations(root = ROOT) {
  const { components } = parseComponentMap(
    readFileSync(new URL('.github/components.json', root), 'utf8')
  );
  return components
    .find(({ name }) => name === 'cli')
    .migrations.reduce(
      (total, file) => total + countMigrations(readFileSync(new URL(file, root), 'utf8')),
      0
    );
}

/**
 * Writes identities (one with a preamble, role, metadata and status), a room both of them joined
 * and one message queued to every member, through `run(args)`, which returns the stdout of the
 * previous release's `tmt`. Returns the ids to look for afterwards.
 */
export function writePriorState(run, { name = 'migration-proof' } = {}) {
  const json = (...args) => JSON.parse(run([...args, '--json']));
  const [first, second] = [`${name}-a`, `${name}-b`];
  const identities = [first, second].map(
    (identity) => json('identity', 'create', identity).identity.id
  );
  json('preamble', 'set', first, 'A preamble written by the previous release.');
  json('role', 'set', '--identity', first, 'A role written by the previous release.');
  json('identity', 'meta', 'set', '--identity', first, 'origin', 'previous');
  json('identity', 'status', 'set', '--identity', first, 'writing state');
  const room = json('room', 'create', `${name}-room`).room.id;
  for (const identity of [first, second]) json('room', 'join', room, '--identity', identity);
  const sent = json('room', 'send', room, 'Queued by the previous release.', '--identity', first);
  const requests = sent.items.map(({ requestId }) => requestId.replace(/^req_/, ''));
  return [...identities, room, ...requests];
}

/** Whether any column of any table holds `needle` as text, whatever the schema calls them. */
function holdsValue(database, tables, needle) {
  return tables.some((table) => {
    const columns = database.prepare(`PRAGMA table_info("${table}")`).all();
    const test = columns.map(({ name }) => `instr(CAST("${name}" AS TEXT), ?) > 0`).join(' OR ');
    return (
      columns.length > 0 &&
      database
        .prepare(`SELECT 1 FROM "${table}" WHERE ${test} LIMIT 1`)
        .get(...columns.map(() => needle)) !== undefined
    );
  });
}

/**
 * The state of a database file, read from a copy so the proof's own bytes stay untouched: how many
 * migrations it records, how many rows each table holds, what SQLite's integrity and foreign key
 * checks report, and which of `ids` no table holds.
 */
export function snapshotState(databasePath, ids = []) {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt state snapshot '));
  try {
    const copy = path.join(directory, 'state.db');
    for (const suffix of ['', '-wal']) {
      if (existsSync(databasePath + suffix)) copyFileSync(databasePath + suffix, copy + suffix);
    }
    const database = new DatabaseSync(copy);
    try {
      const tables = database
        .prepare(
          `SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             AND name != '_migrations' AND sql NOT LIKE 'CREATE VIRTUAL%' ORDER BY name`
        )
        .all()
        .map(({ name }) => name);
      const problems = [
        ...database
          .prepare('PRAGMA integrity_check')
          .all()
          .map((row) => row.integrity_check)
          .filter((result) => result !== 'ok')
          .map((result) => `integrity_check: ${result}`),
        ...database
          .prepare('PRAGMA foreign_key_check')
          .all()
          .map(
            (row) =>
              `foreign_key_check: ${row.table} row ${row.rowid} has no parent in ${row.parent}`
          ),
      ];
      return {
        migrations: database.prepare('SELECT count(*) AS count FROM _migrations').get().count,
        tables: Object.fromEntries(
          tables.map((table) => [
            table,
            database.prepare(`SELECT count(*) AS count FROM "${table}"`).get().count,
          ])
        ),
        problems,
        missing: ids.filter((id) => !holdsValue(database, tables, id)),
      };
    } finally {
      database.close();
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

/**
 * What is wrong with the state the candidate opened, as sentences, or nothing: it records exactly
 * the `expected` migrations of the candidate's source, SQLite finds no fault, no written id is
 * gone, and no table that held rows lost any or vanished. A table the candidate added is fine.
 */
export function checkMigratedState({ before, after, expected }) {
  const problems = [...after.problems];
  if (after.migrations !== expected) {
    problems.push(
      `${after.migrations} migrations are recorded after the candidate opened the state, ${expected} expected from its source (${before.migrations} before)`
    );
  }
  for (const id of after.missing)
    problems.push(`no table holds ${id}, which the previous release wrote`);
  for (const [table, rows] of Object.entries(before.tables)) {
    if (rows === 0) continue;
    if (!(table in after.tables)) problems.push(`table ${table} held ${rows} rows and is gone`);
    else if (after.tables[table] < rows)
      problems.push(`table ${table} held ${rows} rows and holds ${after.tables[table]}`);
  }
  return problems;
}
