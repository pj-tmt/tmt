use super::*;
use crate::test_support::TestDirectory;
use serde_json::{Value, json};
use std::{ffi::OsStr, fs};

const CAPTURED: &str = include_str!("../../../runtime/fixtures/codex-caller-session.json");

struct Fixture {
    directory: TestDirectory,
    environment: ProviderEnvironment,
    coordinates: CallerSession,
    database: PathBuf,
    rollout: PathBuf,
    header: Value,
}

impl Fixture {
    fn new() -> Self {
        let captured: Value = serde_json::from_str(CAPTURED).unwrap();
        let directory = TestDirectory::new();
        let home = directory.path.join(".codex");
        let rollout = home.join("sessions/2026/10/09/root.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        let header = captured["header"].clone();
        fs::write(&rollout, format!("{header}\nnot a header; never read\n")).unwrap();
        let database = home.join("state_5.sqlite");
        let connection = Connection::open(&database).unwrap();
        let definitions = captured["index"]["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|column| {
                format!(
                    "{} {} {} {} DEFAULT {}",
                    column["name"].as_str().unwrap(),
                    column["type"].as_str().unwrap(),
                    if column["notNull"] == 1 {
                        "NOT NULL"
                    } else {
                        ""
                    },
                    if column["primaryKey"] == 1 {
                        "PRIMARY KEY"
                    } else {
                        ""
                    },
                    if column["type"] == "TEXT" { "''" } else { "0" }
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        connection
            .execute_batch(&format!("CREATE TABLE threads ({definitions});"))
            .unwrap();
        let coordinates = coordinates(Some(OsStr::new(
            captured["environment"]["CODEX_THREAD_ID"].as_str().unwrap(),
        )))
        .unwrap();
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, source) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    coordinates.session.as_str(),
                    rollout.to_str().unwrap(),
                    captured["index"]["row"]["source"].as_str().unwrap()
                ],
            )
            .unwrap();
        connection.close().unwrap();
        let environment = ProviderEnvironment::from_parts(
            &directory.path,
            &directory.path,
            vec![],
            [("CODEX_HOME", home)],
        );
        Self {
            directory,
            environment,
            coordinates,
            database,
            rollout,
            header,
        }
    }

    fn admit(&self) -> Result<(), CallerSessionRefusal> {
        root(
            &self.coordinates,
            &self.environment,
            Instant::now() + Duration::from_millis(500),
        )
    }
    fn header(&self, header: &Value) {
        fs::write(&self.rollout, format!("{header}\n")).unwrap();
    }
}

#[test]
fn captured_current_version_root_is_read_only_and_exact_id() {
    let fixture = Fixture::new();
    let before = fs::read(&fixture.database).unwrap();
    assert_eq!(fixture.admit(), Ok(()));
    assert_eq!(fs::read(&fixture.database).unwrap(), before);
    assert!(!fixture.database.with_extension("sqlite-wal").exists());
    assert!(!fixture.database.with_extension("sqlite-shm").exists());
    assert!(indexed_path(&fixture.database, "unrelated").is_err());
    for value in [
        None,
        Some(OsStr::new("not-a-thread")),
        Some(OsStr::new("01234567-89AB-7CDE-8FAB-0123456789AB")),
    ] {
        assert!(coordinates(value).is_none());
    }
}

#[test]
fn changed_index_columns_table_and_missing_database_refuse_without_repair() {
    for change in [
        "ALTER TABLE threads RENAME COLUMN source TO origin",
        "ALTER TABLE threads ADD COLUMN unknown TEXT",
        "DROP TABLE threads",
    ] {
        let fixture = Fixture::new();
        Connection::open(&fixture.database)
            .unwrap()
            .execute_batch(change)
            .unwrap();
        let before = fs::read(&fixture.database).unwrap();
        assert_eq!(fixture.admit(), Err(CallerSessionRefusal::IndexShape));
        assert_eq!(fs::read(&fixture.database).unwrap(), before);
    }
    let fixture = Fixture::new();
    fs::remove_file(&fixture.database).unwrap();
    assert_eq!(fixture.admit(), Err(CallerSessionRefusal::IndexUnavailable));
    assert!(!fixture.database.exists());
}

#[test]
fn held_index_refuses_immediately_and_missing_wal_shm_is_not_created() {
    let fixture = Fixture::new();
    let connection = Connection::open(&fixture.database).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    assert_eq!(fixture.admit(), Err(CallerSessionRefusal::IndexUnavailable));
    connection.execute_batch("ROLLBACK").unwrap();
    let wal = fixture.database.with_extension("sqlite-wal");
    fs::write(&wal, "held WAL without shared memory").unwrap();
    assert_eq!(fixture.admit(), Err(CallerSessionRefusal::IndexUnavailable));
    assert!(!fixture.database.with_extension("sqlite-shm").exists());
    assert_eq!(
        fs::read_to_string(wal).unwrap(),
        "held WAL without shared memory"
    );
}

