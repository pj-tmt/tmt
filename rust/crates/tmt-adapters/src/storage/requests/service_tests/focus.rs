use super::support::{Fixture, NOW_MS, service};
use crate::storage::test_support::{Operation, concurrent_pair};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    operation::new_operation_id,
    request::{
        Originator, PrepareRequest, RequestError, RequestKind, RequestRoute, RequestService,
        ResponseProof, SubmitResponse, WakeState,
        focus::{
            DeliveryPolicy, FocusKind, FocusOpportunity, FocusPolicyWrite, FocusRejection,
            FocusState,
        },
        notification::NotificationPolicy,
    },
};

fn identity(f: &mut Fixture, name: &str) -> String {
    create_or_resolve(&mut f.storage, name, Lifetime::Saved)
        .unwrap()
        .identity
        .id
}
fn policy(target: &str, owner: &str, revision: u64, until: u64) -> FocusPolicyWrite {
    FocusPolicyWrite {
        identity_id: target.into(),
        owner_identity_id: owner.into(),
        setter_identity_id: owner.into(),
        expected_revision: revision,
        until_ms: until,
    }
}
fn request(target: &str, sender: Option<&str>, id: &str) -> PrepareRequest {
    PrepareRequest {
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
fn held(
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
fn start() -> (Fixture, String, String, String) {
    let mut f = Fixture::new();
    let target = f.identity_id.clone();
    let owner = identity(&mut f, "Focus Owner");
    let sender = identity(&mut f, "Focus Sender");
    service(&mut f)
        .write_focus(policy(&target, &owner, 0, NOW_MS + 1000))
        .unwrap();
    (f, target, owner, sender)
}

#[test]
fn focus_admission_atomically_publishes_detached_attention_and_notice_policy() {
    let (mut f, target, _, sender) = start();
    let p = held(&mut f, &target, &sender, "held");
    assert_eq!(p.focus_until_ms, Some(NOW_MS + 1000));
    let a = service(&mut f).get_attempt(&p.attempt_id).unwrap().unwrap();
    assert!(!a.wait_active);
    assert_eq!(a.status, tmt_core::request::AttemptStatus::Queued);
    assert!(!a.cadence_reserved);
    assert!(service(&mut f).notification("held").unwrap().is_some());
    assert!(!service(&mut f).claim_wake("held").unwrap().claimed);
    let (items, count) = service(&mut f)
        .focus_checklist_items(&target, None, 0, 128)
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
fn focus_enabled_between_publication_and_wake_blocks_the_original_claim() {
    let (mut f, target, owner, sender) = start();
    service(&mut f)
        .write_focus(policy(&target, &owner, 1, 0))
        .unwrap();
    let mut input = request(&target, Some(&sender), "late-focus");
    input.wait = false;
    let prepared = service(&mut f)
        .enqueue_delivery(input, "late-attempt".into(), 7, DeliveryPolicy::default())
        .unwrap();
    assert_eq!(prepared.focus_until_ms, None);
    service(&mut f)
        .write_focus(policy(&target, &owner, 2, NOW_MS + 1000))
        .unwrap();
    let wake = service(&mut f).claim_wake("late-focus").unwrap();
    assert!(!wake.claimed);
    assert_eq!(wake.state, WakeState::NotAttempted);
    assert_eq!(wake.focus_until_ms, Some(NOW_MS + 1000));
    assert_eq!(
        service(&mut f)
            .focus_checklist_items(&target, None, 0, 128)
            .unwrap()
            .1,
        1
    );
    service(&mut f)
        .write_focus(policy(&target, &owner, 3, 0))
        .unwrap();
    // Clearing after admission cannot erase the captured held decision.
    assert_eq!(wake.focus_until_ms, Some(NOW_MS + 1000));
    let after_clear = service(&mut f).claim_wake("late-focus").unwrap();
    assert!(!after_clear.claimed);
    assert_eq!(after_clear.focus_until_ms, Some(0));
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
            .focus_reply_context("obsolete")
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
            .focus_reply_context("obsolete")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        service(&mut f)
            .focus_checklist_items(&target, None, 0, 128)
            .unwrap()
            .1,
        0
    );
    assert!(
        service(&mut f)
            .claim_focus_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                FocusOpportunity::TurnBoundary
            )
            .unwrap()
            .is_none()
    );
    f.storage.close().unwrap();
}

#[test]
fn owner_and_urgent_bypass_focus_but_explicit_inbox_stays_pull_only() {
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
                    kind: FocusKind::Review,
                },
            )
            .unwrap();
        assert_eq!(p.focus_until_ms, None);
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
            FocusKind::Review
        );
    }
    assert_eq!(
        service(&mut f).focus_policies(&[target]).unwrap()[0].held_count,
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
            .claim_focus_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                FocusOpportunity::Idle
            )
            .unwrap()
            .is_none()
    );
    let id = new_operation_id();
    let token = new_operation_id();
    let batch = service(&mut f)
        .claim_focus_checklist(
            &target,
            id.clone(),
            token.clone(),
            FocusOpportunity::TurnBoundary,
        )
        .unwrap()
        .unwrap();
    held(&mut f, &target, &sender, "c");
    let (items, count) = service(&mut f)
        .focus_checklist_items(&target, Some(&id), 0, 1)
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(items[0].request_id, "a");
    let (next, count) = service(&mut f)
        .focus_checklist_items(&target, Some(&id), items[0].sequence, 128)
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(next[0].request_id, "b");
    assert_eq!(batch.through_sequence, next[0].sequence);
    assert!(
        service(&mut f)
            .claim_focus_checklist(
                &target,
                id.clone(),
                token.clone(),
                FocusOpportunity::TurnBoundary
            )
            .unwrap()
            .is_none()
    );
    assert!(
        service(&mut f)
            .claim_focus_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                FocusOpportunity::TurnBoundary
            )
            .unwrap()
            .is_none()
    );
    assert!(
        service(&mut f)
            .settle_focus_checklist(&target, &id, &token, FocusState::Delivered)
            .unwrap()
    );
    assert!(
        !service(&mut f)
            .settle_focus_checklist(&target, &id, &token, FocusState::Delivered)
            .unwrap()
    );
    assert!(
        service(&mut f)
            .settle_focus_checklist(&target, &id, &token, FocusState::Unsent)
            .is_err()
    );
    assert!(!service(&mut f).claim_wake("a").unwrap().claimed);
    let (items, count) = service(&mut f)
        .focus_checklist_items(&target, None, 0, 128)
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
                .write_focus(policy(&target, &owner, 1, 0))
                .unwrap();
        } else {
            f.set_now(NOW_MS + 1000);
        }
        let batch = service(&mut f)
            .claim_focus_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                FocusOpportunity::Idle,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            service(&mut f)
                .focus_checklist_items(&target, Some(&batch.id), 0, 128)
                .unwrap()
                .1,
            1
        );
        let (_, replyable) = service(&mut f).focus_reply_context("a").unwrap().unwrap();
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
        assert!(!service(&mut f).focus_reply_context("a").unwrap().unwrap().1);
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
    for state in [FocusState::Unsent, FocusState::Uncertain] {
        let batch = service(&mut f)
            .claim_focus_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                FocusOpportunity::TurnBoundary,
            )
            .unwrap()
            .unwrap();
        assert!(matches!(
            service(&mut f).settle_focus_checklist(&target, &batch.id, &new_operation_id(), state),
            Err(RequestError::Focus(FocusRejection::AttemptMismatch))
        ));
        service(&mut f)
            .settle_focus_checklist(&target, &batch.id, &batch.attempt_token, state)
            .unwrap();
    }
    assert!(
        service(&mut f)
            .claim_focus_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                FocusOpportunity::TurnBoundary
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
            RequestService::new(s, || NOW_MS)
                .write_focus(policy(&target, &owner, 1, until))
                .is_ok()
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
                .claim_focus_checklist(
                    &target,
                    new_operation_id(),
                    new_operation_id(),
                    FocusOpportunity::TurnBoundary,
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
        .write_focus(policy(&sender, &owner, 0, NOW_MS + 1000))
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
        .focus_checklist_items(&sender, None, 0, 128)
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(items[0].kind, FocusKind::Result);
    assert_ne!(
        service(&mut f).notification("a").unwrap().unwrap().reply,
        WakeState::Sent
    );
    f.storage.close().unwrap();
}

#[test]
fn focus_set_after_reply_enqueue_fences_frame_and_joined_input_claims() {
    use tmt_core::{endpoint::ProcessIncarnation, request::notification::batch::SendClaim};
    for joined in [false, true] {
        let mut f = Fixture::new();
        f.set_now(crate::request_runtime::wall_time_ms());
        let target = f.identity_id.clone();
        let owner = identity(&mut f, "Owner");
        let sender = identity(&mut f, "Sender");
        service(&mut f)
            .write_focus(policy(
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
            .write_focus(policy(
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
            service(&mut f).focus_policies(&[sender.clone()]).unwrap()[0].held_count,
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
            service(&mut f).focus_policies(&[sender]).unwrap()[0].held_count,
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
fn focus_reads_do_not_housekeep_acknowledge_or_renew_retained_data() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "read");
    let before = f.storage.change_cursor().unwrap();
    let original = service(&mut f)
        .get_attempt("attempt-read")
        .unwrap()
        .unwrap();
    service(&mut f).focus_policies(&[target.clone()]).unwrap();
    service(&mut f)
        .focus_checklist_items(&target, None, 0, 1)
        .unwrap();
    service(&mut f).focus_reply_context("read").unwrap();
    service(&mut f).focus_result_context("read").unwrap();
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
