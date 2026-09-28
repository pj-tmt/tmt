//! Office storage schema v1 and the source-shape check that guards raw copies.

use rusqlite::Connection;

pub(crate) const VERSION: i64 = 1;
const SQL: &str = include_str!("schema/001.sql");

/// Office-owned tables in copy order: parents before children, and retained
/// legacy blocks before worlds, because the schema 28 trigger rejects block
/// inserts once a world layout exists.
pub(crate) const OFFICE_TABLES: &[&str] = &[
    "office_local_blocks",
    "office_local_worlds",
    "office_local_profiles",
    "office_board_state",
    "office_board_entries",
    "office_board_operations",
    "office_whiteboards",
    "office_whiteboard_operations",
    "office_whiteboard_snapshots",
    "office_whiteboard_snapshot_images",
    "office_prop_packs",
    "office_prop_catalog",
    "office_avatar_packs",
    "office_avatar_catalog",
];

/// Core-owned references Office replaces with preflight; the only allowed
/// difference between the core and Office definitions of these tables.
const CORE_REFERENCE: &str = " REFERENCES identities(id)";

pub(crate) fn install(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(&format!(
        "BEGIN IMMEDIATE;\n{SQL}\nINSERT INTO _office_schema (version, name) VALUES ({VERSION}, 'office-storage-v1');\nCOMMIT;"
    ))
}

type Objects = Vec<(String, String, String)>;

fn office_objects(connection: &Connection) -> rusqlite::Result<Objects> {
    let placeholders = vec!["?"; OFFICE_TABLES.len()].join(", ");
    let mut statement = connection.prepare(&format!(
        "SELECT type, name, sql FROM sqlite_master \
         WHERE sql IS NOT NULL AND tbl_name IN ({placeholders}) ORDER BY type, name"
    ))?;
    statement
        .query_map(rusqlite::params_from_iter(OFFICE_TABLES), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect()
}

/// Rejects a source whose Office tables, indexes or triggers differ from the
/// Office schema, so raw cells are never copied into a mismatched shape.
pub(crate) fn source_matches(source: &Connection) -> rusqlite::Result<Result<(), String>> {
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(SQL)?;
    let expected = office_objects(&expected)?;
    let actual: Objects = office_objects(source)?
        .into_iter()
        .map(|(kind, name, sql)| (kind, name, sql.replacen(CORE_REFERENCE, "", 1)))
        .collect();
    if actual == expected {
        return Ok(Ok(()));
    }
    let first = expected
        .iter()
        .zip(&actual)
        .find(|(left, right)| left != right)
        .map(|(left, _)| format!("{} {}", left.0, left.1))
        .unwrap_or_else(|| "object count".to_owned());
    Ok(Err(format!(
        "The core database's Office schema differs from Office storage v{VERSION} at {first}."
    )))
}
