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

mod send_now {
    use super::*;
    use crate::cron_service::test_support::{Fixture, USER};

    fn send_now_calls(fixture: &Fixture) -> Vec<Vec<String>> {
        fixture.model()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|call| call["command"].as_array())
            .filter(|command| command[0] == "send-now")
            .map(|command| {
                command
                    .iter()
                    .map(|part| part.as_str().unwrap().to_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn it_asks_core_once_as_the_person_and_reports_the_state_core_moved_to() {
        let fixture = Fixture::new();
        fixture.change_model(|model| {
            model["requestShow"] = json!({"messageState":{"state":"batched","canSendNow":true,
                "reason":{"text":"batched to save usage, sends when the agent finishes this turn"}}});
            model["afterSendNow"] = json!({"messageState":{"state":"delivered","canSendNow":false,
                "reason":{"text":"delivered"}}});
        });
        let notice = send_now(&fixture.core, USER, "req_7", "sol").unwrap();
        assert_eq!(notice, "sol: ✓ delivered", "Core's state, in Core's words");
        assert_eq!(
            send_now_calls(&fixture),
            [["send-now", "req_7", "--identity", USER, "--json"]],
            "one send-now, as the person, for exactly that request"
        );
    }

    #[test]
    fn an_already_delivered_message_is_reported_not_sent_again() {
        let fixture = Fixture::new();
        fixture.change_model(|model| {
            model["sendNowStatus"] = json!("already");
            model["requestShow"] = json!({"messageState":{"state":"working","canSendNow":false,
                "reason":{"text":"the agent has started on it"}}});
        });
        assert_eq!(
            send_now(&fixture.core, USER, "req_7", "sol").unwrap(),
            "sol: ⠹ the agent has started on it"
        );
        assert_eq!(send_now_calls(&fixture).len(), 1);
    }

    #[test]
    fn a_refusal_is_core_s_sentence_and_is_never_retried() {
        for (code, sentence) in [
            ("SEND_NOW_NOT_ALLOWED", "Core's sentence for who may ask."),
            (
                "DELIVERY_UNCERTAIN",
                "Core's sentence for an unconfirmed send.",
            ),
        ] {
            let fixture = Fixture::new();
            fixture.change_model(|model| {
                model["sendNowFailure"] = json!({"code": code, "message": sentence});
            });
            let error = send_now(&fixture.core, USER, "req_7", "sol").unwrap_err();
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                (code, sentence)
            );
            assert_eq!(send_now_calls(&fixture).len(), 1, "{code} is not retried");
        }
    }

    #[test]
    fn a_failed_state_read_after_a_send_still_reports_the_send() {
        let fixture = Fixture::new();
        fixture.change_model(|model| model["requestShowFailure"] = json!(true));
        assert_eq!(
            send_now(&fixture.core, USER, "req_7", "sol").unwrap(),
            "Sent now to sol."
        );
    }
}
