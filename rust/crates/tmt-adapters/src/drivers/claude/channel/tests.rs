use super::*;
use crate::process::{CommandError, CommandOutput, runtime::ProcessObservation};
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
const IDENTITY: &str = "33333333-3333-4333-8333-333333333333";
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
            pane_incarnation: None,
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

fn server() -> ServerEvidence {
    ServerEvidence {
        host: HostKind::Tmux,
        server_id: "server".into(),
        socket_path: "/tmp/tmux-test".into(),
        server_pid: 1,
        server_start_time: "start".into(),
    }
}

fn address(server: &ServerEvidence) -> PaneAddress<'_> {
    PaneAddress {
        server,
        pane_id: "%1",
        pane_pid: 2,
    }
}

fn pane_record() -> PaneRecord {
    PaneRecord::of(&address(&server())).unwrap()
}

/// The enrollment `tmt run --channel` writes before Claude starts.
fn intent(owner: &ProcessIncarnation) -> Record {
    Record {
        version: RECORD_VERSION,
        binding_id: BINDING.into(),
        identity_id: Some(IDENTITY.into()),
        generation: GENERATION.into(),
        launch_owner: Process::of(owner),
        pane: Some(pane_record()),
        foreground: None,
        claude: None,
    }
}

fn ready(owner: &ProcessIncarnation, claude: &ProcessIncarnation) -> Record {
    Record {
        claude: Some(Process::of(claude)),
        ..intent(owner)
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
    let scratch = Scratch::new();
    publish(&scratch.0, &ready(&ended, &recorded));
    let before = fs::read(record_path(&scratch.0, BINDING)).unwrap();
    assert_eq!(
        deliver(
            &scratch,
            &entry(BINDING, Some(ended.clone()), Some(live_owner())),
            MESSAGE
        ),
        ActionResult::Unsupported
    );
    assert_eq!(
        fs::read(record_path(&scratch.0, BINDING)).unwrap(),
        before,
        "the record is untouched"
    );
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
        assert_stale(&scratch, &ended, running, name);
    }
    // A record that never named a valid Claude cannot prove that a live runtime
    // is a different one: the launch owner may have died before the handshake
    // while its original child still runs and is still the observed runtime.
    for (name, record) in [
        ("pending record", intent(&ended)),
        (
            "unparseable recorded Claude",
            Record {
                claude: Some(Process {
                    pid: 0,
                    start: String::new(),
                }),
                ..intent(&ended)
            },
        ),
    ] {
        let scratch = Scratch::new();
        publish(&scratch.0, &record);
        assert_stale(&scratch, &ended, Some(live_owner()), name);
    }
    // Stale is actionable: it says how to recover.
    assert!(
        ChannelFault::Stale
            .reason()
            .contains("Relaunch the agent with `tmt run`")
    );
}

