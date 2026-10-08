use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::test_support::TestDirectory;

use super::*;

struct Fixture {
    directory: TestDirectory,
    database: PathBuf,
}

#[test]
fn compiled_schema_export_matches_actual_source_bytes_and_the_storage_owner() {
    let compiled = Storage::compiled_schema();
    let working_directory = std::env::current_dir().unwrap();
    let root = schema_source_root(&working_directory).expect("runtime checkout source tree");
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.database).unwrap();
    assert_eq!(compiled.version, storage.health().unwrap().schema_version);
    assert!(compiled.sources.len() <= 64);
    for source in &compiled.sources {
        assert_eq!(
            source.sha256,
            tmt_core::content_digest::sha256(&fs::read(root.join(source.path)).unwrap()),
            "{}",
            source.path
        );
    }
    let record = crate::native_install::compiled_application_schema(&"a".repeat(40)).unwrap();
    assert_eq!(record["databases"][0]["domain"], "tmt-core-db");
    assert_eq!(record["databases"][0]["version"], compiled.version);
    assert_eq!(
        record["source_files"].as_array().unwrap().len(),
        compiled.sources.len()
    );
    assert!(crate::native_install::compiled_application_schema("main").is_err());
    storage.close().unwrap();
}

// Docker builds under /native and runs the copied test binary under /workspace.
// The byte oracle must read the runtime checkout, not a compiled absolute path.
fn schema_source_root(working_directory: &Path) -> Option<&Path> {
    working_directory
        .ancestors()
        .find(|root| root.join("rust/Cargo.toml").is_file())
}

#[test]
fn compiled_schema_byte_oracle_uses_relocated_runtime_sources_and_detects_changes() {
    let working_directory = std::env::current_dir().unwrap();
    let original = schema_source_root(&working_directory).unwrap();
    let directory = TestDirectory::new();
    let runtime_root = directory.path.join("workspace");
    let nested = runtime_root.join("rust/crates/tmt-adapters");
    fs::create_dir_all(&nested).unwrap();
    fs::write(runtime_root.join("rust/Cargo.toml"), b"fixture marker").unwrap();
    assert!(schema_source_root(&directory.path.join("native/rust")).is_none());
    assert_eq!(schema_source_root(&nested), Some(runtime_root.as_path()));
    let sources = Storage::compiled_schema().sources;
    for source in &sources {
        let path = runtime_root.join(source.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(original.join(source.path), &path).unwrap();
        assert_eq!(
            source.sha256,
            tmt_core::content_digest::sha256(&fs::read(path).unwrap()),
            "{}",
            source.path
        );
    }
    let changed = &sources[0];
    fs::write(
        runtime_root.join(changed.path),
        b"changed after compilation",
    )
    .unwrap();
    assert_ne!(
        changed.sha256,
        tmt_core::content_digest::sha256(&fs::read(runtime_root.join(changed.path)).unwrap())
    );
}

#[test]
fn application_schema_observation_does_not_create_or_guess_missing_history() {
    let fixture = Fixture::new();
    assert!(Storage::application_schema(&fixture.database).is_err());
    assert!(!fixture.database.parent().unwrap().exists());
    fs::create_dir_all(fixture.database.parent().unwrap()).unwrap();
    let connection = Connection::open(&fixture.database).unwrap();
    connection.execute_batch("PRAGMA user_version = 99; CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES ('retained');").unwrap();
    assert!(Storage::application_schema(&fixture.database).is_err());
    assert_eq!(
        connection
            .query_row("SELECT value FROM sentinel", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "retained"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name = '_migrations'",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn application_schema_observation_reads_wal_and_future_versions_without_migration() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.database).unwrap();
    let connection = storage.connection().unwrap();
    // This intentionally contradicts the application record: user_version is
    // not Core's schema owner, and the newest committed history is still in WAL.
    connection.execute_batch("PRAGMA wal_autocheckpoint = 0; PRAGMA user_version = 1; INSERT INTO _migrations VALUES (50, 'future migration', 'now');").unwrap();
    assert_eq!(Storage::application_schema(&fixture.database).unwrap(), 50);
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_eq!(storage.health().unwrap().schema_version, 50);
    storage.close().unwrap();
    assert_eq!(Storage::application_schema(&fixture.database).unwrap(), 50);
}

#[test]
fn application_schema_observation_refuses_gaps_and_changed_known_migrations() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.database).unwrap();
    assert_eq!(Storage::application_schema(&fixture.database).unwrap(), 49);
    let connection = storage.connection().unwrap();
    connection
        .execute_batch("INSERT INTO _migrations VALUES (51, 'gap', 'now');")
        .unwrap();
    assert!(Storage::application_schema(&fixture.database).is_err());
    connection.execute_batch("DELETE FROM _migrations WHERE version = 51; UPDATE _migrations SET name = 'unverified' WHERE version = 1;").unwrap();
    assert!(Storage::application_schema(&fixture.database).is_err());
    storage.close().unwrap();
}

impl Fixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        Self {
            database: directory.path.join("state").join("tmux-team.db"),
            directory,
        }
    }
}

