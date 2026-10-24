use super::support::{Fixture, NOW_MS, preamble_count, service};
use crate::storage::test_support::{Operation, concurrent_pair};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    operation::new_operation_id,
    request::{
        Originator, PreambleReservation, PrepareRequest, RequestError, RequestKind, RequestRoute,
        RequestService, ResponseProof, SubmitResponse, WakeState,
        digest::{
            DeliveryPolicy, DigestKind, DigestOpportunity, DigestPolicyWrite, DigestRejection,
            DigestState,
        },
        notification::NotificationPolicy,
    },
};

#[test]
fn notice_admission_errors_never_report_a_claimed_wake() {
    let mut f = Fixture::new();
    let hint = tmt_core::request::notification::OriginatorHint {
        request_id: "missing-request".into(),
        originator_id: f.identity_id.clone(),
        recipient_id: None,
        kind: tmt_core::request::notification::HintKind::Reply,
        timeout_ms: 1000,
    };
    let error = crate::delivery::notify(&mut f.storage, &hint).unwrap_err();
    assert!(error.message.contains("Request attempt was not found"));
    let error = crate::reply_notice::prepare(&mut f.storage, &hint)
        .err()
        .unwrap();
    assert!(error.message.contains("Request attempt was not found"));
    f.storage.close().unwrap();
}

#[test]
fn reply_notice_admission_error_keeps_unattempted_batch_pending_for_a_fenced_retry() {
    use tmt_core::{endpoint::ProcessIncarnation, request::notification::batch::SendClaim};
    let (mut f, target, owner, sender) = start();
    f.set_now(crate::request_runtime::wall_time_ms());
    service(&mut f)
        .write_digest(policy(
            &target,
            &owner,
            1,
            tmt_core::limits::MAX_JS_SAFE_INTEGER,
        ))
        .unwrap();
    let prepared = held(&mut f, &target, &sender, "admission-error");
    let (_, hint) = service(&mut f)
        .submit_response_with_hint(
            SubmitResponse {
                request_id: "admission-error".into(),
                proof: ResponseProof::Compact(tmt_core::request::correlation::response_token(
                    "admission-error",
                    &prepared.attempt_id,
                    &RequestRoute::Inbox {
                        recipient_identity_id: target,
                    },
                )),
                body: "Durable final".into(),
            },
            None,
        )
        .unwrap();
    let hint = hint.unwrap();
    let binding = new_operation_id();
    let batch = f
        .storage
        .queue_reply_notice(
            &hint,
            &binding,
            "queued",
            0,
            0,
            crate::request_runtime::wall_time_ms(),
        )
        .unwrap();
    let worker = ProcessIncarnation::new(12, "original fixture worker").unwrap();
    assert!(
        f.storage
            .claim_reply_notice_worker(&batch, &worker)
            .unwrap()
    );
    let SendClaim::Ready(notices) = f
        .storage
        .claim_reply_notice_send(&batch.id, &worker)
        .unwrap()
    else {
        panic!("original fixture worker must claim the queued frame");
    };
    service(&mut f)
        .write_digest(policy(
            &sender,
            &owner,
            0,
            tmt_core::limits::MAX_JS_SAFE_INTEGER,
        ))
        .unwrap();
    let db = rusqlite::Connection::open(&f.database).unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_digest_notice BEFORE INSERT ON focus_items
        BEGIN SELECT RAISE(ABORT, 'injected digest hold failure'); END;",
    )
    .unwrap();
    assert!(crate::delivery::notify(&mut f.storage, &hint).is_err());
    assert!(
        crate::delivery::send_reply_notices(
            &mut f.storage,
            &batch,
            &worker,
            &notices,
            std::time::Duration::ZERO
        )
        .is_err()
    );
    let pending = f.storage.reply_notice_batch(&batch.id).unwrap().unwrap();
    assert!(pending.pending && pending.sending);
    assert_eq!(
        f.storage
            .pending_reply_notice_batches(&binding)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT attempted FROM reply_notices WHERE request_id='admission-error'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        service(&mut f)
            .notification("admission-error")
            .unwrap()
            .unwrap()
            .reply,
        WakeState::Claimed
    );
    assert!(
        matches!(service(&mut f).get_response("admission-error").unwrap(),
        tmt_core::request::ResponseLookup::Available(response) if response.body == "Durable final")
    );

    db.execute_batch("DROP TRIGGER fail_digest_notice").unwrap();
    // Simulate the caller's independent proof that the exact old fixture worker
    // is gone. Both release and replacement election remain incarnation-fenced.
    assert!(f.storage.release_reply_notice_send(&pending).unwrap());
    let replacement = ProcessIncarnation::new(12, "replacement fixture worker").unwrap();
    let pending = f.storage.reply_notice_batch(&batch.id).unwrap().unwrap();
    assert!(
        f.storage
            .claim_reply_notice_worker(&pending, &replacement)
            .unwrap()
    );
    let SendClaim::Ready(notices) = f
        .storage
        .claim_reply_notice_send(&batch.id, &replacement)
        .unwrap()
    else {
        panic!("replacement must receive the untouched pending frame");
    };
    crate::delivery::send_reply_notices(
        &mut f.storage,
        &batch,
        &replacement,
        &notices,
        std::time::Duration::ZERO,
    )
    .unwrap();
    assert!(f.storage.reply_notice_batch(&batch.id).unwrap().is_none());
    assert_eq!(
        service(&mut f)
            .digest_checklist_items(&sender, None, 0, 128)
            .unwrap()
            .1,
        1
    );
    assert_eq!(
        service(&mut f)
            .notification("admission-error")
            .unwrap()
            .unwrap()
            .reply,
        WakeState::Unavailable
    );
    drop(db);
    f.storage.close().unwrap();
}

#[test]
fn request_housekeeping_prunes_empty_settled_checklists_but_retains_unknown_claims() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "retained-digest");
    let unsent = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &unsent.id,
            &unsent.attempt_token,
            DigestState::Unsent,
        )
        .unwrap();
    let delivered = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &delivered.id,
            &delivered.attempt_token,
            DigestState::Delivered,
        )
        .unwrap();
    held(&mut f, &target, &sender, "uncertain-digest");
    let uncertain = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &uncertain.id,
            &uncertain.attempt_token,
            DigestState::Uncertain,
        )
        .unwrap();
    held(&mut f, &target, &sender, "claimed-digest");
    let claimed = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    let db = rusqlite::Connection::open(&f.database).unwrap();
    let count = || {
        db.query_row("SELECT COUNT(*) FROM focus_checklists", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
    };
    service(&mut f).cleanup().unwrap();
    assert_eq!(count(), 4);
    f.set_now(NOW_MS + tmt_core::retention::METADATA_SETTLEMENT_FLOOR_MS);
    service(&mut f).cleanup().unwrap();
    assert_eq!(count(), 3); // Empty definitely-unsent bookkeeping only.
    assert!(
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM focus_checklists WHERE id=?)",
            [&delivered.id],
            |row| row.get::<_, bool>(0)
        )
        .unwrap()
    );
    f.set_now(NOW_MS + 7 * super::support::DAY_MS);
    service(&mut f).cleanup().unwrap();
    assert_eq!(count(), 2);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM focus_items", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    for (batch, state) in [(&claimed, "claimed"), (&uncertain, "uncertain")] {
        assert_eq!(
            db.query_row(
                "SELECT state FROM focus_checklists WHERE id=?",
                [&batch.id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            state
        );
    }
    assert!(
        service(&mut f)
            .digest_policies(std::slice::from_ref(&target))
            .unwrap()[0]
            .active_checklist
            .as_ref()
            .is_some_and(|b| b.id == claimed.id)
    );
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .is_none()
    );
    drop(db);
    f.storage.close().unwrap();
}

