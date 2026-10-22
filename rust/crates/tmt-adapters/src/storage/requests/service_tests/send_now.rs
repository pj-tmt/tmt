//! Send now (#4378): the originator or a person takes one message out of the Digest hold
//! and the ordinary wake types it. Only the hold is skipped; every other rule is the
//! state's own, so a message that reached the agent is never typed twice.
use super::{
    digest::{held, identity, mark_user, request, start},
    support::{Fixture, service},
};
use tmt_core::{
    operation::new_operation_id,
    request::{
        RequestError, WakeState,
        delivery::{DeliveryReason, DeliveryState, SendNowOutcome, SendNowRejection},
        digest::{DeliveryPolicy, DigestOpportunity},
    },
};

fn rejected(f: &mut Fixture, id: &str, caller: &str) -> Option<SendNowRejection> {
    match service(f).send_now(id, caller) {
        Err(RequestError::SendNow(reason)) => Some(reason),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

fn held_items(f: &mut Fixture, target: &str) -> Vec<String> {
    service(f)
        .digest_checklist_items(target, None, 0, 128)
        .unwrap()
        .0
        .into_iter()
        .map(|item| item.request_id)
        .collect()
}

#[test]
fn only_the_originator_or_a_person_may_send_a_held_message_now() {
    let (mut f, target, _, sender) = start();
    let bystander = identity(&mut f, "Bystander");
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person, "true");
    held(&mut f, &target, &sender, "mine");
    held(&mut f, &target, &sender, "theirs");
    // Neither the recipient nor an unrelated agent may push someone else's message.
    for caller in [&bystander, &target] {
        assert_eq!(
            rejected(&mut f, "mine", caller),
            Some(SendNowRejection::NotAllowed)
        );
    }
    assert_eq!(held_items(&mut f, &target), ["mine", "theirs"]);
    assert_eq!(
        service(&mut f).send_now("mine", &sender).unwrap(),
        SendNowOutcome::Release {
            recipient_identity_id: target.clone()
        }
    );
    // A person may send anyone's message now.
    assert_eq!(
        service(&mut f).send_now("theirs", &person).unwrap(),
        SendNowOutcome::Release {
            recipient_identity_id: target.clone()
        }
    );
    f.storage.close().unwrap();
}

#[test]
fn send_now_skips_only_the_hold_and_leaves_the_other_held_messages_held() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "now");
    held(&mut f, &target, &sender, "later");
    assert!(!service(&mut f).claim_wake("now").unwrap().claimed);
    service(&mut f).send_now("now", &sender).unwrap();
    assert_eq!(held_items(&mut f, &target), ["later"]);
    // Marked urgent, the ordinary wake is claimed instead of held again.
    assert!(
        service(&mut f)
            .request_delivery_policy("now")
            .unwrap()
            .urgent
    );
    assert!(service(&mut f).claim_wake("now").unwrap().claimed);
    // The rest of the batch is untouched, still held with its own countdown.
    assert!(
        !service(&mut f)
            .request_delivery_policy("later")
            .unwrap()
            .urgent
    );
    assert!(!service(&mut f).claim_wake("later").unwrap().claimed);
    f.storage.close().unwrap();
}

#[test]
fn a_message_that_reached_the_agent_is_reported_and_never_typed_again() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "a");
    service(&mut f).send_now("a", &sender).unwrap();
    assert!(service(&mut f).claim_wake("a").unwrap().claimed);
    // Typing in flight: nothing to press yet.
    assert_eq!(
        rejected(&mut f, "a", &sender),
        Some(SendNowRejection::NotAvailable(
            DeliveryReason::WaitingForDelivery
        ))
    );
    service(&mut f)
        .settle_request_delivery("a", WakeState::Sent)
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            service(&mut f).send_now("a", &sender).unwrap(),
            SendNowOutcome::Already(DeliveryState::Delivered)
        );
    }
    assert_eq!(
        service(&mut f)
            .request_detail("a")
            .unwrap()
            .item
            .delivery_view
            .state,
        DeliveryState::Delivered
    );
    f.storage.close().unwrap();
}

#[test]
fn a_message_that_may_have_reached_the_agent_is_never_resendable() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "maybe");
    service(&mut f).send_now("maybe", &sender).unwrap();
    assert!(service(&mut f).claim_wake("maybe").unwrap().claimed);
    service(&mut f)
        .settle_request_delivery("maybe", WakeState::Uncertain)
        .unwrap();
    assert_eq!(
        rejected(&mut f, "maybe", &sender),
        Some(SendNowRejection::Unconfirmed)
    );
    f.storage.close().unwrap();
}

#[test]
fn an_unreachable_recipient_gets_one_more_wake_and_nothing_else_changes() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "away");
    service(&mut f).send_now("away", &sender).unwrap();
    assert!(service(&mut f).claim_wake("away").unwrap().claimed);
    service(&mut f)
        .settle_request_delivery("away", WakeState::Unavailable)
        .unwrap();
    let view = service(&mut f)
        .request_detail("away")
        .unwrap()
        .item
        .delivery_view;
    assert_eq!(
        (view.state, view.reason, view.can_send_now),
        (DeliveryState::NotSent, DeliveryReason::Unreachable, true)
    );
    service(&mut f).send_now("away", &sender).unwrap();
    assert!(
        service(&mut f).claim_wake("away").unwrap().claimed,
        "the wake that typed nothing is reopened"
    );
    f.storage.close().unwrap();
}

#[test]
fn a_message_sealed_in_a_checklist_is_not_released() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "sealed");
    service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        rejected(&mut f, "sealed", &sender),
        Some(SendNowRejection::NotAvailable(
            DeliveryReason::WaitingForDelivery
        ))
    );
    let kept: i64 = rusqlite::Connection::open(&f.database)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM focus_items WHERE request_id = 'sealed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kept, 1, "a sealed reference stays with its checklist");
    f.storage.close().unwrap();
}

#[test]
fn only_an_open_ordinary_request_to_an_identified_recipient_can_be_sent_now() {
    let (mut f, target, _, sender) = start();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person, "true");
    // A passthrough is typed at once or refused; an announcement has no reply to wait for.
    let mut announcement = request(&target, Some(&sender), "announce");
    announcement.kind = tmt_core::request::RequestKind::Announcement;
    announcement.wait = false;
    service(&mut f)
        .enqueue_delivery(
            announcement,
            "attempt-announce".into(),
            7,
            DeliveryPolicy::default(),
        )
        .unwrap();
    assert_eq!(
        rejected(&mut f, "announce", &sender),
        Some(SendNowRejection::NotApplicable)
    );
    let typed = DeliveryPolicy {
        mode: tmt_core::request::digest::DeliveryMode::Passthrough,
        ..DeliveryPolicy::default()
    };
    service(&mut f)
        .enqueue_delivery(
            request(&target, Some(&person), "typed"),
            "attempt-typed".into(),
            7,
            typed,
        )
        .unwrap();
    assert_eq!(
        rejected(&mut f, "typed", &person),
        Some(SendNowRejection::NotApplicable)
    );
    // A withdrawn request is closed.
    held(&mut f, &target, &sender, "withdrawn");
    service(&mut f)
        .withdraw_request(&sender, "withdrawn", "no longer needed")
        .unwrap();
    assert_eq!(
        rejected(&mut f, "withdrawn", &sender),
        Some(SendNowRejection::Closed)
    );
    assert!(matches!(
        service(&mut f).send_now("missing", &sender),
        Err(RequestError::NotFound)
    ));
    f.storage.close().unwrap();
}