#[test]
fn open_enforces_connection_features_and_private_files() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.database).unwrap();
    assert_eq!(
        storage.health().unwrap(),
        StorageHealth {
            path: fixture.database.clone(),
            schema_version: 49,
            journal_mode: "wal",
            foreign_keys: true,
            busy_timeout_ms: 5000,
            synchronous: "normal",
            fts5: true,
        }
    );
    let connection = storage.connection().unwrap();
    connection.execute_batch("CREATE VIRTUAL TABLE temp.search_probe USING fts5(content); INSERT INTO temp.search_probe VALUES ('native durable reply');").unwrap();
    let result: String = connection
        .query_row(
            "SELECT content FROM temp.search_probe WHERE search_probe MATCH 'durable'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(result, "native durable reply");
    assert!(
        connection
            .execute(
                "INSERT INTO role_profiles VALUES ('missing', 'role', 'now')",
                []
            )
            .is_err()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(fixture.database.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for suffix in ["", "-wal", "-shm"] {
            let file = PathBuf::from(format!("{}{suffix}", fixture.database.display()));
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    storage.close().unwrap();
    storage.close().unwrap();
    assert_eq!(storage.health().unwrap_err().code, StorageErrorCode::Closed);
    assert_eq!(
        storage
            .checkpoint(CheckpointMode::Passive)
            .unwrap_err()
            .code,
        StorageErrorCode::Closed
    );
}

#[test]
fn failed_checkpoint_still_closes_and_rolls_back_the_active_transaction() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.database).unwrap();
    storage.connection().unwrap().execute_batch("BEGIN IMMEDIATE; INSERT INTO identities (id, name, canonical_name, created_at, updated_at) VALUES ('uncommitted', 'Test', 'test', 'now', 'now');").unwrap();
    assert_eq!(storage.close().unwrap_err().code, StorageErrorCode::Busy);
    assert_eq!(storage.health().unwrap_err().code, StorageErrorCode::Closed);
    storage.close().unwrap();
    let verification = Connection::open(&fixture.database).unwrap();
    let count: i64 = verification
        .query_row("SELECT COUNT(*) FROM identities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "close must release the failed checkpoint's writer and roll back"
    );
    verification
        .execute_batch("BEGIN IMMEDIATE; COMMIT;")
        .unwrap();
}

#[test]
fn health_rejects_changed_connection_policy() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.database).unwrap();
    storage
        .connection()
        .unwrap()
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    assert_eq!(
        storage.health().unwrap_err().code,
        StorageErrorCode::IncompatibleSchema
    );
    storage.close().unwrap();
}

#[test]
fn open_errors_preserve_existing_files() {
    let fixture = Fixture::new();
    fs::write(fixture.database.parent().unwrap(), b"unrelated file").unwrap();
    let error = Storage::open(&fixture.database)
        .err()
        .expect("directory collision must fail");
    assert_eq!(error.code, StorageErrorCode::Permission);
    assert_eq!(
        fs::read(fixture.database.parent().unwrap()).unwrap(),
        b"unrelated file"
    );

    let corrupt = fixture.directory.path.join("corrupt.db");
    fs::write(&corrupt, b"not a SQLite database").unwrap();
    let error = Storage::open(&corrupt)
        .err()
        .expect("corrupt data must fail");
    assert_eq!(error.code, StorageErrorCode::Corrupt);
    assert_eq!(fs::read(corrupt).unwrap(), b"not a SQLite database");
}