#[test]
fn a_retired_snapshot_member_does_not_finish_its_untouched_reply_batch_peer() {
    use tmt_core::{endpoint::ProcessIncarnation, request::notification::batch::SendClaim};
    let (mut f, target, owner, sender) = start();
    f.set_now(crate::request_runtime::wall_time_ms());
    service(&mut f)
        .write_digest(policy(
            &target,
            &owner,
            1,
            tmt_core::limits::MAX_JS_SAFE_INTEGER,
        ))
        .unwrap();
    let binding = new_operation_id();
    let mut batch = None;
    for id in ["a-retired-member", "b-retained-peer"] {
        let prepared = held(&mut f, &target, &sender, id);
        let (_, hint) = service(&mut f)
            .submit_response_with_hint(
                SubmitResponse {
                    request_id: id.into(),
                    proof: ResponseProof::Compact(tmt_core::request::correlation::response_token(
                        id,
                        &prepared.attempt_id,
                        &RequestRoute::Inbox {
                            recipient_identity_id: target.clone(),
                        },
                    )),
                    body: format!("Final for {id}"),
                },
                None,
            )
            .unwrap();
        let queued = f
            .storage
            .queue_reply_notice(
                &hint.unwrap(),
                &binding,
                "queued",
                1000,
                0,
                crate::request_runtime::wall_time_ms(),
            )
            .unwrap();
        if let Some(previous) = &batch {
            assert_eq!(previous, &queued.id);
        }
        batch = Some(queued.id);
    }
    let batch = f
        .storage
        .reply_notice_batch(&batch.unwrap())
        .unwrap()
        .unwrap();
    let worker = ProcessIncarnation::new(12, "snapshot fixture worker").unwrap();
    assert!(
        f.storage
            .claim_reply_notice_worker(&batch, &worker)
            .unwrap()
    );
    let SendClaim::Ready(notices) = f
        .storage
        .claim_reply_notice_send(&batch.id, &worker)
        .unwrap()
    else {
        panic!("fixture worker must snapshot both retained frames");
    };
    assert_eq!(notices.len(), 2);
    // Canonical deletion can race the preflight after its send snapshot. Preserve
    // real foreign-key cascades; the peer remains valid and never attempted.
    let db = rusqlite::Connection::open(&f.database).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute(
        "DELETE FROM request_attempts WHERE request_id='a-retired-member'",
        [],
    )
    .unwrap();
    let error = crate::delivery::send_reply_notices(
        &mut f.storage,
        &batch,
        &worker,
        &notices,
        std::time::Duration::ZERO,
    )
    .unwrap_err();
    assert!(error.message.contains("Request attempt was not found"));
    let pending = f.storage.reply_notice_batch(&batch.id).unwrap().unwrap();
    assert!(pending.pending && pending.sending);
    assert_eq!(
        db.query_row(
            "SELECT attempted FROM reply_notices WHERE request_id='b-retained-peer'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        f.storage
            .pending_reply_notice_batches(&binding)
            .unwrap()
            .len(),
        1
    );
    assert!(
        matches!(service(&mut f).get_response("b-retained-peer").unwrap(),
        tmt_core::request::ResponseLookup::Available(response) if response.body == "Final for b-retained-peer")
    );
    drop(db);
    f.storage.close().unwrap();
}

pub(super) fn identity(f: &mut Fixture, name: &str) -> String {
    create_or_resolve(&mut f.storage, name, Lifetime::Saved)
        .unwrap()
        .identity
        .id
}
pub(super) fn policy(target: &str, owner: &str, revision: u64, until: u64) -> DigestPolicyWrite {
    DigestPolicyWrite {
        identity_id: target.into(),
        owner_identity_id: owner.into(),
        setter_identity_id: owner.into(),
        expected_revision: revision,
        until_ms: until,
        max_hold_ms: None,
    }
}
pub(super) fn request(target: &str, sender: Option<&str>, id: &str) -> PrepareRequest {
    PrepareRequest {
        origin: tmt_core::request::RequestOrigin::Person,
        room_id: None,
        kind: RequestKind::Request,
        request_id: id.into(),
        message: format!("First line for {id}\nSecond line"),
        route: RequestRoute::Inbox {
            recipient_identity_id: target.into(),
        },
        wait: true,
        expires_at_ms: NOW_MS + 3_600_000,
        originator: sender.map_or(Originator::Unknown, |id| Originator::Explicit(id.into())),
        recipient_identity_id: Some(target.into()),
        preamble: None,
    }
}
pub(super) fn held(
    f: &mut Fixture,
    target: &str,
    sender: &str,
    id: &str,
) -> tmt_core::request::PreparedRequest {
    service(f)
        .prepare_delivery(
            request(target, Some(sender), id),
            format!("attempt-{id}"),
            7,
            DeliveryPolicy::default(),
            Some(NotificationPolicy {
                deadline_ms: NOW_MS + 1000,
                timeout_ms: 1000,
                waiter: None,
            }),
        )
        .unwrap()
}
pub(super) fn start() -> (Fixture, String, String, String) {
    let mut f = Fixture::new();
    let target = f.identity_id.clone();
    let owner = identity(&mut f, "Digest Owner");
    let sender = identity(&mut f, "Digest Sender");
    service(&mut f)
        .write_digest(policy(&target, &owner, 0, NOW_MS + 1000))
        .unwrap();
    (f, target, owner, sender)
}

#[test]
fn digest_admission_atomically_publishes_detached_attention_and_notice_policy() {
    let (mut f, target, _, sender) = start();
    let p = held(&mut f, &target, &sender, "held");
    assert_eq!(p.digest_until_ms, Some(NOW_MS + 1000));
    let a = service(&mut f).get_attempt(&p.attempt_id).unwrap().unwrap();
    assert!(!a.wait_active);
    assert_eq!(a.status, tmt_core::request::AttemptStatus::Queued);
    assert!(!a.cadence_reserved);
    assert!(service(&mut f).notification("held").unwrap().is_some());
    assert!(!service(&mut f).claim_wake("held").unwrap().claimed);
    let (items, count) = service(&mut f)
        .digest_checklist_items(&target, None, 0, 128)
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(items[0].request_id, "held");
    assert_eq!(
        service(&mut f)
            .list_incoming(&target, None, None, None)
            .unwrap()
            .items
            .len(),
        1
    );
    f.storage.close().unwrap();
}

#[test]
fn digest_enabled_between_publication_and_wake_blocks_the_original_claim() {
    let (mut f, target, owner, sender) = start();
    service(&mut f)
        .write_digest(policy(&target, &owner, 1, 0))
        .unwrap();
    let mut input = request(&target, Some(&sender), "late-digest");
    input.wait = false;
    input.preamble = Some(PreambleReservation {
        identity_id: target.clone(),
        every: 3,
    });
    let prepared = service(&mut f)
        .enqueue_delivery(input, "late-attempt".into(), 7, DeliveryPolicy::default())
        .unwrap();
    assert_eq!(prepared.digest_until_ms, None);
    assert_eq!(preamble_count(&f.database, &target), 1);
    service(&mut f)
        .write_digest(policy(&target, &owner, 2, NOW_MS + 1000))
        .unwrap();
    let wake = service(&mut f).claim_wake("late-digest").unwrap();
    assert!(!wake.claimed);
    assert_eq!(wake.state, WakeState::NotAttempted);
    assert_eq!(wake.digest_until_ms, Some(NOW_MS + 1000));
    assert_eq!(preamble_count(&f.database, &target), 0);
    assert!(
        !service(&mut f)
            .get_attempt(&prepared.attempt_id)
            .unwrap()
            .unwrap()
            .cadence_reserved
    );
    assert_eq!(
        service(&mut f)
            .digest_checklist_items(&target, None, 0, 128)
            .unwrap()
            .1,
        1
    );
    service(&mut f)
        .write_digest(policy(&target, &owner, 3, 0))
        .unwrap();
    // Clearing after admission cannot erase the captured held decision.
    assert_eq!(wake.digest_until_ms, Some(NOW_MS + 1000));
    let after_clear = service(&mut f).claim_wake("late-digest").unwrap();
    assert!(!after_clear.claimed);
    assert_eq!(after_clear.digest_until_ms, Some(0));
    f.storage.close().unwrap();
}

#[test]
fn withdrawn_and_expired_members_never_offer_a_live_reply_or_renew_retention() {
    let (mut f, target, _, sender) = start();
    let prepared = held(&mut f, &target, &sender, "obsolete");
    service(&mut f)
        .withdraw_request(&sender, "obsolete", "superseded")
        .unwrap();
    assert!(
        !service(&mut f)
            .digest_reply_context("obsolete")
            .unwrap()
            .unwrap()
            .1
    );
    let before = service(&mut f)
        .get_attempt(&prepared.attempt_id)
        .unwrap()
        .unwrap();
    f.set_now(before.retention_expires_at_ms);
    assert!(
        service(&mut f)
            .digest_reply_context("obsolete")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        service(&mut f)
            .digest_checklist_items(&target, None, 0, 128)
            .unwrap()
            .1,
        0
    );
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .is_none()
    );
    f.storage.close().unwrap();
}

