use super::*;
use crate::test_support::TestDirectory;
use rusqlite::Connection;
use serde_json::Value;
const ID: &str = "11111111-1111-4111-8111-111111111111";
fn paths(directory: &TestDirectory) -> ConfigPaths {
    ConfigPaths::resolve(
        &directory.path,
        &directory.path,
        Some(&directory.path),
        None,
    )
}
fn call(session: &mut Session, operation: &str, input: Value) -> Result<Value, Fault> {
    let body = json!({"version":1,"operation":operation,"input":input});
    let bytes = session.execute(decode(&body.to_string())?)?;
    Ok(serde_json::from_slice(&bytes).unwrap())
}
fn seed(paths: &ConfigPaths) {
    Storage::open(&paths.database).unwrap().close().unwrap();
    Connection::open(&paths.database).unwrap().execute("INSERT INTO identities (id,name,canonical_name,created_at,updated_at,lifetime) VALUES (?,'Ada','ada','t','t','saved')", [ID]).unwrap();
}
#[test]
fn discovery_in_a_session_does_not_open_or_create_storage() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let mut session = Session::new(paths.clone());
    for _ in 0..2 {
        assert_eq!(
            call(&mut session, "capabilities", json!({})).unwrap()["session"]["framing"],
            "ndjson"
        );
        assert_eq!(
            call(&mut session, "storage.root", json!({})).unwrap()["dataRoot"],
            paths.global_dir.to_str().unwrap()
        );
    }
    session.close().unwrap();
    assert!(!paths.database.exists());
    assert_eq!(std::fs::read_dir(&directory.path).unwrap().count(), 0);
}
#[test]
fn one_connection_reuses_initialization_but_reads_current_identity_and_settings() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    seed(&paths);
    let notes = paths.global_dir.join("notes").join(ID);
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("notes.md"), "private fixture").unwrap();
    let mut session = Session::new(paths.clone());
    assert_eq!(
        call(&mut session, "notes.read", json!({"identityId":ID})).unwrap()["name"],
        "Ada"
    );
    let oracle = Connection::open(&paths.database).unwrap();
    let name: String = oracle
        .query_row("SELECT name FROM _migrations WHERE version=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    // A sentinel in this private fixture proves the full migration validation is not repeated.
    oracle
        .execute(
            "UPDATE _migrations SET name='fixture-invalid-history' WHERE version=1",
            [],
        )
        .unwrap();
    oracle
        .execute(
            "UPDATE identities SET name='Grace',canonical_name='grace' WHERE id=?",
            [ID],
        )
        .unwrap();
    assert_eq!(
        call(&mut session, "notes.read", json!({"identityId":ID})).unwrap()["name"],
        "Grace"
    );
    assert!(
        execute(&paths, Request::ChangeCursor).is_err(),
        "a fresh open must observe the sentinel"
    );
    oracle
        .execute("UPDATE _migrations SET name=? WHERE version=1", [name])
        .unwrap();
    std::fs::write(&paths.global_config, "not JSON").unwrap();
    assert_eq!(
        call(&mut session, "user.identity", json!({}))
            .unwrap_err()
            .code(),
        "CONFIG_ERROR"
    );
    std::fs::remove_file(&paths.global_config).unwrap();
    assert!(call(&mut session, "changes.cursor", json!({})).is_ok());
    oracle
        .execute("UPDATE identities SET retired_at_ms=1 WHERE id=?", [ID])
        .unwrap();
    assert_eq!(
        call(&mut session, "notes.read", json!({"identityId":ID}))
            .unwrap_err()
            .code(),
        "NOTEBOOK_IDENTITY_NOT_FOUND"
    );
    session.close().unwrap();
}
#[test]
fn a_failed_mutation_releases_its_transaction_before_the_next_request() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    seed(&paths);
    let mut session = Session::new(paths.clone());
    call(&mut session, "changes.cursor", json!({})).unwrap();
    let oracle = Connection::open(&paths.database).unwrap();
    oracle.execute_batch("CREATE TRIGGER reject_fixture_name BEFORE UPDATE OF name ON identities BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    assert!(
        call(
            &mut session,
            "identity.rename",
            json!({"identityId":ID,"expectedName":"Ada","name":"Lost"})
        )
        .is_err()
    );
    let name: String = oracle
        .query_row("SELECT name FROM identities WHERE id=?", [ID], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "Ada");
    // A different connection can write immediately after the failed request.
    oracle
        .execute_batch("DROP TRIGGER reject_fixture_name")
        .unwrap();
    oracle
        .execute(
            "UPDATE identities SET name='Grace',canonical_name='grace' WHERE id=?",
            [ID],
        )
        .unwrap();
    let found = call(
        &mut session,
        "references.resolve",
        json!({"identityIds":[ID]}),
    )
    .unwrap();
    assert_eq!(found["identities"][0]["name"], "Grace");
    session.close().unwrap();
}

#[test]
fn an_out_of_band_migration_refuses_the_next_request_before_any_effect() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    seed(&paths);
    let mut session = Session::new(paths.clone());
    call(&mut session, "changes.cursor", json!({})).unwrap();
    let oracle = Connection::open(&paths.database).unwrap();
    oracle
        .execute_batch("BEGIN IMMEDIATE; CREATE TABLE future_fixture(value TEXT);")
        .unwrap();
    oracle
        .execute(
            "INSERT INTO _migrations(version,name,applied_at) VALUES (?,'future fixture','t')",
            [Storage::compiled_schema_version() + 1],
        )
        .unwrap();
    oracle.execute_batch("COMMIT").unwrap();
    assert_eq!(
        call(
            &mut session,
            "identity.rename",
            json!({"identityId":ID,"expectedName":"Ada","name":"Lost"})
        )
        .unwrap_err()
        .code(),
        SESSION_STALE
    );
    assert_eq!(
        call(&mut session, "capabilities", json!({}))
            .unwrap_err()
            .code(),
        SESSION_STALE
    );
    let name: String = oracle
        .query_row("SELECT name FROM identities WHERE id=?", [ID], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "Ada");
    session.close().unwrap();
}
