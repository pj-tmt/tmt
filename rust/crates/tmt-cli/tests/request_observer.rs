//! Detached observer exit status, durable request state and owned log lifetime.
#![cfg(unix)]

mod support;

use nix::poll::{PollFd, PollFlags, poll};
use rusqlite::Connection;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::{
        fd::AsFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tmt_adapters::{request_runtime::wall_time_ms, storage::Storage};
use tmt_core::request::{RequestService, SubmitResponse, notification::NotificationPolicy};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    database: PathBuf,
    request_id: String,
    recipient_id: String,
    deadline_ms: u64,
    child: Option<Child>,
}

impl Fixture {
    fn new(timeout_ms: u64) -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-request-observer-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let mut fixture = Self {
            database: support::state_dir(&root).join("tmux-team.db"),
            root,
            request_id: String::new(),
            recipient_id: String::new(),
            deadline_ms: 0,
            child: None,
        };
        fixture.run(&["identity", "create", "Sender", "--json"]);
        fixture.run(&["identity", "create", "Receiver", "--json"]);
        let queued = fixture.run(&[
            "talk",
            "Receiver",
            "observer probe",
            "--inbox",
            "--detach",
            "--identity",
            "Sender",
            "--json",
        ]);
        let receipt: serde_json::Value = serde_json::from_slice(&queued.stdout).unwrap();
        fixture.request_id = receipt["requestId"].as_str().unwrap().into();
        fixture.recipient_id = receipt["recipientIdentityId"].as_str().unwrap().into();
        fixture.deadline_ms = wall_time_ms() + timeout_ms;
        let mut storage = Storage::open(&fixture.database).unwrap();
        RequestService::new(&mut storage, wall_time_ms)
            .enable_notifications(
                &fixture.request_id,
                NotificationPolicy {
                    deadline_ms: fixture.deadline_ms,
                    timeout_ms,
                    waiter: None,
                },
            )
            .unwrap();
        storage.close().unwrap();
        fixture
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = support::command(&self.root, args);
        command.stdin(Stdio::null()).stdout(Stdio::piped());
        command
    }

    fn wait(&mut self) -> Output {
        support::wait(&mut self.child, Duration::from_secs(10))
            .wait_with_output()
            .unwrap()
    }

    fn run(&mut self, args: &[&str]) -> Output {
        assert!(self.child.is_none());
        self.child = Some(self.command(args).stderr(Stdio::piped()).spawn().unwrap());
        let output = self.wait();
        assert!(output.status.success(), "CLI failed: {output:?}");
        output
    }

    fn log_path(&self) -> PathBuf {
        support::state_dir(&self.root)
            .join("request-observers")
            .join(format!("{}.log", self.request_id))
    }

    fn start(&mut self) -> File {
        assert!(self.child.is_none());
        let path = self.log_path();
        fs::create_dir(path.parent().unwrap()).unwrap();
        let mut log = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        self.child = Some(
            self.command(&["__request-observer", &self.request_id])
                .stderr(Stdio::from(log.try_clone().unwrap()))
                .spawn()
                .unwrap(),
        );
        let child = self.child.as_mut().unwrap();
        let stdout = child.stdout.as_mut().unwrap();
        let mut readiness = [PollFd::new(stdout.as_fd(), PollFlags::POLLIN)];
        assert_eq!(
            poll(&mut readiness, 5000u16).unwrap(),
            1,
            "observer not ready"
        );
        let mut ack = [0];
        stdout.read_exact(&mut ack).unwrap();
        assert_eq!(ack, [b'R']);
        // Read the attached inode even if a clean, short deadline already unlinked it.
        log.seek(SeekFrom::Start(0)).unwrap();
        let mut text = String::new();
        log.read_to_string(&mut text).unwrap();
        assert_eq!(text, format!("observer_pid={}\n", child.id()));
        log
    }

    fn assert_queued(&self) {
        let db = Connection::open(&self.database).unwrap();
        let status: String = db
            .query_row(
                "SELECT status FROM request_attempts WHERE request_id = ?",
                [&self.request_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "queued");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        assert_eq!(fs::read_dir(self.root.join("tmux")).unwrap().count(), 0);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn storage_failure_after_readiness_exits_nonzero_and_keeps_the_owned_log() {
    let mut fixture = Fixture::new(120_000);
    let log = fixture.start();
    let attached = log.metadata().unwrap();
    let db = Connection::open(&fixture.database).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    // Inject only after R and the PID line; the policy read and close still work.
    db.execute_batch("ALTER TABLE request_responses RENAME TO injected_unavailable_responses")
        .unwrap();
    let output = fixture.wait();
    let retained = fs::metadata(fixture.log_path()).expect("failure log must remain");
    assert_eq!(
        (retained.dev(), retained.ino()),
        (attached.dev(), attached.ino())
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let text = fs::read_to_string(fixture.log_path()).unwrap();
    assert!(text.starts_with("observer_pid="));
    assert_eq!(text.lines().count(), 2);
    assert_eq!(
        text.lines().nth(1),
        Some("warning: tmt: request timeout observer unavailable; inspect the retained request")
    );
    fixture.assert_queued();
    let finals: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM injected_unavailable_responses",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(finals, 0);
    let policy: (String, String, bool) = db.query_row(
        "SELECT reply_state, timeout_state, observed FROM request_notifications WHERE request_id = ?",
        [&fixture.request_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(
        policy,
        ("not_attempted".into(), "not_attempted".into(), false)
    );
    assert!(wall_time_ms() < fixture.deadline_ms);
}

#[test]
fn a_final_after_readiness_exits_cleanly_and_removes_the_log() {
    let mut fixture = Fixture::new(120_000);
    let _log = fixture.start();
    let mut storage = Storage::open(&fixture.database).unwrap();
    let mut service = RequestService::new(&mut storage, wall_time_ms);
    let (request_id, proof, _) = service
        .answer_target(&fixture.recipient_id, None, Some(&fixture.request_id))
        .unwrap();
    service
        .submit_response(SubmitResponse {
            request_id,
            proof,
            body: "completed".into(),
        })
        .unwrap();
    storage.close().unwrap();
    let output = fixture.wait();
    assert_eq!(output.status.code(), Some(0));
    assert!(!fixture.log_path().exists());
    let result = fixture.run(&["result", &fixture.request_id.clone(), "--json"]);
    let result: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result["response"], "completed");
}

#[test]
fn a_timeout_exits_cleanly_and_keeps_the_request_queued() {
    let mut fixture = Fixture::new(1000);
    let _log = fixture.start();
    let output = fixture.wait();
    assert_eq!(output.status.code(), Some(0));
    assert!(!fixture.log_path().exists());
    fixture.assert_queued();
    let db = Connection::open(&fixture.database).unwrap();
    let timeout: String = db
        .query_row(
            "SELECT timeout_state FROM request_notifications WHERE request_id = ?",
            [&fixture.request_id],
            |row| row.get(0),
        )
        .unwrap();
    // Unbound identities have no delivery route; the one-shot claim is settled.
    assert_eq!(timeout, "unavailable");
    let finals: i64 = db
        .query_row("SELECT COUNT(*) FROM request_responses", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(finals, 0);
}
