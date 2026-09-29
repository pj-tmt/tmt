use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn snapshot_upgrade_preserves_saved_scene_and_rolls_back_schema_with_history() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema21.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..21].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    let world_id = "11111111-1111-4111-8111-111111111111";
    old.execute(
        "INSERT INTO office_local_worlds (singleton,id,created_at_ms) VALUES (1,?,100)",
        [world_id],
    )
    .unwrap();
    let original = std::str::from_utf8(include_bytes!(
        "../../../../../../extensions/tmt-office/contracts/whiteboard-scene-v1.json"
    ))
    .unwrap();
    old.execute("INSERT INTO office_whiteboards (document_id,world_id,revision,scene,updated_at_ms) VALUES ('lobby',?,1,?,100)", params![world_id, original]).unwrap();
    old.execute_batch("CREATE TRIGGER reject_snapshot_upgrade BEFORE INSERT ON _migrations WHEN NEW.version=22 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(22)
    );
    let observer = Connection::open(&path).unwrap();
    assert_eq!(
        observer
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        21
    );
    assert_eq!(
        observer
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name IN ('office_whiteboard_snapshots', 'office_whiteboard_snapshot_images')",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        observer
            .query_row(
                "SELECT scene FROM office_whiteboards WHERE document_id='lobby'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        original
    );
    observer
        .execute_batch("DROP TRIGGER reject_snapshot_upgrade")
        .unwrap();
    observer.close().unwrap();
    let storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 38);
    let connection = storage.connection().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT scene FROM office_whiteboards WHERE document_id='lobby'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        original
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT (SELECT count(*) FROM office_whiteboard_snapshots) + (SELECT count(*) FROM office_whiteboard_snapshot_images)",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}
