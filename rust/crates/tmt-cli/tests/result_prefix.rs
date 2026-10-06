//! Result selection preserves exact public output under isolated SQLite state.
mod support;

use std::{
    fs,
    path::PathBuf,
    process::{Child, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tmt_adapters::{request_runtime::wall_time_ms, storage::Storage};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{
        Originator, PrepareRequest, RequestKind, RequestRoute, RequestService, ResponseProof,
        SubmitResponse, correlation,
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    child: Option<Child>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-result-prefix-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root, child: None }
    }
    fn seed(&self, id: &str, kind: RequestKind, body: Option<&str>) {
        let mut storage =
            Storage::open(support::state_dir(&self.root).join("tmux-team.db")).unwrap();
        let recipient = create_or_resolve(&mut storage, "Receiver", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        let route = RequestRoute::Inbox {
            recipient_identity_id: recipient.clone(),
        };
        let attempt = format!("attempt-{id}");
        let mut service = RequestService::new(&mut storage, wall_time_ms);
        service
            .enqueue(
                PrepareRequest {
                    kind,
                    room_id: None,
                    request_id: id.into(),
                    message: "original request".into(),
                    route: route.clone(),
                    wait: false,
                    expires_at_ms: wall_time_ms() + 60_000,
                    originator: Originator::Unknown,
                    recipient_identity_id: Some(recipient),
                    preamble: None,
                },
                attempt.clone(),
                7,
            )
            .unwrap();
        if let Some(body) = body {
            service
                .submit_response(SubmitResponse {
                    request_id: id.into(),
                    proof: ResponseProof::Compact(correlation::response_token(
                        id, &attempt, &route,
                    )),
                    body: body.into(),
                })
                .unwrap();
        }
        storage.close().unwrap();
    }
    fn run(&mut self, args: &[&str]) -> Output {
        self.child = Some(
            support::command(&self.root, args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        support::wait(&mut self.child, Duration::from_secs(10))
            .wait_with_output()
            .unwrap()
    }
    fn json(&mut self, args: &[&str]) -> serde_json::Value {
        let output = self.run(args);
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn native_withdrawal_preserves_history_excludes_inbox_and_enforces_reply_races() {
    let mut fixture = Fixture::new();
    for name in ["Sender", "Receiver", "Stranger"] {
        fixture.json(&["identity", "create", name, "--json"]);
    }
    let sent = fixture.json(&[
        "talk",
        "Receiver",
        "original prompt",
        "--inbox",
        "--detach",
        "--identity",
        "Sender",
        "--json",
    ]);
    let id = sent["requestId"].as_str().unwrap();
    let incoming = fixture.json(&[
        "x",
        "show",
        id,
        "--incoming",
        "--identity",
        "Receiver",
        "--json",
    ]);
    let receipt = incoming["exchange"]["reply"]["receipt"].as_str().unwrap();
    let wrong_owner = fixture.run(&[
        "x",
        "withdraw",
        id,
        "--reason",
        "obsolete",
        "--identity",
        "Stranger",
        "--json",
    ]);
    assert_eq!(wrong_owner.status.code(), Some(5));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&wrong_owner.stdout).unwrap()["error"]["code"],
        "REQUEST_ORIGINATOR_MISMATCH"
    );
    assert_eq!(
        fixture.json(&["inbox", "--identity", "Receiver", "--json"])["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let withdrawn = fixture.json(&[
        "x",
        "withdraw",
        id,
        "--reason",
        "already merged",
        "--identity",
        "Sender",
        "--json",
    ]);
    assert_eq!(withdrawn["status"], "withdrawn");
    assert_eq!(withdrawn["changed"], true);
    assert_eq!(withdrawn["reason"], "already merged");
    assert!(withdrawn["withdrawnAtMs"].as_u64().unwrap() > 0);
    let repeated = fixture.json(&[
        "x",
        "withdraw",
        id,
        "--reason",
        "already merged",
        "--identity",
        "Sender",
        "--json",
    ]);
    assert_eq!(repeated["changed"], false);
    assert_eq!(repeated["withdrawnAtMs"], withdrawn["withdrawnAtMs"]);
    let conflict = fixture.run(&[
        "x",
        "withdraw",
        id,
        "--reason",
        "different",
        "--identity",
        "Sender",
        "--json",
    ]);
    assert_eq!(conflict.status.code(), Some(5));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&conflict.stdout).unwrap()["error"]["code"],
        "REQUEST_WITHDRAWAL_CONFLICT"
    );
    let result = fixture.json(&["result", id, "--json"]);
    assert_eq!(result["status"], "withdrawn");
    assert_eq!(result["reason"], "already merged");
    assert_eq!(result["withdrawnAtMs"], withdrawn["withdrawnAtMs"]);
    assert!(result.get("response").is_none());
    let shown = fixture.json(&["x", "show", id, "--identity", "Sender", "--json"]);
    assert_eq!(shown["exchange"]["final"]["status"], "withdrawn");
    assert_eq!(
        shown["exchange"]["final"]["withdrawnAtMs"],
        withdrawn["withdrawnAtMs"]
    );
    assert_eq!(shown["exchange"]["prompt"]["message"], "original prompt");
    assert_eq!(shown["exchange"]["settled"], true);
    assert_eq!(
        fixture.json(&["inbox", "--identity", "Receiver", "--json"])["items"],
        serde_json::json!([])
    );
    let shown_incoming = fixture.json(&[
        "x",
        "show",
        id,
        "--incoming",
        "--identity",
        "Receiver",
        "--json",
    ]);
    assert!(shown_incoming["exchange"].get("reply").is_none());
    let late_reply = fixture.run(&[
        "reply",
        id,
        "--receipt",
        receipt,
        "--message",
        "done",
        "--json",
    ]);
    assert_eq!(late_reply.status.code(), Some(5));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&late_reply.stdout).unwrap()["error"]["code"],
        "REQUEST_WITHDRAWN"
    );
    let human = fixture.run(&["result", id]);
    assert!(human.status.success());
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .contains("already merged")
    );

    let second = fixture.json(&[
        "talk",
        "Receiver",
        "another prompt",
        "--inbox",
        "--detach",
        "--identity",
        "Sender",
        "--json",
    ]);
    let second_id = second["requestId"].as_str().unwrap();
    let incoming = fixture.json(&[
        "x",
        "show",
        second_id,
        "--incoming",
        "--identity",
        "Receiver",
        "--json",
    ]);
    let receipt = incoming["exchange"]["reply"]["receipt"].as_str().unwrap();
    fixture.json(&[
        "reply",
        second_id,
        "--receipt",
        receipt,
        "--message",
        "final body",
        "--json",
    ]);
    let late_withdraw = fixture.run(&[
        "x",
        "withdraw",
        second_id,
        "--reason",
        "obsolete",
        "--identity",
        "Sender",
        "--json",
    ]);
    assert_eq!(late_withdraw.status.code(), Some(5));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&late_withdraw.stdout).unwrap()["error"]["code"],
        "REQUEST_ALREADY_FINAL"
    );
    assert_eq!(
        fixture.json(&["result", second_id, "--json"])["response"],
        "final body"
    );

    fixture.seed("anonymous-withdrawal", RequestKind::Request, None);
    let anonymous = fixture.run(&[
        "x",
        "withdraw",
        "anonymous-withdrawal",
        "--reason",
        "obsolete",
        "--identity",
        "Sender",
        "--json",
    ]);
    assert_eq!(anonymous.status.code(), Some(5));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&anonymous.stdout).unwrap()["error"]["code"],
        "REQUEST_ORIGINATOR_MISMATCH"
    );
    let caller = fixture.run(&["x", "withdraw", id, "--reason", "obsolete", "--json"]);
    assert!(
        !caller.status.success(),
        "anonymous caller must supply a verified or explicit identity"
    );

    let oracle =
        rusqlite::Connection::open(support::state_dir(&fixture.root).join("tmux-team.db")).unwrap();
    let state: (Option<i64>, Option<i64>, String, String) = oracle.query_row(
        "SELECT response_submitted_at_ms, withdrawn_at_ms, withdrawal_reason, message_text FROM request_attempts WHERE request_id=?", [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap();
    assert_eq!(
        state,
        (
            None,
            Some(withdrawn["withdrawnAtMs"].as_i64().unwrap()),
            "already merged".into(),
            "original prompt".into()
        )
    );
    let finals: i64 = oracle
        .query_row(
            "SELECT COUNT(*) FROM request_responses WHERE request_id=?",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(finals, 0);
    let help = fixture.run(&["x", "withdraw", "--help"]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(
        help.contains("Only the recorded originator")
            && help.contains("--reason")
            && help.contains("does not cancel work")
    );
}

#[test]
fn result_prefix_human_and_json_match_full_id_and_help_documents_selection() {
    let mut fixture = Fixture::new();
    let id = "req_82d3556e-0000-4000-8000-000000000000";
    let body = "\u{feff}  exact\r\n\0 🙂  ";
    fixture.seed(id, RequestKind::Request, Some(body));
    for json in [false, true] {
        let full_args = if json {
            vec!["result", id, "--json"]
        } else {
            vec!["result", id]
        };
        let full = fixture.run(&full_args);
        assert!(full.status.success());
        assert!(full.stderr.is_empty());
        for prefix in ["82d3556e", "req_82d3556e", "82d3556e-0"] {
            let args = if json {
                vec!["result", prefix, "--json"]
            } else {
                vec!["result", prefix]
            };
            let selected = fixture.run(&args);
            assert!(selected.status.success(), "{selected:?}");
            assert_eq!(selected.stdout, full.stdout);
            assert!(selected.stderr.is_empty());
        }
        if json {
            let value: serde_json::Value = serde_json::from_slice(&full.stdout).unwrap();
            assert_eq!(value["requestId"], id);
            assert_eq!(value["response"], body);
            assert_eq!(value["bodyBytes"], body.len());
        } else {
            assert_eq!(
                full.stdout,
                format!("Response for request '{id}':\n{body}\n").as_bytes()
            );
        }
    }
    let help = fixture.run(&["result", "--help"]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("at least 8 hex characters"));
    assert!(help.contains("with or without req_"));
    assert!(help.contains("tmt result 0f8e4b52"));
}

#[test]
fn result_prefix_usage_errors_cap_candidates_and_unknown_preserves_unavailable() {
    let mut fixture = Fixture::new();
    let ids: Vec<_> = (0..8)
        .map(|n| format!("req_deadbeef-{n:04x}-4000-8000-000000000000"))
        .collect();
    for id in &ids[6..] {
        fixture.seed(id, RequestKind::Request, None);
    }
    let two = fixture.run(&["result", "deadbeef", "--json"]);
    assert_eq!(two.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&two.stdout).unwrap();
    assert_eq!(value["error"]["code"], "USAGE_ERROR");
    let message = value["error"]["message"].as_str().unwrap();
    assert!(message.contains(&ids[6]) && message.contains(&ids[7]));
    assert!(!message.contains("more"));
    for id in ids[..6].iter().rev() {
        fixture.seed(id, RequestKind::Request, None);
    }
    for json in [false, true] {
        let args = if json {
            vec!["result", "deadbeef", "--json"]
        } else {
            vec!["result", "deadbeef"]
        };
        let ambiguous = fixture.run(&args);
        assert_eq!(ambiguous.status.code(), Some(1));
        let message = if json {
            assert!(ambiguous.stderr.is_empty());
            let value: serde_json::Value = serde_json::from_slice(&ambiguous.stdout).unwrap();
            assert_eq!(value["error"]["code"], "USAGE_ERROR");
            value["error"]["message"].as_str().unwrap().to_owned()
        } else {
            assert!(ambiguous.stdout.is_empty());
            String::from_utf8(ambiguous.stderr).unwrap()
        };
        for id in &ids[..5] {
            assert_eq!(message.matches(id).count(), 1);
        }
        for id in &ids[5..] {
            assert!(!message.contains(id));
        }
        assert!(message.contains("…and 3 more"));
        assert!(message.find(&ids[0]).unwrap() < message.find(&ids[4]).unwrap());
    }
    // A colliding full ID still selects exactly, never an ambiguous prefix.
    let full = fixture.run(&["result", &ids[7], "--json"]);
    assert_eq!(full.status.code(), Some(3));
    let value: serde_json::Value = serde_json::from_slice(&full.stdout).unwrap();
    assert_eq!(value["requestId"], ids[7]);
    assert_eq!(value["error"]["code"], "RESPONSE_NOT_AVAILABLE");
    for prefix in ["deadbee", "req_deadbee", "req_"] {
        let short = fixture.run(&["result", prefix, "--json"]);
        assert_eq!(short.status.code(), Some(1));
        let value: serde_json::Value = serde_json::from_slice(&short.stdout).unwrap();
        assert_eq!(value["error"]["code"], "USAGE_ERROR");
        assert!(
            value["error"]["message"]
                .as_str()
                .unwrap()
                .contains("at least 8")
        );
    }
    let unknown = fixture.run(&["result", "cafebabe", "--json"]);
    assert_eq!(unknown.status.code(), Some(3));
    let value: serde_json::Value = serde_json::from_slice(&unknown.stdout).unwrap();
    assert_eq!(value["status"], "unavailable");
    assert_eq!(value["requestId"], "cafebabe");
    assert_eq!(value["error"]["code"], "RESPONSE_NOT_AVAILABLE");
}

#[test]
fn result_prefix_announcement_keeps_canonical_request_id() {
    let mut fixture = Fixture::new();
    let id = "req_abcdef12-0000-4000-8000-000000000000";
    fixture.seed(id, RequestKind::Announcement, None);
    let output = fixture.run(&["result", "abcdef12", "--json"]);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({"status": "not_required", "requestId": id})
    );
}
