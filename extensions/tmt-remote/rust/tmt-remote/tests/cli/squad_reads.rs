//! Real public CLI/private-socket reads: no provider or Core effect behind an observation.
use super::*;
fn observe(pilot: &Pilot, args: &[&str], expected: Value, answer: Value) -> std::process::Output {
    let socket = super::cloud_message::owner_socket(pilot);
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = socket.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), expected);
        writeln!(stream, "{answer}").unwrap();
        socket.set_nonblocking(true).unwrap();
        assert!(matches!(socket.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
    });
    let output = pilot.command().args(args).output().unwrap();
    peer.join().unwrap();
    output
}
#[test]
fn squad_directory_reads_a_bounded_cache_reply_larger_than_an_ordinary_control_frame() {
    let pilot = Pilot::new();
    let members: Vec<_>=(1..=200).map(|i|serde_json::json!({"id":format!("11111111-1111-4111-8111-{i:012x}"),"name":"worker".repeat(9),"state":"waiting-ci","presence":"offline"})).collect();
    let summary = serde_json::json!({"version":1,"machineName":"Other machine","observedAt":1,"status":"available","squads":[{"squad":"Remote","members":members}]});
    assert!(serde_json::to_vec(&summary).unwrap().len() <= tmt_remote::limits::SQUAD_SUMMARY_BYTES);
    let mut machines: Vec<_> = (0..30).map(|i| serde_json::json!({"remote":format!("peer-{i}"),"status":"available","summary":summary})).collect();
    machines.push(serde_json::json!({"remote":"older","status":"incompatible"}));
    machines.push(serde_json::json!({"remote":"missing","status":"unavailable"}));
    let answer = serde_json::json!({"version":1,"machines":machines});
    assert!(
        serde_json::to_vec(&answer).unwrap().len() > tmt_remote::limits::CLOUD_PAIR_CONTROL_BYTES
    );
    let output = observe(
        &pilot,
        &["squads", "--json"],
        serde_json::json!({"op":"squads.list"}),
        answer.clone(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        answer
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}
#[test]
fn delivery_observes_each_original_state_once_and_rejects_changed_identity_or_raw_reasons() {
    let operation = "44444444-4444-4444-8444-444444444444";
    for state in [
        "queued",
        "delivered",
        "answered",
        "failed",
        "held",
        "unconfirmed",
        "unknown",
    ] {
        let pilot = Pilot::new();
        let answer = serde_json::json!({"version":1,"operationId":operation,"state":state});
        let output = observe(
            &pilot,
            &["delivery", operation, "--remote", "peer", "--json"],
            serde_json::json!({"op":"squads.delivery","remote":"peer","operationId":operation}),
            answer.clone(),
        );
        assert!(
            output.status.success(),
            "{state}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            answer
        );
    }
    for answer in [
        serde_json::json!({"version":1,"operationId":"55555555-5555-4555-8555-555555555555","state":"delivered"}),
        serde_json::json!({"version":1,"operationId":operation,"state":"failed","reason":"PRIVATE provider detail"}),
    ] {
        let pilot = Pilot::new();
        let output = observe(
            &pilot,
            &["delivery", operation, "--remote", "peer", "--json"],
            serde_json::json!({"op":"squads.delivery","remote":"peer","operationId":operation}),
            answer,
        );
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE"));
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
            "REMOTE_INPUT_INVALID"
        );
        let calls = fs::read_to_string(pilot.root.join("calls")).unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            ["api"],
            "a malformed owner reply follows exactly one discovery"
        );
    }
}
#[test]
fn squad_read_invalid_selectors_refuse_before_owner_discovery() {
    let pilot = Pilot::new();
    for args in [
        vec!["delivery", "not-a-uuid", "--remote", "peer", "--json"],
        vec!["delivery", "44444444-4444-4444-8444-444444444444", "--json"],
        vec!["squads", "--remote", "peer", "--json"],
    ] {
        let output = pilot.command().args(args).output().unwrap();
        assert!(!output.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
            "REMOTE_INPUT_INVALID"
        );
        assert!(
            !pilot.root.join("calls").exists(),
            "invalid input cannot discover Core storage"
        );
    }
}
