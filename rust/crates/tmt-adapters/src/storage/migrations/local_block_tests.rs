//! Local layout upgrades preserve exact records and remain transactional.
use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn lobby_upgrade_preserves_identity_layout_and_rolls_back_on_history_failure() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema18.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..18].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    let block_id = "11111111-1111-4111-8111-111111111111";
    let layout = r#"{"version":2,"objects":[]}"#;
    old.execute(
        "INSERT INTO office_local_blocks VALUES (?, 'old-id', 7, ?, 123)",
        params![block_id, layout],
    )
    .unwrap();
    old.execute_batch(
        "CREATE TRIGGER reject_nineteenth BEFORE INSERT ON _migrations
        WHEN NEW.version = 19 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
    )
    .unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(19)
    );
    let observer = Connection::open(&path).unwrap();
    let schema: String = observer
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'office_local_blocks'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!schema.contains("target_kind"));
    let staging: i64 = observer
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'office_local_blocks_previous'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(staging, 0);
    observer
        .execute_batch("DROP TRIGGER reject_nineteenth")
        .unwrap();
    observer.close().unwrap();

    let mut upgraded = Storage::open(&path).unwrap();
    assert_eq!(upgraded.health().unwrap().schema_version, 40);
    let connection = upgraded.connection().unwrap();
    let row: (String, String, String, i64, String, i64) = connection.query_row(
        "SELECT block_id, target_kind, identity_id, revision, layout, updated_at_ms FROM office_local_blocks", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).unwrap();
    assert_eq!(
        row,
        (
            block_id.into(),
            "identity".into(),
            "old-id".into(),
            7,
            layout.into(),
            123
        )
    );
    let lobby_id = "22222222-2222-4222-8222-222222222222";
    connection
        .execute(
            "INSERT INTO office_local_blocks VALUES (?, 'lobby', NULL, 1, ?, 124)",
            params![lobby_id, layout],
        )
        .unwrap();
    for (kind, identity) in [
        ("lobby", None),
        ("identity", None),
        ("lobby", Some("old-id")),
    ] {
        assert!(
            connection
                .execute(
                    "INSERT INTO office_local_blocks VALUES (?, ?, ?, 1, ?, 125)",
                    params![
                        "33333333-3333-4333-8333-333333333333",
                        kind,
                        identity,
                        layout
                    ]
                )
                .is_err()
        );
    }
    let foreign_key_failures: i64 = connection
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(foreign_key_failures, 0);
    upgraded.close().unwrap();
}

#[test]
fn customization_upgrade_rolls_back_both_targets_then_preserves_their_exact_bytes() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema19.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..19].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    // Valid JSON close to the old limit: migration must not decode/re-encode it.
    let layout = format!("{}{}", r#"{"version":2,"objects":[]}"#, " ".repeat(4000));
    for (id, kind, identity) in [
        (
            "11111111-1111-4111-8111-111111111111",
            "identity",
            Some("old-id"),
        ),
        ("22222222-2222-4222-8222-222222222222", "lobby", None),
    ] {
        old.execute(
            "INSERT INTO office_local_blocks VALUES (?, ?, ?, 7, ?, 123)",
            params![id, kind, identity, layout],
        )
        .unwrap();
    }
    let rows = |connection: &Connection| {
        connection.prepare(
            "SELECT block_id, target_kind, identity_id, revision, CAST(layout AS BLOB), updated_at_ms
             FROM office_local_blocks ORDER BY block_id",
        ).unwrap().query_map([], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?, row.get::<_, Vec<u8>>(4)?, row.get::<_, i64>(5)?,
        ))).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
    };
    let definitions = |connection: &Connection| {
        connection
            .prepare(
                "SELECT name, sql FROM sqlite_schema WHERE tbl_name = 'office_local_blocks'
             ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let before_rows = rows(&old);
    let before_definitions = definitions(&old);
    old.execute_batch(
        "CREATE TRIGGER reject_twentieth BEFORE INSERT ON _migrations
         WHEN NEW.version = 20 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
    )
    .unwrap();
    old.close().unwrap();

    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(20)
    );
    let observer = Connection::open(&path).unwrap();
    assert_eq!(rows(&observer), before_rows);
    assert_eq!(definitions(&observer), before_definitions);
    assert_eq!(
        observer
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        19
    );
    assert_eq!(
        observer
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name = 'office_local_blocks_previous'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    observer
        .execute_batch("DROP TRIGGER reject_twentieth")
        .unwrap();
    observer.close().unwrap();

    let mut upgraded = Storage::open(&path).unwrap();
    assert_eq!(upgraded.health().unwrap().schema_version, 40);
    let connection = upgraded.connection().unwrap();
    assert_eq!(rows(connection), before_rows);
    let wider = format!("{}{}", r#"{"version":3,"objects":[]}"#, " ".repeat(6000));
    assert_eq!(
        connection
            .execute("UPDATE office_local_blocks SET layout = ?", [&wider])
            .unwrap(),
        2
    );
    let oversized = format!("{}{}", r#"{"version":3,"objects":[]}"#, " ".repeat(8192));
    assert!(
        connection
            .execute("UPDATE office_local_blocks SET layout = ?", [&oversized])
            .is_err()
    );
    upgraded.close().unwrap();
}
