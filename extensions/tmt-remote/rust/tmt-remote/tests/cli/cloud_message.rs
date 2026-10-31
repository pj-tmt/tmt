//! Public prepared-message adapter and its owner-only control handoff.
use super::*;
fn prepared() -> Value {
    serde_json::json!({"version":1,"remote":"peer","createdAtMs":1800000000000_u64,"requestId":"44444444-4444-4444-8444-444444444444","from":{"id":"33333333-3333-4333-8333-333333333333","name":"sender"},"target":{"name":"target"},"message":"plaintext-message-must-not-leak"})
}
fn message(pilot: &Pilot, raw: &[u8]) -> std::process::Output {
    let mut child = pilot
        .command()
        .args(["message", "--json", "--no-open"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(raw).unwrap();
    reap_until(&mut child, Instant::now() + Duration::from_secs(10))
        .expect("bounded message command");
    child.wait_with_output().unwrap()
}
pub(super) fn owner_socket(pilot: &Pilot) -> UnixListener {
    let directory = pilot.root.join("state/remote");
    fs::create_dir_all(&directory).unwrap();
    let socket = UnixListener::bind(directory.join("control.sock")).unwrap();
    fs::set_permissions(
        directory.join("control.sock"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    socket
}
// The command owns the wait budget. A byte also wakes the peer if another
// parallel fork briefly retains a completion descriptor before exec.
struct CommandFinished(UnixStream);
impl Drop for CommandFinished {
    fn drop(&mut self) {
        let _ = self.0.write_all(b"done");
    }
}
fn with_message_owner<T>(
    socket: UnixListener,
    raw: &[u8],
    answer: Option<&Value>,
    command: impl FnOnce() -> T,
) -> (T, Vec<Value>) {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use std::os::fd::AsFd;
    socket.set_nonblocking(true).unwrap();
    std::thread::scope(|scope| {
        let (finished, until_finished) = UnixStream::pair().unwrap();
        let completion = CommandFinished(finished);
        let peer = scope.spawn(move || {
            let mut observed = Vec::new();
            loop {
                let mut ready = [
                    PollFd::new(socket.as_fd(), PollFlags::POLLIN),
                    PollFd::new(until_finished.as_fd(), PollFlags::POLLIN),
                ];
                poll(&mut ready, PollTimeout::NONE).unwrap();
                let completed = ready[1]
                    .revents().unwrap()
                    .intersects(PollFlags::POLLIN | PollFlags::POLLHUP);
                // Drain queued submissions before completion, including a second
                // connection made after the first answer was lost.
                loop {
                    let (mut stream, _) = match socket.accept() {
                        Ok(accepted) => accepted,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) => panic!("fake owner accept: {error}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut line = String::new();
                    BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
                    let request: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(request, serde_json::json!({"op":"cloud.message","remote":null,"request":tmt_remote::canonical::base64url(raw)}));
                    if let Some(answer) = answer {
                        writeln!(stream, "{answer}").unwrap();
                    }
                    observed.push(request);
                }
                if completed {
                    return observed;
                }
            }
        });
        let output = command();
        // Drop also signals cancellation on an assertion/spawn failure; scoped
        // joining then closes the owner before Pilot removes its private root.
        drop(completion);
        (output, peer.join().unwrap())
    })
}
fn assert_one_submission(observed: &[Value]) {
    assert!(
        !observed.is_empty(),
        "the public command must reach its serving owner"
    );
    assert_eq!(observed.len(), 1, "one submission only");
}
#[test]
fn prepared_cloud_message_preserves_owner_outcomes_and_rejects_a_changed_echo() {
    let base =
        serde_json::json!({"version":1,"state":"sent","remote":"peer","target":{"name":"target"}});
    for (answer, expected_state, expected_code) in [
        (base.clone(), Some("sent"), None),
        (
            {
                let mut a = base.clone();
                a["state"] = "held".into();
                a
            },
            Some("held"),
            None,
        ),
        (
            {
                let mut a = base.clone();
                a["state"] = "refused".into();
                a["reason"] = "REMOTE_SCOPE_DENIED".into();
                a
            },
            Some("refused"),
            None,
        ),
        (
            {
                let mut a = base.clone();
                a["remote"] = "other".into();
                a
            },
            Some("unconfirmed"),
            None,
        ),
        (
            {
                let mut a = base.clone();
                a["target"]["name"] = "other".into();
                a
            },
            Some("unconfirmed"),
            None,
        ),
        (
            {
                let mut a = base.clone();
                a["version"] = 2.into();
                a
            },
            Some("unconfirmed"),
            None,
        ),
        (
            {
                let mut a = base.clone();
                a["reason"] = "provider text".into();
                a
            },
            Some("unconfirmed"),
            None,
        ),
        (
            serde_json::json!({"error":{"code":"REMOTE_CONTROL_UNSUPPORTED","message":"Unknown control operation."}}),
            None,
            Some("REMOTE_SERVE_OUTDATED"),
        ),
        (
            serde_json::json!({"error":{"code":"REMOTE_BUDGET_EXHAUSTED","message":"Before mutation"}}),
            None,
            Some("REMOTE_BUDGET_EXHAUSTED"),
        ),
    ] {
        let pilot = Pilot::new();
        let socket = owner_socket(&pilot);
        let expected = serde_json::to_vec(&prepared()).unwrap();
        let raw = expected.clone();
        let owner = std::thread::spawn(move || {
            let (mut stream, _) = socket.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let frame: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(
                frame["request"],
                tmt_remote::canonical::base64url(&expected)
            );
            writeln!(stream, "{answer}").unwrap();
        });
        let output = message(&pilot, &raw);
        owner.join().unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.success(), expected_code.is_none(), "{value}");
        if let Some(state) = expected_state {
            assert_eq!(value["state"], state);
        }
        if let Some(code) = expected_code {
            assert_eq!(value["error"]["code"], code);
        }
        assert!(output.stderr.is_empty());
    }
}
#[test]
fn prepared_cloud_message_selector_and_unsafe_socket_refuse_before_submission() {
    let pilot = Pilot::new();
    let raw = prepared().to_string();
    let mut cmd = pilot.command();
    let mut child = cmd
        .args(["--remote", "other", "message", "--json", "--no-open"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(raw.as_bytes())
        .unwrap();
    reap_until(&mut child, Instant::now() + Duration::from_secs(5)).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
        "REMOTE_SELECTOR_INVALID"
    );
    assert!(!pilot.root.join("calls").exists());
    let socket = owner_socket(&pilot);
    fs::set_permissions(
        pilot.root.join("state/remote/control.sock"),
        fs::Permissions::from_mode(0o660),
    )
    .unwrap();
    let output = message(&pilot, raw.as_bytes());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "REMOTE_STATE_UNSAFE"
    );
    socket.set_nonblocking(true).unwrap();
    assert!(matches!(socket.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
}
#[test]
fn prepared_cloud_message_lost_owner_answer_is_unconfirmed_without_another_submission() {
    for answer in [None, Some(serde_json::json!({"not":"a message answer"}))] {
        let pilot = Pilot::new();
        let socket = owner_socket(&pilot);
        let raw = serde_json::to_vec(&prepared()).unwrap();
        let (output, observed) =
            with_message_owner(socket, &raw, answer.as_ref(), || message(&pilot, &raw));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            serde_json::json!({"version":1,"state":"unconfirmed","remote":"peer","target":{"name":"target"},"reason":"REMOTE_CLOUD_UNCONFIRMED"})
        );
        assert_one_submission(&observed);
        assert!(output.stderr.is_empty());
        assert!(
            !pilot.root.join("state/remote/remote.db").exists(),
            "CLI never opens the serving Store"
        );
    }
}
#[test]
fn lost_answer_fixture_detects_missing_and_extra_submissions_through_completion() {
    for count in [0, 1, 2] {
        let pilot = Pilot::new();
        let socket = owner_socket(&pilot);
        let path = pilot.root.join("state/remote/control.sock");
        let raw = serde_json::to_vec(&prepared()).unwrap();
        let (_, observed) = with_message_owner(socket, &raw, None, || {
            for _ in 0..count {
                let mut stream = UnixStream::connect(&path).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                writeln!(stream, "{}", serde_json::json!({"op":"cloud.message","remote":null,"request":tmt_remote::canonical::base64url(&raw)})).unwrap();
                let mut answer = Vec::new();
                stream.read_to_end(&mut answer).unwrap();
                assert!(answer.is_empty());
            }
        });
        assert_eq!(observed.len(), count);
        let checked = std::panic::catch_unwind(|| assert_one_submission(&observed));
        if count == 1 {
            assert!(checked.is_ok());
        } else {
            let panic =
                checked.expect_err("missing or extra submission must fail the scenario guard");
            let text = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap();
            assert!(text.contains(if count == 0 {
                "must reach its serving owner"
            } else {
                "one submission only"
            }));
        }
        assert!(
            matches!(UnixStream::connect(&path), Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused),
            "fake owner must be closed after command completion"
        );
    }
}
#[test]
fn lost_answer_fixture_closes_its_owner_when_the_command_unwinds() {
    let pilot = Pilot::new();
    let socket = owner_socket(&pilot);
    let raw = serde_json::to_vec(&prepared()).unwrap();
    let failed = std::panic::catch_unwind(|| {
        with_message_owner(socket, &raw, None, || {
            panic!("fixture command failed before submission")
        });
    })
    .expect_err("command failure must propagate after owner cleanup");
    assert_eq!(
        failed.downcast_ref::<&str>(),
        Some(&"fixture command failed before submission")
    );
    assert!(
        matches!(UnixStream::connect(pilot.root.join("state/remote/control.sock")), Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused),
        "fake owner must close before fixture deletion on unwind"
    );
}
#[test]
fn prepared_cloud_command_is_bounded_json_and_never_bootstraps_enrollment() {
    let p = Pilot::new();
    let valid = serde_json::json!({"version":1,"remote":"peer","createdAtMs":1800000000000_u64,"requestId":"44444444-4444-4444-8444-444444444444","from":{"id":"33333333-3333-4333-8333-333333333333","name":"sender"},"target":{"id":"66666666-6666-4666-8666-666666666666","name":"target"},"message":"plaintext-message-must-not-leak"});
    for (value, code) in [
        (valid.clone(), "REMOTE_NOT_RUNNING"),
        (
            {
                let mut v = valid.clone();
                v["target"].as_object_mut().unwrap().remove("id");
                v
            },
            "REMOTE_NOT_RUNNING",
        ),
        (
            {
                let mut v = valid.clone();
                v["target"]["id"] = Value::Null;
                v
            },
            "REMOTE_INPUT_INVALID",
        ),
        (
            {
                let mut v = valid.clone();
                v["target"]["name"] = serde_json::json!("");
                v
            },
            "REMOTE_INPUT_INVALID",
        ),
        (
            {
                let mut v = valid.clone();
                v["version"] = serde_json::json!(2);
                v
            },
            "REMOTE_CLOUD_UPGRADE_REQUIRED",
        ),
        (
            {
                let mut v = valid.clone();
                v["message"] =
                    serde_json::json!("x".repeat(tmt_remote::limits::CLOUD_MESSAGE_BYTES + 1));
                v
            },
            "REMOTE_CLOUD_MESSAGE_TOO_LARGE",
        ),
        (
            {
                let mut v = valid.clone();
                v.as_object_mut().unwrap().remove("remote");
                v
            },
            "REMOTE_INPUT_INVALID",
        ),
    ] {
        let mut child = p
            .command()
            .args(["message", "--json", "--no-open"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(value.to_string().as_bytes()).unwrap();
        drop(stdin);
        reap_until(&mut child, Instant::now() + Duration::from_secs(10))
            .expect("bounded public command");
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(output.stderr.is_empty(), "{:?}", output.stderr);
        let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(answer["error"]["code"], code);
        assert!(
            !String::from_utf8(output.stdout)
                .unwrap()
                .contains("plaintext-message-must-not-leak")
        );
    }
    assert!(
        fs::read_to_string(p.root.join("calls"))
            .unwrap()
            .lines()
            .all(|line| line == "api"),
        "only public storage-root discovery, no identity lookup or dispatch"
    );
    assert!(
        !p.root.join("state").exists(),
        "no enrollment bootstrap or host state discovery"
    );
}
