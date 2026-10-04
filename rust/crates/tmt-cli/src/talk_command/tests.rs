use super::*;
use crate::delivery::Delivery;
use tmt_adapters::storage::{StorageError, StorageErrorCode};
use tmt_core::request::WakeState;

fn correlation() -> Correlation {
    Correlation {
        data_dir: std::env::temp_dir().join("tmt-talk-test"),
        offline: false,
        request_id: "request-talk".into(),
        target: "worker".into(),
        pane: "%1".into(),
        identity: None,
        inbox: false,
        explicit_inbox: false,
        delivery_uncertain: false,
    }
}

fn not_writable() -> RequestError<StorageError> {
    RequestError::Repository(StorageError::new(
        StorageErrorCode::NotWritable,
        "read-only database",
    ))
}

fn busy() -> RequestError<StorageError> {
    RequestError::Repository(StorageError::new(
        StorageErrorCode::Busy,
        "database is busy",
    ))
}

#[test]
fn a_write_without_receipt_is_noted_before_settling_so_settlement_failures_report_it() {
    for failure in [busy(), RequestError::StateInvalid] {
        let mut uncertain = correlation();
        let mut seen = None;
        let failed = settle_delivery(&mut uncertain, &Delivery::Unacknowledged, |state| {
            seen = Some(state);
            Err(failure)
        })
        .unwrap_err();
        assert_eq!(seen, Some(WakeState::Uncertain));
        assert!(uncertain.delivery_uncertain);
        let document = failed.document();
        assert_eq!(failed.code, "REQUEST_STATE_ERROR");
        assert_eq!(document["deliveryState"], "uncertain");
        assert_eq!(document["requestId"], "request-talk");
    }
}

#[test]
fn a_storage_failure_after_a_channel_write_uses_channel_neutral_wording() {
    let mut uncertain = correlation();
    let failed = settle_delivery(&mut uncertain, &Delivery::Unacknowledged, |_| {
        Err(not_writable())
    })
    .unwrap_err();
    assert_eq!(failed.document()["deliveryState"], "uncertain");
    assert!(
        failed
            .message
            .contains("The channel write may have been delivered; do not resend")
    );
    assert!(!failed.message.contains("Pane input"));
}

#[test]
fn settlement_failures_after_a_confirmed_send_keep_their_existing_report() {
    let mut confirmed = correlation();
    let failed =
        settle_delivery(&mut confirmed, &Delivery::Sent, |_| Err(not_writable())).unwrap_err();
    assert!(!confirmed.delivery_uncertain);
    assert!(failed.document().get("deliveryState").is_none());
    assert!(failed.message.contains("Pane input may have happened"));
}

#[test]
fn successful_settlement_records_an_unacknowledged_write_and_nothing_else() {
    let mut uncertain = correlation();
    let mut seen = None;
    settle_delivery(&mut uncertain, &Delivery::Unacknowledged, |state| {
        seen = Some(state);
        Ok(())
    })
    .unwrap();
    assert!(uncertain.delivery_uncertain);
    assert_eq!(seen, Some(WakeState::Uncertain));
    for outcome in [Delivery::Sent, Delivery::Uncertain] {
        let mut other = correlation();
        settle_delivery(&mut other, &outcome, |_| Ok(())).unwrap();
        assert!(!other.delivery_uncertain);
    }
}

#[test]
fn json_status_is_the_attempt_status_and_uncertainty_is_always_stated() {
    let plain = presentation::json_document(correlation(), None);
    assert_eq!(plain["status"], "sent");
    assert!(plain.get("deliveryState").is_none());

    let mut unconfirmed = correlation();
    unconfirmed.delivery_uncertain = true;
    let document = presentation::json_document(unconfirmed, None);
    assert_eq!(document["status"], "sent");
    assert_eq!(document["deliveryState"], "uncertain");
}

#[test]
fn an_unconfirmed_handoff_is_never_worded_as_a_sent_request() {
    let line = presentation::unconfirmed_handoff(&correlation());
    assert_eq!(
        line,
        "Handed request request-talk to worker (%1); delivery is unconfirmed"
    );
    assert!(!line.contains("Sent"));
}

#[test]
fn explicit_and_offline_pending_inbox_report_pull_without_live_notification() {
    let mut queued = correlation();
    queued.inbox = true;
    queued.explicit_inbox = true;
    let document = presentation::json_document(queued.clone(), None);
    assert_eq!(document["status"], "queued");
    assert_eq!(document["requestId"], "request-talk");
    assert_eq!(document["notification"], "not_attempted");
    assert_eq!(document["waitingFor"], "recipient_inbox_pull");
    assert!(document.get("pane").is_none());
    assert!(document.get("deliveryState").is_none());

    // A claimed live route cannot assert that notification was not attempted.
    for offline in [false, true] {
        let mut ordinary = correlation();
        ordinary.inbox = true;
        ordinary.offline = offline;
        let document = presentation::json_document(ordinary, None);
        assert_eq!(
            document["notification"],
            if offline {
                serde_json::json!("not_attempted")
            } else {
                serde_json::Value::Null
            }
        );
        assert_eq!(
            document["waitingFor"],
            if offline {
                serde_json::json!("recipient_inbox_pull")
            } else {
                serde_json::Value::Null
            }
        );
        assert_eq!(
            document["offline"],
            if offline {
                serde_json::json!(true)
            } else {
                serde_json::Value::Null
            }
        );
    }
    let response = FinalResponse {
        request_id: "request-talk".into(),
        attempt_id: "attempt-talk".into(),
        route: tmt_core::request::RequestRoute::Inbox {
            recipient_identity_id: "recipient".into(),
        },
        body: "done".into(),
        body_bytes: 4,
        submitted_at_ms: 1,
        response_expires_at_ms: 2,
    };
    let completed = presentation::json_document(queued, Some(response));
    assert_eq!(completed["status"], "completed");
    assert!(completed.get("notification").is_none());
    assert!(completed.get("waitingFor").is_none());
}

#[test]
fn unverified_delivery_reports_no_live_input_and_recipient_recovery_in_json() {
    let failure = correlation().unverified_delivery();
    let document = failure.document();
    assert_eq!(document["deliveryState"], "not_delivered");
    assert_eq!(document["requestId"], "request-talk");
    assert!(failure.message.contains("not delivered live"));
    assert!(
        document["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("tmt inbox --identity worker --json")
    );
    assert!(
        document["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("new turn or session")
    );
    assert!(
        document["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("tmt resume worker")
    );
}
