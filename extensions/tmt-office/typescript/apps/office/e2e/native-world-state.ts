import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';

/** Read-only storage oracle: do not observe persistence through the HTTP/CLI projection under test. */
export function savedWorld(databasePath: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    const row = database
      .prepare(
        'SELECT id AS worldId, layout_revision AS revision, layout_json AS layout, layout_updated_at_ms AS updatedAtMs FROM office_local_worlds'
      )
      .get() as
      | { worldId: string; revision: number; layout: string; updatedAtMs: number }
      | undefined;
    if (!row) throw new Error('Expected a materialized Office world.');
    return { ...row, layoutBytes: Buffer.from(row.layout) };
  } finally {
    database.close();
  }
}
