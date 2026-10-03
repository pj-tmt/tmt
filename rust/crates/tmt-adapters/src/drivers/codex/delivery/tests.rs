use super::super::record::{Process, Ready};
use super::*;
use crate::test_support::{TestChild, TestDirectory};
use tmt_core::binding::session::ObservedSessionKey;
fn fixture() -> (Record, BindingSessionState) {
    let owner = ProcessIncarnation::new(10, "owner").unwrap();
    let thread = "22222222-2222-4222-8222-222222222222";
    let mut record = Record::new("11111111-1111-4111-8111-111111111111", &owner).unwrap();
    record.ready = Some(Ready {
        server: Process::of(&ProcessIncarnation::new(11, "server").unwrap()),
        port: 49000,
        thread: thread.into(),
    });
    let session = BindingSessionState {
        state: RuntimeState::Running,
        launch_owner: Some(owner),
        key: Some(ObservedSessionKey {
            incarnation: ProcessIncarnation::new(12, "foreground").unwrap(),
            provider_session: Some(ProviderSessionId::new(thread).unwrap()),
        }),
        ..Default::default()
    };
    record.foreground = Foreground::Known(Process::of(&session.key.as_ref().unwrap().incarnation));
    (record, session)
}
#[test]
fn exact_live_owner_server_foreground_and_thread_are_required() {
    let (record, current) = fixture();
    assert_eq!(
        applicable(&record, &current, &|_| RuntimeLiveness::Alive),
        Ok(true)
    );
    for pid in [10, 11, 12] {
        assert!(
            applicable(&record, &current, &|p| if p.pid() == pid {
                RuntimeLiveness::Unknown
            } else {
                RuntimeLiveness::Alive
            })
            .is_err()
        );
    }
    let mut different = current.clone();
    different.key.as_mut().unwrap().provider_session =
        Some(ProviderSessionId::new("another-thread").unwrap());
    assert_eq!(
        applicable(&record, &different, &|_| RuntimeLiveness::Alive),
        Err(ChannelFault::Mismatch)
    );
    different = current.clone();
    different.key.as_mut().unwrap().incarnation =
        record.ready.as_ref().unwrap().server.incarnation().unwrap();
    assert_eq!(
        applicable(&record, &different, &|_| RuntimeLiveness::Alive),
        Err(ChannelFault::Mismatch)
    );
}
#[test]
fn pending_new_opt_in_cannot_fall_back_to_previous_live_binding_owner() {
    let (mut record, mut current) = fixture();
    record.ready = None;
    current.launch_owner = Some(ProcessIncarnation::new(20, "prior-owner").unwrap());
    assert!(applicable(&record, &current, &|_| RuntimeLiveness::Alive).is_err());
    current.launch_owner = None;
    assert!(applicable(&record, &current, &|_| RuntimeLiveness::Alive).is_err());
}
#[test]
fn old_absence_alone_is_terminal_but_proven_different_live_owner_can_baseline() {
    let (record, mut current) = fixture();
    assert_eq!(
        applicable(&record, &current, &|_| RuntimeLiveness::Gone),
        Err(ChannelFault::Stale)
    );
    current.launch_owner = Some(ProcessIncarnation::new(20, "new-owner").unwrap());
    assert_eq!(
        applicable(&record, &current, &|p| if p.pid() == 20 {
            RuntimeLiveness::Alive
        } else {
            RuntimeLiveness::Gone
        }),
        Ok(false)
    );
    assert!(applicable(&record, &current, &|_| RuntimeLiveness::Unknown).is_err());
}
#[test]
fn enrollment_is_read_only_and_never_depends_on_ready_or_live_owner() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let (mut record, _) = fixture_record();
    record.ready = None;
    assert_eq!(enrolled(&fixture.path, &record.binding_id), Ok(false));
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    assert_eq!(enrolled(&fixture.path, &record.binding_id), Ok(true));
    assert!(store.read(&record.binding_id).unwrap() == Some(record.clone()));
    std::fs::write(
        fixture
            .path
            .join("codex")
            .join(format!("{}.json", record.binding_id)),
        b"bad",
    )
    .unwrap();
    assert_eq!(
        enrolled(&fixture.path, &record.binding_id),
        Err(ChannelFault::InvalidRecord)
    );
}
fn fixture_record() -> (Record, BindingSessionState) {
    fixture()
}