fn assert_stale(
    scratch: &Scratch,
    ended: &ProcessIncarnation,
    running: Option<ProcessIncarnation>,
    name: &str,
) {
    assert_eq!(
        fault(deliver(
            scratch,
            &entry(BINDING, Some(ended.clone()), running),
            MESSAGE
        )),
        ("denied", ChannelFault::Stale),
        "{name}"
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
    let other_owner = Process::of(&provider(1));
    let changes: Vec<(&str, Record)> = vec![
        (
            "launch owner",
            Record {
                launch_owner: other_owner,
                claude: Some(Process::of(&claude)),
                ..intent(&owner)
            },
        ),
        (
            "version",
            Record {
                version: 2,
                claude: Some(Process::of(&claude)),
                ..intent(&owner)
            },
        ),
        (
            "binding",
            Record {
                binding_id: "44444444-4444-4444-8444-444444444444".into(),
                claude: Some(Process::of(&claude)),
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

// Enrollment, the lease and the launch contract.

fn user_command() -> RuntimeCommand {
    RuntimeCommand {
        executable: "/opt/claude/bin/claude".into(),
        args: vec!["--model".into(), "sonnet".into()],
    }
}

fn plan<'a>(
    directory: &'a Path,
    owner: &'a ProcessIncarnation,
    command: &'a RuntimeCommand,
    server: &'a ServerEvidence,
) -> ChannelPlan<'a> {
    ChannelPlan {
        binding_id: BINDING,
        identity_id: IDENTITY,
        pane: address(server),
        owner,
        command,
        working_directory: Path::new("/work"),
        tmt: Path::new("/opt/tmt/bin/tmt"),
        directory,
    }
}

/// A lease for tests that move it across threads: the one `enroll` wrote, rebuilt
/// from the record it left.
fn lease(scratch: &Scratch, owner: &ProcessIncarnation) -> Lease {
    let command = user_command();
    let enrolled = ClaudeChannel
        .enroll(&plan(&scratch.0, owner, &command, &server()))
        .unwrap();
    Lease {
        command: enrolled.command().clone(),
        directory: scratch.0.clone(),
        binding_id: BINDING.into(),
        generation: read_record(&scratch.0, BINDING)
            .unwrap()
            .unwrap()
            .generation,
        owner: Process::of(owner),
    }
}

fn withdraw(lease: Lease) {
    Box::new(lease).withdraw();
}

#[test]
fn enrollment_is_durable_before_launch_and_plans_the_command() {
    let scratch = Scratch::new();
    let (owner, user) = (live_owner(), user_command());
    let enrolled = ClaudeChannel
        .enroll(&plan(&scratch.0, &owner, &user, &server()))
        .unwrap();
    let command = enrolled.command();
    // The user's command is untouched; the provider's flags follow it.
    assert_eq!(command.executable, user.executable);
    assert_eq!(command.args[..2], user.args[..]);
    assert_eq!(command.args.len(), 6);
    assert_eq!(command.args[2], MCP_CONFIG_FLAG);
    assert_eq!(command.args[4], CHANNEL_FLAG);
    assert_eq!(command.args[5], format!("server:{SERVER_NAME}").as_str());
    assert!(
        !command
            .args
            .iter()
            .any(|argument| argument == "--strict-mcp-config"),
        "user MCP servers stay available"
    );
    assert!(enrolled.environment().is_empty());
    let config: serde_json::Value =
        serde_json::from_str(command.args[3].to_str().unwrap()).unwrap();
    let servers = config["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 1);
    let declared = &servers[SERVER_NAME];
    assert_eq!(declared["command"], "/opt/tmt/bin/tmt");
    let args: Vec<_> = declared["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert_eq!(args[..3], ["__channel-server", "claude", BINDING]);
    assert_eq!(args[4], scratch.0.to_str().unwrap());
    // The record already exists ("opted in, not ready"), and names the same
    // generation the server will be started with, and this launch's owner.
    let stored = read_record(&scratch.0, BINDING).unwrap().unwrap();
    assert_eq!(stored.generation, args[3]);
    assert_eq!(stored.launch_owner, Process::of(&owner));
    assert_eq!(stored.claude, None);
    assert!(uuid::Uuid::parse_str(args[3]).is_ok());
    // A relaunch is a new generation, replacing the earlier enrollment.
    let again = ClaudeChannel
        .enroll(&plan(&scratch.0, &owner, &user, &server()))
        .unwrap();
    assert_ne!(command.args, again.command().args);
    assert_ne!(
        read_record(&scratch.0, BINDING)
            .unwrap()
            .unwrap()
            .generation,
        args[3]
    );
    // And an enrollment is only ever written to an owner-only directory.
    fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o770)).unwrap();
    assert_eq!(
        ClaudeChannel
            .enroll(&plan(&scratch.0, &owner, &user, &server()))
            .err(),
        Some(ChannelError::Enrollment)
    );
}

#[test]
fn a_command_line_that_already_names_a_channel_is_rejected_before_any_side_effect() {
    let scratch = Scratch::new();
    let owner = live_owner();
    let command = RuntimeCommand {
        executable: "claude".into(),
        args: vec![CHANNEL_FLAG.into(), "server:other".into()],
    };
    let error = ClaudeChannel
        .enroll(&plan(&scratch.0, &owner, &command, &server()))
        .err();
    assert!(matches!(error, Some(ChannelError::UnsupportedArguments(_))));
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
}

#[test]
fn withdrawing_removes_this_launchs_enrollment_and_its_socket_and_then_a_send_sees_none() {
    let scratch = Scratch::new();
    let owner = live_owner();
    let lease = lease(&scratch, &owner);
    drop(UnixListener::bind(socket_path(&scratch.0, BINDING)).unwrap());
    withdraw(lease);
    assert!(!record_path(&scratch.0, BINDING).exists());
    assert!(!socket_path(&scratch.0, BINDING).exists());
    // Afterwards a send sees a session that never opted in.
    assert_eq!(
        deliver(
            &scratch,
            &entry(BINDING, Some(owner), Some(provider(4242))),
            MESSAGE
        ),
        ActionResult::Unsupported
    );
}

// Enrollment ownership: a launch only ever removes or replaces what is its own,
// and every mutation is serialized by the directory lock.

#[test]
fn an_old_withdrawal_after_a_newer_enrollment_keeps_the_new_record_and_socket() {
    let scratch = Scratch::new();
    let owner = live_owner();
    let old = lease(&scratch, &owner);
    // The same launch enrolling again replaces its earlier enrollment.
    let new = lease(&scratch, &owner);
    assert_ne!(old.generation, new.generation);
    let _socket = UnixListener::bind(socket_path(&scratch.0, BINDING)).unwrap();
    let record = fs::read(record_path(&scratch.0, BINDING)).unwrap();
    withdraw(old);
    assert_eq!(fs::read(record_path(&scratch.0, BINDING)).unwrap(), record);
    assert!(socket_path(&scratch.0, BINDING).exists());
    withdraw(new);
    assert!(!record_path(&scratch.0, BINDING).exists());
    assert!(!socket_path(&scratch.0, BINDING).exists());
}

#[test]
fn a_withdrawal_never_removes_an_enrollment_of_another_owner_even_with_the_same_generation() {
    let scratch = Scratch::new();
    let owner = live_owner();
    let mine = lease(&scratch, &owner);
    // Same generation, but another launch owner: not this lease's record.
    let other = Record {
        launch_owner: Process::of(&provider(1)),
        generation: mine.generation.clone(),
        ..intent(&owner)
    };
    publish(&scratch.0, &other);
    let record = fs::read(record_path(&scratch.0, BINDING)).unwrap();
    withdraw(mine);
    assert_eq!(fs::read(record_path(&scratch.0, BINDING)).unwrap(), record);
}

#[test]
fn a_withdrawal_waits_for_the_lock_and_then_finds_the_record_replaced() {
    let scratch = Scratch::new();
    let owner = live_owner();
    let old = lease(&scratch, &owner);
    let replacement = Record {
        generation: "66666666-6666-4666-8666-666666666666".into(),
        ..intent(&owner)
    };
    let blocked = std::thread::scope(|scope| {
        locked(&scratch.0, || {
            let withdrawal = scope.spawn(move || withdraw(old));
            std::thread::sleep(Duration::from_millis(300));
            // An unserialized withdrawal would already have finished by now.
            let blocked = !withdrawal.is_finished();
            // The record is replaced while the old withdrawal waits.
            publish(&scratch.0, &replacement);
            blocked
        })
        .unwrap()
    });
    assert!(blocked, "the withdrawal waits for the directory lock");
    assert_eq!(
        read_record(&scratch.0, BINDING).unwrap().unwrap(),
        replacement,
        "the replacement survives the old withdrawal"
    );
}

#[test]
fn a_lock_that_cannot_be_taken_fails_closed() {
    let scratch = Scratch::new();
    let ran = std::sync::atomic::AtomicBool::new(false);
    let failed = locked(&scratch.0, || {
        locked_within(&scratch.0, Duration::from_millis(100), || {
            ran.store(true, Ordering::SeqCst);
        })
        .is_err()
    })
    .unwrap();
    assert!(failed);
    assert!(!ran.load(Ordering::SeqCst), "the action never ran");
    // Once released, the lock is available again.
    assert!(locked(&scratch.0, || ()).is_ok());
}

/// A live process other than this one, to stand for another launch that still runs.
fn another_live_process() -> (std::process::Child, ProcessIncarnation) {
    let child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let observed = observe_runtime_process(
        &UnixCommandRunner,
        u64::from(child.id()),
        Instant::now() + Duration::from_secs(3),
    );
    match observed {
        Ok(ProcessObservation::Live(incarnation)) => (child, incarnation),
        other => panic!("the child must be observable: {other:?}"),
    }
}

#[test]
fn enrolling_replaces_only_an_enrollment_that_is_over_and_otherwise_leaves_it_untouched() {
    let (owner, user) = (live_owner(), user_command());
    let (mut other, other_live) = another_live_process();
    // Unreadable, or owned by a launch that is alive and different: untouched.
    for (name, bytes) in [
        ("unreadable record", b"{not json".to_vec()),
        (
            "alive and different owner",
            serde_json::to_vec(&intent(&other_live)).unwrap(),
        ),
    ] {
        let scratch = Scratch::new();
        ensure_private_directory(&scratch.0).unwrap();
        fs::write(record_path(&scratch.0, BINDING), &bytes).unwrap();
        assert_eq!(
            ClaudeChannel
                .enroll(&plan(&scratch.0, &owner, &user, &server()))
                .err(),
            Some(ChannelError::Occupied),
            "{name}"
        );
        assert_eq!(
            fs::read(record_path(&scratch.0, BINDING)).unwrap(),
            bytes,
            "{name}: untouched"
        );
    }
    other.kill().unwrap();
    other.wait().unwrap();
    // A conclusively gone owner, or this very launch, is replaced.
    for (name, old_owner) in [("gone owner", dead_owner()), ("same launch", owner.clone())] {
        let scratch = Scratch::new();
        publish(&scratch.0, &intent(&old_owner));
        let before = read_record(&scratch.0, BINDING).unwrap().unwrap();
        ClaudeChannel
            .enroll(&plan(&scratch.0, &owner, &user, &server()))
            .unwrap();
        let after = read_record(&scratch.0, BINDING).unwrap().unwrap();
        assert_ne!(after.generation, before.generation, "{name}");
        assert_eq!(after.launch_owner, Process::of(&owner), "{name}");
    }
}

#[test]
fn the_provider_contract_constants_are_pinned() {
    // Changing any of these needs new provider evidence and a contract update.
    assert_eq!(MINIMUM_VERSION, "2.1.285");
    assert_eq!(TESTED_VERSIONS, ["2.1.285"]);
    assert_eq!(CAPABILITY, "claude/channel");
    assert_eq!(NOTIFICATION_METHOD, "notifications/claude/channel");
    assert_eq!(PROTOCOL_VERSION, "2025-11-25");
    assert_eq!(MCP_CONFIG_FLAG, "--mcp-config");
    assert_eq!(CHANNEL_FLAG, "--dangerously-load-development-channels");
}

/// Reads the fixture through its shell instead of exec'ing an inode that another
/// fork may still hold writable (ETXTBSY), so the product check stays single-shot.
struct ScriptRunner;

impl CommandRunner for ScriptRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        let mut args = vec![request.program.to_owned()];
        args.extend_from_slice(request.args);
        UnixCommandRunner.execute(CommandRequest {
            program: OsStr::new("/bin/sh"),
            args: &args,
            ..request
        })
    }
}

fn probe(scratch: &Scratch, version: &str) -> Result<Option<String>, ChannelError> {
    let script = scratch.0.join("claude");
    fs::write(&script, format!("echo '{version}'\n")).unwrap();
    check_provider(
        &ScriptRunner,
        script.as_os_str(),
        &scratch.0,
        Instant::now() + Duration::from_secs(5),
    )
}

#[test]
fn the_range_rule_accepts_the_minimum_and_newer_builds_of_the_same_major_only() {
    use BuildStatus::{Tested, Untested};
    let line = |number: &str| format!("{number} (Claude Code)");
    // The minimum is the one tested build; newer builds of the same major line are
    // accepted untested, however far the minor or patch moved.
    for (number, expected) in [
        ("2.1.285", Tested),
        ("2.1.286", Untested),
        ("2.1.300", Untested),
        ("2.2.0", Untested),
        ("2.10.0", Untested),
        ("2.999.999", Untested),
    ] {
        assert_eq!(build_status(&line(number)), Some(expected), "{number}");
    }
    // Below the minimum, another major line, and anything that is not a canonical
    // `<major>.<minor>.<patch> (Claude Code)` line are all outside the range.
    for number in [
        "2.1.284", "2.1.0", "2.0.999", "1.9.999", "0.0.0", "3.0.0", "3.1.285", "10.1.285",
    ] {
        assert_eq!(build_status(&line(number)), None, "{number}");
    }
    for text in [
        "2.1.286",
        "2.1.286 (Claude Code) extra",
        "2.1.286 (Claude Code)\nextra",
        "2.1.286 (Other Tool)",
        "v2.1.286 (Claude Code)",
        "2.1.286-beta (Claude Code)",
        "2.1.286.1 (Claude Code)",
        "2.1 (Claude Code)",
        "02.1.286 (Claude Code)",
        "2.01.286 (Claude Code)",
        "2.1.0286 (Claude Code)",
        "2.1.+286 (Claude Code)",
        "2.1.99999999999 (Claude Code)",
        " 2.1.286 (Claude Code)",
        "not a version",
        "",
    ] {
        assert_eq!(build_status(text), None, "{text:?}");
    }
}

#[test]
fn preflight_accepts_the_range_and_advises_only_about_untested_builds() {
    let scratch = Scratch::new();
    assert_eq!(probe(&scratch, "2.1.285 (Claude Code)"), Ok(None));
    for version in ["2.1.286", "2.2.0", "2.9.9"] {
        let advisory = probe(&scratch, &format!("{version} (Claude Code)"))
            .unwrap()
            .expect("an untested build gets an advisory");
        // It names the build and the tested set, and says what happens if the
        // handshake fails: not ready and never pasted.
        assert!(
            advisory.contains(&format!("Claude Code {version} ")),
            "{advisory}"
        );
        assert!(advisory.contains("tested: 2.1.285"), "{advisory}");
        assert!(advisory.contains("nothing is pasted"), "{advisory}");
    }
    for drift in [
        "2.1.284 (Claude Code)",
        "1.0.0 (Claude Code)",
        "3.0.0 (Claude Code)",
        "2.1.286",
        "not a version",
        "",
    ] {
        assert_eq!(
            probe(&scratch, drift),
            Err(ChannelError::ProviderVersion {
                found: drift.into()
            }),
            "{drift:?}"
        );
    }
    let missing = ClaudeChannel.preflight(
        scratch.0.join("absent").as_os_str(),
        &scratch.0,
        Instant::now() + Duration::from_secs(5),
    );
    assert_eq!(missing, Err(ChannelError::ProviderUnavailable));
}

#[test]
fn preflight_refuses_a_directory_whose_socket_path_cannot_fit() {
    let deadline = Instant::now() + Duration::from_secs(1);
    let long = PathBuf::from(format!("/tmp/{}", "d".repeat(80)));
    assert_eq!(
        ClaudeChannel.preflight("claude".as_ref(), &long, deadline),
        Err(ChannelError::PathTooLong)
    );
    assert_eq!(
        ClaudeChannel.preflight("claude".as_ref(), Path::new("relative"), deadline),
        Err(ChannelError::PathTooLong)
    );
}

// The real server, driven through its stdio ends and its socket.

struct Running {
    scratch: Scratch,
    stdin: UnixStream,
    stdout: io::BufReader<UnixStream>,
    thread: JoinHandle<io::Result<()>>,
}

fn wait_for(path: &Path) {
    for _ in 0..500 {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{} never appeared", path.display());
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{what} never happened");
}

fn serve_on(directory: &Path, input: UnixStream, output: UnixStream) -> io::Result<()> {
    let mut output = output;
    ClaudeChannel.serve(
        &ServeRequest {
            binding_id: BINDING,
            generation: GENERATION,
            directory,
        },
        Box::new(io::BufReader::new(input)),
        &mut output,
    )
}

/// The launch owner is this process; `tmt run --channel` has already written
/// the enrollment when Claude starts the server.
fn start() -> Running {
    let scratch = Scratch::new();
    publish(&scratch.0, &intent(&live_owner()));
    let (stdin, server_in) = UnixStream::pair().unwrap();
    let (server_out, stdout) = UnixStream::pair().unwrap();
    stdout
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let directory = scratch.0.clone();
    let thread = std::thread::spawn(move || serve_on(&directory, server_in, server_out));
    wait_for(&socket_path(&scratch.0, BINDING));
    Running {
        scratch,
        stdin,
        stdout: io::BufReader::new(stdout),
        thread,
    }
}

impl Running {
    fn say(&mut self, message: serde_json::Value) {
        writeln!(self.stdin, "{message}").unwrap();
    }

    fn hear(&mut self) -> serde_json::Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn frame(&self, generation: &str, content: &str) -> Reply {
        let mut stream = UnixStream::connect(socket_path(&self.scratch.0, BINDING)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let frame = Frame {
            version: RECORD_VERSION,
            generation: generation.into(),
            content: content.into(),
        };
        writeln!(stream, "{}", serde_json::to_string(&frame).unwrap()).unwrap();
        let mut line = String::new();
        io::BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn stop(self) -> (Scratch, io::Result<()>) {
        drop(self.stdin);
        (self.scratch, self.thread.join().unwrap())
    }
}

fn initialize() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                   "clientInfo": {"name": "claude-code", "version": "2.1.285"}}
    })
}

