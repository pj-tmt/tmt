use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn launch_owner_upgrade_is_atomic_and_preserves_unowned_observations() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema33.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..33].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch(
        "INSERT INTO bindings (id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at,runtime_state,last_transition,runtime_pid,runtime_start_identity,observed_provider_session_id)
         VALUES ('binding','old-id','tmux','%3','server','/tmp/fixture',41,'start',51,'bound','verified','running','started',61,'child-start','session');
         INSERT INTO identity_session_preferences VALUES ('old-id','claude','claude','default','session');
         CREATE TRIGGER reject_owner_migration BEFORE INSERT ON _migrations WHEN NEW.version=34
         BEGIN SELECT RAISE(ABORT, 'injected owner migration failure'); END;",
    ).unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(34)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        33
    );
    assert_eq!(oracle.query_row("SELECT count(*) FROM pragma_table_info('bindings') WHERE name LIKE 'launch_owner_%'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    oracle
        .execute_batch("DROP TRIGGER reject_owner_migration;")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    let observed = oracle.query_row(
        "SELECT runtime_state,runtime_pid,runtime_start_identity,observed_provider_session_id,launch_owner_pid,launch_owner_start_identity FROM bindings",
        [], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,Option<i64>>(4)?,row.get::<_,Option<String>>(5)?)),
    ).unwrap();
    assert_eq!(
        observed,
        (
            "running".into(),
            61,
            "child-start".into(),
            "session".into(),
            None,
            None
        )
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT provider_session_id FROM identity_session_preferences",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "session"
    );
    for statement in [
        "UPDATE bindings SET launch_owner_pid=41",
        "UPDATE bindings SET launch_owner_start_identity='owner'",
        "UPDATE bindings SET launch_owner_pid=0,launch_owner_start_identity='owner'",
    ] {
        assert!(oracle.execute(statement, []).is_err());
    }
    storage.close().unwrap();
}

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
    assert_eq!(storage.health().unwrap().schema_version, 40);
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

#[test]
fn driver_state_upgrade_keeps_remembered_sessions_without_state() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema36.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..36].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute(
        "INSERT INTO identity_session_preferences VALUES ('old-id','claude','claude','default','session')",
        [],
    )
    .unwrap();
    old.close().unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 40);
    storage.close().unwrap();
    let oracle = Connection::open(&path).unwrap();
    let row: (String, Option<String>, Option<i64>, Option<i64>) = oracle
        .query_row(
            "SELECT provider_session_id, driver_state, driver_state_version, stale_at_ms
             FROM identity_session_preferences WHERE identity_id = 'old-id'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row, ("session".into(), None, None, None));
}