#[test]
fn owner_and_urgent_bypass_digest_but_explicit_inbox_stays_pull_only() {
    let (mut f, target, owner, sender) = start();
    for (id, originator, urgent, automatic) in [
        ("owner", owner.as_str(), false, true),
        ("urgent", sender.as_str(), true, true),
        ("pull", sender.as_str(), false, false),
    ] {
        let mut input = request(&target, Some(originator), id);
        input.wait = false;
        let p = service(&mut f)
            .enqueue_delivery(
                input,
                format!("attempt-{id}"),
                7,
                DeliveryPolicy {
                    urgent,
                    automatic,
                    kind: DigestKind::Review,
                    ..DeliveryPolicy::default()
                },
            )
            .unwrap();
        assert_eq!(p.digest_until_ms, None);
        assert_eq!(
            service(&mut f)
                .request_detail(id)
                .unwrap()
                .item
                .delivery_policy
                .urgent,
            urgent
        );
        assert_eq!(
            service(&mut f)
                .show_incoming_request(&target, id)
                .unwrap()
                .exchange
                .delivery_policy
                .kind,
            DigestKind::Review
        );
    }
    assert_eq!(
        service(&mut f).digest_policies(&[target]).unwrap()[0].held_count,
        0
    );
    f.storage.close().unwrap();
}

#[test]
fn checklist_seals_order_and_later_arrivals_cannot_join_or_replay() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "a");
    held(&mut f, &target, &sender, "b");
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::Idle,
                None,
            )
            .unwrap()
            .is_none()
    );
    let id = new_operation_id();
    let token = new_operation_id();
    let batch = service(&mut f)
        .claim_digest_checklist(
            &target,
            id.clone(),
            token.clone(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    held(&mut f, &target, &sender, "c");
    let (items, count) = service(&mut f)
        .digest_checklist_items(&target, Some(&id), 0, 1)
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(items[0].request_id, "a");
    let (next, count) = service(&mut f)
        .digest_checklist_items(&target, Some(&id), items[0].sequence, 128)
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(next[0].request_id, "b");
    assert_eq!(batch.through_sequence, next[0].sequence);
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                id.clone(),
                token.clone(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .is_none()
    );
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .is_none()
    );
    assert!(
        service(&mut f)
            .settle_digest_checklist(&target, &id, &token, DigestState::Delivered)
            .unwrap()
    );
    assert!(
        !service(&mut f)
            .settle_digest_checklist(&target, &id, &token, DigestState::Delivered)
            .unwrap()
    );
    assert!(
        service(&mut f)
            .settle_digest_checklist(&target, &id, &token, DigestState::Unsent)
            .is_err()
    );
    assert!(!service(&mut f).claim_wake("a").unwrap().claimed);
    let (items, count) = service(&mut f)
        .digest_checklist_items(&target, None, 0, 128)
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(items[0].request_id, "c");
    f.storage.close().unwrap();
}

