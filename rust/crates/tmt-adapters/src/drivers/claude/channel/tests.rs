use super::*;
use crate::process::runtime::ProcessObservation;
use std::{
    io::BufRead,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    sync::atomic::{AtomicU32, Ordering},
    thread::JoinHandle,
};
use tmt_core::{
    binding::{
        Binding,
        session::{BindingSessionState, ObservedSessionKey, RuntimeState, SessionTransition},
    },
    endpoint::ServerEvidence,
    host::HostKind,
    identity::{Identity, Lifetime},
};

const BINDING: &str = "11111111-1111-4111-8111-111111111111";
const GENERATION: &str = "22222222-2222-4222-8222-222222222222";
const MESSAGE: &str = "<tmt-reply from=\"lead\">run the tests</tmt-reply>";
/// Long enough for a slow machine, short enough to keep the suite quick.
const SHORT_WAIT: Duration = Duration::from_millis(250);

/// Unix socket paths are length-limited, so the scratch root cannot live under
/// a long per-user temporary directory.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = PathBuf::from(format!(
            "/tmp/tmt-ch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The test process, observed exactly as `tmt run` observes itself: a live
/// launch owner whose start identity really matches.
fn live_owner() -> ProcessIncarnation {
    match observe_runtime_process(
        &UnixCommandRunner,
        u64::from(std::process::id()),
        Instant::now() + Duration::from_secs(3),
    ) {
        Ok(ProcessObservation::Live(incarnation)) => incarnation,
        other => panic!("the test process must be observable: {other:?}"),
    }
}

/// A launch owner that has exited: its PID is reaped, so it is conclusively gone.
fn dead_owner() -> ProcessIncarnation {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = u64::from(child.id());
    child.wait().unwrap();
    ProcessIncarnation::new(pid, "Thu Oct  1 10:00:00 2026").unwrap()
}

fn provider(pid: u64) -> ProcessIncarnation {
    ProcessIncarnation::new(pid, "Thu Oct  1 10:00:00 2026").unwrap()
}

/// A stored binding: its launch owner and Claude runtime as `tmt run` admitted.
fn entry(
    binding_id: &str,
    owner: Option<ProcessIncarnation>,
    observed: Option<ProcessIncarnation>,
) -> BindingEntry {
    BindingEntry {
        identity: Identity {
            id: "33333333-3333-4333-8333-333333333333".into(),
            name: "worker".into(),
            canonical_name: "worker".into(),
            lifetime: Lifetime::Temporary,
            created_at: "2026-10-01T00:00:00Z".into(),
            updated_at: "2026-10-01T00:00:00Z".into(),
        },
        binding: Some(Binding {
            id: binding_id.into(),
            identity_id: "33333333-3333-4333-8333-333333333333".into(),
            server: ServerEvidence {
                host: HostKind::Tmux,
                server_id: "server".into(),
                socket_path: "/tmp/tmux-test".into(),
                server_pid: 1,
                server_start_time: "start".into(),
            },
            pane_id: "%1".into(),
            pane_pid: 2,
            session: BindingSessionState {
                last_transition: Some(SessionTransition::Started),
                state: RuntimeState::Running,
                key: observed.map(|incarnation| ObservedSessionKey {
                    incarnation,
                    provider_session: None,
                }),
                launch_owner: owner,
            },
        }),
    }
}

/// The enrollment `tmt run --channel` writes before Claude starts.
fn intent(owner: &ProcessIncarnation) -> Record {
    Record {
        version: RECORD_VERSION,
        binding_id: BINDING.into(),
        generation: GENERATION.into(),
        launch_owner: process(owner),
        claude: None,
    }
}

fn ready(owner: &ProcessIncarnation, claude: &ProcessIncarnation) -> Record {
    Record {
        claude: Some(process(claude)),
        ..intent(owner)
    }
}

