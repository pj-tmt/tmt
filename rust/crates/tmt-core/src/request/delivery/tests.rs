use super::*;
use crate::request::WakeState;

fn facts(status: AttemptStatus) -> DeliveryFacts {
    DeliveryFacts {
        status,
        inbox: false,
        wake: WakeState::NotAttempted,
        prepared_at_ms: 0,
        settled_at_ms: None,
        hold: None,
        max_hold_ms: None,
        oldest_held_at_ms: None,
        recipient: None,
    }
}

fn activity(phase: ActivityPhase, since_ms: u64) -> Option<RecipientActivity> {
    Some(RecipientActivity { phase, since_ms })
}

fn shown(facts: &DeliveryFacts) -> (&'static str, &'static str, bool) {
    let view = delivery_view(facts, 0);
    (view.state.as_str(), view.reason.code(), view.can_send_now)
}

#[test]
fn a_message_not_yet_handed_over_is_sent_and_can_go_now() {
    for status in [
        AttemptStatus::Prepared,
        AttemptStatus::Sending,
        AttemptStatus::Queued,
    ] {
        assert_eq!(
            shown(&facts(status)),
            ("sent", "waiting_for_delivery", true)
        );
    }
}

#[test]
fn an_approval_prompt_blocks_sending_now_and_says_why() {
    let mut waiting = facts(AttemptStatus::Queued);
    waiting.recipient = activity(ActivityPhase::Waiting, 5);
    assert_eq!(shown(&waiting), ("sent", "awaiting_approval", false));
    waiting.hold = Some(DigestHold::Open { since_ms: 1 });
    assert_eq!(shown(&waiting), ("batched", "held_to_save_usage", false));
}

#[test]
fn a_held_message_counts_down_to_the_oldest_held_plus_the_recorded_hold() {
    let mut held = facts(AttemptStatus::Queued);
    held.hold = Some(DigestHold::Open { since_ms: 1_000 });
    held.oldest_held_at_ms = Some(400);
    held.max_hold_ms = Some(300_000);
    let view = delivery_view(&held, 100_000);
    assert_eq!(
        (
            view.state,
            view.latest_ms,
            view.remaining_ms,
            view.can_send_now
        ),
        (DeliveryState::Batched, Some(300_400), Some(200_400), true)
    );
    // Past the latest time the countdown stops at zero rather than wrapping.
    assert_eq!(delivery_view(&held, 400_000).remaining_ms, Some(0));
    held.oldest_held_at_ms = None;
    assert_eq!(delivery_view(&held, 0).latest_ms, Some(301_000));
    held.max_hold_ms = None;
    let unknown = delivery_view(&held, 0);
    assert_eq!((unknown.latest_ms, unknown.remaining_ms), (None, None));
}

#[test]
fn a_checklist_in_flight_is_sent_and_a_delivered_one_is_delivered() {
    let mut held = facts(AttemptStatus::Queued);
    held.hold = Some(DigestHold::Claimed);
    assert_eq!(shown(&held), ("sent", "waiting_for_delivery", false));
    held.hold = Some(DigestHold::Delivered);
    held.recipient = activity(ActivityPhase::Working, 99);
    assert_eq!(shown(&held), ("delivered", "reached_the_agent", false));
    held.hold = Some(DigestHold::Uncertain);
    assert_eq!(shown(&held), ("unconfirmed", "handed_over", false));
}

#[test]
fn a_sent_message_is_working_only_for_a_turn_that_began_after_the_hand_over() {
    let mut sent = facts(AttemptStatus::Sent);
    sent.settled_at_ms = Some(100);
    assert_eq!(shown(&sent), ("delivered", "reached_the_agent", false));
    sent.recipient = activity(ActivityPhase::Working, 99);
    assert_eq!(shown(&sent), ("delivered", "reached_the_agent", false));
    sent.recipient = activity(ActivityPhase::Working, 100);
    assert_eq!(shown(&sent), ("working", "agent_working", false));
    sent.recipient = activity(ActivityPhase::Waiting, 150);
    assert_eq!(shown(&sent), ("working", "agent_working", false));
    sent.recipient = activity(ActivityPhase::Idle, 150);
    assert_eq!(shown(&sent), ("delivered", "reached_the_agent", false));
}