fn observed(pid: u64) -> ProcessIncarnation {
    match observe_runtime_process(
        &UnixCommandRunner,
        pid,
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap()
    {
        crate::process::runtime::ProcessObservation::Live(value) => value,
        other => panic!("owned process must be live: {other:?}"),
    }
}
fn entry(record: &Record, current: BindingSessionState) -> BindingEntry {
    use tmt_core::{
        binding::Binding,
        endpoint::ServerEvidence,
        host::HostKind,
        identity::{Identity, Lifetime},
    };
    BindingEntry {
        identity: Identity {
            id: "33333333-3333-4333-8333-333333333333".into(),
            name: "owned".into(),
            canonical_name: "owned".into(),
            lifetime: Lifetime::Temporary,
            created_at: "now".into(),
            updated_at: "now".into(),
        },
        binding: Some(Binding {
            id: record.binding_id.clone(),
            identity_id: "33333333-3333-4333-8333-333333333333".into(),
            server: ServerEvidence {
                host: HostKind::Tmux,
                server_id: "44444444-4444-4444-8444-444444444444".into(),
                socket_path: "/unused".into(),
                server_pid: 1,
                server_start_time: "fixture".into(),
            },
            pane_id: "%1".into(),
            pane_pid: 1,
            pane_incarnation: None,
            session: current,
        }),
    }
}
fn real_socket_send(
    reply: u8,
    startup: Option<std::sync::mpsc::Receiver<()>>,
    wait_for_ready: bool,
    waiting: Option<std::sync::mpsc::Sender<()>>,
    release_after_send: &[std::sync::mpsc::Sender<()>],
) -> Sent {
    use nix::poll::{PollFd, PollFlags, poll};
    use serde_json::{Value, json};
    use std::{
        net::TcpListener,
        os::{fd::AsFd, unix::fs::PermissionsExt},
    };
    use tungstenite::Message;
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let mut child = TestChild::new(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let owner = observed(u64::from(std::process::id()));
    let foreground = observed(u64::from(child.child.id()));
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut record = Record::new("11111111-1111-4111-8111-111111111111", &owner).unwrap();
    let attribution_entry = entry(&record, BindingSessionState::default());
    let binding = attribution_entry.binding.as_ref().unwrap();
    record.attribution = Some(
        super::super::record::Attribution::new(
            &binding.identity_id,
            &binding.server,
            &binding.pane_id,
            binding.pane_pid,
        )
        .unwrap(),
    );
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    let thread = "22222222-2222-4222-8222-222222222222";
    record = store
        .ready(
            &record,
            Ready {
                server: Process::of(&owner),
                port,
                thread: thread.into(),
            },
        )
        .unwrap();
    record = store.foreground(&record, &foreground).unwrap();
    let generation = store.generation_directory(&record).unwrap();
    std::fs::create_dir(&generation).unwrap();
    std::fs::set_permissions(&generation, std::fs::Permissions::from_mode(0o700)).unwrap();
    crate::private_file::replace(&generation.join("capability"), b"owned-token").unwrap();
    let current = BindingSessionState {
        state: RuntimeState::Running,
        key: Some(ObservedSessionKey {
            incarnation: foreground,
            provider_session: Some(ProviderSessionId::new(thread).unwrap()),
        }),
        launch_owner: Some(owner),
        ..Default::default()
    };
    let entry = entry(&record, current);
    let (started, ready) = std::sync::mpsc::channel();
    let endpoint = std::thread::spawn(move || {
        if let Some(startup) = startup {
            startup.recv().unwrap();
        }
        started.send(()).unwrap();
        // A pre-connect sender failure must fail this fixture, not leave a
        // peer blocked in accept while the test waits to join it.
        let mut events = [PollFd::new(listener.as_fd(), PollFlags::POLLIN)];
        assert_eq!(poll(&mut events, 3000_u16).unwrap(), 1);
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut socket = match tungstenite::accept(stream) {
            Ok(socket) => socket,
            Err(_) if !wait_for_ready => return listener,
            Err(error) => panic!("ready peer handshake failed: {error}"),
        };
        let first = socket.read();
        if !wait_for_ready {
            // The deliberately unready control has already closed its sole
            // connection. It must not have reached application input.
            assert!(first.is_err());
            return listener;
        }
        let init: Value = serde_json::from_str(first.unwrap().to_text().unwrap()).unwrap();
        socket
            .send(Message::Text(
                json!({"id":init["id"],"result":{"userAgent":"tmt/0.159.3 (owned fixture)"}})
                    .to_string()
                    .into(),
            ))
            .unwrap();
        let _: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        let queue: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(queue["method"], "thread/queue/add");
        assert_eq!(queue["params"]["threadId"], thread);
        if reply != 3 {
            let response = match reply {
                0 => {
                    json!({"id":queue["id"],"result":{"queuedSubmission":{"id":"accepted","clientUserMessageId":queue["params"]["clientUserMessageId"],"input":queue["params"]["input"]}}})
                }
                1 => {
                    json!({"id":queue["id"],"error":{"code":-32603,"message":"internal after enqueue"}})
                }
                _ => {
                    json!({"id":queue["id"],"error":{"code":-32600,"message":format!("session {thread} is archived. Run `codex unarchive {thread}` to unarchive it first.")}})
                }
            };
            socket
                .send(Message::Text(response.to_string().into()))
                .unwrap();
            assert!(socket.read().is_err());
        }
        drop(socket);
        listener
    });
    if let Some(waiting) = waiting {
        waiting.send(()).unwrap();
    }
    if wait_for_ready {
        // Binding a listener and writing Ready do not prove this peer has
        // run. Keep fixture startup outside send's absolute I/O deadline.
        ready.recv().unwrap();
    }
    let paste = std::cell::Cell::new(0);
    let outcome = tmt_core::driver::routing::send_preferred(
        || send(Some(&fixture.path), &entry, "tiny"),
        || {
            paste.set(paste.get() + 1);
            ActionResult::Completed(DeliveryAcceptance::Submitted)
        },
    );
    assert_eq!(paste.get(), 0);
    for release in release_after_send {
        release.send(()).unwrap();
    }
    let listener = endpoint.join().unwrap();
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(store.read(&record.binding_id).unwrap() == Some(record));
    child.child.kill().expect("stop owned foreground fixture");
    child.child.wait().expect("reap owned foreground fixture");
    outcome
}

#[test]
fn real_socket_send_outcomes_are_one_shot_and_zero_fallback() {
    for reply in 0..4 {
        let outcome = real_socket_send(reply, None, true, None, &[]);
        match reply {
            0 => assert_eq!(outcome, ActionResult::Completed(DeliveryAcceptance::Queued)),
            2 => assert_eq!(outcome, denied(ChannelFault::Refused)),
            _ => assert_eq!(outcome, uncertain()),
        }
    }
}

#[test]
fn peer_readiness_keeps_induced_startup_delay_outside_the_send_deadline() {
    use std::sync::mpsc;
    let (release_ready, held_ready) = mpsc::channel();
    let (release_unready, held_unready) = mpsc::channel();
    let (waiting, waits) = mpsc::channel();
    let synchronized =
        std::thread::spawn(move || real_socket_send(0, Some(held_ready), true, Some(waiting), &[]));
    // The synchronized fixture has finished setup and is waiting for its peer.
    // Hold both peers until the old unsynchronized ordering exhausts its real
    // production deadline; this induces startup delay without a sleep or retry.
    waits.recv().unwrap();
    let unready = real_socket_send(
        0,
        Some(held_unready),
        false,
        None,
        &[release_unready, release_ready],
    );
    let ready = synchronized.join().unwrap();
    assert_eq!(unready, denied(ChannelFault::NotReady));
    assert_eq!(ready, ActionResult::Completed(DeliveryAcceptance::Queued));
}

#[test]
fn missing_or_mismatched_bound_attribution_refuses_before_process_or_endpoint_io() {
    for missing in [true, false] {
        let fixture_dir = TestDirectory::new();
        let store = Store::open(&fixture_dir.path).unwrap();
        let (mut record, state) = fixture();
        let entry = entry(&record, state);
        let binding = entry.binding.as_ref().unwrap();
        if !missing {
            let mut attribution = super::super::record::Attribution::new(
                &binding.identity_id,
                &binding.server,
                &binding.pane_id,
                binding.pane_pid,
            )
            .unwrap();
            attribution.pane_pid += 1;
            record.attribution = Some(attribution);
        }
        store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
        assert!(matches!(
            send(Some(&fixture_dir.path), &entry, "tiny"),
            ActionResult::Failed(SendFailure::Denied(RuntimeError::Channel(
                ChannelFault::Mismatch
            )))
        ));
    }
}