fn process(incarnation: &ProcessIncarnation) -> Process {
    Process {
        pid: incarnation.pid(),
        start: incarnation.start_identity().to_owned(),
    }
}

fn publish(directory: &Path, record: &Record) {
    crate::private_file::replace(
        &record_path(directory, &record.binding_id),
        &serde_json::to_vec(record).unwrap(),
    )
    .unwrap();
}

/// A one-connection endpoint standing in for the channel server. It returns the
/// bytes it received so a test can assert the frame, and answers as told.
fn endpoint(directory: &Path, answer: Option<&'static str>) -> JoinHandle<Vec<u8>> {
    let listener = UnixListener::bind(socket_path(directory, BINDING)).unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut line = Vec::new();
        io::BufReader::new(&stream)
            .read_until(b'\n', &mut line)
            .unwrap();
        if let Some(answer) = answer {
            stream.write_all(answer.as_bytes()).unwrap();
        }
        line
    })
}

fn fault(result: Sent) -> (&'static str, ChannelFault) {
    match result {
        ActionResult::Failed(SendFailure::Denied(RuntimeError::Channel(fault))) => {
            ("denied", fault)
        }
        ActionResult::Failed(SendFailure::NotSent(RuntimeError::Channel(fault))) => {
            ("not-sent", fault)
        }
        ActionResult::Failed(SendFailure::Uncertain(RuntimeError::Channel(fault))) => {
            ("uncertain", fault)
        }
        other => panic!("unexpected result {other:?}"),
    }
}

/// A send that must decide immediately (the enrollment is already ready).
fn deliver(scratch: &Scratch, entry: &BindingEntry, message: &str) -> Sent {
    send_within(Some(&scratch.0), entry, message, SHORT_WAIT)
}

#[test]
fn a_session_that_never_opted_in_has_no_record_and_keeps_paste() {
    let scratch = Scratch::new();
    let owner = live_owner();
    assert_eq!(
        deliver(
            &scratch,
            &entry(BINDING, Some(owner), Some(provider(4242))),
            MESSAGE
        ),
        ActionResult::Unsupported
    );
    let unbound = BindingEntry {
        binding: None,
        ..entry(BINDING, None, None)
    };
    assert_eq!(
        deliver(&scratch, &unbound, MESSAGE),
        ActionResult::Unsupported
    );
}

#[test]
fn an_enrollment_whose_own_launch_has_ended_is_stale_and_never_pasted_to() {
    let scratch = Scratch::new();
    let owner = dead_owner();
    publish(&scratch.0, &ready(&owner, &provider(4242)));
    // The launch the enrollment belongs to is still the binding's, and it ended:
    // that is not evidence that the session never opted in.
    assert_eq!(
        fault(deliver(
            &scratch,
            &entry(BINDING, Some(owner), Some(provider(4242))),
            MESSAGE
        )),
        ("denied", ChannelFault::Stale)
    );
}

#[test]
fn a_plain_relaunch_outside_tmt_run_leaves_the_ended_enrollment_stale_for_the_new_runtime() {
    let (ended, recorded) = (dead_owner(), provider(4242));
    // The binding still names the old launch owner; the runtime now observed for
    // it is a live process that is not the Claude the enrollment names.
    for (name, record) in [
        ("ready record", ready(&ended, &recorded)),
        ("pending record", intent(&ended)),
    ] {
        let scratch = Scratch::new();
        publish(&scratch.0, &record);
        let before = fs::read(record_path(&scratch.0, BINDING)).unwrap();
        assert_eq!(
            deliver(
                &scratch,
                &entry(BINDING, Some(ended.clone()), Some(live_owner())),
                MESSAGE
            ),
            ActionResult::Unsupported,
            "{name}"
        );
        assert_eq!(
            fs::read(record_path(&scratch.0, BINDING)).unwrap(),
            before,
            "{name}: the record is untouched"
        );
    }
}