fn through_inbox(wake: WakeState) -> DeliveryFacts {
    DeliveryFacts {
        inbox: true,
        wake,
        prepared_at_ms: 100,
        ..facts(AttemptStatus::Queued)
    }
}

#[test]
fn an_identified_recipient_is_judged_by_its_wake_not_by_its_always_queued_status() {
    let expected = [
        (
            WakeState::NotAttempted,
            ("sent", "waiting_for_delivery", true),
        ),
        (WakeState::Claimed, ("sent", "waiting_for_delivery", false)),
        (WakeState::Sent, ("delivered", "reached_the_agent", false)),
        (WakeState::Unavailable, ("not_sent", "unreachable", true)),
        (WakeState::Uncertain, ("unconfirmed", "handed_over", false)),
    ];
    for (wake, shown_as) in expected {
        assert_eq!(shown(&through_inbox(wake)), shown_as, "{wake:?}");
    }
}

#[test]
fn a_pasted_inbox_message_is_working_only_for_a_turn_that_began_after_it_was_prepared() {
    let mut pasted = through_inbox(WakeState::Sent);
    pasted.recipient = activity(ActivityPhase::Working, 99);
    assert_eq!(shown(&pasted), ("delivered", "reached_the_agent", false));
    pasted.recipient = activity(ActivityPhase::Working, 100);
    assert_eq!(shown(&pasted), ("working", "agent_working", false));
    // A turn is never the message's own while the message is still waiting to be typed.
    let mut waiting = through_inbox(WakeState::NotAttempted);
    waiting.recipient = activity(ActivityPhase::Working, 500);
    assert_eq!(shown(&waiting), ("sent", "waiting_for_delivery", true));
}

#[test]
fn an_uncertain_hand_over_is_never_resendable() {
    let mut uncertain = facts(AttemptStatus::Uncertain);
    uncertain.recipient = activity(ActivityPhase::Idle, 1);
    assert_eq!(shown(&uncertain), ("unconfirmed", "handed_over", false));
}

#[test]
fn a_refused_send_is_not_sent() {
    assert_eq!(
        shown(&facts(AttemptStatus::DefinitelyFailed)),
        ("not_sent", "refused", false)
    );
}

const ALL_REASONS: [DeliveryReason; 8] = {
    use DeliveryReason::*;
    [
        Unreachable,
        WaitingForDelivery,
        AwaitingApproval,
        HeldToSaveUsage,
        ReachedTheAgent,
        AgentWorking,
        HandedOver,
        Refused,
    ]
};

#[test]
fn every_reason_has_its_own_code_and_text() {
    let codes: std::collections::HashSet<_> = ALL_REASONS.iter().map(|r| r.code()).collect();
    let texts: std::collections::HashSet<_> = ALL_REASONS.iter().map(|r| r.text()).collect();
    assert_eq!(
        (codes.len(), texts.len()),
        (ALL_REASONS.len(), ALL_REASONS.len())
    );
}

#[test]
fn the_contract_table_words_every_reason_exactly_as_core_does() {
    const CONTRACT: &str = include_str!("../../../../../../contracts/request-response-v1.md");
    let rows: Vec<Vec<&str>> = CONTRACT
        .lines()
        .skip_while(|line| !line.starts_with("| `reason.code`"))
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|line| line.split('|').map(str::trim).collect())
        .collect();
    assert_eq!(rows.len(), ALL_REASONS.len(), "one contract row per reason");
    for reason in ALL_REASONS {
        let row = rows
            .iter()
            .find(|cells| cells[1] == format!("`{}`", reason.code()))
            .unwrap_or_else(|| panic!("the contract has no row for {}", reason.code()));
        assert_eq!(row[3], reason.text(), "wording of {}", reason.code());
    }
}
