use super::*;
use crate::{process::runtime::ProcessObservation, test_support::TestDirectory};
use serde_json::{Value, json};
use std::{
    net::TcpListener,
    process::{Child, Command, Stdio},
    thread,
};

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn owner(pid: u32) -> tmt_core::endpoint::ProcessIncarnation {
    match observe_runtime_process(&UnixCommandRunner, pid.into(), Instant::now() + START).unwrap() {
        ProcessObservation::Live(value) => value,
        other => panic!("owned process not observable: {other:?}"),
    }
}
fn fixture_command(root: &std::path::Path) -> RuntimeCommand {
    let path = root.join("fake-codex");
    let executable = std::env::current_exe()
        .unwrap()
        .to_str()
        .unwrap()
        .replace('\'', "'\\''");
    let script = format!(
        "#!/bin/sh\nexport TMT_TEST_CODEX_SUPERVISOR=1\nexec '{executable}' --exact drivers::codex::supervisor::tests::fake_server --nocapture\n"
    );
    tmt_test_support::write_executable(&path, script.as_bytes(), 0o700).unwrap();
    RuntimeCommand {
        executable: path.into_os_string(),
        args: vec![
            "resume".into(),
            "22222222-2222-4222-8222-222222222222".into(),
        ],
    }
}

// The production owner executes this isolated fixture instead of a provider.
// It serves the two enrollment calls only; no model, credentials or queue.
#[test]
fn fake_server() {
    if std::env::var_os("TMT_TEST_CODEX_SUPERVISOR").is_none() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    eprintln!("  listening on: ws://{}", listener.local_addr().unwrap());
    for stage in 0..2 {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(START)).unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        let init: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(init["method"], "initialize");
        socket
            .send(tungstenite::Message::text(
                json!({"id":init["id"],"result":{"userAgent":"tmt/0.159.3 (fixture)"}}).to_string(),
            ))
            .unwrap();
        let initialized: Value =
            serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(initialized["method"], "initialized");
        let request: Value =
            serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        if request["method"] == "thread/resume" {
            assert_eq!(
                request["params"]["threadId"],
                "22222222-2222-4222-8222-222222222222"
            );
            assert!(request["params"].get("sandbox").is_none());
            assert!(request["params"].get("approvalPolicy").is_none());
            socket.send(tungstenite::Message::text(json!({"id":request["id"],"result":{"cwd":request["params"]["cwd"],"thread":{"id":"22222222-2222-4222-8222-222222222222"}}}).to_string())).unwrap();
            break;
        }
        assert_eq!(
            request["method"], "thread/loaded/list",
            "fresh owner never starts or resumes a thread"
        );
        let data = if stage == 0 {
            json!([])
        } else {
            json!(["22222222-2222-4222-8222-222222222222"])
        };
        socket
            .send(tungstenite::Message::text(
                json!({"id":request["id"],"result":{"data":data}}).to_string(),
            ))
            .unwrap();
        if stage == 1 {
            let request: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(request["method"], "thread/read");
            socket.send(tungstenite::Message::text(json!({"id":request["id"],"result":{"thread":{"id":"22222222-2222-4222-8222-222222222222","cwd":std::env::current_dir().unwrap(),"ephemeral":false}}}).to_string())).unwrap();
        }
    }
    // Stay alive until the original process owner terminates this endpoint.
    let _ = listener.accept();
}

