//! Schema 40's change cursor: which tables advance it, and that every covered
//! table does so on insert, update and delete. The usage tables advance their own
//! cursor instead (schema 74, #5019).
use std::collections::BTreeSet;

use crate::{storage::Storage, test_support::TestDirectory};
use rusqlite::{Connection, types::Value};

/// Bookkeeping, not records: migration history, extension cutover receipts
/// and the counter itself.
const BOOKKEEPING: &[&str] = &[
    "_migrations",
    "change_cursor",
    "usage_cursor",
    "extension_storage_cutovers",
];

/// The sampler's tables: their writes are telemetry, not a change the board reloads for.
const USAGE: &[&str] = &[
    "consumption_sources",
    "consumption_buckets",
    "consumption_tier5m",
    "consumption_family",
    "consumption_family_files",
];

/// Append-only journals paged on demand, independently of board refreshes.
const ON_DEMAND_JOURNALS: &[&str] = &["orchestration_decisions"];

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

/// Board-relevant tables, excluding bookkeeping, on-demand journals, the sampler's usage tables
/// and extension data.
fn covered(connection: &Connection) -> Vec<String> {
    let owned = extension_owned(connection);
    strings(
        connection,
        "SELECT name FROM sqlite_schema WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .into_iter()
    .filter(|table| {
        !BOOKKEEPING.contains(&table.as_str())
            && !ON_DEMAND_JOURNALS.contains(&table.as_str())
            && !USAGE.contains(&table.as_str())
            && !owned.contains(table)
    })
    .collect()
}

fn counter(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(
            &format!("SELECT value FROM {table} WHERE id = 1"),
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn cursor(connection: &Connection) -> i64 {
    counter(connection, "change_cursor")
}

/// A future table, or a rebuilt one that lost its triggers, fails here until
/// it is covered or deliberately excluded.
#[test]
fn every_board_relevant_core_table_has_the_three_cursor_triggers() {
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
    // Excluded tables, including the on-demand journal, never advance it.
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

/// Insert, a same-value rewrite, each column changed alone, and delete, on `table`: `counter`
/// advances exactly when a row really changed, and `other` never moves.
fn assert_advances(connection: &Connection, table: &str, counter_table: &str, other: &str) {
    let (columns, values) = row_values(connection, table);
    let before = counter(connection, counter_table);
    let untouched = counter(connection, other);
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
    assert_eq!(
        counter(connection, counter_table),
        before + 1,
        "{table} insert"
    );

    // Rewriting every column with its own value is not a change.
    let same: Vec<String> = columns
        .iter()
        .map(|column| format!("{column} = {column}"))
        .collect();
    connection
        .execute(&format!("UPDATE {table} SET {}", same.join(", ")), [])
        .unwrap();
    assert_eq!(
        counter(connection, counter_table),
        before + 1,
        "{table} same-value update"
    );

    // A new value in any compared column is; one column at a time, so a
    // column added later without being compared fails here.
    let mut expected = before + 1;
    for (column, value) in columns.iter().zip(&values) {
        if OBSERVATIONS.contains(&(table, column.as_str())) {
            continue;
        }
        let other_value = match value {
            Value::Integer(_) => Value::Integer(2),
            Value::Blob(_) => Value::Blob(vec![1]),
            Value::Real(_) => Value::Real(2.0),
            _ => Value::Text(format!("{column}-changed")),
        };
        connection
            .execute(&format!("UPDATE {table} SET {column} = ?"), [other_value])
            .unwrap();
        expected += 1;
        assert_eq!(
            counter(connection, counter_table),
            expected,
            "{table}.{column} update"
        );
    }

    connection
        .execute(&format!("DELETE FROM {table}"), [])
        .unwrap();
    assert_eq!(
        counter(connection, counter_table),
        expected + 1,
        "{table} delete"
    );
    assert_eq!(
        counter(connection, other),
        untouched,
        "{table} moved {other}"
    );
}

#[test]
fn insert_update_and_delete_each_advance_the_cursor_on_every_board_relevant_core_table() {
    let (_directory, connection) = migrated();
    connection
        .execute_batch("PRAGMA foreign_keys = OFF; PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    for table in covered(&connection) {
        assert_advances(&connection, &table, "change_cursor", "usage_cursor");
    }
}

/// The sampler's writes wake nothing: they advance the usage cursor and leave the change cursor.
#[test]
fn insert_update_and_delete_each_advance_only_the_usage_cursor_on_every_usage_table() {
    let (_directory, connection) = migrated();
    connection
        .execute_batch("PRAGMA foreign_keys = OFF; PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    for table in USAGE {
        assert_advances(&connection, table, "usage_cursor", "change_cursor");
    }
}

#[test]
fn every_usage_table_has_the_three_usage_cursor_triggers() {
    let (_directory, connection) = migrated();
    for table in USAGE {
        let triggers = strings(
            &connection,
            &format!(
                "SELECT name FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = '{table}' \
                 AND name LIKE '%_advances_%_cursor_on_%' ORDER BY name"
            ),
        );
        let expected: Vec<String> = ["delete", "insert", "update"]
            .iter()
            .map(|operation| format!("{table}_advances_usage_cursor_on_{operation}"))
            .collect();
        assert_eq!(triggers, expected, "{table}");
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
