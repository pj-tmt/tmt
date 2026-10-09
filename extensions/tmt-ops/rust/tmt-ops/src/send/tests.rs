use super::*;
use serde_json::json;

/// A fake `tmt` that prints `document` for `talk` and exits with `status`.
fn fake(name: &str, document: &serde_json::Value, status: u8) -> (Core, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("ops-send-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("tmt");
    crate::test_support::write_ready_executable(
        &fake,
        &format!(
            "#!/bin/sh\necho \"$@\" >> '{calls}'\necho '{document}'\nexit {status}\n",
            calls = dir.join("calls").display()
        ),
    );
    (Core::at(fake), dir)
}

fn calls(dir: &std::path::Path) -> usize {
    std::fs::read_to_string(dir.join("calls"))
        .unwrap_or_default()
        .lines()
        .count()
}

fn notice(name: &str, document: serde_json::Value) -> String {
    let (core, dir) = fake(name, &document, 0);
    let accepted = talk(&core, "p", "me", "sol", "check").unwrap();
    assert_eq!(accepted.request, "req_1");
    assert_eq!(calls(&dir), 1, "one talk, never a resend");
    let _ = std::fs::remove_dir_all(dir);
    accepted.describe("Message", "sol")
}

#[test]
fn only_a_pane_write_reads_as_sent() {
    assert_eq!(
        notice("sent", json!({"requestId": "req_1", "status": "sent"})),
        "Message sent to sol (req_1)."
    );
    assert_eq!(
        notice("queued", json!({"requestId": "req_1", "status": "queued"})),
        "Message queued in sol's inbox (req_1)."
    );
    assert_eq!(
        notice(
            "offline",
            json!({"requestId": "req_1", "status": "queued", "offline": true,
                   "notification": "not_attempted", "waitingFor": "recipient_inbox_pull"})
        ),
        "Message queued for sol (req_1): offline, it waits for their inbox."
    );
    assert_eq!(
        notice(
            "held",
            json!({"requestId": "req_1", "status": "queued", "focus": true,
                   "notification": "held", "waitingFor": "focus_checklist"})
        ),
        "Message held for sol (req_1) until their focus checklist ends."
    );
    assert_eq!(
        notice(
            "uncertain",
            json!({"requestId": "req_1", "status": "sent", "deliveryState": "uncertain"})
        ),
        "Message handed to sol (req_1); delivery is unconfirmed, do not resend."
    );
    assert_eq!(
        notice("unknown", json!({"requestId": "req_1"})),
        "Message queued in sol's inbox (req_1).",
        "an unreported status never claims delivery"
    );
}

#[test]
fn a_note_names_its_row() {
    let (core, dir) = fake(
        "note",
        &json!({"requestId": "req_1", "status": "queued", "offline": true}),
        0,
    );
    let accepted = annotate(&core, "p", "me", "sol", "alpha", "look").unwrap();
    assert_eq!(
        accepted.describe("Note on alpha", "sol"),
        "Note on alpha queued for sol (req_1): offline, it waits for their inbox."
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failure_after_acceptance_names_the_retained_request_once() {
    let failed = json!({"requestId": "req_9", "status": "withdrawn",
        "error": {"code": "TALK_DELIVERY_FAILED", "message": "The pane refused input."}});
    let (core, dir) = fake("failed", &failed, 1);
    let error = talk(&core, "p", "me", "sol", "check").unwrap_err();
    assert_eq!(error.code, "TALK_DELIVERY_FAILED");
    assert_eq!(error.request.as_deref(), Some("req_9"));
    assert_eq!(
        error.message,
        "The pane refused input. Request req_9 is retained; do not resend."
    );
    assert_eq!(calls(&dir), 1, "a failure is not retried");
    let _ = std::fs::remove_dir_all(dir);

    let named = json!({"requestId": "req_9",
        "error": {"code": "TALK_TIMEOUT", "message": "Request req_9 timed out."}});
    let (core, dir) = fake("named", &named, 1);
    let error = talk(&core, "p", "me", "sol", "check").unwrap_err();
    assert_eq!(error.message, "Request req_9 timed out.", "already named");
    let _ = std::fs::remove_dir_all(dir);

    let refused = json!({"error": {"code": "TARGET_NOT_FOUND", "message": "No sol."}});
    let (core, dir) = fake("refused", &refused, 1);
    let error = talk(&core, "p", "me", "sol", "check").unwrap_err();
    assert_eq!((error.request, error.message.as_str()), (None, "No sol."));
    let _ = std::fs::remove_dir_all(dir);
}