#[test]
fn expiry_and_clear_expose_one_backlog_without_renewing_receipts() {
    for clear in [false, true] {
        let (mut f, target, owner, sender) = start();
        let p = held(&mut f, &target, &sender, "a");
        let before = service(&mut f).get_attempt(&p.attempt_id).unwrap().unwrap();
        if clear {
            service(&mut f)
                .write_digest(policy(&target, &owner, 1, 0))
                .unwrap();
        } else {
            f.set_now(NOW_MS + 1000);
        }
        let batch = service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::Idle,
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            service(&mut f)
                .digest_checklist_items(&target, Some(&batch.id), 0, 128)
                .unwrap()
                .1,
            1
        );
        let (_, replyable) = service(&mut f).digest_reply_context("a").unwrap().unwrap();
        assert!(replyable);
        assert_eq!(
            before.retention_expires_at_ms,
            service(&mut f)
                .get_attempt(&p.attempt_id)
                .unwrap()
                .unwrap()
                .retention_expires_at_ms
        );
        service(&mut f)
            .submit_response(SubmitResponse {
                request_id: "a".into(),
                proof: ResponseProof::Compact(tmt_core::request::correlation::response_token(
                    "a",
                    &p.attempt_id,
                    &RequestRoute::Inbox {
                        recipient_identity_id: target.clone(),
                    },
                )),
                body: "Exact final".into(),
            })
            .unwrap();
        assert!(
            !service(&mut f)
                .digest_reply_context("a")
                .unwrap()
                .unwrap()
                .1
        );
        assert!(matches!(
            service(&mut f).get_response("a").unwrap(),
            tmt_core::request::ResponseLookup::Available(_)
        ));
        f.storage.close().unwrap();
    }
}

#[test]
fn definitely_unsent_releases_members_but_uncertainty_never_replays() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "a");
    for state in [DigestState::Unsent, DigestState::Uncertain] {
        let batch = service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .unwrap();
        assert!(matches!(
            service(&mut f).settle_digest_checklist(&target, &batch.id, &new_operation_id(), state),
            Err(RequestError::Digest(DigestRejection::AttemptMismatch))
        ));
        service(&mut f)
            .settle_digest_checklist(&target, &batch.id, &batch.attempt_token, state)
            .unwrap();
    }
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .is_none()
    );
    assert!(!service(&mut f).claim_wake("a").unwrap().claimed);
    f.storage.close().unwrap();
}

#[test]
fn concurrent_policy_writers_and_checklist_claims_have_one_winner() {
    let (mut f, target, owner, sender) = start();
    held(&mut f, &target, &sender, "a");
    let writes: [Operation<bool>; 2] = [NOW_MS + 2000, NOW_MS + 3000].map(|until| {
        let target = target.clone();
        let owner = owner.clone();
        Box::new(move |s: &mut crate::storage::Storage| {
            match RequestService::new(s, || NOW_MS).write_digest(policy(&target, &owner, 1, until))
            {
                Ok(_) => true,
                Err(RequestError::Digest(DigestRejection::Conflict)) => false,
                other => panic!("Expected one policy success and one revision conflict: {other:?}"),
            }
        }) as Operation<bool>
    });
    assert_eq!(
        concurrent_pair(&f.database, writes)
            .into_iter()
            .filter(|v| *v)
            .count(),
        1
    );
    let claims: [Operation<bool>; 2] = [0, 1].map(|_| {
        let target = target.clone();
        Box::new(move |s: &mut crate::storage::Storage| {
            RequestService::new(s, || NOW_MS)
                .claim_digest_checklist(
                    &target,
                    new_operation_id(),
                    new_operation_id(),
                    DigestOpportunity::TurnBoundary,
                    None,
                )
                .unwrap()
                .is_some()
        }) as Operation<bool>
    });
    assert_eq!(
        concurrent_pair(&f.database, claims)
            .into_iter()
            .filter(|v| *v)
            .count(),
        1
    );
    f.storage.close().unwrap();
}

#[test]
fn result_notice_is_held_and_deduplicated_independently_from_incoming() {
    let (mut f, target, owner, sender) = start();
    service(&mut f)
        .write_digest(policy(&sender, &owner, 0, NOW_MS + 1000))
        .unwrap();
    let p = held(&mut f, &target, &sender, "a");
    service(&mut f)
        .submit_response(SubmitResponse {
            request_id: "a".into(),
            proof: ResponseProof::Compact(tmt_core::request::correlation::response_token(
                "a",
                &p.attempt_id,
                &RequestRoute::Inbox {
                    recipient_identity_id: target,
                },
            )),
            body: "final".into(),
        })
        .unwrap();
    assert!(service(&mut f).hold_reply_notice("a").unwrap());
    assert!(service(&mut f).hold_reply_notice("a").unwrap());
    let (items, count) = service(&mut f)
        .digest_checklist_items(&sender, None, 0, 128)
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(items[0].kind, DigestKind::Result);
    assert_ne!(
        service(&mut f).notification("a").unwrap().unwrap().reply,
        WakeState::Sent
    );
    f.storage.close().unwrap();
}

#[test]
fn digest_set_after_reply_enqueue_fences_frame_and_joined_input_claims() {
    use tmt_core::{endpoint::ProcessIncarnation, request::notification::batch::SendClaim};
    for joined in [false, true] {
        let mut f = Fixture::new();
        f.set_now(crate::request_runtime::wall_time_ms());
        let target = f.identity_id.clone();
        let owner = identity(&mut f, "Owner");
        let sender = identity(&mut f, "Sender");
        service(&mut f)
            .write_digest(policy(
                &target,
                &owner,
                0,
                tmt_core::limits::MAX_JS_SAFE_INTEGER,
            ))
            .unwrap();
        let p = held(&mut f, &target, &sender, "frame");
        let (_, hint) = service(&mut f)
            .submit_response_with_hint(
                SubmitResponse {
                    request_id: "frame".into(),
                    proof: ResponseProof::Compact(tmt_core::request::correlation::response_token(
                        "frame",
                        &p.attempt_id,
                        &RequestRoute::Inbox {
                            recipient_identity_id: target,
                        },
                    )),
                    body: "final".into(),
                },
                None,
            )
            .unwrap();
        let hint = hint.unwrap();
        let batch = f
            .storage
            .queue_reply_notice(
                &hint,
                &new_operation_id(),
                "old queued result",
                0,
                0,
                crate::request_runtime::wall_time_ms(),
            )
            .unwrap();
        let worker = ProcessIncarnation::new(12, "owned worker").unwrap();
        let stale = ProcessIncarnation::new(12, "old worker").unwrap();
        assert!(
            f.storage
                .claim_reply_notice_worker(&batch, &worker)
                .unwrap()
        );
        assert!(matches!(
            f.storage
                .claim_reply_notice_send(&batch.id, &worker)
                .unwrap(),
            SendClaim::Ready(_)
        ));
        service(&mut f)
            .write_digest(policy(
                &sender,
                &owner,
                0,
                tmt_core::limits::MAX_JS_SAFE_INTEGER,
            ))
            .unwrap();
        assert!(
            !f.storage
                .mark_reply_notice_attempted(&batch.id, "frame", &stale)
                .unwrap()
        );
        assert_eq!(
            service(&mut f)
                .digest_policies(std::slice::from_ref(&sender))
                .unwrap()[0]
                .held_count,
            0
        );
        let claimed = if joined {
            f.storage
                .mark_reply_notice_fallback_attempted(&batch.id, &worker)
        } else {
            f.storage
                .mark_reply_notice_attempted(&batch.id, "frame", &worker)
        }
        .unwrap();
        assert!(!claimed);
        assert_eq!(
            service(&mut f).digest_policies(&[sender]).unwrap()[0].held_count,
            1
        );
        assert_eq!(
            service(&mut f)
                .notification("frame")
                .unwrap()
                .unwrap()
                .reply,
            WakeState::Unavailable
        );
        f.storage.finish_reply_notice_batch(&batch.id).unwrap();
        assert!(f.storage.reply_notice_batch(&batch.id).unwrap().is_none());
        f.storage.close().unwrap();
    }
}

