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

#[test]
fn reply_batches_upgrade_schema_43_atomically_and_preserve_automatic_identity_provenance() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema43.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 43).unwrap();
    old.execute("INSERT INTO identities (id, name, canonical_name, lifetime, created_at, updated_at, auto_named) VALUES ('automatic', 'claude-012345678abc', 'claude-012345678abc', 'temporary', 'created', 'updated', 1)", []).unwrap();
    old.execute_batch(
        "CREATE TRIGGER reject_reply_batches BEFORE INSERT ON _migrations WHEN NEW.version=44
      BEGIN SELECT RAISE(ABORT, 'injected reply batch migration failure'); END;",
    )
    .unwrap();
    old.close().unwrap();

    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(44)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        43
    );
    assert_eq!(oracle.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('reply_notice_batches', 'reply_notices')", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    let retained = || {
        oracle.query_row("SELECT name, lifetime, created_at, updated_at, auto_named FROM identities WHERE id='automatic'", [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?))).unwrap()
    };
    let expected = (
        "claude-012345678abc".into(),
        "temporary".into(),
        "created".into(),
        "updated".into(),
        1,
    );
    assert_eq!(retained(), expected);
    oracle
        .execute_batch("DROP TRIGGER reject_reply_batches")
        .unwrap();
    let mut upgraded = Storage::open(&path).unwrap();
    assert_eq!(upgraded.health().unwrap().schema_version, 48);
    assert_eq!(retained(), expected);
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM reply_notice_batches", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM reply_notices", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    upgraded.close().unwrap();
    let mut reopened = Storage::open(&path).unwrap();
    assert_eq!(reopened.health().unwrap().schema_version, 48);
    assert_eq!(retained(), expected);
    reopened.close().unwrap();
}
