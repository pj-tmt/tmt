//! Schema-17 upgrade preserves exact catalog content and rolls back atomically.
use super::*;
use crate::{office_prop, storage::Storage, test_support::TestDirectory};

#[test]
fn prop_bound_upgrade_preserves_content_and_rolls_back_on_history_failure() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema17.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..17].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    let pack = office_prop::builtin_pack();
    let prop = &pack.pack().props[0];
    let layout = serde_json::json!({
        "version": 2,
        "objects": [{
            "prop": format!("{}/{}", pack.digest(), prop.key),
            "footprint": prop.footprint,
            "x": 4, "y": 5, "rotation": 0
        }]
    })
    .to_string();
    old.execute(
        "INSERT INTO office_local_blocks VALUES ('00000000-0000-4000-8000-000000000001', 'old-id', 3, ?, 123)",
        [&layout],
    ).unwrap();
    old.execute(
        "INSERT INTO office_prop_packs VALUES (?, ?, ?, 7, 123)",
        params![pack.digest(), pack.bytes(), pack.pack().props.len() as i64],
    )
    .unwrap();
    old.execute_batch(
        "UPDATE office_prop_catalog SET revision = 7;
        CREATE TRIGGER reject_eighteenth BEFORE INSERT ON _migrations
        WHEN NEW.version = 18 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
    )
    .unwrap();
    old.close().unwrap();

    let error = Storage::open(&path)
        .err()
        .expect("history failure must roll back schema 18");
    assert_eq!(error.migration_version, Some(18));
    let observer = Connection::open(&path).unwrap();
    let original_schema: String = observer
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'office_prop_packs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(original_schema.contains("131072"));
    let staging_tables: i64 = observer
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'office_prop_packs_previous'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(staging_tables, 0);
    let history_count: i64 = observer
        .query_row(
            "SELECT count(*) FROM _migrations WHERE version = 18",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history_count, 0);
    let preserved: (Vec<u8>, String) = observer
        .query_row(
            "SELECT bytes, layout FROM office_prop_packs CROSS JOIN office_local_blocks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(preserved, (pack.bytes().to_vec(), layout.clone()));
    observer
        .execute_batch("DROP TRIGGER reject_eighteenth")
        .unwrap();
    observer.close().unwrap();

    let mut upgraded = Storage::open(&path).unwrap();
    assert_eq!(upgraded.health().unwrap().schema_version, 33);
    let connection = upgraded.connection().unwrap();
    let row: (String, Vec<u8>, i64, i64, i64) = connection.query_row(
        "SELECT digest, bytes, prop_count, installed_revision, installed_at_ms FROM office_prop_packs",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).unwrap();
    assert_eq!(
        row,
        (
            pack.digest().into(),
            pack.bytes().to_vec(),
            pack.pack().props.len() as i64,
            7,
            123
        )
    );
    let revision: i64 = connection
        .query_row("SELECT revision FROM office_prop_catalog", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(revision, 7);
    let block: (String, i64, String, i64) = connection
        .query_row(
            "SELECT identity_id, revision, layout, updated_at_ms FROM office_local_blocks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(block, ("old-id".into(), 3, layout, 123));
    let history_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM _migrations WHERE version = 18",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history_count, 1);
    let role: String = connection
        .query_row(
            "SELECT content FROM role_profiles WHERE identity_id = 'old-id'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(role, "original role");
    upgraded.close().unwrap();
}