#[test]
fn launcher_sigkill_reaps_server_but_keeps_live_foreground_enrollment() {
    run_lifetime(false, true);
}
#[test]
fn explicit_withdraw_after_foreground_reap_retires_enrollment() {
    run_lifetime(true, true);
}
#[test]
fn crash_before_foreground_publication_preserves_unknown_after_server_cleanup() {
    run_lifetime(false, false);
}
fn run_lifetime(explicit: bool, published: bool) {
    let fixture = TestDirectory::new();
    let root = fixture.path.canonicalize().unwrap();
    let store = Store::open(&root).unwrap();
    let (mut control, helper) = UnixStream::pair().unwrap();
    // Only this launcher process inherits the writer endpoint. The fake
    // foreground intentionally survives launcher SIGKILL, like a shared-tty TUI.
    let mut launcher = ChildGuard(
        Command::new("/bin/sleep")
            .arg("60")
            .stdin(File::from(OwnedFd::from(control.try_clone().unwrap())))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut foreground = ChildGuard(Command::new("/bin/sleep").arg("60").spawn().unwrap());
    let record = Record::new(
        "11111111-1111-4111-8111-111111111111",
        &owner(launcher.0.id()),
    )
    .unwrap();
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    let independent = Record::new(
        "33333333-3333-4333-8333-333333333333",
        &owner(std::process::id()),
    )
    .unwrap();
    store
        .create(&independent, |_| RuntimeLiveness::Alive)
        .unwrap();
    let command = fixture_command(&root);
    // A read timeout is a fixture failure bound, not product EOF evidence.
    helper
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let helper_root = root.clone();
    let helper_record = record.clone();
    let joined = thread::spawn(move || {
        let mut output = helper.try_clone().unwrap();
        serve(
            &ServeRequest {
                directory: &helper_root,
                binding_id: &helper_record.binding_id,
                generation: &helper_record.generation,
            },
            Box::new(std::io::BufReader::new(helper)),
            &mut output,
        )
    });
    let start = Start {
        resume_session: Some("22222222-2222-4222-8222-222222222222".into()),
        executable: command.executable.as_bytes().to_vec(),
        args: command
            .args
            .iter()
            .map(|arg| arg.as_bytes().to_vec())
            .collect(),
        cwd: root.as_os_str().as_bytes().to_vec(),
    };
    write_frame(
        &mut control,
        &serde_json::to_vec(&start).unwrap(),
        Instant::now() + START,
    )
    .unwrap();
    let ready: Ready =
        serde_json::from_slice(&read_frame(&mut control, Instant::now() + START).unwrap()).unwrap();
    assert_eq!(
        ready.session.as_deref(),
        Some("22222222-2222-4222-8222-222222222222")
    );
    if published {
        store
            .foreground(&record, &owner(foreground.0.id()))
            .unwrap();
    }
    assert_eq!(
        matches!(
            store.read(&record.binding_id).unwrap().unwrap().foreground,
            super::super::record::Foreground::Known(_)
        ),
        published
    );
    let server = store
        .read(&record.binding_id)
        .unwrap()
        .unwrap()
        .ready
        .unwrap();
    let generation = store.generation_directory(&record).unwrap();
    assert!(generation.join("capability").exists());
    if explicit {
        foreground.0.kill().unwrap();
        foreground.0.wait().unwrap();
        control.write_all(b"W").unwrap();
    }
    drop(control);
    launcher.0.kill().unwrap();
    launcher.0.wait().unwrap();
    joined.join().unwrap().unwrap();
    assert!(!generation.exists());
    assert_eq!(
        observe_runtime_process(
            &UnixCommandRunner,
            server.server.pid,
            Instant::now() + START
        )
        .unwrap(),
        ProcessObservation::Gone
    );
    assert_eq!(store.read(&record.binding_id).unwrap().is_some(), !explicit);
    if !explicit {
        assert!(
            !store
                .read(&record.binding_id)
                .unwrap()
                .unwrap()
                .ended(&|process| {
                    observe_runtime_process(
                        &UnixCommandRunner,
                        process.pid(),
                        Instant::now() + START,
                    )
                    .unwrap()
                    .matches(process)
                }),
            "server cleanup alone must not prove foreground end"
        );
        assert!(
            foreground.0.try_wait().unwrap().is_none(),
            "foreground must survive launcher kill"
        );
    }
    assert!(store.read(&independent.binding_id).unwrap() == Some(independent));
}

#[test]
fn startup_failure_cleans_only_owned_record_without_waiting_for_eof() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let record = Record::new(
        "11111111-1111-4111-8111-111111111111",
        &owner(std::process::id()),
    )
    .unwrap();
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    let start = Start {
        resume_session: Some("22222222-2222-4222-8222-222222222222".into()),
        executable: fixture.path.join("absent").as_os_str().as_bytes().to_vec(),
        args: vec![],
        cwd: fixture.path.as_os_str().as_bytes().to_vec(),
    };
    let mut bytes = serde_json::to_vec(&start).unwrap();
    bytes.push(b'\n');
    assert!(
        serve(
            &ServeRequest {
                directory: &fixture.path,
                binding_id: &record.binding_id,
                generation: &record.generation
            },
            Box::new(std::io::Cursor::new(bytes)),
            &mut Vec::new()
        )
        .is_err()
    );
    assert!(store.read(&record.binding_id).unwrap().is_none());
    assert!(!store.generation_directory(&record).unwrap().exists());
}