#[test]
fn digest_reads_do_not_housekeep_acknowledge_or_renew_retained_data() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "read");
    let before = f.storage.change_cursor().unwrap();
    let original = service(&mut f)
        .get_attempt("attempt-read")
        .unwrap()
        .unwrap();
    service(&mut f)
        .digest_policies(std::slice::from_ref(&target))
        .unwrap();
    service(&mut f)
        .digest_checklist_items(&target, None, 0, 1)
        .unwrap();
    service(&mut f).digest_reply_context("read").unwrap();
    service(&mut f).digest_result_context("read").unwrap();
    assert_eq!(f.storage.change_cursor().unwrap(), before);
    assert_eq!(
        service(&mut f)
            .get_attempt("attempt-read")
            .unwrap()
            .unwrap(),
        original
    );
    f.storage.close().unwrap();
}

#[test]
fn due_now_seals_only_the_tick_range_at_idle_and_keeps_policy_active() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "tick-first");
    held(&mut f, &target, &sender, "tick-second");
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::Idle,
                None,
            )
            .unwrap()
            .is_none()
    );
    let due = service(&mut f).make_digest_due(&target).unwrap();
    assert_eq!(due.held_count, 2);
    assert_eq!(service(&mut f).make_digest_due(&target).unwrap(), due);
    f.set_now(NOW_MS + 100);
    held(&mut f, &target, &sender, "tick-later");
    let stats = service(&mut f)
        .digest_stats(std::slice::from_ref(&target))
        .unwrap()
        .remove(0);
    assert_eq!(
        (stats.held_count, stats.due_count, stats.oldest_held_age_ms),
        (3, 2, Some(100))
    );
    assert_eq!(stats.next_eligible_at_ms, Some(NOW_MS + 100));
    let batch = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::Idle,
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(batch.through_sequence, due.through_sequence);
    let (items, count) = service(&mut f)
        .digest_checklist_items(&target, Some(&batch.id), 0, 128)
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(
        items
            .iter()
            .map(|i| i.request_id.as_str())
            .collect::<Vec<_>>(),
        ["tick-first", "tick-second"]
    );
    assert!(
        service(&mut f)
            .settle_digest_checklist(
                &target,
                &batch.id,
                &batch.attempt_token,
                DigestState::Delivered
            )
            .unwrap()
    );
    assert!(
        !service(&mut f)
            .settle_digest_checklist(
                &target,
                &batch.id,
                &batch.attempt_token,
                DigestState::Delivered
            )
            .unwrap()
    );
    let stats = service(&mut f)
        .digest_stats(std::slice::from_ref(&target))
        .unwrap()
        .remove(0);
    assert_eq!(
        (stats.held_count, stats.due_count, stats.delivered_digests),
        (1, 0, 1)
    );
    assert_eq!(stats.next_eligible_at_ms, Some(NOW_MS + 1000));
    assert!(
        service(&mut f)
            .digest_policies(std::slice::from_ref(&target))
            .unwrap()[0]
            .policy
            .as_ref()
            .unwrap()
            .active(NOW_MS + 100)
    );
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::Idle,
                None,
            )
            .unwrap()
            .is_none()
    );
    // A verified turn boundary retains its existing additional eligibility.
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .is_some()
    );
}

#[test]
fn due_watermark_and_delivery_counter_survive_restart_and_retention_cleanup() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "restart-due");
    service(&mut f).make_digest_due(&target).unwrap();
    f.storage.close().unwrap();
    f.storage = crate::storage::Storage::open(&f.database).unwrap();
    let batch = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::Idle,
            None,
        )
        .unwrap()
        .unwrap();
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &batch.id,
            &batch.attempt_token,
            DigestState::Delivered,
        )
        .unwrap();
    f.set_now(NOW_MS + 7 * super::support::DAY_MS);
    service(&mut f).cleanup().unwrap();
    f.storage.close().unwrap();
    f.storage = crate::storage::Storage::open(&f.database).unwrap();
    let stats = service(&mut f)
        .digest_stats(std::slice::from_ref(&target))
        .unwrap()
        .remove(0);
    assert_eq!(
        (
            stats.held_count,
            stats.delivered_digests,
            stats.oldest_held_age_ms,
            stats.next_eligible_at_ms
        ),
        (0, 1, None, None)
    );
    let db = rusqlite::Connection::open(&f.database).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM focus_checklists WHERE id=?",
            [&batch.id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn arrival_context_uses_the_recorded_driver_usage_once_and_preserves_unknown() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "unknown-context");
    let db = rusqlite::Connection::open(&f.database).unwrap();
    let usage = crate::runtime::driver_state::Usage::new(182340, Some(200000), NOW_MS - 1).unwrap();
    let state = crate::runtime::driver_state::after_start(
        Some("claude-sonnet"),
        Some(usage),
        None,
        NOW_MS - 1,
    )
    .unwrap();
    db.execute("INSERT INTO identity_session_preferences(identity_id,remembered_harness,runtime_mode,provider_session_id,driver_state_version,driver_state) VALUES(?1,'claude','default','arrival-session',?2,?3)", rusqlite::params![target,state.version(),state.document()]).unwrap();
    held(&mut f, &target, &sender, "known-context");
    db.execute("UPDATE identity_session_preferences SET driver_state=NULL,driver_state_version=NULL WHERE identity_id=?",[&target]).unwrap();
    // A repeated held admission must preserve the first arrival observation.
    assert!(!service(&mut f).claim_wake("known-context").unwrap().claimed);
    let (items, _) = service(&mut f)
        .digest_checklist_items(&target, None, 0, 128)
        .unwrap();
    assert_eq!(
        (
            items[0].context_tokens_at_arrival,
            items[0].context_observed_at_ms
        ),
        (None, None)
    );
    assert_eq!(
        (
            items[1].context_tokens_at_arrival,
            items[1].context_observed_at_ms
        ),
        (Some(182340), Some(NOW_MS - 1))
    );
    f.storage.close().unwrap();
    f.storage = crate::storage::Storage::open(&f.database).unwrap();
    let (items, _) = service(&mut f)
        .digest_checklist_items(&target, None, 0, 128)
        .unwrap();
    assert_eq!(items[1].context_tokens_at_arrival, Some(182340));
}

