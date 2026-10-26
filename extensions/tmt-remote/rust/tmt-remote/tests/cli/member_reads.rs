//! The real private control socket consumes the same roster, never a second Store opener.
use super::*;

fn request(pilot: &Pilot, value: Value) -> Value {
    let mut stream = UnixStream::connect(pilot.root.join("state/remote/control.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    writeln!(stream, "{value}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
pub(super) fn assert_shared_member_reads(pilot: &Pilot, client: &str) {
    let calls = fs::read(pilot.root.join("calls")).ok();
    let roster = request(
        pilot,
        serde_json::json!({"op":"members.list","cursor":null,"limit":50}),
    );
    assert_eq!(roster["administrator"], true);
    assert_eq!(roster["members"].as_array().unwrap().len(), 1);
    let member = roster["members"][0]["memberId"].clone();
    let machines = request(
        pilot,
        serde_json::json!({"op":"members.machines","memberId":member,"cursor":null,"limit":50}),
    );
    assert_eq!(machines["machines"].as_array().unwrap().len(), 1);
    assert_eq!(machines["machines"][0]["clientId"], client);
    assert_eq!(machines["machines"][0]["liveSessionCount"], 0);
    assert!(machines["machines"][0]["lastActivityAtMs"].is_null());
    assert_eq!(
        request(
            pilot,
            serde_json::json!({"op":"members.list","cursor":null,"limit":50,"admin":true})
        )["error"]["code"],
        "REMOTE_INPUT_INVALID"
    );
    assert_eq!(
        request(
            pilot,
            serde_json::json!({"op":"members.rename","memberId":member,"name":"No terminal mutation"})
        )["error"]["code"],
        "REMOTE_CONTROL_UNSUPPORTED"
    );
    assert_eq!(
        fs::read(pilot.root.join("calls")).ok(),
        calls,
        "member control reads never ask Core"
    );
}
