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
fn owner_socket(pilot: &Pilot) -> UnixListener {
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
    use std::os::fd::AsFd;
    for answer in [None, Some(serde_json::json!({"not":"a message answer"}))] {
        let pilot = Pilot::new();
        let socket = owner_socket(&pilot);
        let raw = serde_json::to_vec(&prepared()).unwrap();
        let expected = raw.clone();
        let peer = std::thread::spawn(move || {
            let mut ready = [nix::poll::PollFd::new(
                socket.as_fd(),
                nix::poll::PollFlags::POLLIN,
            )];
            if nix::poll::poll(&mut ready, 2000u16).unwrap() == 0 {
                return None;
            }
            let (mut stream, _) = socket.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(
                request,
                serde_json::json!({"op":"cloud.message","remote":null,"request":tmt_remote::canonical::base64url(&expected)})
            );
            if let Some(answer) = answer {
                writeln!(stream, "{answer}").unwrap();
            }
            drop(stream);
            socket.set_nonblocking(true).unwrap();
            assert!(
                matches!(socket.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
                "one submission only"
            );
            Some(request)
        });
        let output = message(&pilot, &raw);
        let observed = peer.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            serde_json::json!({"version":1,"state":"unconfirmed","remote":"peer","target":{"name":"target"},"reason":"REMOTE_CLOUD_UNCONFIRMED"})
        );
        assert!(
            observed.is_some(),
            "the public command must reach its serving owner"
        );
        assert!(output.stderr.is_empty());
        assert!(
            !pilot.root.join("state/remote/remote.db").exists(),
            "CLI never opens the serving Store"
        );
    }
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
