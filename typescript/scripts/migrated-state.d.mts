export interface StateSnapshot {
  /** Rows of the `_migrations` history. */
  readonly migrations: number;
  /** Rows per table, without `_migrations`, SQLite's own tables and virtual tables. */
  readonly tables: Readonly<Record<string, number>>;
  /** What `integrity_check` and `foreign_key_check` reported, as sentences. */
  readonly problems: readonly string[];
  /** The requested ids that no column of any table holds. */
  readonly missing: readonly string[];
}

export function expectedMigrations(root?: URL): number;
export function writePriorState(
  run: (args: string[]) => string,
  options?: { name?: string }
): string[];
export function snapshotState(databasePath: string, ids?: readonly string[]): StateSnapshot;
export function checkMigratedState(input: {
  before: StateSnapshot;
  after: StateSnapshot;
  expected: number;
}): string[];
