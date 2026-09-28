//! Schema 36 records extension cutovers and fences Office rows after Office's receipt.
use std::collections::BTreeMap;

use crate::{storage::Storage, test_support::TestDirectory};
use rusqlite::Connection;

const FENCE: &str = "Office data moved to Office storage";

fn fenced(result: rusqlite::Result<usize>) -> bool {
    matches!(result, Err(error) if error.to_string().contains(FENCE))
}

#[test]
fn office_receipt_fences_every_office_table_for_already_open_writers() {
    let directory = TestDirectory::new();
    let path = directory.path.join("state.db");
    Storage::open(&path).unwrap().close().unwrap();
    let old_writer = Connection::open(&path).unwrap();
    let mut triggers = BTreeMap::<String, Vec<String>>::new();
    let mut statement = old_writer
        .prepare(
            "SELECT tbl_name, name FROM sqlite_schema WHERE type = 'trigger' \
             AND sql LIKE '%extension_storage_cutovers%' ORDER BY tbl_name, name",
        )
        .unwrap();
    for row in statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
    {
        let (table, trigger) = row.unwrap();
        triggers.entry(table).or_default().push(trigger);
    }
    drop(statement);
    assert_eq!(triggers.len(), 14);
    for (table, names) in &triggers {
        assert!(table.starts_with("office_"), "{table}");
        assert!(
            !table.starts_with("office_meeting_"),
            "{table} is core-owned"
        );
        let suffixes: Vec<&str> = names
            .iter()
            .map(|name| name.rsplit('_').next().unwrap())
            .collect();
        assert_eq!(suffixes, ["delete", "insert", "update"], "{table}");
    }

    // Before any receipt, and with another extension's receipt, Office rows stay writable.
    let receipts = Connection::open(&path).unwrap();
    receipts
        .execute(
            "INSERT INTO extension_storage_cutovers VALUES ('squad', 1, 2, 1, ?, 3)",
            ["b".repeat(64)],
        )
        .unwrap();
    assert_eq!(
        old_writer
            .execute("UPDATE office_board_state SET revision = revision", [])
            .unwrap(),
        1
    );

    receipts
        .execute(
            "INSERT INTO extension_storage_cutovers VALUES ('office', 1, 2, 2, ?, 3)",
            ["a".repeat(64)],
        )
        .unwrap();
    for table in triggers.keys() {
        // BEFORE triggers run ahead of constraint checks, so an empty row reaches the fence.
        assert!(
            fenced(old_writer.execute(&format!("INSERT INTO {table} DEFAULT VALUES"), [])),
            "{table} insert"
        );
    }
    for table in [
        "office_board_state",
        "office_prop_catalog",
        "office_avatar_catalog",
    ] {
        assert!(
            fenced(old_writer.execute(&format!("UPDATE {table} SET revision = revision"), [])),
            "{table} update"
        );
        assert!(
            fenced(old_writer.execute(&format!("DELETE FROM {table}"), [])),
            "{table} delete"
        );
    }
    // Core-owned rows, including meeting rooms, stay writable.
    old_writer
        .execute("DELETE FROM office_meeting_rooms", [])
        .unwrap();
    let reopened = Storage::open(&path).unwrap();
    assert!(
        reopened
            .extension_storage_cutover("office")
            .unwrap()
            .is_some()
    );
}