#[test]
fn digest_counter_failure_rolls_back_settlement_and_non_delivery_never_counts() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "counter-failure");
    let batch = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    let db = rusqlite::Connection::open(&f.database).unwrap();
    db.execute_batch("CREATE TRIGGER fail_digest_counter BEFORE INSERT ON digest_counters BEGIN SELECT RAISE(ABORT,'injected counter refusal'); END;").unwrap();
    assert!(
        service(&mut f)
            .settle_digest_checklist(
                &target,
                &batch.id,
                &batch.attempt_token,
                DigestState::Delivered
            )
            .is_err()
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM focus_checklists WHERE id=?",
            [&batch.id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "claimed"
    );
    assert_eq!(
        service(&mut f)
            .digest_stats(std::slice::from_ref(&target))
            .unwrap()[0]
            .delivered_digests,
        0
    );
    db.execute_batch("DROP TRIGGER fail_digest_counter")
        .unwrap();
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &batch.id,
            &batch.attempt_token,
            DigestState::Delivered,
        )
        .unwrap();
    for state in [DigestState::Unsent, DigestState::Uncertain] {
        held(&mut f, &target, &sender, state.as_str());
        service(&mut f).make_digest_due(&target).unwrap();
        let next = service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::Idle,
                None,
            )
            .unwrap()
            .unwrap();
        service(&mut f)
            .settle_digest_checklist(&target, &next.id, &next.attempt_token, state)
            .unwrap();
        assert_eq!(
            service(&mut f)
                .digest_stats(std::slice::from_ref(&target))
                .unwrap()[0]
                .delivered_digests,
            1
        );
    }
}

/// The reserved `user` entry, written the way `tmt identity meta set` stores it.
pub(super) fn mark_user(f: &Fixture, id: &str, value: &str) {
    let db = rusqlite::Connection::open(&f.database).unwrap();
    db.execute(
        "INSERT INTO identity_metadata(identity_id, key, value) VALUES(?1, 'user', ?2)
         ON CONFLICT(identity_id, key) DO UPDATE SET value = excluded.value",
        [id, value],
    )
    .unwrap();
}

#[test]
fn a_person_is_never_held_while_agents_still_are() {
    let (mut f, target, _, sender) = start();
    let person = identity(&mut f, "Ben");
    let held_by = |f: &mut Fixture, from: &str, id: &str| {
        service(f)
            .prepare_delivery(
                request(&target, Some(from), id),
                format!("attempt-{id}"),
                7,
                DeliveryPolicy::default(),
                None,
            )
            .unwrap()
            .digest_until_ms
            .is_some()
    };
    // Unmarked, a person is an identity like any other.
    assert!(held_by(&mut f, &person, "before-marking"));
    mark_user(&f, &person, "true");
    assert!(!held_by(&mut f, &person, "person"));
    assert!(held_by(&mut f, &sender, "agent"));
    // Only the exact entry counts, and it is read when the message arrives.
    for value in ["false", "yes", "True", ""] {
        mark_user(&f, &person, value);
        assert!(
            held_by(&mut f, &person, &format!("value-{value}")),
            "{value:?}"
        );
    }
    mark_user(&f, &person, "true");
    assert!(!held_by(&mut f, &person, "person-again"));
    // A retired person is no longer one.
    let retired = identity(&mut f, "Retired person");
    mark_user(&f, &retired, "true");
    rusqlite::Connection::open(&f.database)
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms = ?1 WHERE id = ?2",
            rusqlite::params![NOW_MS as i64, retired],
        )
        .unwrap();
    assert!(!service(&mut f).identity_is_user(&retired).unwrap());
    assert!(service(&mut f).identity_is_user(&person).unwrap());
    // The person's messages are not in the held set, so no checklist can seal them.
    let (items, count) = service(&mut f)
        .digest_checklist_items(&target, None, 0, 128)
        .unwrap();
    assert_eq!(count, 6);
    assert!(
        items
            .iter()
            .all(|item| !item.request_id.starts_with("person"))
    );
    f.storage.close().unwrap();
}

#[test]
fn a_reply_from_a_person_is_never_held_but_an_agents_reply_is() {
    use tmt_core::request::notification::HintKind;
    let (mut f, target, owner, sender) = start();
    f.set_now(crate::request_runtime::wall_time_ms());
    // The target keeps its requests held so they can be answered; the sender is the
    // originator, so its policy decides what happens to the reply notices.
    service(&mut f)
        .write_digest(policy(
            &target,
            &owner,
            1,
            tmt_core::limits::MAX_JS_SAFE_INTEGER,
        ))
        .unwrap();
    service(&mut f)
        .write_digest(policy(
            &sender,
            &owner,
            0,
            tmt_core::limits::MAX_JS_SAFE_INTEGER,
        ))
        .unwrap();
    let answered = |f: &mut Fixture, id: &str| {
        let prepared = held(f, &target, &sender, id);
        service(f)
            .submit_response_with_hint(
                SubmitResponse {
                    request_id: id.into(),
                    proof: ResponseProof::Compact(tmt_core::request::correlation::response_token(
                        id,
                        &prepared.attempt_id,
                        &RequestRoute::Inbox {
                            recipient_identity_id: target.clone(),
                        },
                    )),
                    body: format!("answer {id}"),
                },
                None,
            )
            .unwrap();
        crate::digest::hold_notice(&mut f.storage, id, HintKind::Reply).unwrap()
    };
    // The replier is the request's recipient: an agent's reply waits in the originator's digest...
    assert!(answered(&mut f, "agent-reply"));
    // ...and once that recipient is a person, their reply is never held.
    mark_user(&f, &target, "true");
    assert!(!answered(&mut f, "person-reply"));
    let held_results: Vec<String> = service(&mut f)
        .digest_checklist_items(&sender, None, 0, 128)
        .unwrap()
        .0
        .into_iter()
        .filter(|item| item.source == tmt_core::request::digest::DigestSource::Result)
        .map(|item| item.request_id)
        .collect();
    assert_eq!(held_results, ["agent-reply"]);
    // A person's reply is also admitted without the fixed reply window.
    let hint = tmt_core::request::notification::OriginatorHint {
        request_id: "person-reply".into(),
        originator_id: sender.clone(),
        recipient_id: Some(target.clone()),
        kind: HintKind::Reply,
        timeout_ms: 1000,
    };
    assert!(matches!(
        crate::reply_notice::prepare(&mut f.storage, &hint).unwrap(),
        crate::reply_notice::Prepared::Immediate(_)
    ));
    f.storage.close().unwrap();
}

