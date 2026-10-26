//! `tmt storage` reports the local stores read-only, and the internal
//! compaction step switches the Core database to incremental auto-vacuum only
//! when nothing else is writing (#5036).
#![cfg(unix)]
mod support;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::Duration,
};
use tmt_adapters::storage::Storage;

struct Fixture {
    root: PathBuf,
    child: Option<Child>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("tmt-storage-{}-{name}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self { root, child: None }
    }

    fn database(&self) -> PathBuf {
        support::state_dir(&self.root).join("tmt.db")
    }

    /// Runs `tmt` to completion and returns its exit code, stdout and stderr.
    fn run(&mut self, args: &[&str]) -> (i32, String, String) {
        let stdout_path = self.root.join("stdout");
        let stderr_path = self.root.join("stderr");
        let mut command = support::command(&self.root, args);
        command
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout_path).unwrap())
            .stderr(fs::File::create(&stderr_path).unwrap());
        self.child = Some(command.spawn().unwrap());
        let status = support::wait(&mut self.child, Duration::from_secs(60))
            .wait()
            .unwrap();
        (
            status.code().unwrap(),
            fs::read_to_string(stdout_path).unwrap(),
            fs::read_to_string(stderr_path).unwrap(),
        )
    }

    fn json(&mut self, args: &[&str]) -> Value {
        let (code, stdout, stderr) = self.run(args);
        assert_eq!((code, stderr.as_str()), (0, ""), "{args:?}: {stdout}");
        serde_json::from_str(&stdout).unwrap()
    }

    /// A real Core database in which most pages of a real table were freed.
    fn freed_store(&self) {
        let path = self.database();
        Storage::open(&path).unwrap().close().unwrap();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
                 INSERT INTO exchange_flow
                   (sender_identity_id, recipient_identity_id, kind, bucket_start_ms, count, bytes)
                 SELECT hex(zeroblob(500)) || i, 'recipient', 'ack', i * 1800000, 1, 0 FROM n",
                [],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM exchange_flow WHERE bucket_start_ms > 50 * 1800000",
                [],
            )
            .unwrap();
        connection.close().unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn store_bytes(path: &Path) -> (Vec<u8>, Option<Vec<u8>>) {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    (fs::read(path).unwrap(), fs::read(PathBuf::from(wal)).ok())
}

#[test]
fn missing_stores_are_reported_absent_and_nothing_is_created() {
    let mut fixture = Fixture::new("absent");
    let state = support::state_dir(&fixture.root);
    fs::create_dir_all(&state).unwrap();

    let report = fixture.json(&["storage", "--json"]);

    let root = state.to_str().unwrap();
    assert_eq!(
        report,
        json!({
            "dataRoot": root,
            "database": {"path": format!("{root}/tmt.db"), "present": false},
            "notes": {"path": format!("{root}/notes"), "present": false},
            "colab": {"path": format!("{root}/colab/space.db"), "present": false},
            "compaction": {"pending": false, "command": null},
        })
    );
    let (code, human, stderr) = fixture.run(&["storage"]);
    assert_eq!((code, stderr.as_str()), (0, ""));
    assert_eq!(human.matches("  state  absent\n").count(), 3, "{human}");
    assert!(!human.contains("hint:"), "{human}");
    assert!(names(&state).is_empty(), "{:?}", names(&state));
}

#[test]
fn compaction_shrinks_the_store_and_the_report_follows_it() {
    let mut fixture = Fixture::new("compact");
    fixture.freed_store();
    let state = support::state_dir(&fixture.root);
    fs::create_dir_all(state.join("notes/id-1")).unwrap();
    fs::write(state.join("notes/id-1/notes.md"), b"notes").unwrap();
    fs::create_dir_all(state.join("colab")).unwrap();
    fs::write(state.join("colab/space.db"), vec![0; 4096]).unwrap();
    fs::write(state.join("colab/space.db.pre-v12.bak"), vec![0; 1024]).unwrap();
    let files = names(&state);

    let before = fixture.json(&["storage", "--json"]);
    assert_eq!(names(&state), files, "the report changes nothing");
    assert_eq!(before["database"]["autoVacuum"], "none");
    assert!(
        before["database"]["freePages"].as_u64().unwrap() > 300,
        "{before}"
    );
    assert_eq!(
        before["compaction"],
        json!({"pending": true, "command": "tmt upgrade"})
    );
    assert_eq!(before["notes"]["bytes"], 5);
    assert_eq!(before["colab"]["backups"][0]["bytes"], 1024);
    let (_, human, _) = fixture.run(&["storage"]);
    assert!(human.contains("  compaction   pending\n"), "{human}");
    assert!(
        human.ends_with("hint: finish the one-time compaction of tmt.db with: tmt upgrade\n"),
        "{human}"
    );

    let compacted = fixture.json(&["__native-compact-database", "--json"]);

    assert_eq!(compacted["status"], "compacted", "{compacted}");
    let (bytes_before, bytes_after) = (
        compacted["bytesBefore"].as_u64().unwrap(),
        compacted["bytesAfter"].as_u64().unwrap(),
    );
    assert!(bytes_after * 3 < bytes_before, "{compacted}");
    let after = fixture.json(&["storage", "--json"]);
    assert_eq!(after["database"]["autoVacuum"], "incremental");
    assert_eq!(after["database"]["freePages"], 0);
    assert_eq!(after["database"]["bytes"], bytes_after);
    assert_eq!(
        after["compaction"],
        json!({"pending": false, "command": null})
    );
    let connection = Connection::open(fixture.database()).unwrap();
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM exchange_flow", [], |row| row.get(0))
        .unwrap();
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!((rows, integrity.as_str()), (50, "ok"));
    drop(connection);
    assert_eq!(
        fixture.json(&["__native-compact-database", "--json"]),
        json!({"status": "current"})
    );
}

#[test]
fn an_active_writer_refuses_compaction_and_the_database_is_unchanged() {
    let mut fixture = Fixture::new("busy");
    fixture.freed_store();
    let writer = Connection::open(fixture.database()).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let bytes = store_bytes(&fixture.database());

    assert_eq!(
        fixture.json(&["__native-compact-database", "--json"]),
        json!({"status": "in-use"})
    );
    let (code, human, _) = fixture.run(&["__native-compact-database"]);
    assert_eq!(code, 0);
    assert_eq!(
        human,
        "The Core database was not compacted: tmt.db is in use by another writer; nothing changed.\n"
    );

    assert_eq!(store_bytes(&fixture.database()), bytes);
    writer.execute_batch("ROLLBACK").unwrap();
    drop(writer);
    let report = fixture.json(&["storage", "--json"]);
    assert_eq!(report["compaction"]["pending"], true);
}

#[test]
fn without_a_database_compaction_is_absent_and_creates_nothing() {
    let mut fixture = Fixture::new("no-database");
    let state = support::state_dir(&fixture.root);
    fs::create_dir_all(&state).unwrap();
    assert_eq!(
        fixture.json(&["__native-compact-database", "--json"]),
        json!({"status": "absent"})
    );
    assert!(names(&state).is_empty());
}