#[test]
fn an_ended_enrollment_stays_stale_unless_a_different_live_runtime_is_proven() {
    let ended = dead_owner();
    let other_ended = ProcessIncarnation::new(ended.pid(), "Thu Oct  1 11:00:00 2026").unwrap();
    // (case, the Claude the record names, the runtime now observed for the binding)
    for (name, recorded, running) in [
        (
            "the observed runtime is the live one the record names",
            live_owner(),
            Some(live_owner()),
        ),
        (
            "the observed runtime is the ended one the record names",
            provider(4242),
            Some(provider(4242)),
        ),
        (
            "a different runtime that is not alive",
            provider(4242),
            Some(other_ended),
        ),
        ("no runtime observed", provider(4242), None),
    ] {
        let scratch = Scratch::new();
        publish(&scratch.0, &ready(&ended, &recorded));
        assert_eq!(
            fault(deliver(
                &scratch,
                &entry(BINDING, Some(ended.clone()), running),
                MESSAGE
            )),
            ("denied", ChannelFault::Stale),
            "{name}"
        );
    }
    // Stale is actionable: it says how to recover.
    assert!(
        ChannelFault::Stale
            .reason()
            .contains("Relaunch the agent with `tmt run`")
    );
}

#[test]
fn an_enrolled_write_is_unacknowledged_and_carries_the_exact_frame() {
    let scratch = Scratch::new();
    let (owner, claude) = (live_owner(), provider(4242));
    publish(&scratch.0, &ready(&owner, &claude));
    let server = endpoint(&scratch.0, Some("{\"written\":true}\n"));
    assert_eq!(
        deliver(
            &scratch,
            &entry(BINDING, Some(owner), Some(claude)),
            MESSAGE
        ),
        ActionResult::Completed(DeliveryAcceptance::Unacknowledged)
    );
    let frame: Frame = serde_json::from_slice(&server.join().unwrap()).unwrap();
    assert_eq!(frame.version, RECORD_VERSION);
    assert_eq!(frame.generation, GENERATION);
    assert_eq!(frame.content, MESSAGE);
}

