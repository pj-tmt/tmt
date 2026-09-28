use super::*;
use std::{
    cell::{Cell, RefCell},
    error::Error,
    io,
    time::{Duration, Instant},
};
use tmt_core::{
    endpoint::ServerEvidence,
    request::{FinalResponse, RequestEndpoint},
};

struct FakeRuntime {
    base: Instant,
    elapsed_ms: Cell<u64>,
    interrupted: Cell<bool>,
    waits: RefCell<Vec<Instant>>,
    fail_wait: Cell<bool>,
}

impl FakeRuntime {
    fn new() -> Self {
        Self {
            base: Instant::now(),
            elapsed_ms: Cell::new(0),
            interrupted: Cell::new(false),
            waits: RefCell::new(Vec::new()),
            fail_wait: Cell::new(false),
        }
    }

    fn at(&self, elapsed_ms: u64) -> Instant {
        self.base + Duration::from_millis(elapsed_ms)
    }

    fn set_elapsed(&self, elapsed_ms: u64) {
        self.elapsed_ms.set(elapsed_ms);
    }

    fn waited_ms(&self) -> Vec<u64> {
        self.waits
            .borrow()
            .iter()
            .map(|deadline| deadline.duration_since(self.base).as_millis() as u64)
            .collect()
    }
}

impl ObserverRuntime for FakeRuntime {
    fn now(&self) -> Instant {
        self.at(self.elapsed_ms.get())
    }

    fn interrupted(&self) -> bool {
        self.interrupted.get()
    }

    fn wait_until(&self, deadline: Instant) -> io::Result<()> {
        self.waits.borrow_mut().push(deadline);
        if self.fail_wait.get() {
            return Err(io::Error::other("wait failed"));
        }
        self.set_elapsed(deadline.duration_since(self.base).as_millis() as u64);
        Ok(())
    }
}

fn correlation() -> Correlation {
    Correlation {
        offline: false,
        request_id: "request-observe".into(),
        target: "worker".into(),
        pane: "%1".into(),
        identity: None,
        inbox: false,
    }
}

fn response(body: &str) -> FinalResponse {
    FinalResponse {
        request_id: "request-observe".into(),
        attempt_id: "attempt-observe".into(),
        route: tmt_core::request::RequestRoute::Pane(RequestEndpoint {
            server: ServerEvidence {
                server_id: "server".into(),
                socket_path: "/tmp/tmux.sock".into(),
                server_pid: 41,
                server_start_time: "server-start".into(),
            },
            pane_id: "%1".into(),
            pane_pid: 42,
        }),
        body: body.into(),
        body_bytes: body.len() as u64,
        submitted_at_ms: 123,
        response_expires_at_ms: 456,
    }
}

fn assert_failure(failure: &Failure, code: &'static str, status: u8, message: &str) {
    assert_eq!(failure.code, code);
    assert_eq!(failure.status, status);
    assert_eq!(failure.message, message);
}

fn assert_correlated(failure: &Failure) {
    let debug = format!("{failure:?}");
    assert!(
        debug.contains("request-observe"),
        "missing request correlation: {debug}"
    );
    assert!(
        debug.contains("worker"),
        "missing target correlation: {debug}"
    );
    assert!(debug.contains("%1"), "missing pane correlation: {debug}");
}