#[test]
fn an_item_held_before_its_sender_became_a_person_is_still_marked() {
    let (mut f, target, owner, sender) = start();
    // `read` uses the wall clock, so the request must be live in it.
    let wall = crate::request_runtime::wall_time_ms();
    f.set_now(wall);
    service(&mut f)
        .write_digest(policy(
            &target,
            &owner,
            1,
            tmt_core::limits::MAX_JS_SAFE_INTEGER,
        ))
        .unwrap();
    let mut early = request(&target, Some(&sender), "early");
    early.expires_at_ms = wall + 3_600_000;
    service(&mut f)
        .prepare_delivery(
            early,
            "attempt-early".into(),
            7,
            DeliveryPolicy::default(),
            None,
        )
        .unwrap();
    let batch = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    let page = |f: &mut Fixture| {
        crate::digest::read(&mut f.storage, &target, Some(&batch.id), 0, 128).unwrap()
    };
    assert_eq!(page(&mut f)["items"][0]["fromUser"], false);
    let text = crate::digest::text(&mut f.storage, &batch).unwrap();
    assert!(text.contains("▚ fyi · Digest Sender\n"), "{text}");
    assert!(!text.contains("(user)"), "{text}");
    mark_user(&f, &sender, "true");
    assert_eq!(page(&mut f)["items"][0]["fromUser"], true);
    let text = crate::digest::text(&mut f.storage, &batch).unwrap();
    assert!(text.contains("▚ fyi · Digest Sender (user)\n"), "{text}");
    f.storage.close().unwrap();
}

#[test]
fn a_claim_seals_only_the_prefix_up_to_its_limit_and_leaves_the_rest_held_in_order() {
    let (mut f, target, _, sender) = start();
    for id in ["one", "two", "three"] {
        held(&mut f, &target, &sender, id);
    }
    let ids = |items: &[tmt_core::request::digest::DigestItem]| {
        items
            .iter()
            .map(|item| item.request_id.clone())
            .collect::<Vec<_>>()
    };
    let claimable = service(&mut f)
        .digest_claimable_items(&target, DigestOpportunity::TurnBoundary, 128)
        .unwrap();
    assert_eq!(ids(&claimable), ["one", "two", "three"]);
    // While Digest is active an idle opportunity reaches nothing until the range is due.
    assert!(
        service(&mut f)
            .digest_claimable_items(&target, DigestOpportunity::Idle, 128)
            .unwrap()
            .is_empty()
    );
    let batch = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            Some(claimable[1].sequence),
        )
        .unwrap()
        .unwrap();
    assert_eq!(batch.through_sequence, claimable[1].sequence);
    let (sealed, _) = service(&mut f)
        .digest_checklist_items(&target, Some(&batch.id), 0, 128)
        .unwrap();
    assert_eq!(ids(&sealed), ["one", "two"]);
    let rest = service(&mut f)
        .digest_claimable_items(&target, DigestOpportunity::TurnBoundary, 128)
        .unwrap();
    assert_eq!(ids(&rest), ["three"]);
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &batch.id,
            &batch.attempt_token,
            DigestState::Delivered,
        )
        .unwrap();
    let next = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    let (sealed, _) = service(&mut f)
        .digest_checklist_items(&target, Some(&next.id), 0, 128)
        .unwrap();
    assert_eq!(ids(&sealed), ["three"]);
    f.storage.close().unwrap();
}

#[test]
fn a_limit_below_the_first_item_seals_nothing() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "one");
    let first = service(&mut f)
        .digest_claimable_items(&target, DigestOpportunity::TurnBoundary, 128)
        .unwrap()[0]
        .sequence;
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                Some(first - 1),
            )
            .unwrap()
            .is_none()
    );
    f.storage.close().unwrap();
}

/// The paste is capped at whole items: the longest ordered prefix that fits is
/// sealed, and the first item is always sealed even when it alone exceeds the cap.
#[test]
fn the_paste_fits_whole_items_in_order_and_an_oversized_first_item_goes_alone() {
    let wall = crate::request_runtime::wall_time_ms();
    for (sizes, sealed) in [
        (vec![14_000, 14_000, 14_000], 2),
        (vec![40_000, 100], 1),
        (vec![100, 100, 100], 3),
        (vec![100, 40_000, 100], 1),
    ] {
        let (mut f, target, owner, sender) = start();
        f.set_now(wall);
        service(&mut f)
            .write_digest(policy(
                &target,
                &owner,
                1,
                tmt_core::limits::MAX_JS_SAFE_INTEGER,
            ))
            .unwrap();
        for (index, size) in sizes.iter().enumerate() {
            let id = format!("item-{index}");
            let mut prepared = request(&target, Some(&sender), &id);
            prepared.message = "x".repeat(*size);
            prepared.expires_at_ms = wall + 3_600_000;
            service(&mut f)
                .prepare_delivery(
                    prepared,
                    format!("attempt-{id}"),
                    7,
                    DeliveryPolicy::default(),
                    None,
                )
                .unwrap();
        }
        let through = crate::digest::fitting_through(
            &mut f.storage,
            &target,
            DigestOpportunity::TurnBoundary,
            true,
        )
        .unwrap();
        // A provider hook carries at most the cap, so it never takes an oversized first
        // item; the longest fitting prefix is the same otherwise.
        let hook = crate::digest::fitting_through(
            &mut f.storage,
            &target,
            DigestOpportunity::TurnBoundary,
            false,
        )
        .unwrap();
        assert_eq!(hook, (sizes[0] < 30_000).then_some(through).flatten());
        let (batch, text) =
            crate::digest::claim(&mut f.storage, &target, DigestOpportunity::TurnBoundary)
                .unwrap()
                .unwrap();
        assert_eq!(Some(batch.through_sequence), through, "{sizes:?}");
        let (items, _) = service(&mut f)
            .digest_checklist_items(&target, Some(&batch.id), 0, 128)
            .unwrap();
        assert_eq!(items.len(), sealed, "{sizes:?}");
        for index in 0..sealed {
            assert!(text.contains(&"x".repeat(sizes[index])), "{sizes:?}");
        }
        // A sealed item that shares the paste never pushes it past the cap.
        if sealed > 1 {
            assert!(text.len() <= crate::digest::DIGEST_BYTES, "{sizes:?}");
        }
        f.storage.close().unwrap();
    }
}

#[path = "digest_history.rs"]
mod history;