/// The positive control above differs from each case by one condition. A
/// mismatch must never reach paste, so none of these may be `NotSent` or
/// `Unsupported`, and the decision precedes any connection.
#[test]
fn a_record_that_does_not_match_the_stored_binding_denies_and_never_connects() {
    let (owner, claude) = (live_owner(), provider(4242));
    let cases: Vec<(&str, Record, BindingEntry)> = vec![
        (
            "other Claude process",
            ready(&owner, &claude),
            entry(BINDING, Some(owner.clone()), Some(provider(4243))),
        ),
        (
            "other start identity",
            Record {
                claude: Some(Process {
                    pid: 4242,
                    start: "Thu Oct  1 10:00:01 2026".into(),
                }),
                ..ready(&owner, &claude)
            },
            entry(BINDING, Some(owner.clone()), Some(claude.clone())),
        ),
        (
            "no runtime observation",
            ready(&owner, &claude),
            entry(BINDING, Some(owner.clone()), None),
        ),
        (
            "binding recorded no launch owner",
            ready(&owner, &claude),
            entry(BINDING, None, Some(claude.clone())),
        ),
        (
            "other launch that cannot be proven current",
            ready(&owner, &claude),
            entry(BINDING, Some(provider(1)), Some(claude.clone())),
        ),
        (
            "other binding",
            Record {
                binding_id: "44444444-4444-4444-8444-444444444444".into(),
                ..ready(&owner, &claude)
            },
            entry(BINDING, Some(owner.clone()), Some(claude.clone())),
        ),
        (
            "unknown record version",
            Record {
                version: 2,
                ..ready(&owner, &claude)
            },
            entry(BINDING, Some(owner.clone()), Some(claude.clone())),
        ),
    ];
    for (name, record, entry) in cases {
        let scratch = Scratch::new();
        // `other binding` is stored under this binding's name on purpose.
        fs::write(
            record_path(&scratch.0, BINDING),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let server = endpoint(&scratch.0, Some("{\"written\":true}\n"));
        assert_eq!(
            fault(deliver(&scratch, &entry, MESSAGE)),
            ("denied", ChannelFault::Mismatch),
            "{name}"
        );
        // Had the driver connected, the endpoint would already have a frame.
        // This connection is the test's own and lets the endpoint thread end.
        let mut probe = UnixStream::connect(socket_path(&scratch.0, BINDING)).unwrap();
        probe.write_all(b"\n").unwrap();
        assert_eq!(server.join().unwrap(), b"\n", "{name}: no frame was sent");
    }
}

#[test]
fn an_unreadable_record_denies_instead_of_treating_the_session_as_unenrolled() {
    let scratch = Scratch::new();
    let entry = entry(BINDING, Some(live_owner()), Some(provider(4242)));
    for bytes in [&b"{not json"[..], &[b' '; 5000][..]] {
        fs::write(record_path(&scratch.0, BINDING), bytes).unwrap();
        assert_eq!(
            fault(deliver(&scratch, &entry, MESSAGE)),
            ("denied", ChannelFault::InvalidRecord)
        );
    }
    // A record path that is not a regular file is not evidence either.
    fs::remove_file(record_path(&scratch.0, BINDING)).unwrap();
    fs::create_dir(record_path(&scratch.0, BINDING)).unwrap();
    assert_eq!(
        fault(deliver(&scratch, &entry, MESSAGE)),
        ("denied", ChannelFault::InvalidRecord)
    );
}

#[test]
fn a_record_in_a_directory_others_can_write_is_no_evidence() {
    let scratch = Scratch::new();
    let (owner, claude) = (live_owner(), provider(4242));
    publish(&scratch.0, &ready(&owner, &claude));
    fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o770)).unwrap();
    assert_eq!(
        fault(deliver(
            &scratch,
            &entry(BINDING, Some(owner), Some(claude)),
            MESSAGE
        )),
        ("denied", ChannelFault::InvalidRecord)
    );
}

#[test]
fn an_opted_in_session_whose_ready_channel_is_unreachable_is_never_pasted_to() {
    let scratch = Scratch::new();
    let (owner, claude) = (live_owner(), provider(4242));
    publish(&scratch.0, &ready(&owner, &claude));
    let entry = entry(BINDING, Some(owner), Some(claude));
    // The record is valid but its server is gone: no byte moved, yet the
    // session opted in, so the outcome is terminal rather than `NotSent`.
    assert_eq!(
        fault(deliver(&scratch, &entry, MESSAGE)),
        ("denied", ChannelFault::Unreachable)
    );
    // A stale socket file nobody listens on is refused the same way.
    drop(UnixListener::bind(socket_path(&scratch.0, BINDING)).unwrap());
    assert_eq!(
        fault(deliver(&scratch, &entry, MESSAGE)),
        ("denied", ChannelFault::Unreachable)
    );
}

// The startup seam: an opted-in session whose channel is not ready yet.

#[test]
fn a_send_racing_the_handshake_waits_and_uses_the_channel_never_paste() {
    let scratch = Scratch::new();
    let (owner, claude) = (live_owner(), provider(4242));
    publish(&scratch.0, &intent(&owner));
    let entry = entry(BINDING, Some(owner.clone()), Some(claude.clone()));
    let server = endpoint(&scratch.0, Some("{\"written\":true}\n"));
    let directory = scratch.0.clone();
    let handshake = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        publish(&directory, &ready(&owner, &claude));
    });
    let result = send_within(Some(&scratch.0), &entry, MESSAGE, Duration::from_secs(10));
    handshake.join().unwrap();
    assert_eq!(
        result,
        ActionResult::Completed(DeliveryAcceptance::Unacknowledged)
    );
    let frame: Frame = serde_json::from_slice(&server.join().unwrap()).unwrap();
    assert_eq!(frame.content, MESSAGE);
}

