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
        recipient_start_refusal: None,
        recipient_session_recovery: None,
        recipient_name: None,
        answer: ReturnedAnswer::None,
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
fn session_learning_recovery_is_bounded_presentation_and_never_delivery_authority() {
    let mut sent = facts(AttemptStatus::Sent);
    sent.recipient_start_refusal = Some(StartRefusalReason::SessionNotLearned);
    sent.recipient_session_recovery =
        Some("Exit the agent in its pane, then run: tmt run -- Worker codex".into());
    let view = delivery_view(&sent, 0);
    assert_eq!(view.reason.code(), "session_not_bound");
    assert!(view.reason_text.ends_with("tmt run -- Worker codex"));
    assert!(!view.can_send_now);
    sent.status = AttemptStatus::Queued;
    assert_eq!(shown(&sent), ("sent", "waiting_for_delivery", true));
    sent.status = AttemptStatus::Sent;
    for invalid in ["x".repeat(257), "unsafe\ncommand".into()] {
        sent.recipient_session_recovery = Some(invalid);
        let view = delivery_view(&sent, 0);
        assert!(view.reason_text.len() <= 256);
        assert!(!view.reason_text.chars().any(char::is_control));
        assert!(!view.reason_text.contains("unsafe"));
    }
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

#[test]
fn every_reason_has_its_own_code_and_text() {
    use DeliveryReason::*;
    let all = [
        Unreachable,
        WaitingForDelivery,
        AwaitingApproval,
        HeldToSaveUsage,
        ReachedTheAgent,
        AgentWorking,
        HandedOver,
        Refused,
    ];
    let codes: std::collections::HashSet<_> = all.iter().map(|r| r.code()).collect();
    let texts: std::collections::HashSet<_> = all.iter().map(|r| r.text()).collect();
    assert_eq!((codes.len(), texts.len()), (all.len(), all.len()));
}

// The sentences are the wording table of the contract (Send now): the CLI, the extension API
// and the Remote door show them as they are, so a change here is a change to every client.
#[test]
fn every_send_now_refusal_has_one_pinned_sentence() {
    for (rejection, code, sentence) in [
        (
            SendNowRejection::NotAllowed,
            "SEND_NOW_NOT_ALLOWED",
            "Could not send now: only its sender or the owner can send it early",
        ),
        (
            SendNowRejection::NotApplicable,
            "SEND_NOW_NOT_APPLICABLE",
            "Could not send now: it is not an ordinary message to an agent",
        ),
        (
            SendNowRejection::Closed,
            "SEND_NOW_CLOSED",
            "Could not send now: it already has an answer, or was withdrawn or expired",
        ),
        (
            SendNowRejection::Unconfirmed,
            "SEND_NOW_UNCONFIRMED",
            "Could not send now: an earlier delivery is unconfirmed, do not resend",
        ),
        (
            SendNowRejection::NotAvailable(DeliveryReason::AwaitingApproval),
            "SEND_NOW_NOT_AVAILABLE",
            "Could not send now: waiting for the agent's approval prompt to clear",
        ),
        (
            SendNowRejection::NotAvailable(DeliveryReason::WaitingForDelivery),
            "SEND_NOW_NOT_AVAILABLE",
            "Could not send now: waiting for delivery",
        ),
    ] {
        assert_eq!(
            (rejection.code(), rejection.message().as_str()),
            (code, sentence)
        );
    }
    for (untyped, code, sentence) in [
        (
            SendNowUntyped::AwaitingApproval,
            "DELIVERY_AWAITING_APPROVAL",
            "Could not send now: the agent is waiting for approval on its machine",
        ),
        (
            SendNowUntyped::Unreachable,
            "SEND_NOW_UNREACHABLE",
            "Could not send now: the agent could not be reached",
        ),
        (
            SendNowUntyped::ChannelUnavailable,
            "DELIVERY_CHANNEL_UNAVAILABLE",
            "Could not send now: the agent's channel is not available",
        ),
        (
            SendNowUntyped::Uncertain,
            "DELIVERY_UNCERTAIN",
            "It may have been sent. Delivery is not confirmed, so do not send it again.",
        ),
    ] {
        assert_eq!((untyped.code(), untyped.message()), (code, sentence));
    }
}

#[test]
fn a_refused_start_explains_delivery_without_reusing_the_old_sessions_activity() {
    let mut sent = facts(AttemptStatus::Sent);
    sent.recipient_start_refusal = Some(StartRefusalReason::DriverMismatch);
    sent.recipient_name = Some("worker's name".into());
    sent.recipient = activity(ActivityPhase::Working, 100);
    let view = delivery_view(&sent, 0);
    assert_eq!(shown(&sent), ("delivered", "session_not_bound", false));
    assert_eq!(
        view.reason_text,
        "session not bound · exit the agent in its pane, then run: tmt resume -- 'worker'\\''s name'"
    );
    sent.status = AttemptStatus::Uncertain;
    assert_eq!(shown(&sent), ("unconfirmed", "handed_over", false));
    sent.status = AttemptStatus::DefinitelyFailed;
    assert_eq!(shown(&sent), ("not_sent", "refused", false));
    sent.status = AttemptStatus::Sent;
    sent.recipient_start_refusal = None;
    assert_eq!(shown(&sent), ("working", "agent_working", false));
}

#[test]
fn a_live_previous_session_offers_move_instead_of_a_plain_resume() {
    let mut sent = facts(AttemptStatus::Sent);
    sent.recipient_start_refusal = Some(StartRefusalReason::PreviousSessionLive);
    sent.recipient_name = Some("worker's name".into());
    let text = delivery_view(&sent, 0).reason_text;
    assert_eq!(
        text,
        "session not bound · the old session is still running; move it with: tmt resume --move -- 'worker'\\''s name'"
    );
    assert!(text.len() < 256);
    assert_eq!(text.lines().count(), 1);
}

#[test]
fn other_refusals_require_exiting_before_resume() {
    for refusal in [
        StartRefusalReason::DriverMismatch,
        StartRefusalReason::PreviousSessionUnknown,
        StartRefusalReason::PrivateProcessUnverified,
        StartRefusalReason::SharedRuntime,
        StartRefusalReason::PaneProcess,
        StartRefusalReason::AdmissionRefused,
    ] {
        let mut sent = facts(AttemptStatus::Sent);
        sent.recipient_start_refusal = Some(refusal);
        sent.recipient_name = Some("worker".into());
        let text = delivery_view(&sent, 0).reason_text;
        assert_eq!(
            text,
            "session not bound · exit the agent in its pane, then run: tmt resume -- worker"
        );
        assert!(text.len() < 256);
        assert_eq!(text.lines().count(), 1);
    }
}

#[test]
fn resume_guidance_stays_one_line_below_256_bytes_for_long_names() {
    // Four-byte scalars and quote expansion each exceed the byte budget despite
    // the valid 64-scalar name limit. Never truncate a shell word into a command.
    for name in [
        "🙂".repeat(64),
        "'".repeat(64),
        "line\u{2028}separator".into(),
    ] {
        for (refusal, expected) in [
            (
                StartRefusalReason::DriverMismatch,
                "session not bound · exit the agent in its pane, then run tmt resume there",
            ),
            (
                StartRefusalReason::PreviousSessionLive,
                "session not bound · the old session is still running; run tmt resume --move in its bound pane",
            ),
        ] {
            let mut sent = facts(AttemptStatus::Sent);
            sent.recipient_start_refusal = Some(refusal);
            sent.recipient_name = Some(name.clone());
            let text = delivery_view(&sent, 0).reason_text;
            assert_eq!(text, expected);
            assert!(text.len() < 256);
            assert!(!text.contains(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']));
        }
    }
}

#[test]
fn what_came_back_outranks_how_the_hand_over_went() {
    // A reply ends the road wherever the message was: held, never pasted, or being worked on.
    let reply = |facts: DeliveryFacts| DeliveryFacts {
        answer: ReturnedAnswer::Final,
        ..facts
    };
    for base in [
        through_inbox(WakeState::NotAttempted),
        through_inbox(WakeState::Unavailable),
        through_inbox(WakeState::Sent),
        DeliveryFacts {
            hold: Some(DigestHold::Open { since_ms: 100 }),
            ..through_inbox(WakeState::NotAttempted)
        },
        DeliveryFacts {
            recipient: activity(ActivityPhase::Working, 500),
            ..through_inbox(WakeState::Sent)
        },
    ] {
        assert_eq!(
            shown(&reply(base)),
            ("answered", "answered_by_reply", false)
        );
    }
    // The turn a passthrough started ended with a final message.
    let mut typed = through_inbox(WakeState::Sent);
    typed.recipient = activity(ActivityPhase::Working, 900);
    assert_eq!(shown(&typed), ("working", "agent_working", false));
    typed.answer = ReturnedAnswer::Turn;
    assert_eq!(shown(&typed), ("answered", "answered_by_turn", false));
    // No answer was captured: unconfirmed, and never `working`, however busy the agent is now.
    typed.answer = ReturnedAnswer::NoCapture { bound: false };
    assert_eq!(shown(&typed), ("unconfirmed", "no_turn_started", false));
    typed.answer = ReturnedAnswer::NoCapture { bound: true };
    assert_eq!(
        shown(&typed),
        ("unconfirmed", "turn_ended_without_answer", false)
    );
    // A capture still waiting changes nothing.
    typed.answer = ReturnedAnswer::None;
    assert_eq!(shown(&typed), ("working", "agent_working", false));
}