struct ReadyOutput {
    stream: UnixStream,
    malformed: bool,
}
impl Write for ReadyOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes != b"\n" {
            if self.malformed {
                self.stream.write_all(b"{}")?;
            } else {
                let mut ready: Ready = serde_json::from_slice(bytes)?;
                ready.session = Some("44444444-4444-4444-8444-444444444444".into());
                self.stream.write_all(&serde_json::to_vec(&ready)?)?;
            }
        } else {
            self.stream.write_all(bytes)?;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

#[test]
fn rejected_ready_retires_without_a_foreground_handoff() {
    for malformed in [true, false] {
        let fixture = TestDirectory::new();
        let root = fixture.path.canonicalize().unwrap();
        let store = Store::open(&root).unwrap();
        let record = Record::new(
            "11111111-1111-4111-8111-111111111111",
            &owner(std::process::id()),
        )
        .unwrap();
        store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
        let command = fixture_command(&root);
        let start = Start {
            resume_session: Some("22222222-2222-4222-8222-222222222222".into()),
            executable: command.executable.as_bytes().to_vec(),
            args: command
                .args
                .iter()
                .map(|arg| arg.as_bytes().to_vec())
                .collect(),
            cwd: root.as_os_str().as_bytes().to_vec(),
        };
        let (control, helper) = UnixStream::pair().unwrap();
        helper.set_read_timeout(Some(START)).unwrap();
        let helper_root = root.clone();
        let helper_record = record.clone();
        let joined = thread::spawn(move || {
            let mut output = ReadyOutput {
                stream: helper.try_clone().unwrap(),
                malformed,
            };
            serve(
                &ServeRequest {
                    directory: &helper_root,
                    binding_id: &helper_record.binding_id,
                    generation: &helper_record.generation,
                },
                Box::new(std::io::BufReader::new(helper)),
                &mut output,
            )
        });
        let mut supervisor = Supervisor {
            store,
            record: record.clone(),
            control: Some(control),
            // This test owns the serve thread directly and joins it below.
            job: None,
            command,
            environment: vec![],
            session: None,
        };
        assert!(
            supervisor
                .initialize(&serde_json::to_vec(&start).unwrap(), Instant::now() + START)
                .is_err()
        );
        joined.join().unwrap().unwrap();
        assert!(supervisor.control.is_none());
        assert!(supervisor.store.read(&record.binding_id).unwrap().is_none());
        assert!(
            !supervisor
                .store
                .generation_directory(&record)
                .unwrap()
                .exists()
        );
    }
}

struct FailedReady<'a> {
    store: &'a Store,
    binding: &'a str,
    server: Option<super::super::record::Process>,
    flush_only: bool,
    written: Vec<u8>,
}
impl Write for FailedReady<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.server = Some(
            self.store
                .read(self.binding)?
                .unwrap()
                .ready
                .unwrap()
                .server,
        );
        if self.flush_only {
            self.written.extend_from_slice(bytes);
            Ok(bytes.len())
        } else {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::ErrorKind::BrokenPipe.into())
    }
}