#[test]
fn a_channel_that_never_completes_its_handshake_is_not_ready_and_never_pasted() {
    let scratch = Scratch::new();
    let (owner, claude) = (live_owner(), provider(4242));
    publish(&scratch.0, &intent(&owner));
    // An endpoint that would record any byte proves none was sent.
    let server = endpoint(&scratch.0, Some("{\"written\":true}\n"));
    let started = Instant::now();
    assert_eq!(
        fault(deliver(
            &scratch,
            &entry(BINDING, Some(owner), Some(claude)),
            MESSAGE
        )),
        ("denied", ChannelFault::NotReady)
    );
    assert!(
        started.elapsed() >= SHORT_WAIT && started.elapsed() < SHORT_WAIT * 20,
        "the wait is bounded"
    );
    let mut probe = UnixStream::connect(socket_path(&scratch.0, BINDING)).unwrap();
    probe.write_all(b"\n").unwrap();
    assert_eq!(server.join().unwrap(), b"\n", "no frame was sent");
}

#[test]
fn an_enrollment_replaced_or_removed_while_waiting_is_a_mismatch_not_a_fallback() {
    let (owner, claude) = (live_owner(), provider(4242));
    for replacement in [
        Some(Record {
            generation: "55555555-5555-4555-8555-555555555555".into(),
            ..intent(&owner)
        }),
        None,
    ] {
        let scratch = Scratch::new();
        publish(&scratch.0, &intent(&owner));
        let entry = entry(BINDING, Some(owner.clone()), Some(claude.clone()));
        let directory = scratch.0.clone();
        let change = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            match replacement {
                Some(record) => publish(&directory, &record),
                None => fs::remove_file(record_path(&directory, BINDING)).unwrap(),
            }
        });
        let result = send_within(Some(&scratch.0), &entry, MESSAGE, Duration::from_secs(10));
        change.join().unwrap();
        assert_eq!(fault(result), ("denied", ChannelFault::Mismatch));
    }
}

#[test]
fn an_enrollment_whose_identity_changes_under_the_same_generation_is_denied_without_a_frame() {
    let (owner, claude) = (live_owner(), provider(4242));
    let other_owner = process(&provider(1));
    let changes: Vec<(&str, Record)> = vec![
        (
            "launch owner",
            Record {
                launch_owner: other_owner,
                claude: Some(process(&claude)),
                ..intent(&owner)
            },
        ),
        (
            "version",
            Record {
                version: 2,
                claude: Some(process(&claude)),
                ..intent(&owner)
            },
        ),
        (
            "binding",
            Record {
                binding_id: "44444444-4444-4444-8444-444444444444".into(),
                claude: Some(process(&claude)),
                ..intent(&owner)
            },
        ),
    ];
    for (name, changed) in changes {
        let scratch = Scratch::new();
        publish(&scratch.0, &intent(&owner));
        // The endpoint would record any frame; the same generation stays, and
        // the Claude process matches the binding, so only the identity differs.
        let server = endpoint(&scratch.0, Some("{\"written\":true}\n"));
        let entry = entry(BINDING, Some(owner.clone()), Some(claude.clone()));
        let directory = scratch.0.clone();
        let rewrite = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            // Published atomically at this binding's path on purpose (also for the
            // other-binding case), so the polling send never sees a partial file.
            crate::private_file::replace(
                &record_path(&directory, BINDING),
                &serde_json::to_vec(&changed).unwrap(),
            )
            .unwrap();
        });
        let result = send_within(Some(&scratch.0), &entry, MESSAGE, Duration::from_secs(10));
        rewrite.join().unwrap();
        assert_eq!(fault(result), ("denied", ChannelFault::Mismatch), "{name}");
        let mut probe = UnixStream::connect(socket_path(&scratch.0, BINDING)).unwrap();
        probe.write_all(b"\n").unwrap();
        assert_eq!(server.join().unwrap(), b"\n", "{name}: no frame was sent");
    }
}

