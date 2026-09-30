//! Schema 40's change cursor: which tables advance it, and that every covered
//! table does so on insert, update and delete.
use std::collections::BTreeSet;

use crate::{storage::Storage, test_support::TestDirectory};
use rusqlite::{Connection, types::Value};

/// Bookkeeping, not records: migration history, extension cutover receipts
/// and the counter itself.
const BOOKKEEPING: &[&str] = &["_migrations", "change_cursor", "extension_storage_cutovers"];

/// Columns whose refresh is an observation made by reads, not a change.
const OBSERVATIONS: &[(&str, &str)] = &[("bindings", "last_verified_at")];

fn migrated() -> (TestDirectory, Connection) {
    let directory = TestDirectory::new();
    let path = directory.path.join("state.db");
    Storage::open(&path).unwrap().close().unwrap();
    let connection = Connection::open(&path).unwrap();
    (directory, connection)
}

fn strings(connection: &Connection, sql: &str) -> Vec<String> {
    let mut statement = connection.prepare(sql).unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Tables whose data belongs to an extension: fenced by its storage cutover.
fn extension_owned(connection: &Connection) -> BTreeSet<String> {
    strings(
        connection,
        "SELECT DISTINCT tbl_name FROM sqlite_schema WHERE type = 'trigger' \
         AND sql LIKE '%extension_storage_cutovers%'",
    )
    .into_iter()
    .collect()
}

/// Every table that is neither bookkeeping nor extension-owned.
fn covered(connection: &Connection) -> Vec<String> {
    let owned = extension_owned(connection);
    strings(
        connection,
        "SELECT name FROM sqlite_schema WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .into_iter()
    .filter(|table| !BOOKKEEPING.contains(&table.as_str()) && !owned.contains(table))
    .collect()
}

fn cursor(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap()
}

/// A future table, or a rebuilt one that lost its triggers, fails here until
/// it is covered or deliberately excluded.
#[test]
fn every_core_table_has_the_three_cursor_triggers() {
    let (_directory, connection) = migrated();
    let tables = covered(&connection);
    assert!(tables.len() >= 18, "{tables:?}");
    for table in &tables {
        let triggers = strings(
            &connection,
            &format!(
                "SELECT name FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = '{table}' \
                 AND name LIKE '%_advances_change_cursor_on_%' ORDER BY name"
            ),
        );
        let expected: Vec<String> = ["delete", "insert", "update"]
            .iter()
            .map(|operation| format!("{table}_advances_change_cursor_on_{operation}"))
            .collect();
        assert_eq!(triggers, expected, "{table}");
    }
    // Extension-owned tables and bookkeeping never advance it.
    let stray = strings(
        &connection,
        "SELECT tbl_name FROM sqlite_schema WHERE type = 'trigger' \
         AND name LIKE '%_advances_change_cursor_on_%'",
    );
    assert!(
        stray.iter().all(|table| tables.contains(table)),
        "{stray:?}"
    );
}

/// A value of the declared type for every column, so a row can be written
/// with checks and foreign keys off: the triggers are what is under test.
fn row_values(connection: &Connection, table: &str) -> (Vec<String>, Vec<Value>) {
    let mut statement = connection
        .prepare(&format!(
            "SELECT name, type FROM pragma_table_info('{table}')"
        ))
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .map(|(name, kind)| {
            let value = match kind.to_ascii_uppercase().as_str() {
                kind if kind.contains("INT") => Value::Integer(1),
                kind if kind.contains("BLOB") => Value::Blob(vec![0]),
                kind if kind.contains("REAL") => Value::Real(1.0),
                _ => Value::Text("x".into()),
            };
            (name, value)
        })
        .unzip()
}

#[test]
fn insert_update_and_delete_each_advance_the_cursor_on_every_core_table() {
    let (_directory, connection) = migrated();
    connection
        .execute_batch("PRAGMA foreign_keys = OFF; PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    for table in covered(&connection) {
        let (columns, values) = row_values(&connection, &table);
        let before = cursor(&connection);
        connection
            .execute(
                &format!(
                    "INSERT INTO {table} ({}) VALUES ({})",
                    columns.join(", "),
                    vec!["?"; columns.len()].join(", ")
                ),
                rusqlite::params_from_iter(&values),
            )
            .unwrap_or_else(|error| panic!("{table}: {error}"));
        assert_eq!(cursor(&connection), before + 1, "{table} insert");

        // Rewriting every column with its own value is not a change.
        let same: Vec<String> = columns
            .iter()
            .map(|column| format!("{column} = {column}"))
            .collect();
        connection
            .execute(&format!("UPDATE {table} SET {}", same.join(", ")), [])
            .unwrap();
        assert_eq!(cursor(&connection), before + 1, "{table} same-value update");

        // A new value in any compared column is; one column at a time, so a
        // column added later without being compared fails here.
        let mut expected = before + 1;
        for (column, value) in columns.iter().zip(&values) {
            if OBSERVATIONS.contains(&(table.as_str(), column.as_str())) {
                continue;
            }
            let other = match value {
                Value::Integer(_) => Value::Integer(2),
                Value::Blob(_) => Value::Blob(vec![1]),
                Value::Real(_) => Value::Real(2.0),
                _ => Value::Text(format!("{column}-changed")),
            };
            connection
                .execute(&format!("UPDATE {table} SET {column} = ?"), [other])
                .unwrap();
            expected += 1;
            assert_eq!(cursor(&connection), expected, "{table}.{column} update");
        }

        connection
            .execute(&format!("DELETE FROM {table}"), [])
            .unwrap();
        assert_eq!(cursor(&connection), expected + 1, "{table} delete");
    }
}

#[test]
fn an_observation_column_alone_is_not_a_change() {
    let (_directory, connection) = migrated();
    connection
        .execute_batch("PRAGMA foreign_keys = OFF; PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    for (table, column) in OBSERVATIONS {
        let (columns, values) = row_values(&connection, table);
        connection
            .execute(
                &format!(
                    "INSERT INTO {table} ({}) VALUES ({})",
                    columns.join(", "),
                    vec!["?"; columns.len()].join(", ")
                ),
                rusqlite::params_from_iter(&values),
            )
            .unwrap();
        let before = cursor(&connection);
        connection
            .execute(&format!("UPDATE {table} SET {column} = 'later'"), [])
            .unwrap();
        assert_eq!(cursor(&connection), before, "{table}.{column}");
    }
}

#[test]
fn extension_owned_tables_do_not_advance_the_cursor() {
    let (_directory, connection) = migrated();
    let owned = extension_owned(&connection);
    assert!(!owned.is_empty());
    connection
        .execute_batch("PRAGMA foreign_keys = OFF; PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    let before = cursor(&connection);
    for table in &owned {
        let (columns, values) = row_values(&connection, table);
        // Some of these tables' own triggers refuse synthetic rows; the cursor
        // must not move either way.
        let _ = connection.execute(
            &format!(
                "INSERT INTO {table} ({}) VALUES ({})",
                columns.join(", "),
                vec!["?"; columns.len()].join(", ")
            ),
            rusqlite::params_from_iter(&values),
        );
        let _ = connection.execute(&format!("DELETE FROM {table}"), []);
    }
    assert_eq!(cursor(&connection), before);
}