#[test]
fn active_wal_snapshot_does_not_change_database_or_sidecar_bytes() {
    let fixture = Fixture::new();
    let writer = Connection::open(&fixture.database).unwrap();
    writer
        .execute_batch("PRAGMA journal_mode=WAL; UPDATE threads SET title='fixture';")
        .unwrap();
    let files = [
        &fixture.database,
        &fixture.database.with_extension("sqlite-wal"),
        &fixture.database.with_extension("sqlite-shm"),
    ];
    let before: Vec<_> = files
        .iter()
        .map(fs::read)
        .collect::<Result<_, _>>()
        .unwrap();
    // Production observes in its own supervised worker, never in the provider
    // writer's SQLite library. An in-process reader would share the writer's
    // existing writable mmap despite readonly_shm=1 on the new connection.
    let executable = std::env::current_exe().unwrap();
    let source: Value = serde_json::from_str(CAPTURED).unwrap();
    let session = source["environment"]["CODEX_THREAD_ID"].as_str().unwrap();
    let arguments = [
        std::ffi::OsString::from(format!(
            "TMT_TEST_CALLER_INDEX={}",
            fixture.database.to_str().unwrap()
        )),
        format!("TMT_TEST_CALLER_SESSION={session}").into(),
        executable.into_os_string(),
        "--exact".into(),
        "drivers::codex::caller_session::tests::readonly_index_child".into(),
        "--nocapture".into(),
    ];
    use crate::process::{CommandRequest, CommandRunner, UnixCommandRunner};
    UnixCommandRunner
        .execute(CommandRequest {
            program: OsStr::new("/usr/bin/env"),
            args: &arguments,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(5),
            max_output_bytes: 4096,
        })
        .unwrap();
    let after: Vec<_> = files
        .iter()
        .map(fs::read)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(before, after);
    drop(writer);
}

#[test]
fn readonly_index_child() {
    let Some(database) = std::env::var_os("TMT_TEST_CALLER_INDEX") else {
        return;
    };
    let session = std::env::var("TMT_TEST_CALLER_SESSION").unwrap();
    assert!(indexed_path(Path::new(&database), &session).is_ok());
}

#[test]
fn unknown_headers_and_same_process_subagents_degrade_to_not_recorded() {
    let fixture = Fixture::new();
    for (path, value, expected) in [
        (
            "kind",
            json!("changed_record"),
            CallerSessionRefusal::HeaderShape,
        ),
        (
            "id",
            json!("different-thread"),
            CallerSessionRefusal::NotRoot,
        ),
        (
            "session_id",
            json!("parent-thread"),
            CallerSessionRefusal::NotRoot,
        ),
        (
            "source",
            json!({"subagent":{"parent_thread_id":"parent"}}),
            CallerSessionRefusal::NotRoot,
        ),
        ("source", json!("unknown"), CallerSessionRefusal::NotRoot),
        (
            "parent_thread_id",
            json!("parent"),
            CallerSessionRefusal::NotRoot,
        ),
    ] {
        let mut header = fixture.header.clone();
        if path == "kind" {
            header["type"] = value;
        } else {
            header["payload"][path] = value;
        }
        fixture.header(&header);
        assert_eq!(fixture.admit(), Err(expected), "{path}");
    }
    for key in ["id", "session_id", "source"] {
        let mut header = fixture.header.clone();
        header["payload"].as_object_mut().unwrap().remove(key);
        fixture.header(&header);
        assert_eq!(
            fixture.admit(),
            Err(CallerSessionRefusal::HeaderShape),
            "missing {key}"
        );
    }
    let connection = Connection::open(&fixture.database).unwrap();
    for source in ["subagent", "mcp", "unknown"] {
        connection
            .execute("UPDATE threads SET source = ?", [source])
            .unwrap();
        assert_eq!(fixture.admit(), Err(CallerSessionRefusal::NotRoot));
    }
}

#[test]
fn missing_partial_oversized_outside_or_linked_headers_refuse() {
    let fixture = Fixture::new();
    for bytes in [b"incomplete".to_vec(), vec![b'x'; 64 * 1024 + 1]] {
        fs::write(&fixture.rollout, bytes).unwrap();
        assert_eq!(
            fixture.admit(),
            Err(CallerSessionRefusal::HeaderUnavailable)
        );
    }
    fs::remove_file(&fixture.rollout).unwrap();
    assert_eq!(
        fixture.admit(),
        Err(CallerSessionRefusal::HeaderUnavailable)
    );
    let outside = fixture.directory.path.join("outside.jsonl");
    fs::write(&outside, format!("{}\n", fixture.header)).unwrap();
    std::os::unix::fs::symlink(&outside, &fixture.rollout).unwrap();
    assert_eq!(
        fixture.admit(),
        Err(CallerSessionRefusal::HeaderUnavailable)
    );
}

#[test]
fn configured_sqlite_home_precedes_environment_and_unsupported_shapes_refuse() {
    let fixture = Fixture::new();
    let home = fixture.database.parent().unwrap();
    let config = home.join("config.toml");
    let environment = ProviderEnvironment::from_parts(
        &fixture.directory.path,
        &fixture.directory.path,
        vec![],
        [
            ("CODEX_HOME", home.to_path_buf()),
            ("CODEX_SQLITE_HOME", "  relative-index  ".into()),
        ],
    );
    assert_eq!(
        sqlite_home(&environment),
        Some(fixture.directory.path.join("relative-index"))
    );
    fs::write(
        &config,
        format!("sqlite_home = {:?}\n", home.to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(sqlite_home(&environment), Some(home.to_path_buf()));
    for text in [
        "sqlite_home = 42",
        "sqlite_home = 'relative'",
        "[profiles.custom]\nmodel='custom'",
        "broken = [",
    ] {
        fs::write(&config, text).unwrap();
        assert_eq!(fixture.admit(), Err(CallerSessionRefusal::Configuration));
    }
    fs::write(&config, vec![b' '; 64 * 1024 + 1]).unwrap();
    assert_eq!(fixture.admit(), Err(CallerSessionRefusal::Configuration));
}