#[test]
fn a_refusal_by_the_endpoint_denies_and_a_missing_receipt_is_uncertain() {
    let (owner, claude) = (live_owner(), provider(4242));
    for (answer, expected) in [
        (
            Some("{\"refused\":\"generation\"}\n"),
            ("denied", ChannelFault::Refused),
        ),
        (None, ("uncertain", ChannelFault::Uncertain)),
        (Some("not json\n"), ("uncertain", ChannelFault::Uncertain)),
        (
            Some("{\"written\":false}\n"),
            ("uncertain", ChannelFault::Uncertain),
        ),
        // Ambiguous or empty answers never count as a write.
        (
            Some("{\"written\":true,\"refused\":\"generation\"}\n"),
            ("uncertain", ChannelFault::Uncertain),
        ),
        (Some("{}\n"), ("uncertain", ChannelFault::Uncertain)),
        (Some("\n"), ("uncertain", ChannelFault::Uncertain)),
    ] {
        let scratch = Scratch::new();
        publish(&scratch.0, &ready(&owner, &claude));
        let server = endpoint(&scratch.0, answer);
        assert_eq!(
            fault(deliver(
                &scratch,
                &entry(BINDING, Some(owner.clone()), Some(claude.clone())),
                MESSAGE
            )),
            expected,
            "{answer:?}"
        );
        server.join().unwrap();
    }
}

#[test]
fn an_oversized_payload_denies_before_connecting() {
    let scratch = Scratch::new();
    let (owner, claude) = (live_owner(), provider(4242));
    publish(&scratch.0, &ready(&owner, &claude));
    // No endpoint exists: connecting would be `Denied(Unreachable)`, so `TooLarge`
    // proves the size check came before any connection.
    let message = "x".repeat(CONTENT_LIMIT + 1);
    assert_eq!(
        fault(deliver(
            &scratch,
            &entry(BINDING, Some(owner), Some(claude)),
            &message
        )),
        ("denied", ChannelFault::TooLarge)
    );
}

// An enrollment applies only to the exact launch that created it.

#[test]
fn a_different_launch_proven_current_leaves_the_old_enrollment_untouched_and_pastes() {
    let (current, claude) = (live_owner(), provider(4242));
    // A plain relaunch of the binding: the old launch ended, or is still around.
    for old in [dead_owner(), provider(1)] {
        let scratch = Scratch::new();
        publish(&scratch.0, &ready(&old, &provider(4141)));
        let before = fs::read(record_path(&scratch.0, BINDING)).unwrap();
        // An endpoint that would record any byte proves none was sent.
        let server = endpoint(&scratch.0, Some("{\"written\":true}\n"));
        assert_eq!(
            deliver(
                &scratch,
                &entry(BINDING, Some(current.clone()), Some(claude.clone())),
                MESSAGE
            ),
            ActionResult::Unsupported
        );
        assert_eq!(fs::read(record_path(&scratch.0, BINDING)).unwrap(), before);
        let mut probe = UnixStream::connect(socket_path(&scratch.0, BINDING)).unwrap();
        probe.write_all(b"\n").unwrap();
        assert_eq!(server.join().unwrap(), b"\n", "no frame was sent");
    }
}

#[test]
fn an_ended_owner_alone_never_proves_that_a_different_launch_is_current() {
    let (ended, claude) = (dead_owner(), provider(4242));
    let record = ready(&ended, &claude);
    let other_ended = ProcessIncarnation::new(ended.pid(), "Thu Oct  1 11:00:00 2026").unwrap();
    for (name, owner) in [
        ("binding recorded no launch owner", None),
        (
            "binding's launch owner is another ended launch",
            Some(other_ended),
        ),
        (
            "binding's launch owner is not a live match",
            Some(provider(1)),
        ),
    ] {
        let scratch = Scratch::new();
        publish(&scratch.0, &record);
        assert_eq!(
            fault(deliver(
                &scratch,
                &entry(BINDING, owner, Some(claude.clone())),
                MESSAGE
            )),
            ("denied", ChannelFault::Mismatch),
            "{name}"
        );
    }
}