#[test]
fn returns_the_exact_final_response_before_the_deadline() {
    let runtime = FakeRuntime::new();
    let expected = response("\u{feff} exact\r\nbody\0");
    let reads = Cell::new(0);
    let actual = observe(
        || {
            reads.set(reads.get() + 1);
            Ok(Some(expected.clone()))
        },
        &correlation(),
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap();

    assert_eq!(actual, expected);
    assert_eq!(reads.get(), 1);
    assert!(runtime.waited_ms().is_empty());
}

#[test]
fn does_not_read_when_deadline_is_already_equal() {
    let runtime = FakeRuntime::new();
    runtime.set_elapsed(1_000);
    let reads = Cell::new(0);
    let failure = observe(
        || {
            reads.set(reads.get() + 1);
            Ok(None)
        },
        &correlation(),
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap_err();

    assert_failure(
        &failure,
        "TIMEOUT",
        4,
        "Timed out waiting for worker after 1s",
    );
    assert_eq!(reads.get(), 0);
    assert_correlated(&failure);
}

#[test]
fn response_read_crossing_deadline_is_lost_without_a_second_read() {
    let runtime = FakeRuntime::new();
    let reads = Cell::new(0);
    let expected = response("arrived at the deadline");
    let failure = observe(
        || {
            reads.set(reads.get() + 1);
            runtime.set_elapsed(1_000);
            Ok(Some(expected.clone()))
        },
        &correlation(),
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap_err();

    assert_failure(
        &failure,
        "TIMEOUT",
        4,
        "Timed out waiting for worker after 1s",
    );
    assert_eq!(reads.get(), 1);
}

#[test]
fn interruption_before_or_after_read_wins_without_waiting() {
    let runtime = FakeRuntime::new();
    runtime.interrupted.set(true);
    let reads = Cell::new(0);
    let before = observe(
        || {
            reads.set(reads.get() + 1);
            Ok(None)
        },
        &correlation(),
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap_err();
    assert_failure(
        &before,
        "INTERRUPTED",
        1,
        "Interrupted while waiting for a durable reply.",
    );
    assert_eq!(reads.get(), 0);

    runtime.interrupted.set(false);
    let after_reads = Cell::new(0);
    let after = observe(
        || {
            after_reads.set(after_reads.get() + 1);
            runtime.interrupted.set(true);
            Ok(Some(response("too late")))
        },
        &correlation(),
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap_err();
    assert_failure(
        &after,
        "INTERRUPTED",
        1,
        "Interrupted while waiting for a durable reply.",
    );
    assert_eq!(after_reads.get(), 1);
    assert!(runtime.waited_ms().is_empty());
}

#[test]
fn preserves_read_failure_without_polling_or_rewriting_it() {
    let runtime = FakeRuntime::new();
    let reads = Cell::new(0);
    let failure = observe(
        || {
            reads.set(reads.get() + 1);
            Err(Failure::new("READ_ERROR", "read failed", 7)
                .caused_by(io::Error::other("read cause")))
        },
        &correlation(),
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap_err();

    assert_failure(&failure, "READ_ERROR", 7, "read failed");
    assert_eq!(failure.source().unwrap().to_string(), "read cause");
    assert_eq!(reads.get(), 1);
}

#[test]
fn clips_no_response_polling_to_the_deadline() {
    let runtime = FakeRuntime::new();
    let reads = Cell::new(0);
    let failure = observe(
        || {
            reads.set(reads.get() + 1);
            Ok(None)
        },
        &correlation(),
        runtime.at(1_000),
        1.0,
        0.4,
        &runtime,
    )
    .unwrap_err();

    assert_failure(
        &failure,
        "TIMEOUT",
        4,
        "Timed out waiting for worker after 1s",
    );
    assert_eq!(reads.get(), 3);
    assert_eq!(runtime.waited_ms(), [400, 800, 1_000]);
}

#[test]
fn rounds_positive_submillisecond_polling_to_a_progressing_wait() {
    let runtime = FakeRuntime::new();
    let reads = Cell::new(0);
    let failure = observe(
        || {
            reads.set(reads.get() + 1);
            Ok(None)
        },
        &correlation(),
        runtime.at(3),
        0.003,
        0.0001,
        &runtime,
    )
    .unwrap_err();

    assert_failure(
        &failure,
        "TIMEOUT",
        4,
        "Timed out waiting for worker after 0.003s",
    );
    assert_eq!(reads.get(), 3);
    assert_eq!(runtime.waited_ms(), [1, 2, 3]);
}

#[test]
fn retains_wait_failure_and_correlation() {
    let runtime = FakeRuntime::new();
    runtime.fail_wait.set(true);
    let failure = observe(
        || Ok(None),
        &correlation(),
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap_err();

    assert_failure(&failure, "ERROR", 1, "Could not wait for a durable reply.");
    assert_correlated(&failure);
    assert_eq!(failure.source().unwrap().to_string(), "wait failed");
}
