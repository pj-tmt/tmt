use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn notifications_migrate_atomically_without_opting_historical_requests_in() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema34.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..34].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    let count: i64 = old
        .query_row("SELECT count(*) FROM request_attempts", [], |row| {
            row.get(0)
        })
        .unwrap();
    old.execute_batch("CREATE TRIGGER reject_notification_migration BEFORE INSERT ON _migrations WHEN NEW.version=35
      BEGIN SELECT RAISE(ABORT, 'injected notification migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(35)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        34
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='request_notifications'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    oracle
        .execute_batch("DROP TRIGGER reject_notification_migration")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM request_notifications", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        count
    );
    storage.close().unwrap();
}
