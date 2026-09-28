use super::support::*;
use tmt_core::request::{
    Originator, ResponseProof, Settlement, SubmitResponse, WakeState,
    notification::NotificationPolicy,
};

fn fixture(wait: bool, enabled: bool) -> Fixture {
    let mut fixture = Fixture::new();
    let input = prepare_input(
        &fixture,
        "notify",
        endpoint("%1", 1),
        wait,
        NOW_MS + 60_000,
        Originator::Explicit(fixture.identity_id.clone()),
        false,
    );
    service(&mut fixture)
        .prepare(input, "attempt".into(), 1)
        .unwrap();
    if enabled {
        service(&mut fixture)
            .enable_notifications(
                "notify",
                NotificationPolicy {
                    deadline_ms: NOW_MS + 1000,
                    timeout_ms: 1000,
                    waiter: wait.then(|| {
                        tmt_core::binding::session::RuntimeIncarnation::new(42, "fixture-start")
                            .unwrap()
                    }),
                },
            )
            .unwrap();
    }
    service(&mut fixture).begin_send("attempt").unwrap();
    service(&mut fixture)
        .settle("attempt", Settlement::Sent)
        .unwrap();
    fixture
}

fn reply() -> SubmitResponse {
    SubmitResponse {
        request_id: "notify".into(),
        proof: ResponseProof::Recorded {
            attempt_id: "attempt".into(),
            endpoint: endpoint("%1", 1),
        },
        body: "durable final".into(),
    }
}

#[test]
fn elapsed_notification_deadline_is_observed_not_rejected_during_publication() {
    let mut fixture = Fixture::new();
    let input = prepare_input(
        &fixture,
        "elapsed",
        endpoint("%1", 1),
        false,
        NOW_MS + 60_000,
        Originator::Explicit(fixture.identity_id.clone()),
        false,
    );
    service(&mut fixture)
        .prepare(input, "elapsed-attempt".into(), 1)
        .unwrap();
    service(&mut fixture)
        .enable_notifications(
            "elapsed",
            NotificationPolicy {
                deadline_ms: NOW_MS,
                timeout_ms: 1,
                waiter: None,
            },
        )
        .unwrap();
    service(&mut fixture).begin_send("elapsed-attempt").unwrap();
    service(&mut fixture)
        .settle("elapsed-attempt", Settlement::Sent)
        .unwrap();
    assert!(
        service(&mut fixture)
            .claim_timeout_hint("elapsed")
            .unwrap()
            .is_some()
    );
    assert!(
        service(&mut fixture)
            .claim_timeout_hint("elapsed")
            .unwrap()
            .is_none()
    );
}

#[test]
fn detached_reply_claim_is_durable_and_retry_never_notifies_twice() {
    let mut fixture = fixture(false, true);
    let (first, hint) = service(&mut fixture)
        .submit_response_with_hint(reply(), None)
        .unwrap();
    assert!(hint.is_some());
    assert_eq!(
        service(&mut fixture)
            .notification("notify")
            .unwrap()
            .unwrap()
            .reply,
        WakeState::Claimed
    );
    let (retry, hint) = service(&mut fixture)
        .submit_response_with_hint(reply(), None)
        .unwrap();
    assert_eq!(first, retry);
    assert!(hint.is_none(), "a lost claimant is not a retry lease");
}

#[test]
fn blocking_waiter_delivers_final_without_any_pasted_hint() {
    let mut fixture = fixture(true, true);
    assert!(
        service(&mut fixture)
            .submit_response_with_hint(reply(), None)
            .unwrap()
            .1
            .is_none()
    );
    assert!(
        service(&mut fixture)
            .finish_wait("attempt", true)
            .unwrap()
            .is_none()
    );
    let value = service(&mut fixture)
        .notification("notify")
        .unwrap()
        .unwrap();
    assert!(value.observed);
    assert_eq!(value.reply, WakeState::NotAttempted);
    assert!(
        service(&mut fixture)
            .submit_response_with_hint(reply(), None)
            .unwrap()
            .1
            .is_none()
    );
}

#[test]
fn interrupted_waiter_hands_a_racing_final_to_one_callback() {
    let mut fixture = fixture(true, true);
    assert!(
        service(&mut fixture)
            .submit_response_with_hint(reply(), None)
            .unwrap()
            .1
            .is_none()
    );
    assert!(
        service(&mut fixture)
            .finish_wait("attempt", false)
            .unwrap()
            .is_some()
    );
    assert!(
        service(&mut fixture)
            .finish_wait("attempt", false)
            .unwrap()
            .is_none()
    );
}

#[test]
fn timeout_and_late_final_have_independent_claims() {
    let mut fixture = fixture(false, true);
    assert!(
        service(&mut fixture)
            .claim_timeout_hint("notify")
            .unwrap()
            .is_none()
    );
    fixture.set_now(NOW_MS + 1000);
    assert!(
        service(&mut fixture)
            .claim_timeout_hint("notify")
            .unwrap()
            .is_some()
    );
    assert!(
        service(&mut fixture)
            .claim_timeout_hint("notify")
            .unwrap()
            .is_none()
    );
    assert!(
        service(&mut fixture)
            .submit_response_with_hint(reply(), None)
            .unwrap()
            .1
            .is_some()
    );
}

#[test]
fn final_wins_timeout_and_explicit_queue_only_never_pushes() {
    let mut silent = fixture(false, false);
    assert!(
        service(&mut silent)
            .submit_response_with_hint(reply(), None)
            .unwrap()
            .1
            .is_none()
    );
    let mut fixture = fixture(false, true);
    assert!(
        service(&mut fixture)
            .submit_response_with_hint(reply(), None)
            .unwrap()
            .1
            .is_some()
    );
    fixture.set_now(NOW_MS + 1000);
    assert!(
        service(&mut fixture)
            .claim_timeout_hint("notify")
            .unwrap()
            .is_none()
    );
}

#[test]
fn gone_waiter_is_released_only_for_the_first_valid_final_with_matching_ownership() {
    let mut fixture = fixture(true, true);
    let policy = service(&mut fixture)
        .notification("notify")
        .unwrap()
        .unwrap()
        .policy;
    let mut invalid = reply();
    invalid.request_id = "missing".into();
    assert!(
        service(&mut fixture)
            .submit_response_with_hint(invalid, Some(&policy))
            .is_err()
    );
    let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT wait_active FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(
        service(&mut fixture)
            .submit_response_with_hint(reply(), Some(&policy))
            .unwrap()
            .1
            .is_some()
    );
    assert_eq!(
        oracle
            .query_row("SELECT wait_active FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        service(&mut fixture)
            .submit_response_with_hint(reply(), Some(&policy))
            .unwrap()
            .1
            .is_none()
    );
}
