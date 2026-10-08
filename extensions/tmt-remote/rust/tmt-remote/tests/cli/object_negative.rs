//! Shipped Remote with no Colab admission adapter. This transport-only fixture
//! completes the neutral handshake and refuses callbacks; it encodes no Colab
//! content policy and is never routed consumer acceptance.
use super::*;
use tmt_extension_objects::{
    Admission, BeginInput, Budgets, Bus, Bytes32, Call, Caps, Chunk, ConfigInput, Counter,
    Decision, ErrorCode, Expect, Frame, Origin, Outcome, PartInput, Policy, ReadInput, Request,
    Sha256Hex, StatusInput, TransferInput, Uuid4, accept_head,
};

fn empty_objects(pilot: &Pilot) {
    let db = rusqlite::Connection::open_with_flags(
        pilot.root.join("state/remote/objects.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    for table in ["namespaces", "intents"] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "a refused request allocated {table}");
    }
    let tree = pilot.root.join("state/colab/objects");
    if tree.exists() {
        for entry in fs::read_dir(tree).unwrap() {
            let entry = entry.unwrap();
            assert!(
                entry.file_type().unwrap().is_dir(),
                "payload file allocated"
            );
            assert_eq!(
                fs::read_dir(entry.path()).unwrap().count(),
                0,
                "payload allocated"
            );
        }
    }
}

fn negative_head(stream: &mut UnixStream, deadline: Instant) -> Result<Vec<u8>, String> {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        let left = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or("negative fixture deadline")?;
        stream
            .set_read_timeout(Some(left))
            .map_err(|e| e.to_string())?;
        stream.read_exact(&mut byte).map_err(|e| e.to_string())?;
        head.push(byte[0]);
        if head.len() > 8192 || Instant::now() >= deadline {
            return Err("negative fixture head bound".into());
        }
    }
    Ok(head)
}

fn accept_negative(listener: UnixListener) -> Result<Bus, String> {
    use nix::poll::{PollFd, PollFlags, poll};
    use std::os::fd::AsFd;
    let mut ready = [PollFd::new(listener.as_fd(), PollFlags::POLLIN)];
    if poll(
        &mut ready,
        u16::try_from(STARTUP.as_millis()).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?
        == 0
    {
        return Err("negative fixture received no setup".into());
    }
    let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let head = negative_head(&mut stream, deadline)?;
    let text = std::str::from_utf8(&head).map_err(|e| e.to_string())?;
    // This is a transport-only stand-in, not an independent admission authority.
    // The neutral acceptor still validates every standard field and generation.
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::trim)
            .map(str::to_owned)
            .ok_or_else(|| format!("missing {name}"))
    };
    let expect = Expect {
        host: field("Host: ")?,
        mount: field("tmt-mount: ")?,
    };
    let link = accept_head(stream, &head, &[], &expect, &Budgets::contract(), deadline)
        .map_err(|e| format!("handshake {e:?}"))?;
    Bus::start(link, Budgets::contract(), Caps::contract(), None).map_err(|e| format!("bus {e:?}"))
}

fn calls() -> Vec<Call> {
    let id = Uuid4::parse("11111111-1111-4111-8111-111111111111").unwrap();
    let namespace = Bytes32::from_bytes([1; 32]);
    let key = Bytes32::from_bytes([2; 32]);
    let policy = Policy::new(b"{}".to_vec()).unwrap();
    let digest = Sha256Hex::from_bytes([3; 32]);
    vec![
        Call::Config(ConfigInput {
            namespace,
            policy: policy.clone(),
        }),
        Call::Begin(BeginInput {
            transfer_id: id,
            namespace,
            opaque_key: key,
            policy: policy.clone(),
            payload_sha256: digest,
            payload_bytes: 1,
        }),
        Call::Part(PartInput {
            transfer_id: id,
            index: 0,
            bytes: Chunk::new(vec![4]).unwrap(),
        }),
        Call::Commit(TransferInput { transfer_id: id }),
        Call::Discard(TransferInput { transfer_id: id }),
        Call::Status(StatusInput {
            transfer_id: id,
            namespace,
            policy: policy.clone(),
        }),
        Call::Read(ReadInput {
            namespace,
            opaque_key: key,
            policy,
            payload_sha256: digest,
            payload_bytes: 1,
            offset: 0,
            count: 1,
        }),
    ]
}

