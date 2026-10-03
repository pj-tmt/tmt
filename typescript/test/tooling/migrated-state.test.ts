import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { afterAll, describe, expect, it } from 'vite-plus/test';
import type { StateSnapshot } from '../../scripts/migrated-state.mjs';

// Publication attribution imports an async ESM graph; do not load it with require().
const { DatabaseSync } = process.getBuiltinModule('node:sqlite');
const { checkMigratedState, expectedMigrations, snapshotState, writePriorState } =
  await import('../../scripts/migrated-state.mjs');

const directory = mkdtempSync(path.join(tmpdir(), 'migrated-state-'));
afterAll(() => rmSync(directory, { recursive: true, force: true }));

let counter = 0;
/** A database file with a `_migrations` history of `migrations` rows and the statements run on it. */
function database(migrations: number, statements: string[] = []): string {
  const file = path.join(directory, `state-${(counter += 1)}.db`);
  const connection = new DatabaseSync(file);
  connection.exec('CREATE TABLE _migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL)');
  for (let version = 1; version <= migrations; version += 1) {
    connection.exec(`INSERT INTO _migrations VALUES (${version}, 'migration ${version}')`);
  }
  for (const statement of statements) connection.exec(statement);
  connection.close();
  return file;
}

const IDENTITIES = [
  'CREATE TABLE identities (id TEXT PRIMARY KEY, name TEXT)',
  "INSERT INTO identities VALUES ('id-1', 'a'), ('id-2', 'b')",
];

describe('snapshotState', () => {
  it('counts the rows of every table but the migration history and finds ids anywhere', () => {
    const file = database(3, [
      ...IDENTITIES,
      'CREATE TABLE empty_table (value TEXT)',
      'CREATE TABLE requests (key TEXT, payload BLOB)',
      "INSERT INTO requests VALUES ('req_abc-123', x'00'), ('other', 'carries the id-2 inside')",
    ]);
    expect(snapshotState(file, ['id-1', 'abc-123', 'id-2', 'absent'])).toEqual({
      migrations: 3,
      tables: { empty_table: 0, identities: 2, requests: 2 },
      problems: [],
      missing: ['absent'],
    });
  });

  it('reports a foreign key violation and reads a copy, leaving the file as it was', () => {
    const file = database(1, [
      'PRAGMA foreign_keys = OFF',
      'CREATE TABLE parents (id INTEGER PRIMARY KEY)',
      'CREATE TABLE children (parent INTEGER REFERENCES parents(id))',
      'INSERT INTO children VALUES (7)',
    ]);
    const bytes = readFileSync(file);
    expect(snapshotState(file).problems).toEqual([
      'foreign_key_check: children row 1 has no parent in parents',
    ]);
    expect(readFileSync(file).equals(bytes)).toBe(true);
  });
});

describe('snapshotState integrity', () => {
  it('reports what integrity_check finds, such as a row that breaks a CHECK constraint', () => {
    const file = database(1, [
      'PRAGMA ignore_check_constraints = ON',
      'CREATE TABLE amounts (value INTEGER CHECK (value > 0))',
      'INSERT INTO amounts VALUES (-1)',
    ]);
    expect(snapshotState(file).problems).toEqual([
      expect.stringMatching(/^integrity_check: .*amounts/),
    ]);
  });
});

describe('checkMigratedState', () => {
  const snapshot = (overrides: Partial<StateSnapshot> = {}): StateSnapshot => ({
    migrations: 39,
    tables: { identities: 2, requests: 3, empty_table: 0 },
    problems: [],
    missing: [],
    ...overrides,
  });
  const check = (after: Partial<StateSnapshot>, expected = 41) =>
    checkMigratedState({ before: snapshot(), after: snapshot(after), expected });

  it('accepts a state with the expected migrations, every row and new tables', () => {
    expect(
      check({ migrations: 41, tables: { identities: 2, requests: 5, extra: 1, empty_table: 0 } })
    ).toEqual([]);
  });

  it('names a candidate that did not apply the migrations of its source', () => {
    expect(check({ migrations: 39 })).toEqual([
      '39 migrations are recorded after the candidate opened the state, 41 expected from its source (39 before)',
    ]);
  });

  it('names a table that lost rows, one that vanished and an id that is gone', () => {
    expect(
      check({ migrations: 41, tables: { identities: 1 }, missing: ['id-2'], problems: ['x'] })
    ).toEqual([
      'x',
      'no table holds id-2, which the previous release wrote',
      'table identities held 2 rows and holds 1',
      'table requests held 3 rows and is gone',
    ]);
  });

  it('lets a table that held no rows vanish', () => {
    expect(check({ migrations: 41, tables: { identities: 2, requests: 3 } })).toEqual([]);
  });
});

describe('writePriorState', () => {
  it('runs the state commands of the previous release and returns the ids it wrote', () => {
    const calls: string[][] = [];
    const ids = writePriorState((args) => {
      calls.push(args);
      const [command, subcommand] = args;
      if (command === 'identity' && subcommand === 'create')
        return JSON.stringify({ identity: { id: `uuid-${args[2]}` } });
      if (command === 'room' && subcommand === 'create')
        return JSON.stringify({ room: { id: 'room-uuid' } });
      if (command === 'room' && subcommand === 'send')
        return JSON.stringify({ items: [{ requestId: 'req_one' }, { requestId: 'req_two' }] });
      return '{}';
    });
    expect(ids).toEqual([
      'uuid-migration-proof-a',
      'uuid-migration-proof-b',
      'room-uuid',
      'one',
      'two',
    ]);
    expect(calls.every((args) => args.at(-1) === '--json')).toBe(true);
    expect(calls.map((args) => args.slice(0, 2).join(' '))).toEqual([
      'identity create',
      'identity create',
      'preamble set',
      'role set',
      'identity meta',
      'identity status',
      'room create',
      'room join',
      'room join',
      'room send',
    ]);
  });
});

describe('expectedMigrations', () => {
  it('counts the entries of the CLI migration list named in the component map', () => {
    const root = path.join(directory, 'repository');
    mkdirSync(path.join(root, '.github'), { recursive: true });
    mkdirSync(path.join(root, 'rust'), { recursive: true });
    writeFileSync(
      path.join(root, '.github/components.json'),
      JSON.stringify({
        components: {
          cli: { package: 'tmt-cli', owns: ['.'], migrations: ['rust/migrations.rs'] },
        },
      })
    );
    writeFileSync(
      path.join(root, 'rust/migrations.rs'),
      'const MIGRATIONS: &[Migration] = &[\n  Migration { sql: "a" },\n  Migration { sql: "b" },\n];\n'
    );
    expect(expectedMigrations(pathToFileURL(`${root}/`))).toBe(2);
  });

  it('reads the real repository, whose CLI lists its migrations', () => {
    expect(expectedMigrations()).toBeGreaterThan(40);
  });
});
