//! Office storage schema and the source-shape check that guards raw copies.

use rusqlite::Connection;

/// Office schema versions in order; staging from an earlier build is upgraded
/// when it is published.
const MIGRATIONS: &[(&str, &str)] = &[
    ("office-storage-v1", include_str!("schema/001.sql")),
    ("office-activation", include_str!("schema/002.sql")),
    ("office-retired-rooms", include_str!("schema/003.sql")),
];
pub(crate) const VERSION: i64 = MIGRATIONS.len() as i64;
/// The Office tables' shape, which the core copy must match.
const SQL: &str = MIGRATIONS[0].1;

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
    upgrade(connection, 0)
}

/// Brings an existing Office database to the current schema. The version is
/// read inside the immediate transaction, so concurrent openers apply each
/// migration once; a newer schema is left untouched and reported.
pub(crate) fn upgrade_existing(connection: &Connection) -> rusqlite::Result<Result<(), String>> {
    connection.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        let version: i64 = connection.query_row(
            "SELECT coalesce(max(version), 0) FROM _office_schema",
            [],
            |row| row.get(0),
        )?;
        if version > VERSION {
            return Ok(Err(format!(
                "Office storage uses schema {version}, newer than this Office supports; update Office."
            )));
        }
        for (index, (name, sql)) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT INTO _office_schema (version, name) VALUES (?, ?)",
                rusqlite::params![index as i64 + 1, name],
            )?;
        }
        Ok(Ok(()))
    })();
    match result {
        Ok(outcome) => {
            connection.execute_batch("COMMIT")?;
            Ok(outcome)
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Applies every migration after `from` in one transaction.
pub(crate) fn upgrade(connection: &Connection, from: i64) -> rusqlite::Result<()> {
    let mut batch = "BEGIN IMMEDIATE;\n".to_owned();
    for (index, (name, sql)) in MIGRATIONS.iter().enumerate().skip(from as usize) {
        batch.push_str(&format!(
            "{sql}\nINSERT INTO _office_schema (version, name) VALUES ({}, '{name}');\n",
            index + 1
        ));
    }
    batch.push_str("COMMIT;");
    connection.execute_batch(&batch)
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
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get(2)?))
        })?
        .filter(|object| {
            !matches!(object, Ok((kind, name, _)) if kind == "trigger" && is_core_fence(name))
        })
        .collect()
}

/// Core schema 36 fences each Office table with these triggers after the
/// cutover receipt; they belong to core, not to the Office table shape.
pub(crate) fn core_fences() -> impl Iterator<Item = String> {
    OFFICE_TABLES.iter().flat_map(|table| {
        ["insert", "update", "delete"]
            .map(|operation| format!("{table}_after_office_cutover_{operation}"))
    })
}

fn is_core_fence(name: &str) -> bool {
    core_fences().any(|fence| fence == name)
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
