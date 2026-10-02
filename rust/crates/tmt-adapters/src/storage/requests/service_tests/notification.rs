use super::support::*;
use crate::storage::Storage;
use tmt_core::request::{
    Originator, ResponseProof, Settlement, SubmitResponse, WakeState,
    notification::{
        NotificationPolicy,
        batch::{Notice, SendClaim},
    },
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
                        tmt_core::endpoint::ProcessIncarnation::new(42, "fixture-start").unwrap()
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

fn batch_hint(fixture: &mut Fixture, id: &str) -> tmt_core::request::notification::OriginatorHint {
    let attempt = format!("{id}-attempt");
    let input = prepare_input(
        fixture,
        id,
        endpoint("%1", 1),
        false,
        NOW_MS + 60_000,
        tmt_core::request::Originator::Explicit(fixture.identity_id.clone()),
        false,
    );
    service(fixture).prepare(input, attempt.clone(), 1).unwrap();
    service(fixture)
        .enable_notifications(
            id,
            NotificationPolicy {
                deadline_ms: NOW_MS + 1000,
                timeout_ms: 1000,
                waiter: None,
            },
        )
        .unwrap();
    service(fixture).begin_send(&attempt).unwrap();
    service(fixture).settle(&attempt, Settlement::Sent).unwrap();
    service(fixture)
        .submit_response_with_hint(
            SubmitResponse {
                request_id: id.into(),
                proof: ResponseProof::Recorded {
                    attempt_id: attempt,
                    endpoint: endpoint("%1", 1),
                },
                body: format!("reply {id}"),
            },
            None,
        )
        .unwrap()
        .1
        .unwrap()
}

#[test]
fn reply_batch_reopens_with_one_fixed_window_and_all_request_ids() {
    let mut fixture = Fixture::new();
    let hints = ["one", "two", "three"].map(|id| batch_hint(&mut fixture, id));
    let first = fixture
        .storage
        .queue_reply_notice(&hints[0], "binding", "notice one", 5000, 2000, NOW_MS)
        .unwrap();
    // Separate storage handles represent independent tmt invocations.
    let mut reopened = Storage::open(&fixture.database).unwrap();
    for (index, hint) in hints.iter().enumerate().skip(1) {
        let batch = reopened
            .queue_reply_notice(
                hint,
                "binding",
                &format!("notice {index}"),
                5000,
                2000,
                NOW_MS + index as u64 * 1000,
            )
            .unwrap();
        assert_eq!(batch.id, first.id);
        assert_eq!(
            batch.due_ms,
            NOW_MS + 5000,
            "later arrivals never slide the window"
        );
    }
    let worker = tmt_core::endpoint::ProcessIncarnation::new(123, "worker-start").unwrap();
    assert!(reopened.claim_reply_notice_worker(&first, &worker).unwrap());
    assert!(
        !fixture
            .storage
            .claim_reply_notice_worker(&first, &worker)
            .unwrap(),
        "stale startup snapshots cannot elect twice"
    );
    let members = claim_send(&mut reopened, &first.id, &worker);
    assert_eq!(members.len(), 3);
    assert_eq!(
        members
            .iter()
            .map(|v| v.request_id.as_str())
            .collect::<Vec<_>>(),
        vec!["one", "three", "two"]
    );
    assert_eq!(
        fixture
            .storage
            .claim_reply_notice_send(&first.id, &worker)
            .unwrap(),
        SendClaim::Lost
    );
    assert!(
        reopened
            .mark_reply_notice_fallback_attempted(&first.id, &worker)
            .unwrap()
    );
    reopened
        .settle_reply_notice_batch(&first.id, WakeState::Uncertain)
        .unwrap();
    assert!(reopened.reply_notice_batch(&first.id).unwrap().is_none());
    for hint in &hints {
        assert_eq!(
            service(&mut fixture)
                .notification(&hint.request_id)
                .unwrap()
                .unwrap()
                .reply,
            WakeState::Uncertain
        );
        assert!(matches!(
            service(&mut fixture)
                .get_response(&hint.request_id)
                .unwrap(),
            tmt_core::request::ResponseLookup::Available(_)
        ));
    }
    reopened.close().unwrap();
}

#[test]
fn disabled_batching_separates_notices_and_crash_claims_never_reopen() {
    let mut fixture = Fixture::new();
    let first = batch_hint(&mut fixture, "first");
    let second = batch_hint(&mut fixture, "second");
    let a = fixture
        .storage
        .queue_reply_notice(&first, "binding", "first notice", 0, 2000, NOW_MS)
        .unwrap();
    let b = fixture
        .storage
        .queue_reply_notice(&second, "binding", "second notice", 0, 2000, NOW_MS)
        .unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(a.due_ms, NOW_MS);
    let worker = tmt_core::endpoint::ProcessIncarnation::new(123, "start").unwrap();
    fixture
        .storage
        .claim_reply_notice_worker(&a, &worker)
        .unwrap();
    claim_send(&mut fixture.storage, &a.id, &worker);
    assert!(
        fixture
            .storage
            .mark_reply_notice_fallback_attempted(&a.id, &worker)
            .unwrap()
    );
    let mut reopened = Storage::open(&fixture.database).unwrap();
    let crashed = reopened.reply_notice_batch(&a.id).unwrap().unwrap();
    assert!(
        crashed.sending,
        "a crash after claim never authorizes another send"
    );
    reopened.close().unwrap();
    assert!(
        !fixture
            .storage
            .claim_reply_notice_worker(&crashed, &worker)
            .unwrap()
    );
    let retry = fixture
        .storage
        .queue_reply_notice(&first, "binding", "changed", 5000, 0, NOW_MS + 1000)
        .unwrap();
    assert_eq!(
        retry.id, a.id,
        "the same request cannot create a second notice"
    );
}

#[test]
fn partial_channel_recovery_keeps_known_outcomes_and_never_resends_an_attempted_frame() {
    let mut fixture = Fixture::new();
    let hints = ["accepted", "crashed", "untouched"].map(|id| batch_hint(&mut fixture, id));
    let batch = fixture
        .storage
        .queue_reply_notice(&hints[0], "binding", "accepted", 5000, 2000, NOW_MS)
        .unwrap();
    for hint in &hints[1..] {
        fixture
            .storage
            .queue_reply_notice(hint, "binding", &hint.request_id, 5000, 2000, NOW_MS + 1)
            .unwrap();
    }
    let old = tmt_core::endpoint::ProcessIncarnation::new(123, "old").unwrap();
    fixture
        .storage
        .claim_reply_notice_worker(&batch, &old)
        .unwrap();
    claim_send(&mut fixture.storage, &batch.id, &old);
    assert!(
        fixture
            .storage
            .mark_reply_notice_attempted(&batch.id, "accepted", &old)
            .unwrap()
    );
    fixture
        .storage
        .settle_reply_notice_member(&batch.id, "accepted", WakeState::Sent)
        .unwrap();
    assert!(
        fixture
            .storage
            .mark_reply_notice_attempted(&batch.id, "crashed", &old)
            .unwrap()
    );
    let snapshot = fixture
        .storage
        .reply_notice_batch(&batch.id)
        .unwrap()
        .unwrap();
    assert!(snapshot.sending && snapshot.pending);
    let new = tmt_core::endpoint::ProcessIncarnation::new(124, "new").unwrap();
    // The service caller must first prove the old incarnation gone; this SQL
    // port then matches that snapshot atomically against the current owner.
    assert!(
        fixture
            .storage
            .claim_reply_notice_worker(&snapshot, &new)
            .unwrap()
    );
    let remaining = claim_send(&mut fixture.storage, &batch.id, &new);
    assert_eq!(
        remaining
            .iter()
            .map(|n| n.request_id.as_str())
            .collect::<Vec<_>>(),
        vec!["untouched"]
    );
    assert_eq!(
        service(&mut fixture)
            .notification("accepted")
            .unwrap()
            .unwrap()
            .reply,
        WakeState::Sent
    );
    assert_eq!(
        service(&mut fixture)
            .notification("crashed")
            .unwrap()
            .unwrap()
            .reply,
        WakeState::Uncertain
    );
    assert_eq!(
        service(&mut fixture)
            .notification("untouched")
            .unwrap()
            .unwrap()
            .reply,
        WakeState::Claimed
    );
    assert!(
        !fixture
            .storage
            .mark_reply_notice_attempted(&batch.id, "untouched", &old)
            .unwrap()
    );
    assert!(
        fixture
            .storage
            .mark_reply_notice_attempted(&batch.id, "untouched", &new)
            .unwrap()
    );
    fixture
        .storage
        .settle_reply_notice_member(&batch.id, "untouched", WakeState::Sent)
        .unwrap();
    fixture
        .storage
        .finish_reply_notice_batch(&batch.id)
        .unwrap();
    assert!(
        fixture
            .storage
            .reply_notice_batch(&batch.id)
            .unwrap()
            .is_none()
    );
}

fn claim_send(
    storage: &mut Storage,
    id: &str,
    worker: &tmt_core::endpoint::ProcessIncarnation,
) -> Vec<Notice> {
    match storage.claim_reply_notice_send(id, worker).unwrap() {
        SendClaim::Ready(notices) => notices,
        other => panic!("Expected owned pane transport, got {other:?}"),
    }
}

#[test]
fn separate_deferred_notices_serialize_per_pane_and_release_after_settlement() {
    let mut fixture = Fixture::new();
    let hints = ["first", "second", "other-pane"].map(|id| batch_hint(&mut fixture, id));
    let batches = hints
        .iter()
        .enumerate()
        .map(|(i, hint)| {
            fixture
                .storage
                .queue_reply_notice(
                    hint,
                    if i == 2 { "other" } else { "binding" },
                    &hint.request_id,
                    0,
                    2000,
                    NOW_MS,
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    let workers = [123, 124, 125]
        .map(|pid| tmt_core::endpoint::ProcessIncarnation::new(pid, "start").unwrap());
    for (batch, worker) in batches.iter().zip(&workers) {
        assert!(
            fixture
                .storage
                .claim_reply_notice_worker(batch, worker)
                .unwrap()
        );
    }
    let mut second = Storage::open(&fixture.database).unwrap();
    assert_eq!(
        claim_send(&mut fixture.storage, &batches[0].id, &workers[0]).len(),
        1
    );
    let busy = fixture
        .storage
        .reply_notice_batch(&batches[0].id)
        .unwrap()
        .unwrap();
    assert_eq!(
        second
            .claim_reply_notice_send(&batches[1].id, &workers[1])
            .unwrap(),
        SendClaim::Waiting(busy)
    );
    assert_eq!(
        claim_send(&mut second, &batches[2].id, &workers[2]).len(),
        1,
        "another pane remains independent"
    );
    assert!(
        fixture
            .storage
            .mark_reply_notice_fallback_attempted(&batches[0].id, &workers[0])
            .unwrap()
    );
    fixture
        .storage
        .settle_reply_notice_batch(&batches[0].id, WakeState::Sent)
        .unwrap();
    let remaining = claim_send(&mut second, &batches[1].id, &workers[1]);
    assert_eq!(remaining[0].request_id, "second");
    second.close().unwrap();
}

#[test]
fn proven_dead_sender_releases_the_pane_without_replaying_attempted_input() {
    let mut fixture = Fixture::new();
    let first = batch_hint(&mut fixture, "first");
    let second = batch_hint(&mut fixture, "second");
    let a = fixture
        .storage
        .queue_reply_notice(&first, "binding", "first", 0, 2000, NOW_MS)
        .unwrap();
    let b = fixture
        .storage
        .queue_reply_notice(&second, "binding", "second", 0, 2000, NOW_MS)
        .unwrap();
    let old = tmt_core::endpoint::ProcessIncarnation::new(123, "old").unwrap();
    let new = tmt_core::endpoint::ProcessIncarnation::new(124, "new").unwrap();
    assert!(fixture.storage.claim_reply_notice_worker(&a, &old).unwrap());
    assert!(fixture.storage.claim_reply_notice_worker(&b, &new).unwrap());
    claim_send(&mut fixture.storage, &a.id, &old);
    assert!(
        fixture
            .storage
            .mark_reply_notice_fallback_attempted(&a.id, &old)
            .unwrap()
    );
    let SendClaim::Waiting(active) = fixture
        .storage
        .claim_reply_notice_send(&b.id, &new)
        .unwrap()
    else {
        panic!("sender must wait")
    };
    let mut stale = active.clone();
    stale.worker = Some(new.clone());
    assert!(
        !fixture.storage.release_reply_notice_send(&stale).unwrap(),
        "a stale process observation must not unlock the pane"
    );
    // The caller's exact-Gone proof is outside SQL; release compares its snapshot.
    assert!(fixture.storage.release_reply_notice_send(&active).unwrap());
    assert!(!fixture.storage.release_reply_notice_send(&active).unwrap());
    assert!(fixture.storage.reply_notice_batch(&a.id).unwrap().is_none());
    assert_eq!(
        claim_send(&mut fixture.storage, &b.id, &new)[0].request_id,
        "second"
    );
    assert_eq!(
        service(&mut fixture)
            .notification("first")
            .unwrap()
            .unwrap()
            .reply,
        WakeState::Uncertain
    );
    assert!(matches!(
        service(&mut fixture).get_response("first").unwrap(),
        tmt_core::request::ResponseLookup::Available(_)
    ));
}