/// A request that Digest holds keeps the raw attempt status `queued` after its checklist is
/// delivered (#4838). Every exchange view carries `messageState`, which does move on.
#[test]
fn every_exchange_view_reads_the_checklist_state_while_the_attempt_stays_queued() {
    use tmt_core::request::{AttemptStatus, delivery::DeliveryState};
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "a");
    let views = |f: &mut Fixture| {
        let incoming = service(f)
            .list_incoming(&target, None, None, None)
            .unwrap()
            .items
            .remove(0)
            .exchange;
        let shown = service(f).show_exchange(&sender, "a").unwrap().exchange;
        let listed = service(f)
            .list_exchanges(&sender, None, None)
            .unwrap()
            .items
            .remove(0);
        let open = service(f)
            .open_requests(&target, None, None)
            .unwrap()
            .items
            .remove(0);
        [
            incoming.message_state.state,
            shown.message_state.state,
            listed.message_state.state,
            open.message_state.state,
        ]
    };
    assert_eq!(views(&mut f), [DeliveryState::Batched; 4]);

    let (id, token) = (new_operation_id(), new_operation_id());
    service(&mut f)
        .claim_digest_checklist(
            &target,
            id.clone(),
            token.clone(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(views(&mut f), [DeliveryState::Sent; 4]);

    assert!(
        service(&mut f)
            .settle_digest_checklist(&target, &id, &token, DigestState::Delivered)
            .unwrap()
    );
    assert_eq!(views(&mut f), [DeliveryState::Delivered; 4]);
    let shown = service(&mut f)
        .show_exchange(&sender, "a")
        .unwrap()
        .exchange;
    assert_eq!(shown.delivery, AttemptStatus::Queued);
    f.storage.close().unwrap();
}

/// Typed at an idle opportunity, then settled `delivered` (#4858): the checklist the tick pastes.
fn type_idle_checklist(
    f: &mut Fixture,
    target: &str,
) -> tmt_core::request::digest::DigestChecklist {
    service(f).make_digest_due(target).unwrap();
    let typed = service(f)
        .claim_digest_checklist(
            target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::Idle,
            None,
        )
        .unwrap()
        .unwrap();
    assert!(
        service(f)
            .settle_digest_checklist(
                target,
                &typed.id,
                &typed.attempt_token,
                DigestState::Delivered
            )
            .unwrap()
    );
    typed
}

fn message_state_of(f: &mut Fixture, request: &str) -> tmt_core::request::delivery::DeliveryState {
    service(f)
        .request_detail(request)
        .unwrap()
        .item
        .delivery_view
        .state
}

/// The 17:21 timeline of #4858: a checklist typed at an idle opportunity into a turn the
/// provider never announced is settled `delivered` and was never a prompt. The recipient's
/// next Stop-hook checklist must offer the same requests again, keyed by request id.
#[test]
fn a_checklist_typed_into_a_running_turn_is_offered_again_at_the_next_stop() {
    use tmt_core::request::delivery::DeliveryState;
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "req-first");
    held(&mut f, &target, &sender, "req-second");
    type_idle_checklist(&mut f, &target);
    // Delivered to the pane, carried by no prompt yet: the sender reads it as unconfirmed.
    assert_eq!(
        message_state_of(&mut f, "req-first"),
        DeliveryState::Unconfirmed
    );

    // The turn ends; no prompt carried the typed checklist. The Stop hook offers it again.
    let offered = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .expect("the unconfirmed items are offered at the next Stop");
    let (items, count) = service(&mut f)
        .digest_checklist_items(&target, Some(&offered.id), 0, 128)
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(
        items
            .iter()
            .map(|i| i.request_id.as_str())
            .collect::<Vec<_>>(),
        ["req-first", "req-second"]
    );
    assert_eq!(message_state_of(&mut f, "req-first"), DeliveryState::Sent);
    assert!(
        service(&mut f)
            .settle_digest_checklist(
                &target,
                &offered.id,
                &offered.attempt_token,
                DigestState::Delivered
            )
            .unwrap()
    );
    // The hook's output is the provider's own channel: that delivery needs no confirmation,
    // and nothing is offered a third time.
    assert_eq!(
        message_state_of(&mut f, "req-first"),
        DeliveryState::Delivered
    );
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .is_none()
    );
    f.storage.close().unwrap();
}

/// The agent may answer a typed request from its inbox, or see a paste mid-turn, without a prompt
/// naming it. The next Stop offers only what still waits for an answer.
#[test]
fn an_answered_request_is_not_offered_again_at_the_next_stop() {
    let (mut f, target, _, sender) = start();
    let answered = held(&mut f, &target, &sender, "req-answered");
    held(&mut f, &target, &sender, "req-open");
    type_idle_checklist(&mut f, &target);
    service(&mut f)
        .submit_response(SubmitResponse {
            request_id: "req-answered".into(),
            proof: ResponseProof::Compact(tmt_core::request::correlation::response_token(
                "req-answered",
                &answered.attempt_id,
                &RequestRoute::Inbox {
                    recipient_identity_id: target.clone(),
                },
            )),
            body: "done from the inbox".into(),
        })
        .unwrap();

    let claimable = service(&mut f)
        .digest_claimable_items(&target, DigestOpportunity::TurnBoundary, 128)
        .unwrap();
    assert_eq!(
        claimable
            .iter()
            .map(|item| item.request_id.as_str())
            .collect::<Vec<_>>(),
        ["req-open"]
    );

    let offered = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .expect("the request still waiting is offered");
    let (items, count) = service(&mut f)
        .digest_checklist_items(&target, Some(&offered.id), 0, 128)
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(items[0].request_id, "req-open");
    f.storage.close().unwrap();
}

/// A prompt that carries the typed checklist confirms it: the next Stop has nothing to offer.
#[test]
fn a_prompt_that_carries_the_typed_checklist_confirms_it() {
    use tmt_core::request::delivery::DeliveryState;
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "req-first");
    held(&mut f, &target, &sender, "req-second");
    type_idle_checklist(&mut f, &target);

    let confirm = |f: &mut Fixture, prompt: &str| {
        let connection = f.storage.connection().unwrap();
        crate::storage::requests::confirm_digest_by_prompt(connection, &target, prompt, NOW_MS + 1)
            .unwrap()
    };
    // A prompt that names no request of the checklist confirms nothing.
    assert_eq!(confirm(&mut f, "an ordinary prompt"), 0);
    assert_eq!(
        message_state_of(&mut f, "req-first"),
        DeliveryState::Unconfirmed
    );
    // One request of the checklist is enough: the checklist is typed or it is not.
    assert_eq!(
        confirm(
            &mut f,
            "TMT Digest checklist ... tmt reply req-second --receipt v2_x"
        ),
        1
    );
    assert_eq!(
        message_state_of(&mut f, "req-first"),
        DeliveryState::Delivered
    );
    assert_eq!(
        confirm(&mut f, "tmt reply req-second again"),
        0,
        "confirmed once"
    );
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .is_none()
    );
    f.storage.close().unwrap();
}