#[test]
fn missing_open_path_does_not_claim_a_permission_failure() {
    let fixture = Fixture::new();
    let missing = fixture.directory.path.join("absent").join("missing.db");
    let error = Connection::open_with_flags(&missing, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .unwrap_err();
    assert_eq!(
        classify_open(error, "Open storage", &missing).code,
        StorageErrorCode::Permission
    );
}

#[test]
fn concurrent_openers_commit_each_migration_only_once() {
    let fixture = Fixture::new();
    // Establish WAL independently without applying any migration. This tests
    // migration convergence, not SQLite's separate journal-mode transition.
    fs::create_dir(fixture.database.parent().unwrap()).unwrap();
    let initial = Connection::open(&fixture.database).unwrap();
    initial.pragma_update(None, "journal_mode", "WAL").unwrap();
    initial.close().unwrap();
    for result in concurrent_opens(&fixture) {
        assert_eq!(result.unwrap(), 49);
    }
    assert_complete_history(&fixture);
}

#[test]
fn cold_open_race_initializes_wal_for_every_caller() {
    for _ in 0..4 {
        let fixture = Fixture::new();
        for result in concurrent_opens(&fixture) {
            assert_eq!(result.unwrap(), 49);
        }
        assert_complete_history(&fixture);
    }
}

fn concurrent_opens(fixture: &Fixture) -> Vec<Result<u32, StorageError>> {
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    let mut storage = Storage::open(&fixture.database)?;
                    let health = storage.health()?;
                    assert_eq!(health.journal_mode, "wal");
                    assert_eq!(health.busy_timeout_ms, 5000);
                    storage.close()?;
                    Ok::<_, StorageError>(health.schema_version)
                })
            })
            .collect::<Vec<_>>();
        // Scoped threads are joined before fixture cleanup even after a panic.
        let results = handles
            .into_iter()
            .map(|handle| handle.join())
            .collect::<Vec<_>>();
        results.into_iter().map(Result::unwrap).collect()
    })
}

fn assert_complete_history(fixture: &Fixture) {
    let verification = Connection::open(&fixture.database).unwrap();
    let count: i64 = verification
        .query_row("SELECT COUNT(*) FROM _migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 49);
    let check: String = verification
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(check, "ok");
}

#[test]
fn wal_retry_budget_preserves_busy_and_releases_failed_open() {
    use std::error::Error;
    let fixture = Fixture::new();
    fs::create_dir(fixture.database.parent().unwrap()).unwrap();
    let blocker = Connection::open(&fixture.database).unwrap();
    blocker.execute_batch("CREATE TABLE retained(value TEXT); INSERT INTO retained VALUES ('original'); BEGIN EXCLUSIVE;").unwrap();
    let started = Instant::now();
    let error = Storage::open(&fixture.database)
        .err()
        .expect("exclusive lock must block WAL setup");
    let elapsed = started.elapsed();
    eprintln!("WAL contention ended after {elapsed:?}");
    assert_eq!(error.code, StorageErrorCode::Busy);
    assert!(error.retryable);
    assert_eq!(error.message, "Configure WAL failed");
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<rusqlite::Error>()
            .unwrap()
            .sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    assert!(
        elapsed >= BUSY_TIMEOUT.saturating_sub(Duration::from_millis(100)),
        "retry ended too early: {elapsed:?}"
    );
    assert!(
        elapsed < BUSY_TIMEOUT + Duration::from_secs(1),
        "retry exceeded budget: {elapsed:?}"
    );
    blocker.execute_batch("ROLLBACK").unwrap();
    blocker.close().unwrap();
    let mut recovered = Storage::open(&fixture.database).unwrap();
    assert_eq!(recovered.health().unwrap().journal_mode, "wal");
    assert_eq!(recovered.health().unwrap().busy_timeout_ms, 5000);
    assert_eq!(
        recovered
            .connection()
            .unwrap()
            .query_row("SELECT value FROM retained", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "original"
    );
    recovered.close().unwrap();
    assert_complete_history(&fixture);
}

#[test]
fn wal_setup_rejects_non_wal_and_non_busy_results_without_retrying() {
    let memory = Connection::open_in_memory().unwrap();
    let started = Instant::now();
    assert_eq!(
        configure_wal(&memory).unwrap_err().code,
        StorageErrorCode::IncompatibleSchema
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    let fixture = Fixture::new();
    fs::create_dir(fixture.database.parent().unwrap()).unwrap();
    fs::write(&fixture.database, b"not a SQLite database").unwrap();
    let corrupt = Connection::open(&fixture.database).unwrap();
    let started = Instant::now();
    assert_eq!(
        configure_wal(&corrupt).unwrap_err().code,
        StorageErrorCode::Corrupt
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    corrupt.close().unwrap();
    assert_eq!(
        fs::read(&fixture.database).unwrap(),
        b"not a SQLite database"
    );
}
