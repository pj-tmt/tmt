use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn whiteboard_upgrade_preserves_prior_content_and_rolls_back_with_history() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema20.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..20].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch("CREATE TRIGGER reject_whiteboard_upgrade BEFORE INSERT ON _migrations WHEN NEW.version=21 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(21)
    );
    let observer = Connection::open(&path).unwrap();
    assert_eq!(
        observer
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        20
    );
    assert_eq!(observer.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('office_whiteboards','office_whiteboard_operations')", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    observer
        .execute_batch("DROP TRIGGER reject_whiteboard_upgrade")
        .unwrap();
    observer.close().unwrap();
    let storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 41);
    let connection = storage.connection().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT content FROM role_profiles WHERE identity_id='old-id'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "original role"
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM office_whiteboards", [], |row| row
                .get::<_, i64>(0))
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
