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
        data_dir: std::env::temp_dir().join("tmt-observer-test"),
        offline: false,
        unbound: false,
        request_id: "request-observe".into(),
        target: "worker".into(),
        pane: "%1".into(),
        identity: None,
        inbox: false,
        explicit_inbox: false,
        delivery_uncertain: false,
        digest_until_ms: None,
    }
}

fn response(body: &str) -> FinalResponse {
    FinalResponse {
        request_id: "request-observe".into(),
        attempt_id: "attempt-observe".into(),
        route: tmt_core::request::RequestRoute::Pane(RequestEndpoint {
            server: ServerEvidence {
                host: tmt_core::host::HostKind::Tmux,
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
    for failure in [&before, &after] {
        assert_eq!(
            failure.document().to_string(),
            r#"{"error":{"code":"INTERRUPTED","message":"Interrupted while waiting for a durable reply.","suggestion":"Inspect with 'tmt result request-observe' and 'tmt check worker' before deciding whether to retry."},"requestId":"request-observe","target":"worker","pane":"%1"}"#,
        );
        let mut human = Vec::new();
        failure
            .write_human(&mut human, tmt_cli_style::Terminal::PLAIN)
            .unwrap();
        assert_eq!(
            human,
            b"error: Interrupted while waiting for a durable reply\nhint: Inspect with 'tmt result request-observe' and 'tmt check worker' before deciding whether to retry\n",
        );
    }
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

#[test]
fn a_timeout_after_an_unacknowledged_write_reports_uncertain_delivery_and_no_resend() {
    let runtime = FakeRuntime::new();
    runtime.set_elapsed(1_000);
    let mut unconfirmed = correlation();
    unconfirmed.delivery_uncertain = true;
    let failure = observe(
        || Ok(None),
        &unconfirmed,
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap_err();
    assert_eq!(failure.code, "TIMEOUT");
    assert_eq!(failure.status, 4);
    assert!(failure.message.contains("delivery was uncertain"));
    assert!(failure.message.contains("do not resend"));
    let document = failure.document();
    assert_eq!(document["deliveryState"], "uncertain");
    assert_eq!(document["status"], "timeout");
    assert_eq!(document["requestId"], "request-observe");
    // A confirmed delivery states nothing about uncertainty.
    let confirmed = observe(
        || Ok(None),
        &correlation(),
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap_err();
    assert!(confirmed.document().get("deliveryState").is_none());
    assert!(!confirmed.message.contains("uncertain"));
}

#[test]
fn an_unbound_recipient_times_out_with_result_and_exact_recipient_inspection_commands() {
    let runtime = FakeRuntime::new();
    let mut recipient = correlation();
    recipient.unbound = true;
    recipient.inbox = true;
    recipient.offline = true;
    recipient.pane.clear();
    recipient.identity = Some(Identity {
        id: "49000000-0000-4000-8000-000000000001".into(),
        name: "Worker's Team".into(),
        canonical_name: "worker's team".into(),
        lifetime: tmt_core::identity::Lifetime::Saved,
        created_at: "created".into(),
        updated_at: "updated".into(),
    });
    let failure = observe(
        || Ok(None),
        &recipient,
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
        "worker has not responded within 1s; the request is still queued for inbox pull",
    );
    assert_eq!(runtime.waited_ms(), [400, 800, 1_000]);
    let document = failure.document();
    assert_eq!(document["status"], "timeout");
    assert!(document.get("pane").is_none());
    let suggestion = document["error"]["suggestion"].as_str().unwrap();
    assert!(suggestion.contains("tmt result request-observe --json"));
    assert!(
        suggestion.contains("tmt inbox --identity '49000000-0000-4000-8000-000000000001' --json")
    );
    assert!(suggestion.contains("tmt x show request-observe --incoming --identity '49000000-0000-4000-8000-000000000001' --json"));
    assert!(suggestion.contains("do not resend"));
    assert!(!suggestion.contains("tmt resume"));
}

#[test]
fn an_unbound_recipient_returns_the_exact_reply_after_a_bounded_poll() {
    let runtime = FakeRuntime::new();
    let mut recipient = correlation();
    recipient.unbound = true;
    recipient.inbox = true;
    let mut expected = response("exact inbox reply\n");
    expected.route = tmt_core::request::RequestRoute::Inbox {
        recipient_identity_id: "recipient".into(),
    };
    let reads = Cell::new(0);
    let actual = observe(
        || {
            reads.set(reads.get() + 1);
            Ok((reads.get() == 2).then(|| expected.clone()))
        },
        &recipient,
        runtime.at(1_000),
        1.0,
        0.25,
        &runtime,
    )
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(reads.get(), 2);
    assert_eq!(runtime.waited_ms(), [250]);
}