#[test]
fn shipped_local_without_colab_adapter_refuses_all_methods_before_ledger_effects() {
    for decision in [Decision::Deny, Decision::Unavailable] {
        let mut pilot = Pilot::new();
        let directory = pilot.root.join("state/colab");
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let listener = UnixListener::bind(directory.join("door.sock")).unwrap();
        fs::set_permissions(
            directory.join("door.sock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        std::thread::scope(|scope| {
            let peer = scope.spawn(|| accept_negative(listener));
            let ready: Value =
                serde_json::from_str(&start_door(&mut pilot, &["--port", "0"], false)).unwrap();
            assert_eq!(ready["state"], "ready");
            assert_eq!(ready["startupCoreCalls"], 2);
            let bus = peer
                .join()
                .unwrap()
                .expect("standard negative fixture handshake");
            empty_objects(&pilot);
            for (index, call) in calls().into_iter().enumerate() {
                let method = call.method();
                let id = Counter::new(index as u64 + 1).unwrap();
                bus.send(&Frame::Request(Request {
                    generation: bus.generation(),
                    request_id: id,
                    origin: Origin::LocalExtension,
                    call,
                }))
                .expect("live negative fixture request");
                let deadline = Instant::now() + Duration::from_secs(6);
                loop {
                    match bus.recv(Some(deadline)).expect("bounded refusal") {
                        Frame::Admit(admit) => {
                            assert_eq!(admit.request_id, id);
                            bus.send(&Frame::Admission(Admission {
                                generation: bus.generation(),
                                callback_id: admit.callback_id,
                                request_id: id,
                                decision,
                            }))
                            .expect("live callback refusal");
                        }
                        Frame::Result(result) => {
                            assert_eq!(result.method, method);
                            assert_eq!(result.request_id, id);
                            let error = if matches!(
                                method,
                                tmt_extension_objects::Method::Part
                                    | tmt_extension_objects::Method::Commit
                                    | tmt_extension_objects::Method::Discard
                            ) || decision == Decision::Unavailable
                            {
                                ErrorCode::Unavailable
                            } else {
                                ErrorCode::Denied
                            };
                            assert_eq!(result.outcome, Outcome::Failure(error));
                            break;
                        }
                        frame => panic!("unexpected negative fixture frame {frame:?}"),
                    }
                }
                empty_objects(&pilot);
            }
            // A missing answer is unavailable authority, never permission to
            // allocate. The sent callback timeout ends this generation.
            let call = calls().remove(0);
            let id = Counter::new(8).unwrap();
            bus.send(&Frame::Request(Request {
                generation: bus.generation(),
                request_id: id,
                origin: Origin::LocalExtension,
                call,
            }))
            .expect("live unanswered callback request");
            assert!(matches!(
                bus.recv(Some(Instant::now() + Duration::from_secs(6)))
                    .expect("acquire callback"),
                Frame::Admit(_)
            ));
            assert!(
                bus.recv(Some(Instant::now() + Duration::from_secs(6)))
                    .is_err(),
                "unanswered callback kept its generation live"
            );
            empty_objects(&pilot);
            assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
            wait_stopped(pilot.child.as_mut().unwrap());
            assert!(
                bus.recv(Some(Instant::now() + Duration::from_secs(3)))
                    .is_err(),
                "stopped channel leaked"
            );
            drop(bus);
        });
    }
}