#[test]
fn the_server_declares_the_channel_and_no_tools_then_delivers_one_frame_at_a_time() {
    let mut running = start();
    let owner = live_owner();
    running.say(initialize());
    let initialized = running.hear();
    let result = &initialized["result"];
    assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(
        result["capabilities"],
        serde_json::json!({"experimental": {CAPABILITY: {}}}),
        "the channel capability and nothing else, so no tools"
    );
    assert_eq!(result["serverInfo"]["name"], SERVER_NAME);
    // Before the client's `initialized` the enrollment is still "not ready":
    // a send reports that, and a frame is refused, not written.
    assert_eq!(
        read_record(&running.scratch.0, BINDING)
            .unwrap()
            .unwrap()
            .claude,
        None
    );
    assert_eq!(
        running.frame(GENERATION, MESSAGE).refused.as_deref(),
        Some("not_ready")
    );
    running.say(serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    wait_until("the enrollment becoming ready", || {
        read_record(&running.scratch.0, BINDING)
            .unwrap()
            .is_some_and(|record| record.claude.is_some())
    });

    // The record names this test process's parent as "Claude", the same
    // observation the server made, and keeps the launch owner untouched.
    let stored = read_record(&running.scratch.0, BINDING).unwrap().unwrap();
    let claude_process = stored.claude.clone().unwrap();
    assert_eq!(
        claude_process.pid,
        u64::try_from(nix::unistd::getppid().as_raw()).unwrap()
    );
    assert_eq!(stored.generation, GENERATION);
    assert_eq!(stored.launch_owner, Process::of(&owner));
    let claude = claude_process.incarnation().unwrap();
    assert_eq!(
        fs::metadata(socket_path(&running.scratch.0, BINDING))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    // A wrong generation is refused and produces no notification.
    assert_eq!(
        running
            .frame("55555555-5555-4555-8555-555555555555", "stale")
            .refused
            .as_deref(),
        Some("generation")
    );
    assert_eq!(
        send(
            Some(&running.scratch.0),
            &entry(BINDING, Some(owner), Some(claude)),
            MESSAGE
        ),
        ActionResult::Completed(DeliveryAcceptance::Unacknowledged)
    );
    // The first line after the handshake is the accepted frame, not "stale".
    let notification = running.hear();
    assert_eq!(notification["method"], NOTIFICATION_METHOD);
    assert_eq!(notification["params"]["content"], MESSAGE);
    assert!(notification.get("id").is_none(), "one-way notification");

    // A ping and an unknown request keep their JSON-RPC shapes.
    running.say(serde_json::json!({"jsonrpc": "2.0", "id": 9, "method": "ping"}));
    assert_eq!(running.hear()["result"], serde_json::json!({}));
    running.say(serde_json::json!({"jsonrpc": "2.0", "id": 10, "method": "tools/list"}));
    assert_eq!(running.hear()["error"]["code"], -32601);

    // Claude going away ends the server and its socket; the enrollment stays
    // with the launch, which withdraws it.
    let (scratch, result) = running.stop();
    result.unwrap();
    assert!(!socket_path(&scratch.0, BINDING).exists());
    assert!(record_path(&scratch.0, BINDING).exists());
}

#[test]
fn a_server_that_never_completes_its_handshake_leaves_the_enrollment_not_ready() {
    let mut running = start();
    running.say(initialize());
    running.hear();
    // Claude closes the session before `initialized`: a failed handshake.
    let (scratch, result) = running.stop();
    result.unwrap();
    let record = read_record(&scratch.0, BINDING).unwrap().unwrap();
    assert_eq!(record.claude, None, "never became ready");
    // A later send therefore reports not ready with no byte and no paste,
    // rather than treating the session as not opted in.
    let owner = live_owner();
    assert_eq!(
        fault(send_within(
            Some(&scratch.0),
            &entry(BINDING, Some(owner), Some(provider(4242))),
            MESSAGE,
            SHORT_WAIT
        )),
        ("denied", ChannelFault::NotReady)
    );
}

#[test]
fn a_server_cannot_create_an_enrollment_and_validates_its_arguments_first() {
    let scratch = Scratch::new();
    let serve = |binding: &str, generation: &str, directory: &Path| {
        ClaudeChannel.serve(
            &ServeRequest {
                binding_id: binding,
                generation,
                directory,
            },
            Box::new(io::BufReader::new(io::empty())),
            &mut Vec::new(),
        )
    };
    for error in [
        serve("../escape", GENERATION, &scratch.0),
        serve(BINDING, "not-a-uuid", &scratch.0),
        serve(BINDING, GENERATION, Path::new("relative")),
    ] {
        assert_eq!(error.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
    let created = |scratch: &Scratch| {
        fs::read_dir(&scratch.0)
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().file_name() != ".lock")
            .count()
    };
    // Started by hand, with no enrollment from `tmt run --channel`: refused,
    // and it must not have made one or bound a socket.
    assert_eq!(
        serve(BINDING, GENERATION, &scratch.0).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(created(&scratch), 0);
    // An enrollment of another generation is not this server's.
    publish(
        &scratch.0,
        &Record {
            generation: "55555555-5555-4555-8555-555555555555".into(),
            ..intent(&live_owner())
        },
    );
    assert_eq!(
        serve(BINDING, GENERATION, &scratch.0).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    // A directory others can write to cannot hold the endpoint.
    fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o770)).unwrap();
    assert_eq!(
        serve(BINDING, GENERATION, &scratch.0).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn a_second_server_for_a_live_binding_is_refused_and_a_stale_socket_is_replaced() {
    let serve = |directory: &Path| {
        ClaudeChannel.serve(
            &ServeRequest {
                binding_id: BINDING,
                generation: GENERATION,
                directory,
            },
            Box::new(io::BufReader::new(io::empty())),
            &mut Vec::new(),
        )
    };
    let running = start();
    let enrollment = read_record(&running.scratch.0, BINDING).unwrap().unwrap();
    let error = serve(&running.scratch.0).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    let (scratch, result) = running.stop();
    result.unwrap();
    let socket = socket_path(&scratch.0, BINDING);
    assert!(!socket.exists(), "the first server removed its socket");
    assert_eq!(
        read_record(&scratch.0, BINDING).unwrap(),
        Some(enrollment.clone())
    );

    // A crashed predecessor leaves a socket file for the same enrolled launch.
    drop(UnixListener::bind(&socket).unwrap());

    // Drop is not proof of staleness in a multithreaded process that forks.
    // Observe conclusive refusal before the one replacement attempt; never
    // retry serve or reinterpret an inconclusive production probe.
    wait_until(
        "the fixture socket becoming conclusively stale",
        || match UnixStream::connect(&socket) {
            Ok(_) => false,
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => true,
            Err(error) => panic!("unexpected fixture socket probe: {error}"),
        },
    );
    assert!(socket.exists(), "a crashed predecessor left a socket file");
    // EOF ends the replacement after it binds. Connection polling would also
    // succeed on an inherited predecessor listener and cannot prove readiness.
    serve(&scratch.0).unwrap();
    assert!(!socket.exists(), "the replacement removed its own socket");
    assert_eq!(read_record(&scratch.0, BINDING).unwrap(), Some(enrollment));
}

#[test]
fn a_server_removes_its_socket_only_while_generation_and_launch_owner_both_match() {
    let running = start();
    // Same generation, but the record now names another launch owner.
    let changed = Record {
        launch_owner: Process::of(&provider(1)),
        ..intent(&live_owner())
    };
    publish(&running.scratch.0, &changed);
    let (scratch, result) = running.stop();
    result.unwrap();
    assert!(socket_path(&scratch.0, BINDING).exists());
    assert_eq!(read_record(&scratch.0, BINDING).unwrap().unwrap(), changed);
}

#[test]
fn a_socket_this_user_cannot_open_fails_the_start_and_is_left_alone() {
    // Permission modes do not bind root, which can still open the socket.
    if nix::unistd::geteuid().is_root() {
        return;
    }
    let scratch = Scratch::new();
    publish(&scratch.0, &intent(&live_owner()));
    // A socket this user cannot open says nothing about its owner.
    drop(UnixListener::bind(socket_path(&scratch.0, BINDING)).unwrap());
    fs::set_permissions(
        socket_path(&scratch.0, BINDING),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    let error = ClaudeChannel
        .serve(
            &ServeRequest {
                binding_id: BINDING,
                generation: GENERATION,
                directory: &scratch.0,
            },
            Box::new(io::BufReader::new(io::empty())),
            &mut Vec::new(),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        fs::symlink_metadata(socket_path(&scratch.0, BINDING)).is_ok(),
        "the socket file was neither replaced nor removed"
    );
}

#[test]
fn a_late_readiness_publish_cannot_clobber_a_newer_enrollment() {
    let scratch = Scratch::new();
    let owner = live_owner();
    publish(&scratch.0, &intent(&owner));
    let newer = Record {
        generation: "66666666-6666-4666-8666-666666666666".into(),
        ..intent(&owner)
    };
    let ready = std::sync::atomic::AtomicBool::new(false);
    let request = ServeRequest {
        binding_id: BINDING,
        generation: GENERATION,
        directory: &scratch.0,
    };
    let blocked = std::thread::scope(|scope| {
        locked(&scratch.0, || {
            let publishing =
                scope.spawn(|| server::publish(&request, &Process::of(&owner), &ready));
            std::thread::sleep(Duration::from_millis(500));
            let blocked = !publishing.is_finished();
            // A newer enrollment lands while the server's publish waits.
            publish(&scratch.0, &newer);
            blocked
        })
        .unwrap()
    });
    assert!(blocked, "the publish waits for the directory lock");
    assert_eq!(read_record(&scratch.0, BINDING).unwrap().unwrap(), newer);
    assert!(!ready.load(Ordering::SeqCst), "and it never reports ready");
}

#[test]
fn a_late_old_server_neither_binds_over_nor_unlinks_a_replacement_enrollments_socket() {
    let running = start();
    let replacement = Record {
        generation: "66666666-6666-4666-8666-666666666666".into(),
        ..intent(&live_owner())
    };
    // The launch was re-enrolled while this server was still running.
    publish(&running.scratch.0, &replacement);
    let (scratch, result) = running.stop();
    result.unwrap();
    assert!(
        socket_path(&scratch.0, BINDING).exists(),
        "the socket of a replacement enrollment is not the old server's to remove"
    );
    assert_eq!(
        read_record(&scratch.0, BINDING).unwrap().unwrap(),
        replacement
    );
    // And a server of the replaced generation can no longer start at all.
    assert_eq!(
        ClaudeChannel
            .serve(
                &ServeRequest {
                    binding_id: BINDING,
                    generation: GENERATION,
                    directory: &scratch.0,
                },
                Box::new(io::BufReader::new(io::empty())),
                &mut Vec::new(),
            )
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
}

// A pane lookup reads this driver's records and observes only the exact processes
// each record names; it never consults a stored binding and never lists the
// process table. The pids below are above any real pid, and `Ps` answers the one
// `ps` query a lookup makes for a single process.
mod pane {
    use super::*;
    use crate::process::{CommandFailure, CommandRequest};
    use std::cell::Cell;

    const OWNER: u64 = 2_000_000_020;
    const FOREGROUND: u64 = 2_000_000_030;
    const CLAUDE: u64 = 2_000_000_040;
    const STARTED: &str = "ps-v1:Sun Sep 27 10:00:00 2026";
    const STARTED_LINE: &str = "Sun Sep 27 10:00:00 2026 S\n";
    const REUSED_LINE: &str = "Mon Sep 28 09:00:00 2026 S\n";
    const OTHER_BINDING: &str = "99999999-9999-4999-8999-999999999999";

    fn at(pid: u64) -> ProcessIncarnation {
        ProcessIncarnation::new(pid, STARTED).unwrap()
    }

    /// The processes a test says exist, by the `lstart` line `ps` would print.
    struct Ps {
        started: Vec<(u64, &'static str)>,
        observed: Cell<u32>,
    }

    impl Ps {
        fn new(started: &[(u64, &'static str)]) -> Self {
            Self {
                started: started.to_vec(),
                observed: Cell::new(0),
            }
        }

        fn running(pids: &[u64]) -> Self {
            let started: Vec<_> = pids.iter().map(|pid| (*pid, STARTED_LINE)).collect();
            Self::new(&started)
        }
    }

    impl CommandRunner for Ps {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            let args: Vec<_> = request
                .args
                .iter()
                .map(|arg| arg.to_str().unwrap())
                .collect();
            match &args[3..] {
                ["-p", pid, "-o", "lstart=", "-o", "stat="] => {
                    self.observed.set(self.observed.get() + 1);
                    let pid: u64 = pid.parse().unwrap();
                    match self.started.iter().find(|(known, _)| *known == pid) {
                        Some((_, line)) => Ok(CommandOutput {
                            stdout: line.as_bytes().to_vec(),
                            stderr: vec![],
                        }),
                        None => Err(CommandError::new(CommandFailure::Exit {
                            code: Some(1),
                            signal: None,
                        })),
                    }
                }
                other => panic!("unexpected ps query {other:?}"),
            }
        }
    }

    fn ask(
        scratch: &Scratch,
        ps: &Ps,
        pane: &PaneAddress<'_>,
        binding_id: Option<&str>,
    ) -> Result<PaneEvidence, EvidenceError> {
        pane_enrolled(
            ps,
            &scratch.0,
            pane,
            binding_id,
            Instant::now() + Duration::from_secs(5),
        )
    }

    /// The pane the records name, as the launcher persisted it.
    fn here(scratch: &Scratch, ps: &Ps) -> Result<PaneEvidence, EvidenceError> {
        ask(scratch, ps, &address(&server()), None)
    }

    fn record_for(foreground: Option<u64>, claude: Option<u64>) -> Record {
        Record {
            foreground: foreground.map(|pid| Process::of(&at(pid))),
            claude: claude.map(|pid| Process::of(&at(pid))),
            ..intent(&at(OWNER))
        }
    }

    fn enrolled() -> Result<PaneEvidence, EvidenceError> {
        Ok(PaneEvidence {
            enrolled: true,
            skipped: vec![],
        })
    }

    fn none() -> Result<PaneEvidence, EvidenceError> {
        Ok(PaneEvidence::default())
    }

    #[test]
    fn a_lookup_without_records_costs_one_directory_read_and_ignores_every_other_file() {
        let scratch = Scratch::new();
        let ps = Ps::running(&[OWNER]);
        assert_eq!(here(&scratch, &ps), none());
        assert_eq!(
            pane_enrolled(
                &ps,
                &scratch.0.join("absent"),
                &address(&server()),
                None,
                Instant::now() + Duration::from_secs(5)
            ),
            none()
        );
        // Sockets, the lock, another driver's subdirectory and its differently named
        // files are not this driver's records and are never parsed as them.
        fs::write(scratch.0.join(LOCK_FILE), "").unwrap();
        fs::write(scratch.0.join(format!("{BINDING}.sock")), "").unwrap();
        fs::create_dir(scratch.0.join("codex")).unwrap();
        fs::write(
            scratch.0.join("codex").join(format!("{BINDING}.json")),
            "{}",
        )
        .unwrap();
        fs::write(
            scratch.0.join(format!("{BINDING}.codex.json")),
            "not a claude record",
        )
        .unwrap();
        fs::write(scratch.0.join("notes.json"), "[]").unwrap();
        assert_eq!(here(&scratch, &ps), none());
        assert_eq!(ps.observed.get(), 0, "nothing was observed");
    }

    #[test]
    fn a_record_attributed_to_the_pane_is_enrolled_through_each_exactly_live_process() {
        let scratch = Scratch::new();
        // The launch owner alone, before the foreground was published.
        publish(&scratch.0, &record_for(None, None));
        let ps = Ps::running(&[OWNER]);
        assert_eq!(here(&scratch, &ps), enrolled());
        assert_eq!(
            ps.observed.get(),
            1,
            "one observation of this record's process"
        );
        // The foreground outliving its launcher (the launcher was killed).
        publish(&scratch.0, &record_for(Some(FOREGROUND), None));
        assert_eq!(here(&scratch, &Ps::running(&[FOREGROUND])), enrolled());
        // The provider process outliving both, before and after the handshake.
        publish(&scratch.0, &record_for(Some(FOREGROUND), Some(CLAUDE)));
        assert_eq!(here(&scratch, &Ps::running(&[CLAUDE])), enrolled());
    }

    #[test]
    fn a_record_of_another_pane_or_server_is_never_evidence_and_never_observed() {
        let scratch = Scratch::new();
        publish(&scratch.0, &record_for(None, None));
        let ps = Ps::running(&[OWNER]);
        let mut elsewhere = server();
        elsewhere.server_pid += 1;
        let mut restarted = server();
        restarted.server_start_time = "later".into();
        let mut moved = server();
        moved.socket_path = "/tmp/tmux-other".into();
        for (name, server, pane_id, pane_pid) in [
            ("another pane", server(), "%2", 2),
            ("another pane process", server(), "%1", 3),
            ("another server pid", elsewhere, "%1", 2),
            ("a restarted server", restarted, "%1", 2),
            ("another socket", moved, "%1", 2),
        ] {
            let pane = PaneAddress {
                server: &server,
                pane_id,
                pane_pid,
            };
            assert_eq!(ask(&scratch, &ps, &pane, None), none(), "{name}");
        }
        assert_eq!(ps.observed.get(), 0, "an unrelated record is not observed");
    }

    #[test]
    fn a_reused_pid_or_an_ended_launch_is_not_evidence() {
        let scratch = Scratch::new();
        publish(&scratch.0, &record_for(Some(FOREGROUND), Some(CLAUDE)));
        // Every recorded process is gone: the launch ended and nothing blocks.
        let ps = Ps::new(&[]);
        assert_eq!(here(&scratch, &ps), none());
        assert_eq!(ps.observed.get(), 3);
        // The pids now belong to other incarnations.
        let reused = Ps::new(&[
            (OWNER, REUSED_LINE),
            (FOREGROUND, REUSED_LINE),
            (CLAUDE, REUSED_LINE),
        ]);
        assert_eq!(here(&scratch, &reused), none());
    }

    #[test]
    fn a_launch_that_never_recorded_its_foreground_is_unknown_and_terminal_with_a_named_recovery() {
        let scratch = Scratch::new();
        publish(&scratch.0, &record_for(None, None));
        let file = record_path(&scratch.0, BINDING);
        // The launcher is gone and nothing says where the agent went.
        let error = here(&scratch, &Ps::new(&[])).unwrap_err();
        assert_eq!(error.fault, ChannelFault::Unverifiable);
        assert_eq!(error.path.as_deref(), Some(file.as_path()));
        let message = error.message();
        for named in [
            "pane %1",
            "/tmp/tmux-test",
            &format!(
                "rm -- '{}' '{}'",
                file.display(),
                socket_path(&scratch.0, BINDING).display()
            ),
        ] {
            assert!(message.contains(named), "{named}: {message}");
        }
        // A reused launcher pid is a different process, not evidence of this one.
        let reused = Ps::new(&[(OWNER, REUSED_LINE)]);
        assert_eq!(
            here(&scratch, &reused).unwrap_err().fault,
            ChannelFault::Unverifiable
        );
        // Other panes are not held up by it.
        let mut other = server();
        other.server_id = "another".into();
        assert_eq!(ask(&scratch, &Ps::new(&[]), &address(&other), None), none());
        // Once the user removes the named files the pane is a plain pane again.
        fs::remove_file(&file).unwrap();
        assert_eq!(here(&scratch, &Ps::new(&[])), none());
    }

    #[test]
    fn the_named_recovery_quotes_an_apostrophe_in_a_path_and_removes_exactly_those_files() {
        assert_eq!(shell_quoted(Path::new("/a/it's")), r"'/a/it'\''s'");
        let scratch = Scratch::new();
        let directory = scratch.0.join("it's here");
        ensure_private_directory(&directory).unwrap();
        publish(&directory, &record_for(None, None));
        fs::write(socket_path(&directory, BINDING), "").unwrap();
        let bystander = directory.join("another.json");
        fs::write(&bystander, "{}").unwrap();
        // The unknown evidence of that record prints the command verbatim.
        let error = pane_enrolled(
            &Ps::new(&[]),
            &directory,
            &address(&server()),
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap_err();
        let command = recovery(&directory, BINDING);
        assert!(error.message().contains(&command), "{}", error.message());
        // Run by a shell as printed, it removes the record and the socket and
        // nothing else, whatever the quote in the path.
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .status()
            .unwrap();
        assert!(status.success(), "{command}");
        assert!(!record_path(&directory, BINDING).exists());
        assert!(!socket_path(&directory, BINDING).exists());
        assert!(bystander.exists());
    }

    #[test]
    fn an_observation_that_cannot_be_told_is_terminal_for_this_pane_and_names_the_recovery() {
        let scratch = Scratch::new();
        publish(&scratch.0, &record_for(Some(FOREGROUND), None));
        let file = record_path(&scratch.0, BINDING);
        for (name, ps) in [
            (
                "the owner is unreadable",
                Ps::new(&[(OWNER, "garbage\n"), (FOREGROUND, STARTED_LINE)]),
            ),
            (
                "the foreground is unreadable",
                Ps::new(&[(FOREGROUND, "garbage\n")]),
            ),
        ] {
            let error = here(&scratch, &ps).unwrap_err();
            assert_eq!(error.fault, ChannelFault::Unverifiable, "{name}");
            assert_eq!(error.path.as_deref(), Some(file.as_path()), "{name}");
            assert!(error.message().contains("rm -- '"), "{name}");
        }
    }

    #[test]
    fn a_record_that_cannot_be_attributed_is_skipped_by_name_and_never_blocks_the_pane() {
        let scratch = Scratch::new();
        let id = |n: u8| format!("{n:08x}-1111-4111-8111-111111111111");
        let corrupt = record_path(&scratch.0, &id(1));
        fs::write(&corrupt, "{ not json").unwrap();
        // Written before pane attribution existed: valid, but names no pane.
        let older = record_path(&scratch.0, &id(2));
        fs::write(
            &older,
            serde_json::to_vec(&Record {
                binding_id: id(2),
                identity_id: None,
                pane: None,
                ..record_for(None, None)
            })
            .unwrap(),
        )
        .unwrap();
        // Another record's contents under this record's name.
        let renamed = record_path(&scratch.0, &id(3));
        fs::write(
            &renamed,
            serde_json::to_vec(&Record {
                binding_id: id(4),
                ..record_for(None, None)
            })
            .unwrap(),
        )
        .unwrap();
        let ps = Ps::running(&[OWNER]);
        let evidence = here(&scratch, &ps).unwrap();
        assert!(!evidence.enrolled);
        let mut skipped = evidence.skipped;
        skipped.sort();
        assert_eq!(skipped, [corrupt.clone(), older, renamed]);
        assert_eq!(
            ps.observed.get(),
            0,
            "an unattributable record is never observed"
        );
        // The same pane with a live attributed record is still enrolled.
        publish(&scratch.0, &record_for(None, None));
        assert!(here(&scratch, &ps).unwrap().enrolled);
    }

    #[test]
    fn an_unreadable_record_named_for_the_panes_own_binding_is_terminal() {
        let scratch = Scratch::new();
        let file = record_path(&scratch.0, BINDING);
        fs::write(&file, "{ not json").unwrap();
        let ps = Ps::running(&[OWNER]);
        // Delivering to a binding: its own record cannot be told, so nothing is pasted.
        let error = ask(&scratch, &ps, &address(&server()), Some(BINDING)).unwrap_err();
        assert_eq!(error.fault, ChannelFault::InvalidRecord);
        assert_eq!(error.path.as_deref(), Some(file.as_path()));
        assert!(error.message().contains("rm -- '"), "{}", error.message());
        // Without a binding it is only an unattributable record.
        assert_eq!(
            here(&scratch, &ps).unwrap().skipped,
            std::slice::from_ref(&file),
            "named, not terminal"
        );
        // A record of another binding does not make this one's evidence invalid.
        assert_eq!(
            ask(&scratch, &ps, &address(&server()), Some(OTHER_BINDING))
                .unwrap()
                .skipped,
            [file]
        );
    }

    #[test]
    fn the_one_deadline_bounds_the_whole_lookup_not_a_count_of_records() {
        let scratch = Scratch::new();
        for index in 0..32 {
            publish(
                &scratch.0,
                &Record {
                    binding_id: format!("{index:08x}-1111-4111-8111-111111111111"),
                    ..record_for(None, None)
                },
            );
        }
        // Many leftovers are read and checked: nothing is capped by count.
        let mut other = server();
        other.server_pid = 7;
        assert_eq!(ask(&scratch, &Ps::new(&[]), &address(&other), None), none());
        // Running out of time before the records are read is unknown, not "none".
        let expired = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        assert_eq!(
            pane_enrolled(
                &Ps::new(&[]),
                &scratch.0,
                &address(&server()),
                None,
                expired
            ),
            Err(EvidenceError::at(ChannelFault::Unverifiable, &scratch.0))
        );
        // A directory that cannot be read at all is unknown too.
        let file = scratch.0.join("not-a-directory");
        fs::write(&file, "").unwrap();
        assert_eq!(
            pane_enrolled(
                &Ps::new(&[]),
                &file,
                &address(&server()),
                None,
                Instant::now() + Duration::from_secs(5)
            ),
            Err(EvidenceError::at(ChannelFault::Unverifiable, &file))
        );
    }

    #[test]
    fn a_record_written_before_attribution_still_parses_and_names_no_pane() {
        let scratch = Scratch::new();
        let old = format!(
            r#"{{"version":1,"bindingId":"{BINDING}","generation":"{GENERATION}","launchOwner":{{"pid":{OWNER},"start":"{STARTED}"}},"claude":null}}"#
        );
        fs::write(record_path(&scratch.0, BINDING), old).unwrap();
        let record = read_record(&scratch.0, BINDING).unwrap().unwrap();
        assert_eq!(
            (record.pane, record.identity_id, record.foreground),
            (None, None, None)
        );
    }

    fn enroll_binding(
        scratch: &Scratch,
        binding_id: &str,
    ) -> Result<Box<dyn ChannelEnrollment>, ChannelError> {
        let command = RuntimeCommand::verbatim(&["claude".into()]).unwrap();
        let owner = live_owner();
        let server = server();
        ClaudeChannel.enroll(&ChannelPlan {
            binding_id,
            identity_id: IDENTITY,
            pane: address(&server),
            owner: &owner,
            command: &command,
            working_directory: &scratch.0,
            tmt: Path::new("/usr/local/bin/tmt"),
            directory: &scratch.0,
        })
    }

    #[test]
    fn enrolling_prunes_only_launches_that_are_over_in_every_recorded_way() {
        let scratch = Scratch::new();
        let live = live_owner();
        let gone = dead_owner();
        let gone_too = provider(gone.pid() + 1_000_000);
        let id = |n: u8| format!("{n:08x}-1111-4111-8111-111111111111");
        let record = |n: u8,
                      owner: &ProcessIncarnation,
                      foreground: Option<&ProcessIncarnation>,
                      claude: Option<&ProcessIncarnation>| Record {
            binding_id: id(n),
            foreground: foreground.map(Process::of),
            claude: claude.map(Process::of),
            ..intent(owner)
        };
        publish(
            &scratch.0,
            &record(1, &gone, Some(&gone_too), Some(&gone_too)),
        ); // over: pruned
        publish(&scratch.0, &record(2, &gone, None, None)); // never recorded: kept
        publish(&scratch.0, &record(3, &live, Some(&gone_too), None)); // owner alive: kept
        publish(&scratch.0, &record(4, &gone, Some(&live), None)); // foreground alive: kept
        publish(&scratch.0, &record(5, &gone, Some(&gone_too), Some(&live))); // Claude alive: kept
        publish(&scratch.0, &record(6, &gone, Some(&gone_too), None)); // over: pruned
        fs::write(scratch.0.join(format!("{}.sock", id(1))), "").unwrap();
        fs::write(
            scratch.0.join(format!("{}.codex.json", id(1))),
            "other driver",
        )
        .unwrap();
        fs::write(record_path(&scratch.0, &id(7)), "{ unreadable").unwrap(); // kept
        let _lease = enroll_binding(&scratch, BINDING).unwrap();
        for pruned in [1, 6] {
            assert!(
                !record_path(&scratch.0, &id(pruned)).exists(),
                "record {pruned} was over"
            );
        }
        assert!(
            !scratch.0.join(format!("{}.sock", id(1))).exists(),
            "and its socket"
        );
        for kept in [2, 3, 4, 5, 7] {
            assert!(
                record_path(&scratch.0, &id(kept)).exists(),
                "record {kept} stays"
            );
        }
        assert!(scratch.0.join(format!("{}.codex.json", id(1))).exists());
        assert!(
            record_path(&scratch.0, BINDING).exists(),
            "the new enrollment"
        );
    }

    #[test]
    fn enrolling_without_a_pane_that_can_be_attributed_is_refused_before_any_side_effect() {
        let scratch = Scratch::new();
        let (owner, command) = (live_owner(), user_command());
        let complete = server();
        let mut no_socket = server();
        no_socket.socket_path.clear();
        let mut no_pid = server();
        no_pid.server_pid = 0;
        for (name, server, pane_id, pane_pid) in [
            ("no pane id", &complete, "", 2),
            ("no pane process", &complete, "%1", 0),
            ("no server socket", &no_socket, "%1", 2),
            ("no server pid", &no_pid, "%1", 2),
        ] {
            let mut planned = plan(&scratch.0, &owner, &command, server);
            planned.pane = PaneAddress {
                server,
                pane_id,
                pane_pid,
            };
            assert_eq!(
                ClaudeChannel.enroll(&planned).err(),
                Some(ChannelError::Unattributed),
                "{name}"
            );
            assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0, "{name}");
        }
    }

    #[test]
    fn the_enrollment_records_the_pane_and_identity_and_the_foreground_only_for_its_own_launch() {
        let scratch = Scratch::new();
        let owner = live_owner();
        let mut mine = lease(&scratch, &owner);
        let written = read_record(&scratch.0, BINDING).unwrap().unwrap();
        assert_eq!(written.pane, Some(pane_record()));
        assert_eq!(written.identity_id.as_deref(), Some(IDENTITY));
        assert_eq!(written.foreground, None);
        // The launcher publishes the exact child it spawned.
        mine.foreground_started(&at(FOREGROUND)).unwrap();
        let published = read_record(&scratch.0, BINDING).unwrap().unwrap();
        assert_eq!(published.foreground, Some(Process::of(&at(FOREGROUND))));
        assert_eq!(
            Record {
                foreground: None,
                ..published
            },
            written
        );
        // A replaced enrollment is not this launch's: nothing is written to it.
        let replacement = Record {
            generation: "66666666-6666-4666-8666-666666666666".into(),
            ..intent(&owner)
        };
        publish(&scratch.0, &replacement);
        assert_eq!(
            mine.foreground_started(&at(CLAUDE)),
            Err(ChannelError::Enrollment)
        );
        assert_eq!(
            read_record(&scratch.0, BINDING).unwrap().unwrap(),
            replacement
        );
        // Neither is a record of another launch owner, or none at all.
        publish(
            &scratch.0,
            &Record {
                generation: mine.generation.clone(),
                launch_owner: Process::of(&at(OWNER)),
                ..intent(&owner)
            },
        );
        assert_eq!(
            mine.foreground_started(&at(CLAUDE)),
            Err(ChannelError::Enrollment)
        );
        fs::remove_file(record_path(&scratch.0, BINDING)).unwrap();
        assert_eq!(
            mine.foreground_started(&at(CLAUDE)),
            Err(ChannelError::Enrollment)
        );
    }

    #[test]
    fn a_withdrawal_keeps_the_record_while_the_provider_may_still_run() {
        let scratch = Scratch::new();
        let owner = live_owner();
        let (mut survivor, alive) = another_live_process();
        for (name, claude, removed) in [
            ("a provider that never started", None, true),
            ("a provider that is gone", Some(dead_owner()), true),
            ("a provider that still runs", Some(alive.clone()), false),
        ] {
            let mine = lease(&scratch, &owner);
            publish(
                &scratch.0,
                &Record {
                    generation: mine.generation.clone(),
                    claude: claude.as_ref().map(Process::of),
                    ..intent(&owner)
                },
            );
            withdraw(mine);
            assert_eq!(
                record_path(&scratch.0, BINDING).exists(),
                !removed,
                "{name}"
            );
            let _ = fs::remove_file(record_path(&scratch.0, BINDING));
        }
        survivor.kill().unwrap();
        survivor.wait().unwrap();
    }

    #[test]
    fn a_new_launch_takes_over_an_old_enrollment_only_when_it_is_positively_over() {
        let (owner, user) = (live_owner(), user_command());
        let (mut survivor, alive) = another_live_process();
        let gone = dead_owner();
        let elsewhere = PaneRecord {
            pane_id: "%9".into(),
            ..pane_record()
        };
        let record = |owner: &ProcessIncarnation,
                      pane: Option<PaneRecord>,
                      foreground: Option<&ProcessIncarnation>,
                      claude: Option<&ProcessIncarnation>| Record {
            pane,
            foreground: foreground.map(Process::of),
            claude: claude.map(Process::of),
            ..intent(owner)
        };
        let (here, there) = (Some(pane_record()), Some(elsewhere));
        for (name, old, replaced) in [
            // Ended: every recorded process is gone, whatever pane it named.
            (
                "ended in this pane",
                record(&gone, here.clone(), Some(&gone), None),
                true,
            ),
            (
                "ended in another pane",
                record(&gone, there.clone(), Some(&gone), Some(&gone)),
                true,
            ),
            (
                "ended without a pane",
                record(&gone, None, Some(&gone), None),
                true,
            ),
            // Unknown: only an explicit relaunch in the very pane the record names.
            (
                "unknown in this pane",
                record(&gone, here.clone(), None, None),
                true,
            ),
            (
                "unknown in another pane",
                record(&gone, there, None, None),
                false,
            ),
            (
                "unknown without a pane",
                record(&gone, None, None, None),
                false,
            ),
            // Alive: never.
            (
                "owner alive",
                record(&alive, here.clone(), None, None),
                false,
            ),
            (
                "foreground alive",
                record(&gone, here.clone(), Some(&alive), None),
                false,
            ),
            (
                "provider alive",
                record(&gone, here, Some(&gone), Some(&alive)),
                false,
            ),
        ] {
            let scratch = Scratch::new();
            publish(&scratch.0, &old);
            let before = fs::read(record_path(&scratch.0, BINDING)).unwrap();
            let result = ClaudeChannel.enroll(&plan(&scratch.0, &owner, &user, &server()));
            let after = fs::read(record_path(&scratch.0, BINDING)).unwrap();
            if replaced {
                assert!(result.is_ok(), "{name}");
                assert_ne!(after, before, "{name}: replaced");
            } else {
                assert_eq!(result.err(), Some(ChannelError::Occupied), "{name}");
                assert_eq!(after, before, "{name}: untouched");
            }
        }
        survivor.kill().unwrap();
        survivor.wait().unwrap();
    }
}
