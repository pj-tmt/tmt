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
pub(super) fn assert_shared_member_reads(pilot: &Pilot, client: &str, expected_member: &str) {
    let calls = fs::read(pilot.root.join("calls")).ok();
    let roster = request(
        pilot,
        serde_json::json!({"op":"members.list","cursor":null,"limit":50}),
    );
    assert_eq!(roster["administrator"], true);
    assert_eq!(roster["members"].as_array().unwrap().len(), 1);
    let member = roster["members"][0]["memberId"].clone();
    assert_eq!(member, expected_member);
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

pub(super) fn assert_public_member_reads(pilot: &Pilot, client: &str, running: bool) {
    let invoke = |words: &[&str]| {
        let output = pilot.command().args(words).output().unwrap();
        assert!(
            output.status.success(),
            "{words:?}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let legacy_before = invoke(&["devices", "--json"]).stdout;
    let roster: Value = serde_json::from_slice(&invoke(&["members", "--json"]).stdout).unwrap();
    assert_eq!(roster["administrator"], true);
    let rows = roster["members"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let member = rows[0]["memberId"].as_str().unwrap();
    let name = rows[0]["displayName"].as_str().unwrap();
    let by_id: Value =
        serde_json::from_slice(&invoke(&["members", "machines", member, "--json"]).stdout).unwrap();
    let by_name: Value =
        serde_json::from_slice(&invoke(&["members", "--json", "machines", name]).stdout).unwrap();
    assert_eq!(by_name, by_id);
    assert_eq!(by_id["machines"].as_array().unwrap().len(), 1);
    assert_eq!(by_id["machines"][0]["clientId"], client);
    if running {
        assert_eq!(by_id["machines"][0]["liveSessionCount"], 0);
    } else {
        assert!(by_id["machines"][0]["liveSessionCount"].is_null());
    }
    let human = String::from_utf8(invoke(&["members"]).stdout).unwrap();
    assert!(human.contains("MEMBERS 1"));
    assert!(human.contains(name) && human.contains(member));
    let missing = pilot
        .command()
        .args(["members", "machines", "Missing member", "--json"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let missing: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(missing["error"]["code"], "REMOTE_MEMBER_NOT_FOUND");
    let legacy_after = invoke(&["devices", "--json"]).stdout;
    assert_eq!(
        legacy_after, legacy_before,
        "new reads preserve exact legacy device JSON bytes"
    );
}
