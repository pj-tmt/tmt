//! Schema 41 (#570): the host columns accept any host name, so bindings,
//! request fences and server records can hold an approved driver's host.
//!
//! SQLite can't alter a CHECK, so each of the four tables is rebuilt: from
//! its own stored definition with exactly one CHECK clause replaced, its rows
//! copied unchanged, and its stored indexes and triggers (the schema-40
//! change-cursor triggers among them) replayed verbatim. Nothing is written by
//! hand a second time. The source must match what migrations 1 through 40
//! produce in a fresh database, for these tables and for everything that
//! mentions them; anything else (a custom column, index or trigger) is
//! refused rather than rebuilt.
//!
//! The rule is the host-name grammar of `tmt-host-grammar`: 1 to 32 of
//! `[a-z0-9-]`, starting with a letter, so `tmux` and `herdr` stay valid.

use super::{StorageError, classify, incompatible};
use rusqlite::{Connection, params};

/// Each rebuilt table, its schema-40 host CHECK, and the widened one.
pub(super) const TABLES: [(&str, &str, &str); 4] = [
    (
        "bindings",
        "CHECK (transport IN ('tmux', 'herdr'))",
        "CHECK (length(transport) BETWEEN 1 AND 32 AND transport GLOB '[a-z]*'
    AND transport NOT GLOB '*[^a-z0-9-]*')",
    ),
    (
        "request_attempts",
        "CHECK (host IS NULL OR (route_kind = 'pane' AND host IN ('tmux', 'herdr')))",
        "CHECK (host IS NULL OR (route_kind = 'pane' AND length(host) BETWEEN 1 AND 32
    AND host GLOB '[a-z]*' AND host NOT GLOB '*[^a-z0-9-]*'))",
    ),
    (
        "request_responses",
        "CHECK (host IS NULL OR host IN ('tmux', 'herdr'))",
        "CHECK (host IS NULL OR (length(host) BETWEEN 1 AND 32
    AND host GLOB '[a-z]*' AND host NOT GLOB '*[^a-z0-9-]*'))",
    ),
    (
        // tmux keeps its own server IDs; every other host may record here.
        "host_servers",
        "CHECK (host IN ('herdr'))",
        "CHECK (host <> 'tmux' AND length(host) BETWEEN 1 AND 32
    AND host GLOB '[a-z]*' AND host NOT GLOB '*[^a-z0-9-]*')",
    ),
];

/// A schema object: type, name, table and whitespace-normalized SQL.
type Object = (String, String, String, String);

/// Every object on the rebuilt tables, and every other object whose SQL
/// mentions one of them, in a stable order.
fn objects(connection: &Connection) -> Result<Vec<Object>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT type, name, tbl_name, coalesce(sql, '') FROM sqlite_schema
             WHERE tbl_name IN (?1, ?2, ?3, ?4)
                OR (sql IS NOT NULL AND (instr(sql, ?1) OR instr(sql, ?2)
                    OR instr(sql, ?3) OR instr(sql, ?4)))
             ORDER BY type, name",
        )
        .map_err(|error| classify(error, "Inspect host source schema"))?;
    let rows = statement
        .query_map(
            params![TABLES[0].0, TABLES[1].0, TABLES[2].0, TABLES[3].0],
            |row| {
                let sql: String = row.get(3)?;
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    // The 039 rename quotes one table name; compare words.
                    sql.replace('"', "")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                ))
            },
        )
        .map_err(|error| classify(error, "Inspect host source schema"))?;
    rows.collect::<Result<_, _>>()
        .map_err(|error| classify(error, "Inspect host source schema"))
}

/// Refuses a source that isn't exactly what migrations 1 through 40 make.
pub(super) fn validate_source(
    connection: &Connection,
    expected: &Connection,
) -> Result<(), StorageError> {
    if objects(connection)? != objects(expected)? {
        return Err(incompatible(
            "Host-name migration requires the schema-40 definitions of bindings, \
             request_attempts, request_responses and host_servers",
        ));
    }
    Ok(())
}

fn stored_sql(
    connection: &Connection,
    kind: &str,
    table: &str,
) -> Result<Vec<String>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT sql FROM sqlite_schema
             WHERE type = ?1 AND tbl_name = ?2 AND sql IS NOT NULL ORDER BY name",
        )
        .map_err(|error| classify(error, "Read host table definition"))?;
    let rows = statement
        .query_map(params![kind, table], |row| row.get(0))
        .map_err(|error| classify(error, "Read host table definition"))?;
    rows.collect::<Result<_, _>>()
        .map_err(|error| classify(error, "Read host table definition"))
}

/// Rebuilds the four tables inside the caller's transaction, with foreign
/// keys already off: dropping `request_attempts` must not cascade into
/// `request_notifications`.
pub(super) fn rebuild(connection: &Connection) -> Result<(), StorageError> {
    for (table, old, new) in TABLES {
        let definitions = stored_sql(connection, "table", table)?;
        let [definition] = definitions.as_slice() else {
            return Err(incompatible("Host-name migration found no single table"));
        };
        if definition.matches(old).count() != 1 {
            return Err(incompatible(
                "Host-name migration requires exactly one schema-40 host rule",
            ));
        }
        let body = &definition[definition
            .find('(')
            .ok_or_else(|| incompatible("Host-name migration found no column list"))?..];
        let staged = format!("{table}_041");
        let create = format!("CREATE TABLE {staged} {}", body.replacen(old, new, 1));
        let dependents: Vec<String> = ["index", "trigger"]
            .into_iter()
            .map(|kind| stored_sql(connection, kind, table))
            .collect::<Result<Vec<_>, _>>()?
            .concat();
        connection
            .execute_batch(&create)
            .and_then(|()| {
                connection.execute_batch(&format!(
                    "INSERT INTO {staged} SELECT * FROM {table};
                     DROP TABLE {table};
                     ALTER TABLE {staged} RENAME TO {table};"
                ))
            })
            .map_err(|error| classify(error, "Rebuild host table"))?;
        for sql in dependents {
            connection
                .execute_batch(&sql)
                .map_err(|error| classify(error, "Restore host table index or trigger"))?;
        }
    }
    Ok(())
}