// Without the configuration there is no way to tell whether a session opted in.

#[test]
fn undiscoverable_configuration_is_terminal_and_never_not_sent() {
    let (owner, claude) = (live_owner(), provider(4242));
    let enrolled = entry(BINDING, Some(owner), Some(claude));
    assert_eq!(
        fault(send_within(None, &enrolled, MESSAGE, SHORT_WAIT)),
        ("denied", ChannelFault::Unverifiable)
    );
    // No binding means nothing could have opted in, whatever the configuration.
    let unbound = BindingEntry {
        binding: None,
        ..entry(BINDING, None, None)
    };
    assert_eq!(
        send_within(None, &unbound, MESSAGE, SHORT_WAIT),
        ActionResult::Unsupported
    );
}

// One deadline bounds the whole exchange, whatever the endpoint does.

/// An endpoint that takes the frame and then stalls or trickles for far longer
/// than the exchange bound. It ends when the sender closes its side.
fn slow_endpoint(directory: &Path, trickle: bool) -> JoinHandle<()> {
    let listener = UnixListener::bind(socket_path(directory, BINDING)).unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = Vec::new();
        io::BufReader::new(&stream)
            .read_until(b'\n', &mut line)
            .unwrap();
        if trickle {
            // Whitespace forever: each byte renews a per-read timeout, and no
            // newline ever ends the line.
            while stream.write_all(b" ").is_ok() {
                std::thread::sleep(Duration::from_millis(20));
            }
        } else {
            let _ = io::copy(&mut stream, &mut io::sink());
        }
    })
}

#[test]
fn a_trickled_or_silent_reply_is_bounded_and_uncertain_never_a_fallback() {
    let limit = Duration::from_millis(300);
    for trickle in [true, false] {
        let scratch = Scratch::new();
        let server = slow_endpoint(&scratch.0, trickle);
        let mut stream = UnixStream::connect(socket_path(&scratch.0, BINDING)).unwrap();
        let started = Instant::now();
        let result = exchange_within(&mut stream, GENERATION, MESSAGE, limit);
        let elapsed = started.elapsed();
        assert_eq!(
            fault(result),
            ("uncertain", ChannelFault::Uncertain),
            "trickle: {trickle}"
        );
        assert!(
            elapsed >= limit && elapsed < limit * 6,
            "trickle {trickle}: the exchange ended after {elapsed:?}"
        );
        drop(stream);
        server.join().unwrap();
    }
}

#[test]
fn an_endpoint_that_never_reads_cannot_hold_the_write_past_the_deadline() {
    let limit = Duration::from_millis(300);
    let scratch = Scratch::new();
    let listener = UnixListener::bind(socket_path(&scratch.0, BINDING)).unwrap();
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        // Keep the connection open without reading until the test is done.
        let _ = hold.recv();
    });
    let mut stream = UnixStream::connect(socket_path(&scratch.0, BINDING)).unwrap();
    // Far more than a socket buffer holds, so the write cannot complete unread.
    let message = "x".repeat(8 * 1024 * 1024);
    let started = Instant::now();
    let result = exchange_within(&mut stream, GENERATION, &message, limit);
    let elapsed = started.elapsed();
    assert_eq!(fault(result), ("uncertain", ChannelFault::Uncertain));
    assert!(
        elapsed >= limit && elapsed < limit * 6,
        "the exchange ended after {elapsed:?}"
    );
    release.send(()).unwrap();
    server.join().unwrap();
}
