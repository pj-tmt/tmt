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
    assert_eq!(storage.health().unwrap().schema_version, 63);
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
    assert_eq!(storage.health().unwrap().schema_version, 63);
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

#[test]
fn channel_upgrade_preserves_legacy_state_and_validates_preference() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema44.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..44].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch("INSERT INTO identity_session_preferences (identity_id,preferred_harness,remembered_harness,runtime_mode,provider_session_id) VALUES ('old-id','claude','claude','default','session');
        CREATE TRIGGER reject_channel_migration BEFORE INSERT ON _migrations WHEN NEW.version=45 BEGIN SELECT RAISE(ABORT, 'injected channel migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(45)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(oracle.query_row("SELECT count(*) FROM pragma_table_info('identity_session_preferences') WHERE name='channel'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    oracle
        .execute_batch("DROP TRIGGER reject_channel_migration")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row(
                "SELECT provider_session_id,channel FROM identity_session_preferences",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<bool>>(1)?))
            )
            .unwrap(),
        ("session".into(), None)
    );
    for value in ["2", "-1", "'channel'", "0.5"] {
        assert!(
            oracle
                .execute(
                    &format!("UPDATE identity_session_preferences SET channel={value}"),
                    []
                )
                .is_err()
        );
    }
    let cursor = || {
        oracle
            .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    let before = cursor();
    oracle
        .execute("UPDATE identity_session_preferences SET channel=1", [])
        .unwrap();
    assert_eq!(cursor(), before + 1);
    oracle
        .execute("UPDATE identity_session_preferences SET channel=1", [])
        .unwrap();
    assert_eq!(cursor(), before + 1);
    storage.close().unwrap();
}

#[test]
fn consumption_attribution_migration_preserves_legacy_bucket_without_backfill() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema49.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 49).unwrap();
    old.execute_batch("INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES('meter','Meter','meter','t','t','saved');
      INSERT INTO consumption_buckets(identity_id,from_ms,input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous) VALUES('meter',10000,10,3,5,5000,1,0,0);
      CREATE TRIGGER refuse_attribution BEFORE INSERT ON _migrations WHEN NEW.version=50 BEGIN SELECT RAISE(ABORT,'injected migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(50)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(oracle.query_row("SELECT COUNT(*) FROM pragma_table_info('consumption_buckets') WHERE name='details'",[],|row| row.get::<_,i64>(0)).unwrap(),0);
    oracle
        .execute_batch("DROP TRIGGER refuse_attribution;")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(oracle.query_row("SELECT input_tokens,output_tokens,cached_input_tokens,details FROM consumption_buckets",[],|row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,Option<String>>(3)?))).unwrap(),(10,3,5,None));
    let response = storage
        .consumption_history(&["meter".into()], &[5000], 120, 15000, false, None)
        .unwrap();
    let bucket = &response["identities"][0]["windows"][0]["buckets"][0];
    assert_eq!(bucket["inputTokens"], 10);
    assert_eq!(bucket["cachedInputTokens"], 5);
    assert_eq!(bucket["complete"], true);
    assert!(bucket.get("cacheWriteTokens").is_none());
    assert!(bucket.get("byModel").is_none());
    storage.close().unwrap();
}

#[test]
fn inactive_driver_upgrade_is_atomic_and_keeps_the_current_coordinates() {
    let root = TestDirectory::new();
    let path = root.path.join("driver-upgrade.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_through(&mut old, 61).unwrap();
    old.execute_batch("INSERT INTO identity_session_preferences
        (identity_id,preferred_harness,remembered_harness,runtime_mode,provider_session_id,driver_state_version,driver_state,channel)
        VALUES ('old-id','claude','claude','default','kept',1,'{\"model\":\"sonnet\"}',1);
        CREATE TRIGGER reject_driver_records BEFORE INSERT ON _migrations WHEN NEW.version=62
        BEGIN SELECT RAISE(ABORT,'injected driver record migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(62)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='identity_resume_records'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(oracle.query_row("SELECT provider_session_id FROM identity_session_preferences WHERE identity_id='old-id'", [], |row| row.get::<_, String>(0)).unwrap(), "kept");
    oracle
        .execute_batch("DROP TRIGGER reject_driver_records;")
        .unwrap();
    oracle.close().unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 63);
    let preferences = storage.session_preferences("old-id").unwrap();
    assert_eq!(preferences.channel, Some(true));
    assert!(preferences.retained.is_empty());
    assert_eq!(
        preferences
            .remembered
            .as_ref()
            .unwrap()
            .provider_session
            .as_str(),
        "kept"
    );
    assert_eq!(
        preferences
            .remembered
            .as_ref()
            .unwrap()
            .state
            .as_ref()
            .unwrap()
            .document(),
        "{\"model\":\"sonnet\"}"
    );
    storage.close().unwrap();
}

#[test]
fn refused_start_upgrade_rolls_back_its_column_and_cursor_trigger_together() {
    let root = TestDirectory::new();
    let path = root.path.join("refused-start-upgrade.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_through(&mut old, 62).unwrap();
    let trigger: String = old
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='bindings_advances_change_cursor_on_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    old.execute_batch("CREATE TRIGGER reject_refusal BEFORE INSERT ON _migrations WHEN NEW.version=63 BEGIN SELECT RAISE(ABORT,'injected observation migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(63)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(oracle.query_row("SELECT count(*) FROM pragma_table_info('bindings') WHERE name='runtime_start_refusal'", [], |row| row.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(oracle.query_row("SELECT sql FROM sqlite_schema WHERE name='bindings_advances_change_cursor_on_update'", [], |row| row.get::<_,String>(0)).unwrap(), trigger);
    oracle
        .execute_batch("DROP TRIGGER reject_refusal;")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 63);
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM bindings WHERE runtime_start_refusal IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    storage.close().unwrap();
    oracle.close().unwrap();
}
