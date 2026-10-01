//! Real process death while the shared preparation transaction is still open.

use super::support::{Fixture, NOW_MS, endpoint, prepare_input, request_snapshot, service};
use crate::{
    storage::{Storage, StorageError},
    test_support::TestChild,
};
use rusqlite::{Connection, OpenFlags};
use std::{
    env, fs,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tmt_core::request::{
    Originator, PreambleReservation, PrepareRequest, RequestRecords, RequestRepository,
    RequestRoute, RequestService,
};

const CHILD_TEST: &str = "storage::requests::service_tests::crash::preparation_child";
const DATABASE_ENV: &str = "TMT_REQUEST_CRASH_DATABASE";
const OWNER_ENV: &str = "TMT_REQUEST_CRASH_OWNER";
const READY_ENV: &str = "TMT_REQUEST_CRASH_READY";
const RELEASE_ENV: &str = "TMT_REQUEST_CRASH_RELEASE";

#[test]
fn sigkill_rolls_back_preparation_without_losing_committed_state() {
    for kill_before_commit in [true, false] {
        let mut fixture = Fixture::new();
        let baseline = prepare_input(
            &fixture,
            "committed-baseline",
            endpoint("%11", 111),
            false,
            NOW_MS + 60_000,
            Originator::Explicit(fixture.identity_id.clone()),
            true,
        );
        service(&mut fixture)
            .prepare(baseline, "baseline-attempt".into(), 7)
            .unwrap();
        fixture.storage.close().unwrap();
        let before = request_snapshot(&fixture.database);
        let ready = fixture._directory.path.join("transaction-ready");
        let release = fixture._directory.path.join("release-commit");
        let child = Command::new(env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(DATABASE_ENV, &fixture.database)
            .env(OWNER_ENV, &fixture.identity_id)
            .env(READY_ENV, &ready)
            .env(RELEASE_ENV, &release)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut child = TestChild::new(child);
        child.wait_for_content(&ready, "prepared-uncommitted", Duration::from_secs(5));
        // WAL readers must see the committed baseline while the real service's
        // attempt/cadence/attention writes are held inside its transaction.
        assert_eq!(request_snapshot(&fixture.database), before);
        if kill_before_commit {
            assert!(child.child.try_wait().unwrap().is_none());
            child.child.kill().unwrap();
            assert_eq!(
                child.wait_for_exit(Duration::from_secs(3)).signal(),
                Some(9)
            );
        } else {
            fs::write(&release, "commit").unwrap();
            assert!(child.wait_for_exit(Duration::from_secs(3)).success());
        }
        let mut reopened = Storage::open(&fixture.database).unwrap();
        assert_eq!(reopened.health().unwrap().journal_mode, "wal");
        reopened.close().unwrap();
        let after = request_snapshot(&fixture.database);
        let oracle =
            Connection::open_with_flags(&fixture.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert_eq!(
            oracle
                .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        if kill_before_commit {
            assert_eq!(after, before);
        } else {
            // Positive control: readiness cannot be published without the
            // mutation under test; allowing commit must expose all three writes.
            assert_eq!(after[0].len(), 2);
            assert_eq!(after[0][0], before[0][0]);
            assert_ne!(after[0], before[0]);
            assert_eq!(after[1], before[1]);
            assert_ne!(after[2], before[2]);
            assert_ne!(after[3], before[3]);
            assert_eq!(
                oracle
                    .query_row("SELECT reserved_count FROM preamble_counters", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap(),
                2
            );
            assert_eq!(
                oracle
                    .query_row(
                        "SELECT latest_revision FROM request_attention_identities",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                2
            );
        }
    }
}

/// Decorate only the repository's transaction callback, not its SQL or service
/// policy. This pauses after actual prepare writes and before the actual commit.
struct PauseBeforeCommit {
    storage: Storage,
    ready: PathBuf,
    release: PathBuf,
}

impl RequestRepository for PauseBeforeCommit {
    type Error = StorageError;

    fn with_request_transaction<T, E: From<Self::Error>>(
        &mut self,
        operation: impl FnOnce(&mut dyn RequestRecords<Error = Self::Error>) -> Result<T, E>,
    ) -> Result<T, E> {
        let ready = &self.ready;
        let release = &self.release;
        self.storage.with_request_transaction(|records| {
            let result = operation(records)?;
            fs::write(ready, "prepared-uncommitted").unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !release.exists() {
                assert!(
                    Instant::now() < deadline,
                    "parent did not terminate or release crash fixture"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Ok(result)
        })
    }
}

#[test]
fn preparation_child() {
    let Some(database) = env::var_os(DATABASE_ENV) else {
        return;
    };
    let owner = env::var(OWNER_ENV).unwrap();
    let mut repository = PauseBeforeCommit {
        storage: Storage::open(PathBuf::from(database)).unwrap(),
        ready: env::var_os(READY_ENV).unwrap().into(),
        release: env::var_os(RELEASE_ENV).unwrap().into(),
    };
    RequestService::new(&mut repository, || NOW_MS)
        .prepare(
            PrepareRequest {
                room_id: None,
                kind: tmt_core::request::RequestKind::Request,
                request_id: "uncommitted-request".into(),
                message: "uncommitted prompt".into(),
                route: RequestRoute::Pane(endpoint("%12", 112)),
                wait: false,
                expires_at_ms: NOW_MS + 60_000,
                originator: Originator::Explicit(owner.clone()),
                recipient_identity_id: Some(owner.clone()),
                preamble: Some(PreambleReservation {
                    identity_id: owner,
                    every: 3,
                }),
            },
            "uncommitted-attempt".into(),
            7,
        )
        .unwrap();
    repository.storage.close().unwrap();
}
