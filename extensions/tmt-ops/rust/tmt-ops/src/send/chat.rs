//! Person-authored chat through Core's public passthrough capability and dispatch.
use super::{Accepted, Disposition, Intent, LeadRecipient, refused};
use crate::{
    config::Config,
    core::{Core, SquadError},
};
use serde_json::{Value, json};

/// Missing capability is an older Core, not permission to infer from presence.
fn unavailable(document: &Value, recipient: &str) -> Option<&'static str> {
    let row = document["identities"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"].as_str() == Some(recipient) && row["found"] == true);
    let Some(capability) = row
        .map(|row| &row["canPassthrough"])
        .filter(|value| value.is_object())
    else {
        return Some("Direct messages are unavailable.");
    };
    match (
        capability["available"].as_bool(),
        capability["reason"].as_str(),
    ) {
        (Some(true), None) => None,
        (_, Some("no_pane")) => Some("There is no live pane."),
        (_, Some("not_running")) => Some("The session is not running."),
        (_, Some("awaiting_approval")) => Some("The agent is waiting for your approval."),
        _ => Some("Direct messages are unavailable."),
    }
}

/// Runs only in the existing ordered action worker. Capability is an observation;
/// Core revalidates the sender and pane before it creates a passthrough record.
pub(crate) fn chat(
    core: &Core,
    config: &Config,
    sender: &str,
    recipient: &str,
    text: &str,
    fallback: &str,
) -> Result<Accepted, SquadError> {
    if text.trim().is_empty() {
        return Err(refused("Nothing to send."));
    }
    let status = core.api("identities.status", json!({"identityIds":[recipient]}))?;
    let mut reason = unavailable(&status, recipient);
    if reason.is_none() {
        let person = core.json(&["identity", "meta", "ls", "--identity", sender])?;
        if person["identityId"] != sender {
            return Err(refused("The sender identity changed; nothing sent."));
        }
        if person["metadata"]["user"] != "true" {
            reason = Some("The sender is not marked as a person.");
        }
    }
    let passthrough = reason.is_none();
    if (if passthrough { text } else { fallback }).len() > crate::requests::message::MESSAGE_LIMIT {
        return Err(refused(
            "Message including its request context exceeds 1 MiB; nothing sent. Draft retained.",
        ));
    }
    let recipients = vec![LeadRecipient {
        squad: String::new(),
        id: recipient.into(),
        name: recipient.into(),
    }];
    let mut intent = Intent::new(
        sender,
        recipients,
        "request",
        None,
        if passthrough { text } else { fallback },
    )?;
    if passthrough {
        intent.input["mode"] = json!("passthrough");
    }
    // No fallback/retry after a send-time refusal or uncertainty: the mode and
    // source are frozen in the private journal before dispatch.create runs.
    let (_, receipt, warning) = intent.submit(core, config)?;
    let mut result = accepted(&receipt, recipient, passthrough, reason)?;
    result.warning = warning;
    // Acceptance already happened. A failed observation must never turn it into
    // a send failure or admit a second dispatch.
    result.message_state_label = crate::requests::show_request(core, &result.request)
        .ok()
        .and_then(|detail| crate::requests::message_state_label(&detail));
    Ok(result)
}