#[test]
fn ready_write_failure_retires_but_post_frame_flush_failure_preserves() {
    for flush_only in [false, true] {
        let fixture = TestDirectory::new();
        let root = fixture.path.canonicalize().unwrap();
        let store = Store::open(&root).unwrap();
        let record = Record::new(
            "11111111-1111-4111-8111-111111111111",
            &owner(std::process::id()),
        )
        .unwrap();
        store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
        let command = fixture_command(&root);
        let start = Start {
            resume_session: Some("22222222-2222-4222-8222-222222222222".into()),
            executable: command.executable.as_bytes().to_vec(),
            args: command
                .args
                .iter()
                .map(|arg| arg.as_bytes().to_vec())
                .collect(),
            cwd: root.as_os_str().as_bytes().to_vec(),
        };
        let mut bytes = serde_json::to_vec(&start).unwrap();
        bytes.push(b'\n');
        let mut output = FailedReady {
            store: &store,
            binding: &record.binding_id,
            server: None,
            flush_only,
            written: Vec::new(),
        };
        let error = serve(
            &ServeRequest {
                directory: &root,
                binding_id: &record.binding_id,
                generation: &record.generation,
            },
            Box::new(std::io::Cursor::new(bytes)),
            &mut output,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        let server = output
            .server
            .expect("failure must occur after endpoint readiness");
        assert_eq!(
            observe_runtime_process(&UnixCommandRunner, server.pid, Instant::now() + START)
                .unwrap(),
            ProcessObservation::Gone
        );
        assert_eq!(
            store.read(&record.binding_id).unwrap().is_some(),
            flush_only
        );
        if flush_only {
            assert_eq!(output.written.last(), Some(&b'\n'));
            let _: Ready = serde_json::from_slice(&output.written).unwrap();
        }
        assert!(!store.generation_directory(&record).unwrap().exists());
    }
}

#[test]
fn fresh_foreground_discovers_once_and_publishes_only_after_admission() {
    let fixture = TestDirectory::new();
    let root = fixture.path.canonicalize().unwrap();
    let store = Store::open(&root).unwrap();
    let record = Record::new(
        "11111111-1111-4111-8111-111111111111",
        &owner(std::process::id()),
    )
    .unwrap();
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    let mut command = fixture_command(&root);
    command.args = vec![
        "-s".into(),
        "read-only".into(),
        "-a".into(),
        "on-request".into(),
    ];
    let mut lease = Lease::from_record(
        store,
        record.clone(),
        &command,
        &root,
        None,
        Instant::now() + START,
    )
    .unwrap();
    assert!(lease.session().is_none());
    assert!(!lease.command().args.iter().any(|a| a == "resume"));
    assert!(lease.command().args.iter().any(|a| a == "--remote"));
    let foreground = ChildGuard(Command::new("/bin/sleep").arg("30").spawn().unwrap());
    let incarnation = owner(foreground.0.id());
    let mut supervisor = Supervisor {
        store: Store::at(&root),
        record: record.clone(),
        control: None,
        job: None,
        command: lease.command().clone(),
        environment: lease.environment().to_vec(),
        session: None,
    };
    supervisor.foreground_started(&incarnation).unwrap();
    assert!(supervisor.record.ready.is_none());
    assert_eq!(
        supervisor.session.as_ref().unwrap().as_str(),
        "22222222-2222-4222-8222-222222222222"
    );
    ChannelEnrollment::foreground_admitted(&mut supervisor, &incarnation).unwrap();
    assert!(supervisor.record.ready.is_some());
    // No endpoint connection or uniqueness check happens after binding; the
    // fixture accepts no further protocol requests after the one-time gate.
    assert!(ChannelEnrollment::foreground_admitted(&mut supervisor, &incarnation).is_err());
    let generation = supervisor
        .store
        .generation_directory(&supervisor.record)
        .unwrap();
    drop(foreground);
    lease.withdraw().unwrap();
    assert!(!generation.exists());
    assert!(supervisor.store.read(&record.binding_id).unwrap().is_none());
}

#[test]
fn fresh_gate_requires_unique_thread_and_explicit_non_ephemeral_metadata() {
    let id = "22222222-2222-4222-8222-222222222222";
    let session = ProviderSessionId::new(id).unwrap();
    assert!(
        loaded_thread(&json!({"result":{"data":[]}}))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        loaded_thread(&json!({"result":{"data":[id]}})).unwrap(),
        Some(session.clone())
    );
    for response in [
        json!({}),
        json!({"result":{"data":[id,id]}}),
        json!({"result":{"data":[{}]}}),
        json!({"result":{"data":["not-a-uuid"]}}),
        json!({"error":{},"result":{"data":[id]}}),
    ] {
        assert!(loaded_thread(&response).is_err());
    }
    let response = json!({"result":{"thread":{"id":id,"cwd":"/owned","ephemeral":false}}});
    assert!(verify_thread(&response, &session, std::path::Path::new("/owned")).is_ok());
    for field in ["id", "cwd", "ephemeral"] {
        let mut malformed = response.clone();
        malformed["result"]["thread"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(verify_thread(&malformed, &session, std::path::Path::new("/owned")).is_err());
    }
    for flag in [json!(true), json!(null), json!("false")] {
        let mut malformed = response.clone();
        malformed["result"]["thread"]["ephemeral"] = flag;
        assert!(verify_thread(&malformed, &session, std::path::Path::new("/owned")).is_err());
    }
    assert!(verify_thread(&response, &session, std::path::Path::new("/other")).is_err());
}
