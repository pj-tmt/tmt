use super::test_support::seed_history;
use super::*;

// Runner-focused tests use the actual historical migrations. Independent
// TypeScript-created fixtures and full data/schema oracles live in typescript/test/native.
fn historical_connection() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    seed_history(&mut connection);
    connection
}

#[test]
fn blocked_commit_rolls_back_replacement_and_restores_enforcement() {
    let directory = crate::test_support::TestDirectory::new();
    // This raw migration fixture bypasses Storage::open's private-root admission.
    super::super::secure_directory(&directory.path).unwrap();
    let path = directory.path.join("migration.db");
    let mut connection = Connection::open(&path).unwrap();
    seed_history(&mut connection);
    // Rollback journal makes a real reader prevent COMMIT without preventing
    // the immediate writer lock. This tests the commit path, not a SQL trigger.
    connection
        .busy_timeout(std::time::Duration::from_millis(25))
        .unwrap();
    let reader = Connection::open(&path).unwrap();
    reader
        .execute_batch("BEGIN; SELECT * FROM identities;")
        .unwrap();
    // History is already initialized; hold the reader across the actual ninth
    // migration rather than blocking the earlier history-initialization commit.
    let error = apply_identity_lifetime(&mut connection, &MIGRATIONS[8]).unwrap_err();
    assert_eq!(error.code, StorageErrorCode::Busy);
    assert_eq!(error.message, "Commit migration failed");
    assert!(error.retryable);
    reader.execute_batch("ROLLBACK").unwrap();
    assert_old_schema_and_data(&connection);
    assert_enforcement(&connection);
    apply(&mut connection).unwrap();
    assert_enforcement(&connection);
}

fn assert_enforcement(connection: &Connection) {
    assert!(connection.is_autocommit());
    let enabled: bool = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap();
    assert!(enabled);
    assert!(
        connection
            .execute(
                "INSERT INTO role_profiles VALUES ('absent', 'role', 'time')",
                []
            )
            .is_err()
    );
}

fn assert_old_schema_and_data(connection: &Connection) {
    let version: i64 = connection
        .query_row("SELECT MAX(version) FROM _migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 8);
    let columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('identities')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(columns, 5);
    let replacement: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'identities_with_lifetime')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!replacement);
    let identity: (String, String, String, String, String) = connection
        .query_row("SELECT * FROM identities", [], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap();
    assert_eq!(
        identity,
        (
            "old-id".into(),
            "Alice".into(),
            "alice".into(),
            "created".into(),
            "updated".into()
        )
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT content FROM role_profiles WHERE identity_id = 'old-id'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "original role"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT reserved_count FROM preamble_counters WHERE identity_id = 'old-id'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        12
    );
    assert_eq!(connection.query_row("SELECT acknowledged_through FROM request_attention_identities WHERE identity_id = 'old-id'", [], |row| row.get::<_, i64>(0)).unwrap(), 4);
}

#[test]
fn identity_rebuild_restores_enforcement_after_success_and_reopen() {
    let mut connection = historical_connection();
    apply(&mut connection).unwrap();
    assert_enforcement(&connection);
    check_foreign_keys(&connection).unwrap();
    let value: (String, Option<i64>) = connection
        .query_row(
            "SELECT lifetime, retired_at_ms FROM identities WHERE id = 'old-id'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(value, ("saved".into(), None));
    apply(&mut connection).unwrap();
    assert_enforcement(&connection);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM _migrations WHERE version = 9",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn recording_failure_rolls_back_replacement_and_restores_enforcement() {
    let mut connection = historical_connection();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_ninth BEFORE INSERT ON _migrations WHEN NEW.version = 9
         BEGIN SELECT RAISE(ABORT, 'fixture record failure'); END;",
        )
        .unwrap();
    let error = apply(&mut connection).unwrap_err();
    assert_eq!(error.code, StorageErrorCode::Migration);
    assert_eq!(error.migration_version, Some(9));
    assert_old_schema_and_data(&connection);
    assert_enforcement(&connection);
    connection
        .execute_batch("DROP TRIGGER reject_ninth")
        .unwrap();
    apply(&mut connection).unwrap();
    assert_enforcement(&connection);
}

