//! Hidden activation protocol through the real CLI, with private fake-core/socket owners.
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};
const GENERATION: &str = "efdc7a11-b37e-4b86-8baf-c6f613b0df91";
fn deadline() -> String {
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        + 5000)
        .to_string()
}
fn activate(pilot: &Pilot, until: &str) -> std::process::Output {
    pilot
        .command()
        .args([
            "__activate",
            "--extension",
            "colab",
            "--deadline-ms",
            until,
            "--json",
        ])
        .output()
        .unwrap()
}
fn listener(pilot: &Pilot) -> UnixListener {
    let directory = pilot.root.join("state/remote");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    listener
}
#[test]
fn hidden_native_activation_refuses_expired_deadline_without_io() {
    let pilot = Pilot::new();
    let output = activate(&pilot, "1");
    assert!(!output.status.success());
    let answer: Value = serde_json::from_slice(&output.stdout).expect("one typed JSON refusal");
    assert_eq!(answer["error"]["code"], "REMOTE_STATE_UNAVAILABLE");
    assert_eq!(answer["error"]["state"], "deadline");
    assert!(
        !pilot.root.join("calls").exists(),
        "spent deadline must not invoke core"
    );
    assert!(
        !pilot.root.join("state").exists(),
        "activation must not create state"
    );
}
#[test]
fn hidden_activation_is_one_control_exchange_with_typed_refusal_parity() {
    for (reply, code) in [
        (
            serde_json::json!({"activated":true,"version":3,"generation":GENERATION}),
            None,
        ),
        (
            serde_json::json!({"error":{"code":"REMOTE_CONTROL_UNSUPPORTED","message":"Unsupported"}}),
            Some("REMOTE_SERVE_OUTDATED"),
        ),
        (
            serde_json::json!({"error":{"code":"REMOTE_INPUT_INVALID","message":"Unknown control operation."}}),
            Some("REMOTE_SERVE_OUTDATED"),
        ),
        (
            serde_json::json!({"error":{"code":"REMOTE_SCOPE_DENIED","message":"Undeclared extension"}}),
            Some("REMOTE_SCOPE_DENIED"),
        ),
        (
            serde_json::json!({"error":{"code":"REMOTE_STATE_UNAVAILABLE","message":"No factory"}}),
            Some("REMOTE_STATE_UNAVAILABLE"),
        ),
        (
            serde_json::json!({"activated":true,"version":3,"generation":"bad"}),
            Some("REMOTE_IO"),
        ),
    ] {
        let pilot = Pilot::new();
        let socket = listener(&pilot);
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = socket.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&line).unwrap(),
                serde_json::json!({"op":"sync.activate","extension":"colab","version":3})
            );
            writeln!(stream, "{reply}").unwrap();
        });
        let output = activate(&pilot, &deadline());
        peer.join().unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        if let Some(code) = code {
            assert!(!output.status.success());
            assert_eq!(value["error"]["code"], code);
        } else {
            assert!(output.status.success());
            assert_eq!(value, serde_json::json!({"generation":GENERATION}));
        }
        assert!(!pilot.root.join("state/remote/remote.db").exists());
        assert!(!pilot.root.join("state/remote/serve.lock").exists());
    }
}
#[test]
fn hidden_activation_absence_and_unsafe_socket_never_initialize_or_connect() {
    let pilot = Pilot::new();
    let output = activate(&pilot, &deadline());
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "REMOTE_NOT_RUNNING"
    );
    assert!(!pilot.root.join("state").exists());
    let socket = listener(&pilot);
    fs::set_permissions(
        pilot.root.join("state/remote/control.sock"),
        fs::Permissions::from_mode(0o660),
    )
    .unwrap();
    let output = activate(&pilot, &deadline());
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "REMOTE_STATE_UNSAFE"
    );
    socket.set_nonblocking(true).unwrap();
    assert!(matches!(socket.accept(), Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
}
#[test]
fn control_activation_deadline_closes_the_single_silent_connection() {
    let pilot = Pilot::new();
    let socket = listener(&pilot);
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = socket.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap()["op"],
            "sync.activate"
        );
    });
    let start = Instant::now();
    let result = tmt_remote::control::activate_native_until(
        &pilot.root.join("state/remote"),
        "colab",
        start + Duration::from_millis(100),
    );
    assert!(matches!(
        result,
        Err(tmt_remote::control::ActivationFailure::Deadline)
    ));
    peer.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn activation_protocol_is_absent_from_public_help() {
    let pilot = Pilot::new();
    let help = pilot.command().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(
        !String::from_utf8(help.stdout)
            .unwrap()
            .contains("__activate")
    );
}
