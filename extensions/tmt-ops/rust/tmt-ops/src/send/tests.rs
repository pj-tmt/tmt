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
            json!({"requestId": "req_1", "status": "queued", "digest": true,
                   "notification": "held", "waitingFor": "digest_checklist"})
        ),
        "Message held for sol (req_1) until their digest checklist ends."
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
fn a_note_keeps_its_row_in_completion_feedback() {
    let (core, dir) = fake(
        "note",
        &json!({"requestId": "req_1", "status": "queued", "offline": true}),
        0,
    );
    let accepted = talk(&core, "p", "me", "sol", "[p · alpha] look").unwrap();
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

#[test]
fn settled_answers_fall_back_once_without_changing_the_message() {
    for code in [
        "ANSWER_NOT_WAITING",
        "REQUEST_WITHDRAWN",
        "RESPONSE_NOT_REQUIRED",
        "RESPONSE_CONFLICT",
        "RESPONSE_EXPIRED",
    ] {
        let (core, dir) = fake(&format!("settled-{code}"), &json!({}), 0);
        let script = format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> '{calls}'
if [ "$1" = answer ]; then
  printf '%s\n' '{{"error":{{"code":"{code}","message":"Settled"}}}}'
  exit 1
fi
printf '%s\n' '{{"status":"sent","requestId":"new-message"}}'
"#,
            calls = dir.join("calls").display()
        );
        crate::test_support::write_ready_executable(&dir.join("tmt"), &script);
        let text = "The exact text — quoted and unchanged.";
        let AnswerOrTalk::Talk(Ok(sent)) =
            answer_or_talk(&core, "product", "owner", "old-question", "member", text)
        else {
            panic!("a settled answer must become one Talk");
        };
        assert_eq!(sent.request, "new-message");
        let log = std::fs::read_to_string(dir.join("calls")).unwrap();
        assert_eq!(log.lines().count(), 2, "{log}");
        assert_eq!(
            log.lines().next().unwrap(),
            format!("answer --identity owner --request old-question --json -- member {text}")
        );
        assert_eq!(
            log.lines().nth(1).unwrap(),
            format!("talk --identity owner --room squad-product --detach --json -- member {text}")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn an_unknown_or_unrelated_answer_failure_never_sends_again() {
    for code in [
        "ANSWER_ERROR",
        "TALK_TIMEOUT",
        "RESPONSE_INPUT_INVALID",
        "RESPONSE_RECIPIENT_MISMATCH",
        "RESPONSE_RECEIPT_MISMATCH",
        "RESPONSE_STATE_INVALID",
    ] {
        let (core, dir) = fake(
            &format!("no-fallback-{code}"),
            &json!({"error":{"code":code,"message":"Failed"}}),
            1,
        );
        let AnswerOrTalk::Answer(Err(error)) = answer_or_talk(
            &core,
            "product",
            "owner",
            "question",
            "member",
            "Exact draft",
        ) else {
            panic!("unknown failure must remain an answer failure");
        };
        assert_eq!(error.code, code);
        assert_eq!(calls(&dir), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
    let (core, dir) = fake("answer-accepted", &json!({"status":"submitted"}), 0);
    assert!(matches!(
        answer_or_talk(
            &core,
            "product",
            "owner",
            "question",
            "member",
            "Exact draft"
        ),
        AnswerOrTalk::Answer(Ok(()))
    ));
    assert_eq!(calls(&dir), 1);
    std::fs::remove_dir_all(dir).unwrap();
}