fn accepted(
    receipt: &Value,
    recipient: &str,
    passthrough: bool,
    reason: Option<&str>,
) -> Result<Accepted, SquadError> {
    let item = receipt["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| item["recipientId"] == recipient)
        .ok_or_else(|| refused("Dispatch acceptance is unavailable; do not resend."))?;
    if item["acceptance"] != "queued" {
        return Err(refused(
            "The recipient is unavailable; nothing was accepted. Draft retained.",
        ));
    }
    let request = item["requestId"]
        .as_str()
        .filter(|request| !request.is_empty())
        .ok_or_else(|| refused("No request was accepted for this recipient."))?;
    // The stored acceptance receipt has no wake. A recovered receipt remains
    // queued until requests.show supplies authoritative delivery/capture facts.
    let state = receipt["wake"]["status"].as_str();
    let disposition = match state {
        Some("sent") => Disposition::Sent,
        Some("uncertain") => Disposition::Uncertain,
        Some("unavailable") => Disposition::Offline,
        _ => Disposition::Queued,
    };
    Ok(Accepted {
        request: request.into(),
        disposition,
        message_state_label: None,
        passthrough,
        fallback: reason.map(str::to_owned),
        warning: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn core_delivery_text_is_observed_after_acceptance_without_resending_on_read_failure() {
        use crate::cron_service::test_support::{Fixture, LEAD, USER};
        for fails in [false, true] {
            let fixture = Fixture::new();
            let text = "Core changed its words: waiting, not a client guess.";
            fixture.change_model(|model| {
                model["canPassthrough"] = json!({"available":true});
                model["members"][0]["metadata"]["user"] = json!("true");
                model["dispatchWake"] = json!("sent");
                model["requestShowFailure"] = json!(fails);
                model["requestShow"] = json!({"messageState":{"reason":{"text":text}}});
            });
            let accepted = chat(
                &fixture.core,
                &fixture.config,
                USER,
                LEAD,
                "hello",
                "fallback",
            )
            .unwrap();
            assert_eq!(
                accepted.message_state_label.as_deref(),
                (!fails).then_some(text)
            );
            assert_eq!(accepted.disposition, Disposition::Sent);
            let model = fixture.model();
            let operations: Vec<_> = model["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|call| call["request"]["operation"].as_str())
                .collect();
            assert_eq!(
                operations
                    .iter()
                    .filter(|op| **op == "dispatch.create")
                    .count(),
                1
            );
            assert!(
                operations.iter().position(|op| *op == "dispatch.create")
                    < operations.iter().position(|op| *op == "requests.show")
            );
        }
    }

    #[test]
    fn capability_never_guesses_and_reasons_are_an_open_set() {
        assert!(unavailable(&json!({}), "a").is_some());
        for reason in [
            "no_pane",
            "not_running",
            "awaiting_approval",
            "future_reason",
        ] {
            for available in [false, true] {
                let row = json!({"identities":[{"id":"a","found":true,"canPassthrough":{"available":available,"reason":reason}}]});
                assert!(unavailable(&row, "a").is_some(), "{reason}");
            }
        }
        assert_eq!(
            unavailable(
                &json!({"identities":[{"id":"a","found":true,"canPassthrough":{"available":true}}]}),
                "a"
            ),
            None
        );
    }
    #[test]
    fn chat_freezes_mode_and_raw_text_and_keeps_a_request_fallback() {
        use crate::cron_service::test_support::{Fixture, LEAD, USER};
        for direct in [false, true] {
            let fixture = Fixture::new();
            fixture.change_model(|model| {
                model["canPassthrough"] = json!({"available":direct});
                model["members"][0]["metadata"]["user"] = json!("true");
                model["dispatchWake"] = json!("sent");
            });
            let sent = chat(
                &fixture.core,
                &fixture.config,
                USER,
                LEAD,
                "/yes!\n漢字",
                "[product · Sol] /yes!\n漢字",
            )
            .unwrap();
            assert_eq!(sent.passthrough, direct);
            assert_eq!(sent.disposition, Disposition::Sent);
            assert_eq!(sent.fallback.is_none(), direct);
            let model = fixture.model();
            let calls: Vec<_> = model["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|call| call["request"]["operation"] == "dispatch.create")
                .collect();
            assert_eq!(calls.len(), 1);
            let request = &calls[0]["request"];
            assert_eq!(request["identity"], USER);
            assert_eq!(request["input"]["recipientIds"], json!([LEAD]));
            assert!(request["input"].get("room").is_none());
            assert_eq!(
                request["input"]["message"],
                if direct {
                    "/yes!\n漢字"
                } else {
                    "[product · Sol] /yes!\n漢字"
                }
            );
            assert_eq!(
                request["input"]["mode"],
                if direct {
                    json!("passthrough")
                } else {
                    Value::Null
                }
            );
        }
    }

    #[test]
    fn a_non_person_sends_a_request_and_uncertain_delivery_is_never_retried() {
        use crate::cron_service::test_support::{Fixture, LEAD, USER};
        for person in [false, true] {
            let fixture = Fixture::new();
            fixture.change_model(|model| {
                model["canPassthrough"] = json!({"available":true});
                model["members"][0]["metadata"]["user"] =
                    json!(if person { "true" } else { "false" });
                model["dispatchWake"] = json!("uncertain");
            });
            let sent = chat(
                &fixture.core,
                &fixture.config,
                USER,
                LEAD,
                "hello",
                "tagged hello",
            )
            .unwrap();
            assert_eq!(sent.passthrough, person);
            assert_eq!(sent.disposition, Disposition::Uncertain);
            assert_eq!(fixture.model()["wakes"].as_array().unwrap().len(), 1);
        }
    }

    #[test]
    fn a_send_time_refusal_does_not_fall_back_or_store_a_second_intent() {
        use crate::cron_service::test_support::{Fixture, LEAD, USER};
        for code in ["PASSTHROUGH_NOT_ALLOWED", "PASSTHROUGH_UNAVAILABLE"] {
            let fixture = Fixture::new();
            fixture.change_model(|model| {
                model["canPassthrough"] = json!({"available":true});
                model["members"][0]["metadata"]["user"] = json!("true");
                model["dispatchRefusal"] = json!(code);
            });
            let error = chat(
                &fixture.core,
                &fixture.config,
                USER,
                LEAD,
                "raw",
                "fallback",
            )
            .unwrap_err();
            assert_eq!(error.code, code);
            let model = fixture.model();
            let attempts: Vec<_> = model["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|call| call["request"]["operation"] == "dispatch.create")
                .collect();
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0]["request"]["input"]["mode"], "passthrough");
            assert!(model["dispatches"].is_null());
            assert!(model["wakes"].is_null());
            assert_eq!(
                std::fs::read_dir(fixture.directory.join("board-dispatches"))
                    .unwrap()
                    .count(),
                0
            );
        }
    }

    #[test]
    fn accepted_chat_preserves_a_journal_cleanup_warning() {
        use crate::cron_service::test_support::{Fixture, LEAD, USER};
        let fixture = Fixture::new();
        fixture.change_model(|model| {
            model["canPassthrough"] = json!({"available":true});
            model["members"][0]["metadata"]["user"] = json!("true");
            model["dispatchCleanupFailure"] = json!(true);
            model["dispatchWake"] = json!("sent");
        });
        let sent = chat(
            &fixture.core,
            &fixture.config,
            USER,
            LEAD,
            "hello",
            "tagged hello",
        )
        .unwrap();
        assert_eq!(sent.disposition, Disposition::Sent);
        assert!(
            sent.warning
                .as_ref()
                .unwrap()
                .contains("could not remove saved intent")
        );
        assert_eq!(fixture.model()["wakes"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn recovery_does_not_invent_a_wake_or_change_the_mode() {
        use crate::cron_service::test_support::{Fixture, LEAD, USER};
        let fixture = Fixture::new();
        fixture.change_model(|model| {
            model["canPassthrough"] = json!({"available":true});
            model["members"][0]["metadata"]["user"] = json!("true");
            model["loseResponse"] = json!(true);
        });
        let sent = chat(
            &fixture.core,
            &fixture.config,
            USER,
            LEAD,
            "hello",
            "tagged hello",
        )
        .unwrap();
        assert!(sent.passthrough);
        assert_eq!(sent.disposition, Disposition::Queued);
        let model = fixture.model();
        assert_eq!(model["wakes"].as_array().unwrap().len(), 1);
        let dispatch = model["dispatches"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap();
        assert_eq!(dispatch["intent"]["input"]["mode"], "passthrough");
        assert_eq!(dispatch["intent"]["input"]["message"], "hello");
    }
    #[test]
    fn an_oversized_request_fallback_is_refused_before_any_dispatch() {
        use crate::cron_service::test_support::{Fixture, LEAD, USER};
        let fixture = Fixture::new();
        let error = chat(
            &fixture.core,
            &fixture.config,
            USER,
            LEAD,
            "short",
            &"x".repeat(crate::requests::message::MESSAGE_LIMIT + 1),
        )
        .unwrap_err();
        assert!(error.message.contains("Draft retained"));
        assert!(
            !fixture.model()["calls"]
                .as_array()
                .unwrap()
                .iter()
                .any(|call| call["request"]["operation"] == "dispatch.create")
        );
    }
}
