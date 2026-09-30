use super::*;
use std::{
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
        launch_owner: Process::of(owner),
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
    write_record(directory, record).unwrap();
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
    send_within(&scratch.0, entry, message, SHORT_WAIT)
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
fn an_enrollment_whose_launch_owner_is_gone_is_stale_and_never_pasted_to() {
    let scratch = Scratch::new();
    let owner = dead_owner();
    publish(&scratch.0, &ready(&owner, &provider(4242)));
    // An ended enrollment is not evidence that the session never opted in.
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
            "other launch owner",
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
    let result = send_within(&scratch.0, &entry, MESSAGE, Duration::from_secs(10));
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
        let result = send_within(&scratch.0, &entry, MESSAGE, Duration::from_secs(10));
        change.join().unwrap();
        assert_eq!(fault(result), ("denied", ChannelFault::Mismatch));
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
    // No endpoint exists: a connect attempt would be `NotSent`, not `Denied`.
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

// Enrollment, withdrawal and the launch contract.

fn plan<'a>(directory: &'a Path, owner: &'a ProcessIncarnation) -> ChannelPlan<'a> {
    ChannelPlan {
        binding_id: BINDING,
        owner,
        tmt: Path::new("/opt/tmt/bin/tmt"),
        directory,
    }
}

#[test]
fn enrollment_is_durable_before_launch_and_names_a_fresh_generation() {
    let scratch = Scratch::new();
    let owner = live_owner();
    let arguments = ClaudeChannel.enroll(&plan(&scratch.0, &owner)).unwrap();
    assert_eq!(arguments.len(), 4);
    assert_eq!(arguments[0], MCP_CONFIG_FLAG);
    assert_eq!(arguments[2], CHANNEL_FLAG);
    assert_eq!(arguments[3], format!("server:{SERVER_NAME}").as_str());
    assert!(
        !arguments
            .iter()
            .any(|argument| argument == "--strict-mcp-config"),
        "user MCP servers stay available"
    );
    let config: serde_json::Value = serde_json::from_str(arguments[1].to_str().unwrap()).unwrap();
    let servers = config["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 1);
    let server = &servers[SERVER_NAME];
    assert_eq!(server["command"], "/opt/tmt/bin/tmt");
    let args: Vec<_> = server["args"]
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
    let again = ClaudeChannel.enroll(&plan(&scratch.0, &owner)).unwrap();
    assert_ne!(arguments, again);
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
        ClaudeChannel.enroll(&plan(&scratch.0, &owner)),
        Err(ChannelError::Enrollment)
    );
}

#[test]
fn withdrawing_removes_the_enrollment_and_its_socket() {
    let scratch = Scratch::new();
    let owner = live_owner();
    ClaudeChannel.enroll(&plan(&scratch.0, &owner)).unwrap();
    drop(UnixListener::bind(socket_path(&scratch.0, BINDING)).unwrap());
    ClaudeChannel.withdraw(&scratch.0, BINDING);
    assert!(!record_path(&scratch.0, BINDING).exists());
    assert!(!socket_path(&scratch.0, BINDING).exists());
    // Withdrawing what does not exist is not an error.
    ClaudeChannel.withdraw(&scratch.0, BINDING);
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

#[test]
fn the_provider_contract_constants_are_pinned() {
    // Changing any of these needs new provider evidence and a contract update.
    assert_eq!(SUPPORTED_VERSIONS, ["2.1.285 (Claude Code)"]);
    assert_eq!(CAPABILITY, "claude/channel");
    assert_eq!(NOTIFICATION_METHOD, "notifications/claude/channel");
    assert_eq!(PROTOCOL_VERSION, "2025-11-25");
    assert_eq!(MCP_CONFIG_FLAG, "--mcp-config");
    assert_eq!(CHANNEL_FLAG, "--dangerously-load-development-channels");
}

fn probe(scratch: &Scratch, version: &str) -> Result<(), ChannelError> {
    let script = scratch.0.join("claude");
    let _ = fs::remove_file(&script);
    fs::write(&script, format!("#!/bin/sh\necho '{version}'\n")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    // Another test's fork can briefly hold the just-written file open (ETXTBSY);
    // only that launch failure is retried, and only in this fixture.
    let mut result = Err(ChannelError::ProviderUnavailable);
    for _ in 0..40 {
        result = ClaudeChannel.preflight(
            script.as_os_str(),
            &scratch.0,
            Instant::now() + Duration::from_secs(5),
        );
        if result != Err(ChannelError::ProviderUnavailable) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    result
}

#[test]
fn preflight_accepts_only_the_recorded_provider_version() {
    let scratch = Scratch::new();
    assert_eq!(probe(&scratch, "2.1.285 (Claude Code)"), Ok(()));
    assert_eq!(
        probe(&scratch, "2.1.286 (Claude Code)"),
        Err(ChannelError::ProviderVersion {
            found: "2.1.286 (Claude Code)".into()
        })
    );
    assert!(matches!(
        probe(&scratch, "not a version"),
        Err(ChannelError::ProviderVersion { .. })
    ));
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
            &running.scratch.0,
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
            &scratch.0,
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
    // Started by hand, with no enrollment from `tmt run --channel`: refused,
    // and it must not have made one or bound a socket.
    assert_eq!(
        serve(BINDING, GENERATION, &scratch.0).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
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
    let running = start();
    let error = ClaudeChannel
        .serve(
            &ServeRequest {
                binding_id: BINDING,
                generation: GENERATION,
                directory: &running.scratch.0,
            },
            Box::new(io::BufReader::new(io::empty())),
            &mut Vec::new(),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    let (scratch, result) = running.stop();
    result.unwrap();
    // A crashed predecessor leaves a socket file; a new server of the same
    // enrolled launch takes it over.
    drop(UnixListener::bind(socket_path(&scratch.0, BINDING)).unwrap());
    let (stdin, server_in) = UnixStream::pair().unwrap();
    let (server_out, _stdout) = UnixStream::pair().unwrap();
    let directory = scratch.0.clone();
    let thread = std::thread::spawn(move || serve_on(&directory, server_in, server_out));
    wait_until("the replacement server accepting", || {
        UnixStream::connect(socket_path(&scratch.0, BINDING)).is_ok()
    });
    drop(stdin);
    thread.join().unwrap().unwrap();
}