#[test]
fn post_rebuild_foreign_key_violation_rolls_back_the_entire_migration() {
    let mut connection = historical_connection();
    connection
        .execute_batch(
            "CREATE TRIGGER damage_ninth AFTER INSERT ON _migrations WHEN NEW.version = 9
         BEGIN UPDATE role_profiles SET identity_id = 'missing'; END;",
        )
        .unwrap();
    let error = apply(&mut connection).unwrap_err();
    assert_eq!(error.migration_version, Some(9));
    assert_old_schema_and_data(&connection);
    assert_enforcement(&connection);
    check_foreign_keys(&connection).unwrap();
}

#[test]
fn preexisting_foreign_key_damage_is_rejected_not_repaired() {
    let mut connection = historical_connection();
    connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    connection
        .execute(
            "INSERT INTO role_profiles VALUES ('missing', 'keep', 'time')",
            [],
        )
        .unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let error = apply(&mut connection).unwrap_err();
    assert_eq!(error.migration_version, Some(9));
    assert_old_schema_and_data(&connection);
    assert_enforcement(&connection);
    assert_eq!(
        connection
            .query_row(
                "SELECT content FROM role_profiles WHERE identity_id = 'missing'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "keep"
    );
}

#[test]
fn custom_identity_objects_are_not_silently_removed() {
    for statement in [
        "CREATE INDEX custom_identity ON identities(name)",
        "CREATE TRIGGER custom_identity AFTER INSERT ON identities BEGIN SELECT 1; END",
    ] {
        let mut connection = historical_connection();
        connection.execute_batch(statement).unwrap();
        let error = apply(&mut connection).unwrap_err();
        assert_eq!(error.migration_version, Some(9));
        assert_old_schema_and_data(&connection);
        assert_enforcement(&connection);
        let sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name = 'custom_identity'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sql, statement);
    }
}

#[test]
fn lifetime_and_retirement_constraints_preserve_name_and_uuid_boundaries() {
    let mut connection = historical_connection();
    apply(&mut connection).unwrap();
    for lifetime in ["temporary", "saved"] {
        assert!(connection.execute(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES ('new-id', 'ALICE', 'alice', 'new', 'new', ?)",
            [lifetime],
        ).is_err());
    }
    for timestamp in ["0", "-1", "1.5", "9007199254740992", "'invalid'"] {
        assert!(
            connection
                .execute(
                    &format!("UPDATE identities SET retired_at_ms = {timestamp}"),
                    []
                )
                .is_err()
        );
    }
    assert!(
        connection
            .execute("UPDATE identities SET lifetime = 'permanent'", [])
            .is_err()
    );
    connection
        .execute(
            "UPDATE identities SET retired_at_ms = 123 WHERE id = 'old-id'",
            [],
        )
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime)
         VALUES ('new-id', 'ALICE', 'alice', 'new', 'new', 'temporary');",
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "UPDATE identities SET retired_at_ms = NULL WHERE id = 'old-id'",
                []
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "UPDATE identities SET id = 'old-id' WHERE id = 'new-id'",
                []
            )
            .is_err()
    );
    for table in [
        "role_profiles",
        "identity_preambles",
        "preamble_counters",
        "request_attention_identities",
    ] {
        let count: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE identity_id = 'new-id'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "name reuse must not inherit {table}");
    }
    check_foreign_keys(&connection).unwrap();
    assert_enforcement(&connection);
}

#[test]
fn custom_identity_columns_and_their_contents_are_not_discarded() {
    let mut connection = historical_connection();
    connection
        .execute_batch(
            "ALTER TABLE identities ADD COLUMN private_note TEXT;
         UPDATE identities SET private_note = 'user-owned content';",
        )
        .unwrap();
    let error = apply(&mut connection).unwrap_err();
    assert_eq!(error.migration_version, Some(9));
    assert_enforcement(&connection);
    assert_eq!(
        connection
            .query_row("SELECT private_note FROM identities", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "user-owned content"
    );
    assert_eq!(
        connection
            .query_row("SELECT MAX(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        8
    );
}
