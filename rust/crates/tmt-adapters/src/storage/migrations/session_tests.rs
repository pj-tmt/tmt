use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn session_upgrade_preserves_binding_and_rolls_back_observations_with_history() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema32.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..32].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch(
        "INSERT INTO bindings VALUES ('binding', 'old-id', 'tmux', '%3', 'server', '/tmp/fixture', 41, 'start', 51, 'bound', 'verified');
         CREATE TRIGGER reject_session_migration BEFORE INSERT ON _migrations WHEN NEW.version=33
         BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;",
    ).unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(33)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        32
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM pragma_table_info('bindings')",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        11
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='identity_session_preferences'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    oracle
        .execute_batch("DROP TRIGGER reject_session_migration;")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 33);
    let row = oracle.query_row(
        "SELECT id, identity_id, pane_id, bound_at, last_verified_at, runtime_state, last_transition FROM bindings",
        [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, Option<String>>(6)?)),
    ).unwrap();
    assert_eq!(
        row,
        (
            "binding".into(),
            "old-id".into(),
            "%3".into(),
            "bound".into(),
            "verified".into(),
            "unknown".into(),
            None
        )
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM identity_session_preferences",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT content FROM role_profiles WHERE identity_id='old-id'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "original role"
    );
    storage.close().unwrap();
}
