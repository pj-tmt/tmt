use super::{host_names_tests::*, *};
use crate::test_support::TestDirectory;

const TOKEN: &str = "ps-v1:Thu Oct 1 09:00:00 2026";

/// The schema-40 fixture, migrated through 41.
fn schema_41(path: &std::path::Path) -> Connection {
    let mut db = schema_40(path);
    apply_through(&mut db, 41).unwrap();
    db
}

fn incarnation(db: &Connection) -> Option<String> {
    db.query_row(
        "SELECT pane_incarnation FROM bindings WHERE id = 'binding'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

/// Every schema object, by type and name, with whitespace-normalized SQL.
fn schema(db: &Connection) -> Vec<(String, String, String)> {
    let mut statement = db
        .prepare("SELECT type, name, coalesce(sql, '') FROM sqlite_schema ORDER BY type, name")
        .unwrap();
    statement
        .query_map([], |row| {
            let sql: String = row.get(2)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                sql.replace('"', "")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn the_upgrade_adds_an_unknown_pane_incarnation_and_keeps_every_binding() {
    let directory = TestDirectory::new();
    let mut db = schema_41(&directory.path.join("schema41.db"));
    let before = rows(&db, "bindings");
    let before_sql = table_sql(&db, "bindings");

    apply_through(&mut db, 42).unwrap();

    assert_eq!(version(&db), 42);
    assert!(foreign_keys(&db));
    // Every existing binding keeps its values and gains an unknown incarnation.
    let after = rows(&db, "bindings");
    assert_eq!(after.len(), before.len());
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(&new[..old.len()], old.as_slice());
        assert_eq!(new[old.len()..], [rusqlite::types::Value::Null]);
    }
    assert_eq!(incarnation(&db), None);
    // The only change to the table is the one added column, which SQLite
    // places after the last column and before the table constraints.
    let column = "pane_incarnation TEXT CHECK (pane_incarnation IS NULL OR \
        (length(pane_incarnation) BETWEEN 1 AND 256 AND trim(pane_incarnation) <> '' \
        AND pane_incarnation NOT GLOB '*[^ -~]*'))";
    let words = |sql: &str| sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let after_sql = words(&table_sql(&db, "bindings"));
    assert_eq!(after_sql.matches(&words(column)).count(), 1, "{after_sql}");
    assert_eq!(
        after_sql.replacen(&format!(", {}", words(column)), "", 1),
        words(&before_sql)
    );
}

#[test]
fn an_upgraded_database_is_exactly_a_fresh_schema_42() {
    let directory = TestDirectory::new();
    let mut db = schema_41(&directory.path.join("schema41.db"));
    apply_through(&mut db, 42).unwrap();
    assert_eq!(schema(&db), schema(&fresh_through(42).unwrap()));
}

#[test]
fn recording_or_changing_the_incarnation_advances_the_change_cursor() {
    let directory = TestDirectory::new();
    let mut db = schema_41(&directory.path.join("schema41.db"));
    apply_through(&mut db, 42).unwrap();

    let before = cursor(&db);
    db.execute(
        "UPDATE bindings SET pane_incarnation = ?1 WHERE id = 'binding'",
        [TOKEN],
    )
    .unwrap();
    assert_eq!(cursor(&db), before + 1, "recorded");
    db.execute(
        "UPDATE bindings SET pane_incarnation = ?1 WHERE id = 'binding'",
        [TOKEN],
    )
    .unwrap();
    assert_eq!(cursor(&db), before + 1, "the same value is no change");
    db.execute(
        "UPDATE bindings SET last_verified_at = 'later' WHERE id = 'binding'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(&db), before + 1, "an observation alone is no change");
    db.execute(
        "UPDATE bindings SET pane_incarnation = NULL WHERE id = 'binding'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(&db), before + 2, "cleared");
}

#[test]
fn a_pane_incarnation_is_one_to_256_printable_ascii_bytes_not_all_blank() {
    let directory = TestDirectory::new();
    let mut db = schema_41(&directory.path.join("schema41.db"));
    apply_through(&mut db, 42).unwrap();
    let set = |value: &str| {
        db.execute(
            "UPDATE bindings SET pane_incarnation = ?1 WHERE id = 'binding'",
            [value],
        )
    };
    for valid in [TOKEN, "x", &"~".repeat(256)] {
        set(valid).unwrap();
        assert_eq!(incarnation(&db).as_deref(), Some(valid));
    }
    for invalid in [
        "",
        "   ",
        &"x".repeat(257),
        "a\tb",
        "a\nb",
        "caf\u{e9}",
        "\u{7f}",
    ] {
        assert!(set(invalid).is_err(), "{invalid:?} was accepted");
    }
}

#[test]
fn a_failure_while_recording_42_leaves_schema_41_unchanged() {
    let directory = TestDirectory::new();
    let mut db = schema_41(&directory.path.join("schema41.db"));
    db.execute_batch(
        "CREATE TRIGGER fail_42 BEFORE INSERT ON _migrations WHEN NEW.version = 42
         BEGIN SELECT RAISE(ABORT, 'injected'); END",
    )
    .unwrap();
    let before = schema(&db);
    let rows_before = rows(&db, "bindings");

    let error = apply_through(&mut db, 42).unwrap_err();
    assert_eq!(error.migration_version, Some(42));
    assert_eq!(version(&db), 41);
    assert_eq!(schema(&db), before, "no column and the old trigger");
    assert_eq!(rows(&db, "bindings"), rows_before);
    assert!(foreign_keys(&db));
}