#[test]
fn shipped_local_missing_listener_is_empty_and_door_readiness_is_unchanged() {
    for (detached, refusing) in [(false, false), (true, false), (false, true)] {
        let mut pilot = Pilot::new();
        if refusing {
            let directory = pilot.root.join("state/colab");
            fs::create_dir_all(&directory).unwrap();
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
            let socket = directory.join("door.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            fs::set_permissions(socket, fs::Permissions::from_mode(0o600)).unwrap();
            drop(listener); // A stale socket refuses connect; no adapter runs.
        }
        let options = if detached {
            vec!["--background", "--port", "0"]
        } else {
            vec!["--port", "0"]
        };
        let started = Instant::now();
        let ready: Value = serde_json::from_str(&start_door(&mut pilot, &options, false)).unwrap();
        eprintln!(
            "missing adapter {:?} readiness {} ns",
            detached,
            started.elapsed().as_nanos()
        );
        assert_eq!(ready["state"], "ready");
        assert_eq!(ready["startupCoreCalls"], 2);
        assert!(
            fs::read(pilot.root.join("serve.stderr"))
                .unwrap()
                .is_empty()
        );
        let observed = pilot
            .command()
            .args(["status", "--objects", "--json"])
            .output()
            .unwrap();
        assert!(observed.status.success());
        let observed: Value = serde_json::from_slice(&observed.stdout).unwrap();
        assert_eq!(
            observed["objectChannels"],
            serde_json::json!([{"extension":"colab","state":"unavailable","reason":"setup"}])
        );
        let (origin, path, _) = address_parts(ready["address"].as_str().unwrap());
        assert_eq!(
            status_json(&pilot),
            serde_json::json!({"running":true,"origin":origin,"path":path})
        );
        let socket = origin.strip_prefix("http://").unwrap();
        assert!(
            exchange(socket, &format!("GET / HTTP/1.1\r\nHost: {socket}\r\n\r\n"))
                .starts_with("HTTP/1.1 200")
        );
        for method in [
            "config", "begin", "part", "commit", "discard", "status", "read",
        ] {
            assert!(exchange(socket,&format!("POST {path}/objects.{method} HTTP/1.1\r\nHost: {socket}\r\nContent-Length: 0\r\n\r\n")).starts_with("HTTP/1.1 404"));
            empty_objects(&pilot);
        }
        assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
        if !detached {
            wait_stopped(pilot.child.as_mut().unwrap());
        }
        assert!(!pilot.root.join("state/remote/control.sock").exists());
    }
}

#[test]
fn shipped_local_hung_setup_is_bounded_without_releasing_the_fixture() {
    let mut pilot = Pilot::new();
    let directory = pilot.root.join("state/colab");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let listener = UnixListener::bind(directory.join("door.sock")).unwrap();
    fs::set_permissions(
        directory.join("door.sock"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let (entered, observed) = mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        let peer = scope.spawn(|| -> Result<(), String> {
            use nix::poll::{PollFd, PollFlags, poll};
            use std::os::fd::AsFd;
            let mut ready = [PollFd::new(listener.as_fd(), PollFlags::POLLIN)];
            if poll(&mut ready, 30_000u16).map_err(|e| e.to_string())? == 0 {
                return Err("hung fixture received no setup".into());
            }
            let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
            stream
                .set_read_timeout(Some(STARTUP))
                .map_err(|e| e.to_string())?;
            let mut byte = [0];
            stream.read_exact(&mut byte).map_err(|e| e.to_string())?;
            entered.send(Instant::now()).map_err(|e| e.to_string())?;
            // Keep the listener held beyond both 250 ms and the former 15 s setup
            // path. Only Remote's deadline can close it before door readiness.
            loop {
                match stream.read(&mut byte) {
                    Ok(0) => return Ok(()),
                    Ok(_) => {}
                    Err(e) => {
                        return Err(format!("hung fixture was released by its own timeout: {e}"));
                    }
                }
            }
        });
        let ready: Value =
            serde_json::from_str(&start_door(&mut pilot, &["--port", "0"], false)).unwrap();
        let entered = observed.recv_timeout(STARTUP).unwrap();
        let elapsed = entered.elapsed();
        eprintln!("shipped held setup to ready {} ns", elapsed.as_nanos());
        assert!(
            elapsed < Duration::from_secs(5),
            "setup spent the old long budget"
        );
        peer.join().unwrap().unwrap();
        assert_eq!(ready["state"], "ready");
        assert!(
            fs::read_to_string(pilot.root.join("serve.stderr"))
                .unwrap()
                .contains("Object channel unavailable for colab:")
        );
        empty_objects(&pilot);
        assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
        wait_stopped(pilot.child.as_mut().unwrap());
    });
}

#[test]
fn shipped_local_unsafe_ledger_is_reported_and_preserved_without_door_failure() {
    let mut pilot = Pilot::new();
    let directory = pilot.root.join("state/remote");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let ledger = directory.join("objects.db");
    let damaged = b"not an object ledger";
    fs::write(&ledger, damaged).unwrap();
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o600)).unwrap();
    let ready: Value =
        serde_json::from_str(&start_door(&mut pilot, &["--port", "0"], false)).unwrap();
    assert_eq!(ready["state"], "ready");
    assert_eq!(fs::read(&ledger).unwrap(), damaged);
    assert!(
        fs::read_to_string(pilot.root.join("serve.stderr"))
            .unwrap()
            .contains("Object storage unavailable:")
    );
    let output = pilot
        .command()
        .args(["status", "--objects", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        status["objectChannels"],
        serde_json::json!([{"extension":"colab","state":"unavailable","reason":"storage"}])
    );
    assert_eq!(status_json(&pilot)["running"], true);
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
    wait_stopped(pilot.child.as_mut().unwrap());
}

