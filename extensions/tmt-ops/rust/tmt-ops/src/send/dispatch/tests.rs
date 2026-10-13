use super::*;
use crate::cron_service::test_support::{Fixture, LEAD, USER};
use std::os::unix::fs::PermissionsExt;

fn audience() -> Vec<LeadRecipient> {
    vec![LeadRecipient {
        squad: "product".into(),
        id: LEAD.into(),
        name: "Sol".into(),
    }]
}
fn intents(fixture: &Fixture) -> Vec<std::path::PathBuf> {
    fs::read_dir(fixture.directory.join("board-dispatches"))
        .into_iter()
        .flatten()
        .map(|entry| entry.unwrap().path())
        .collect()
}
fn count(fixture: &Fixture, operation: &str) -> usize {
    fixture.model()["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|call| call["request"]["operation"] == operation)
        .count()
}

#[test]
fn queued_acceptance_cleans_the_intent_without_claiming_delivery() {
    let fixture = Fixture::new();
    let outcome = run(
        &fixture.core,
        &fixture.config,
        USER,
        &audience(),
        true,
        "Review the plan",
    )
    .unwrap();
    assert_eq!(outcome, "Sol: queued");
    assert!(intents(&fixture).is_empty());
    assert_eq!(count(&fixture, "dispatch.create"), 1);
    assert_eq!(count(&fixture, "dispatch.show"), 0);
    let model = fixture.model();
    let call = model["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["request"]["operation"] == "dispatch.create")
        .unwrap();
    assert_eq!(call["request"]["input"]["recipientIds"], json!([LEAD]));
    assert_eq!(call["request"]["identity"], USER);
    assert!(crate::config::uuid_like(
        call["request"]["input"]["operationId"].as_str().unwrap()
    ));
}

#[test]
fn lost_acceptance_recovers_once_without_creating_or_waking_again() {
    for failure in [json!(true), json!("storage")] {
        let fixture = Fixture::new();
        fixture.change_model(|model| model["loseResponse"] = failure);
        assert_eq!(
            run(
                &fixture.core,
                &fixture.config,
                USER,
                &audience(),
                true,
                "hello"
            )
            .unwrap(),
            "Sol: queued"
        );
        assert_eq!(count(&fixture, "dispatch.create"), 1);
        assert_eq!(count(&fixture, "dispatch.show"), 1);
        assert_eq!(fixture.model()["wakes"].as_array().unwrap().len(), 1);
        assert!(intents(&fixture).is_empty());
    }
}

#[test]
fn uncertain_acceptance_retains_private_exact_intent_for_recovery() {
    let fixture = Fixture::new();
    fixture.change_model(|model| {
        model["loseResponse"] = json!(true);
        model["dispatchShowFailure"] = json!(true);
    });
    let error = run(
        &fixture.core,
        &fixture.config,
        USER,
        &audience(),
        true,
        "hello",
    )
    .unwrap_err();
    assert!(error.message.contains("Acceptance uncertain"));
    let paths = intents(&fixture);
    assert_eq!(paths.len(), 1);
    assert_eq!(
        fs::metadata(&paths[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let intent: Value = serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap();
    let operation = intent["input"]["operationId"].as_str().unwrap();
    assert_eq!(intent["senderId"], USER);
    assert_eq!(
        intent["input"],
        fixture.model()["dispatches"][operation]["intent"]["input"]
    );
    assert!(error.message.contains(operation));
    assert_eq!(count(&fixture, "dispatch.create"), 1);
    assert_eq!(fixture.model()["wakes"].as_array().unwrap().len(), 1);
}

#[test]
fn changed_sender_or_current_lead_refuses_before_journal_or_dispatch() {
    for changed_sender in [false, true] {
        let fixture = Fixture::new();
        if !changed_sender {
            fixture.change_model(|model| {
                model["members"][1]["metadata"]["squad.product.lead.marker"] = json!("false")
            });
        }
        assert!(
            run(
                &fixture.core,
                &fixture.config,
                if changed_sender { LEAD } else { USER },
                &audience(),
                true,
                "hello"
            )
            .is_err()
        );
        assert!(!fixture.directory.join("board-dispatches").exists());
        assert_eq!(count(&fixture, "dispatch.create"), 0);
    }
}

#[test]
fn acceptance_deduplicates_identity_and_keeps_each_recipients_outcome() {
    let mut recipients = audience();
    recipients.push(LeadRecipient {
        squad: "another".into(),
        ..recipients[0].clone()
    });
    recipients.push(LeadRecipient {
        squad: "other".into(),
        id: USER.into(),
        name: "Ben".into(),
    });
    let receipt = json!({"operationId":"op", "items":[{"recipientId":LEAD,"acceptance":"queued","requestId":"r"},{"recipientId":USER,"acceptance":"recipientUnavailable","requestId":"s"}]});
    assert_eq!(
        acceptance("op", &recipients, &receipt).unwrap(),
        "Sol: queued; Ben: unavailable"
    );
    let mut invalid = receipt;
    invalid["items"][0]["recipientId"] = json!(USER);
    assert!(acceptance("op", &recipients, &invalid).is_err());
}
