//! Public fake-Core caller: provisional requests cannot bootstrap or invoke old Core.
use super::*;
#[test]
fn prepared_cloud_command_is_bounded_json_and_closed_before_enrollment() {
    let p = Pilot::new();
    let valid = serde_json::json!({"version":1,"remote":null,"createdAtMs":1800000000000_u64,"requestId":"44444444-4444-4444-8444-444444444444","from":{"id":"33333333-3333-4333-8333-333333333333","name":"sender"},"target":{"id":"66666666-6666-4666-8666-666666666666","name":"target"},"message":"plaintext-message-must-not-leak"});
    for (value, code) in [
        (valid.clone(), "REMOTE_CLOUD_UPGRADE_REQUIRED"),
        (
            {
                let mut v = valid.clone();
                v["target"].as_object_mut().unwrap().remove("id");
                v
            },
            "REMOTE_CLOUD_UPGRADE_REQUIRED",
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
        !p.root.join("calls").exists(),
        "no Core fallback, identity lookup or dispatch"
    );
    assert!(
        !p.root.join("state").exists(),
        "no enrollment bootstrap or host state discovery"
    );
}
