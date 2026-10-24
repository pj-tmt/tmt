use super::*;
use crate::{process::UnixCommandRunner, test_support::TestDirectory};
use std::{fs, time::Duration};

const MESSAGE: RemoteMessage<'static> = RemoteMessage {
    remote: "bob",
    request_uuid: "55555555-5555-4555-8555-555555555555",
    created_at_ms: 1_700_000_000_000,
    from_id: "11111111-1111-4111-8111-111111111111",
    from_name: "Ben",
    target_id: None,
    target_name: "reviewer",
    message: "hello",
    reply_request_id: "req_55555555-5555-4555-8555-555555555555",
    reply_receipt: "v2_receipt",
};

/// A script standing in for `tmt-remote`: it records its argv and stdin, then runs `body`.
fn script(directory: &std::path::Path, body: &str) -> std::path::PathBuf {
    let path = directory.join("tmt-remote");
    tmt_test_support::write_executable(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s ' \"$@\" > \"{dir}/argv\"\ncat > \"{dir}/stdin\"\n{body}\n",
            dir = directory.display()
        )
        .as_bytes(),
        0o755,
    )
    .unwrap();
    path
}

fn run(body: &str, message: &RemoteMessage<'_>) -> (RemoteMessageOutcome, TestDirectory) {
    let directory = TestDirectory::new();
    let program = script(&directory.path, body);
    let outcome = send(
        &UnixCommandRunner,
        program.as_os_str(),
        message,
        Instant::now() + Duration::from_secs(10),
    );
    (outcome, directory)
}

fn answer(state: &str) -> String {
    format!(
        "printf '%s' '{{\"version\":1,\"state\":\"{state}\",\"remote\":\"bob\",\"target\":{{\"name\":\"reviewer\"}}}}'"
    )
}

#[test]
fn the_request_is_one_json_document_on_stdin_and_the_command_is_message_json() {
    let (outcome, directory) = run(&answer("sent"), &MESSAGE);
    assert_eq!(outcome, RemoteMessageOutcome::Sent);
    assert_eq!(
        fs::read_to_string(directory.path.join("argv")).unwrap(),
        "message --json "
    );
    let stdin: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(directory.path.join("stdin")).unwrap()).unwrap();
    assert_eq!(
        stdin,
        serde_json::json!({
            "version": 1,
            "remote": "bob",
            "requestId": "55555555-5555-4555-8555-555555555555",
            "createdAtMs": 1_700_000_000_000u64,
            "from": {"id": "11111111-1111-4111-8111-111111111111", "name": "Ben"},
            "target": {"name": "reviewer"},
            "message": "hello",
            "reply": {"requestId": "req_55555555-5555-4555-8555-555555555555", "receipt": "v2_receipt"},
        })
    );
}

#[test]
fn a_target_named_by_uuid_carries_its_id_and_expected_name() {
    let message = RemoteMessage {
        target_id: Some("7f0c2f2e-2b1a-4c1e-9d2a-0a1b2c3d4e5f"),
        ..MESSAGE
    };
    let (outcome, directory) = run(&answer("held"), &message);
    assert_eq!(outcome, RemoteMessageOutcome::Held);
    let stdin: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(directory.path.join("stdin")).unwrap()).unwrap();
    assert_eq!(
        stdin["target"],
        serde_json::json!({"id": "7f0c2f2e-2b1a-4c1e-9d2a-0a1b2c3d4e5f", "name": "reviewer"})
    );
}

#[test]
fn every_answer_state_maps_and_only_a_fixed_reason_is_kept() {
    for (state, reason, expected) in [
        ("sent", "", RemoteMessageOutcome::Sent),
        ("held", "", RemoteMessageOutcome::Held),
        (
            "refused",
            "REMOTE_AGENT_NOT_FOUND",
            RemoteMessageOutcome::Refused {
                reason: Some("REMOTE_AGENT_NOT_FOUND".into()),
            },
        ),
        (
            "unconfirmed",
            "REMOTE_CLOUD_UNCONFIRMED",
            RemoteMessageOutcome::Unconfirmed {
                reason: Some("REMOTE_CLOUD_UNCONFIRMED".into()),
            },
        ),
        (
            "unavailable",
            "not a fixed code",
            RemoteMessageOutcome::Unavailable { reason: None },
        ),
    ] {
        let body = format!(
            "printf '%s' '{{\"version\":1,\"state\":\"{state}\",\"remote\":\"bob\",\"target\":{{\"name\":\"reviewer\"}}{reason}}}'",
            reason = if reason.is_empty() {
                String::new()
            } else {
                format!(",\"reason\":\"{reason}\"")
            }
        );
        let (outcome, _directory) = run(&body, &MESSAGE);
        assert_eq!(outcome, expected, "{state}");
    }
}

#[test]
fn an_error_envelope_with_a_nonzero_exit_means_nothing_was_sent() {
    let (outcome, _directory) = run(
        "printf '%s' '{\"error\":{\"code\":\"REMOTE_CLOUD_MESSAGE_TOO_LARGE\",\"message\":\"x\"}}'; exit 1",
        &MESSAGE,
    );
    assert_eq!(
        outcome,
        RemoteMessageOutcome::NotSent {
            code: "REMOTE_CLOUD_MESSAGE_TOO_LARGE".into()
        }
    );
}

#[test]
fn anything_that_may_have_sent_is_unknown_and_never_retried() {
    for body in [
        "exit 0",
        "printf '%s' 'not json'",
        "printf '%s' '{\"version\":2,\"state\":\"sent\",\"remote\":\"bob\"}'",
        "printf '%s' '{\"version\":1,\"state\":\"sent\",\"remote\":\"other\"}'",
        "printf '%s' '{\"version\":1,\"state\":\"maybe\",\"remote\":\"bob\"}'",
        "printf '%s' '{\"error\":{\"code\":\"REMOTE_X\"}}'",
        "printf '%s' '{\"error\":{\"code\":\"lower case\"}}'; exit 1",
        "exit 3",
    ] {
        let (outcome, _directory) = run(body, &MESSAGE);
        assert!(
            matches!(outcome, RemoteMessageOutcome::Unknown { .. }),
            "{body}: {outcome:?}"
        );
    }
}

#[test]
fn a_command_that_does_not_answer_in_time_is_unknown() {
    let directory = TestDirectory::new();
    let program = script(&directory.path, "sleep 30");
    let outcome = send(
        &UnixCommandRunner,
        program.as_os_str(),
        &MESSAGE,
        Instant::now() + Duration::from_millis(300),
    );
    assert!(matches!(outcome, RemoteMessageOutcome::Unknown { .. }));
}

#[test]
fn a_program_that_cannot_start_sent_nothing() {
    let outcome = send(
        &UnixCommandRunner,
        OsStr::new("/nonexistent/tmt-remote"),
        &MESSAGE,
        Instant::now() + Duration::from_secs(5),
    );
    assert_eq!(
        outcome,
        RemoteMessageOutcome::NotSent {
            code: "REMOTE_UNAVAILABLE".into()
        }
    );
}
