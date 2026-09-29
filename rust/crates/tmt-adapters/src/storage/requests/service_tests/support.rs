use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{storage::Storage, test_support::TestDirectory};
use tmt_core::{
    endpoint::ServerEvidence,
    identity::{Lifetime, create_or_resolve},
    request::{
        Originator, PreambleReservation, PrepareRequest, RequestEndpoint, RequestRoute,
        RequestService,
    },
};

pub const NOW_MS: u64 = 1_700_000_000_000;
pub const DAY_MS: u64 = 86_400_000;

pub struct Fixture {
    pub _directory: TestDirectory,
    pub database: PathBuf,
    pub storage: Storage,
    pub identity_id: String,
    pub clock: Arc<AtomicU64>,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = TestDirectory::new();
        let database = directory.path.join("state").join("tmux-team.db");
        let mut storage = Storage::open(&database).expect("open request fixture storage");
        let identity = create_or_resolve(&mut storage, "Request Owner", Lifetime::Saved)
            .expect("create request fixture identity")
            .identity;
        Self {
            _directory: directory,
            database,
            storage,
            identity_id: identity.id,
            clock: Arc::new(AtomicU64::new(NOW_MS)),
        }
    }

    pub fn set_now(&self, value: u64) {
        self.clock.store(value, Ordering::SeqCst);
    }
}

pub fn service<'a>(fixture: &'a mut Fixture) -> RequestService<'a, Storage, impl Fn() -> u64> {
    let clock = Arc::clone(&fixture.clock);
    RequestService::new(&mut fixture.storage, move || clock.load(Ordering::SeqCst))
}

pub fn endpoint(pane_id: &str, pane_pid: u64) -> RequestEndpoint {
    RequestEndpoint {
        server: ServerEvidence {
            host: tmt_core::host::HostKind::Tmux,
            server_id: "server-for-request-tests".into(),
            socket_path: "/tmp/tmt-request-tests.sock".into(),
            server_pid: 41,
            server_start_time: "request-test-server-start".into(),
        },
        pane_id: pane_id.into(),
        pane_pid,
    }
}

pub fn prepare_input(
    fixture: &Fixture,
    request_id: &str,
    target: RequestEndpoint,
    wait: bool,
    expires_at_ms: u64,
    originator: Originator,
    preamble: bool,
) -> PrepareRequest {
    PrepareRequest {
        room_id: None,
        kind: tmt_core::request::RequestKind::Request,
        request_id: request_id.into(),
        message: format!("prompt for {request_id}"),
        route: RequestRoute::Pane(target),
        wait,
        expires_at_ms,
        originator,
        recipient_identity_id: Some(fixture.identity_id.clone()),
        preamble: preamble.then(|| PreambleReservation {
            identity_id: fixture.identity_id.clone(),
            every: 3,
        }),
    }
}

pub fn count_rows(database: &PathBuf, table: &str) -> i64 {
    let connection =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open request SQL oracle");
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count request rows")
}

pub fn preamble_count(database: &PathBuf, identity_id: &str) -> i64 {
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open request SQL oracle");
    connection
        .query_row(
            "SELECT COALESCE(reserved_count, 0) FROM preamble_counters WHERE identity_id = ?",
            [identity_id],
            |row| row.get(0),
        )
        .optional()
        .expect("read request cadence")
        .unwrap_or(0)
}