#[test]
fn shipped_local_refused_object_setup_forwards_page_upgrades_without_origin_and_cools_down() {
    use nix::poll::{PollFd, PollFlags, poll};
    use std::os::fd::AsFd;
    let mut pilot = Pilot::new();
    let ready: Value =
        serde_json::from_str(&start_door(&mut pilot, &["--port", "0"], false)).unwrap();
    let (origin, path, _) = address_parts(ready["address"].as_str().unwrap());
    let directory = pilot.root.join("state/colab");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let listener = UnixListener::bind(directory.join("door.sock")).unwrap();
    fs::set_permissions(
        directory.join("door.sock"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    std::thread::scope(|scope| {
        let peer = scope.spawn(|| -> Result<(), String> {
            let mut failed = None;
            let mut pages = 0;
            while pages != 2 {
                let mut ready = [PollFd::new(listener.as_fd(), PollFlags::POLLIN)];
                if poll(&mut ready, 5000u16).map_err(|e| e.to_string())? == 0 {
                    return Err("page fixture received no request".into());
                }
                let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
                let head = negative_head(&mut stream, Instant::now() + Duration::from_secs(3))?;
                let head = std::str::from_utf8(&head).map_err(|e| e.to_string())?;
                if head.starts_with("GET /.tmt/remote/object-channel-v1 HTTP/1.1\r\n") {
                    if failed.is_some() { return Err("reconnect bypassed the failed-attempt cool-down".into()); }
                    failed = Some(Instant::now());
                    // Refusal may race Remote closing its bounded candidate.
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    continue;
                }
                let failed = failed.ok_or("page upgrade preceded demand setup")?;
                if pages == 1 && failed.elapsed() >= tmt_remote::limits::OBJECT_REACTIVATION_COOLDOWN {
                    return Err("second page request did not reach the fixture inside the cool-down observation window".into());
                }
                assert!(head.starts_with("GET /sync HTTP/1.1\r\n"));
                assert!(head.to_ascii_lowercase().contains("upgrade: websocket\r\n"));
                assert!(!head.to_ascii_lowercase().contains("tmt-origin:"));
                stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\nSec-WebSocket-Protocol: colab-sync-v1\r\n\r\n")
                    .map_err(|e| format!("page reply: {e}"))?;
                pages += 1;
            }
            Ok(())
        });
        let started = Instant::now();
        for _ in 0..2 {
            let socket = origin.strip_prefix("http://").unwrap();
            let mut client = TcpStream::connect(socket).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(client,"GET {path}/x/colab/sync HTTP/1.1\r\nHost: {socket}\r\nOrigin: {origin}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: colab-sync-v1\r\n\r\n").unwrap();
            let mut head = Vec::new();
            let mut byte = [0];
            let deadline = Instant::now() + Duration::from_secs(5);
            while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                let left = deadline
                    .checked_duration_since(Instant::now())
                    .filter(|left| !left.is_zero())
                    .expect("page response deadline");
                client.set_read_timeout(Some(left)).unwrap();
                client.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            assert!(head.starts_with(b"HTTP/1.1 101") && head.ends_with(b"\r\n\r\n"));
        }
        peer.join().unwrap().unwrap();
        eprintln!(
            "two shipped degraded upgrades: {} ns",
            started.elapsed().as_nanos()
        );
        empty_objects(&pilot);
        assert!(
            fs::read(pilot.root.join("serve.stderr"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
        wait_stopped(pilot.child.as_mut().unwrap());
    });
}
