//! One delivery state per sent message, derived from stored facts so every client
//! paints the same state, reason and countdown and keeps no copy of the rules.

use super::{AttemptStatus, WakeState};
use crate::binding::session::{StartRefusalReason, activity::ActivityPhase};

/// What a person sees about one sent message, in the order it normally advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    Sent,
    Batched,
    Delivered,
    Working,
    Unconfirmed,
    NotSent,
    /// The end of the road: a reply was submitted, or the turn a passthrough started ended with a
    /// final message. Nothing after it moves the state (#4910).
    Answered,
}

impl DeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Batched => "batched",
            Self::Delivered => "delivered",
            Self::Working => "working",
            Self::Unconfirmed => "unconfirmed",
            Self::NotSent => "not_sent",
            Self::Answered => "answered",
        }
    }
}

/// Why a message is in its state. Clients show `text` and never write their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryReason {
    WaitingForDelivery,
    AwaitingApproval,
    HeldToSaveUsage,
    ReachedTheAgent,
    WaitingForPerson,
    SessionNotBound,
    AgentWorking,
    HandedOver,
    Unreachable,
    Refused,
    AnsweredByReply,
    AnsweredByTurn,
    AnsweredByFoldedTurn,
    TurnEndedWithoutReply,
    NoTurnStarted,
    TurnEndedWithoutAnswer,
}

impl DeliveryReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::WaitingForDelivery => "waiting_for_delivery",
            Self::AwaitingApproval => "awaiting_approval",
            Self::HeldToSaveUsage => "held_to_save_usage",
            Self::ReachedTheAgent => "reached_the_agent",
            Self::WaitingForPerson => "waiting_for_person",
            Self::SessionNotBound => "session_not_bound",
            Self::AgentWorking => "agent_working",
            Self::HandedOver => "handed_over",
            Self::Unreachable => "unreachable",
            Self::Refused => "refused",
            Self::AnsweredByReply => "answered_by_reply",
            Self::AnsweredByTurn => "answered_by_turn",
            Self::AnsweredByFoldedTurn => "answered_by_folded_turn",
            Self::TurnEndedWithoutReply => "turn_ended_without_reply",
            Self::NoTurnStarted => "no_turn_started",
            Self::TurnEndedWithoutAnswer => "turn_ended_without_answer",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Self::WaitingForDelivery => "waiting for delivery",
            Self::AwaitingApproval => "waiting for the agent's approval prompt to clear",
            Self::HeldToSaveUsage => {
                "batched to save usage, sends when the agent finishes this turn"
            }
            Self::ReachedTheAgent => "delivered",
            Self::WaitingForPerson => "waiting for the person",
            Self::SessionNotBound => "session not bound",
            Self::AgentWorking => "the agent has started on it",
            Self::HandedOver => "handed to the agent, delivery unconfirmed, do not resend",
            Self::Unreachable => "not sent, the agent could not be reached",
            Self::Refused => "not sent, nothing reached the agent",
            Self::AnsweredByReply => "answered",
            Self::AnsweredByTurn => "answered, the agent's turn ended with this reply",
            Self::AnsweredByFoldedTurn => {
                "answered together with the earlier message: it was typed into the running turn"
            }
            Self::TurnEndedWithoutReply => "no reply yet: the agent's turn ended without one",
            Self::NoTurnStarted => {
                "no answer captured: no turn started with this text, the agent may have taken it into a running turn"
            }
            Self::TurnEndedWithoutAnswer => {
                "no answer captured: the agent's turn ended without a final message"
            }
        }
    }
}

/// How far a Digest checklist that holds this message has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestHold {
    /// Still waiting for the checklist, since this time.
    Open {
        since_ms: u64,
    },
    Claimed,
    Delivered,
    /// Typed at an idle opportunity and no prompt has carried it since (#4858): it may
    /// have landed in a running turn. The next Stop offers it again.
    Unconfirmed,
    Uncertain,
}

/// The first turn that began after the message was handed over, as the hook's admitted turn
/// boundaries recorded it (#4911). The recipient's activity alone cannot say that turn is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HandoverTurn {
    /// No boundary was recorded (a message handed over before they were, or a boundary the
    /// hooks never admitted): judged by the recipient's current activity alone.
    #[default]
    Unrecorded,
    Started {
        at_ms: u64,
    },
    Ended,
}

/// A turn the message waits on is not awaited for ever: a turn whose end was never admitted (a
/// killed agent, a closed pane) stops reading `working` after this long, like a bound capture.
pub const WORKING_WINDOW_MS: u64 = crate::request::passthrough::BOUND_WINDOW_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecipientActivity {
    pub phase: ActivityPhase,
    pub since_ms: u64,
}

/// What came back for the message itself, read from the stored final and the passthrough
/// capture as they stand at the read (a capture past its window already reads `no_answer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReturnedAnswer {
    /// Nothing has come back yet.
    #[default]
    None,
    /// A reply was submitted.
    Final,
    /// A passthrough's captured turn ended with a final message; `folded` says the text was
    /// typed into a turn that was already running, so the answer is shared (#4912).
    Turn { folded: bool },
    /// A passthrough's capture ended with no answer; `bound` says a turn did start with its text.
    NoCapture { bound: bool },
}

/// Stored facts only; a message's state never comes from pane text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryFacts {
    pub status: AttemptStatus,
    /// An identified recipient's request stays `Queued`; its paste into the pane is
    /// the wake. A pane-routed request has no wake and settles its own status.
    pub inbox: bool,
    /// The recipient carries the active reserved person mark; no pane inference.
    pub recipient_is_user: bool,
    pub wake: WakeState,
    pub prepared_at_ms: u64,
    pub settled_at_ms: Option<u64>,
    pub hold: Option<DigestHold>,
    /// How long the recipient's Digest rule keeps its oldest held message, as the
    /// rule's owner recorded it with the policy.
    pub max_hold_ms: Option<u64>,
    /// The oldest message held for the recipient: every held message goes out together.
    pub oldest_held_at_ms: Option<u64>,
    /// The driver's activity for the exact live session, when there is one.
    pub recipient: Option<RecipientActivity>,
    /// The turn that began after the hand-over, when the hooks recorded it.
    pub turn: HandoverTurn,
    /// A refused start is an observation, not authority or missing telemetry.
    pub recipient_start_refusal: Option<StartRefusalReason>,
    pub recipient_session_recovery: Option<String>,
    pub recipient_name: Option<String>,
    pub answer: ReturnedAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryView {
    pub state: DeliveryState,
    pub reason: DeliveryReason,
    pub reason_text: String,
    /// The latest moment a batched message goes out, when the rule's owner recorded one.
    pub latest_ms: Option<u64>,
    /// `latest_ms` less the read time, so a client counts down without its own clock.
    pub remaining_ms: Option<u64>,
    /// Whether sending it now applies. Who may ask is the operation's check.
    pub can_send_now: bool,
}

impl DeliveryView {
    pub(crate) fn of(state: DeliveryState, reason: DeliveryReason) -> Self {
        Self {
            state,
            reason,
            reason_text: reason.text().into(),
            latest_ms: None,
            remaining_ms: None,
            can_send_now: false,
        }
    }
}

/// Why a send-now selected nothing to send. It never guesses and never resends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendNowRejection {
    /// Only the request's originator, or a person, may send it now.
    NotAllowed,
    /// Only an identified recipient's ordinary request is held and sent by Core.
    NotApplicable,
    /// A final, a withdrawal or the request's own expiry ended it.
    Closed,
    /// It may already be in the pane; resending could type it twice.
    Unconfirmed,
    /// Its state offers no send-now, with the reason the state gives.
    NotAvailable(DeliveryReason),
}

impl SendNowRejection {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotAllowed => "SEND_NOW_NOT_ALLOWED",
            Self::NotApplicable => "SEND_NOW_NOT_APPLICABLE",
            Self::Closed => "SEND_NOW_CLOSED",
            Self::Unconfirmed => "SEND_NOW_UNCONFIRMED",
            Self::NotAvailable(_) => "SEND_NOW_NOT_AVAILABLE",
        }
    }

    /// The one sentence every caller shows for it (#4455): the CLI, the extension API's fault
    /// message and the Remote door's `reasonText` are these words, so no client keeps a table.
    pub fn message(self) -> String {
        match self {
            Self::NotAllowed => refused("only its sender or the owner can send it early"),
            Self::NotApplicable => refused("it is not an ordinary message to an agent"),
            Self::Closed => refused("it already has an answer, or was withdrawn or expired"),
            Self::Unconfirmed => refused("an earlier delivery is unconfirmed, do not resend"),
            Self::NotAvailable(why) => refused(why.text()),
        }
    }
}

fn refused(reason: &str) -> String {
    format!("Could not send now: {reason}")
}

/// Why a send-now that released the hold still typed nothing, or may have typed it. A request
/// whose paste was refused stays sendable; one whose paste may have happened is never retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendNowUntyped {
    /// The agent is waiting on its user.
    AwaitingApproval,
    /// The agent has no live pane.
    Unreachable,
    /// Its session cannot carry the message.
    ChannelUnavailable,
    /// The paste may have reached the pane.
    Uncertain,
}

impl SendNowUntyped {
    pub fn code(self) -> &'static str {
        match self {
            Self::AwaitingApproval => "DELIVERY_AWAITING_APPROVAL",
            Self::Unreachable => "SEND_NOW_UNREACHABLE",
            Self::ChannelUnavailable => "DELIVERY_CHANNEL_UNAVAILABLE",
            Self::Uncertain => "DELIVERY_UNCERTAIN",
        }
    }

    /// The one sentence every caller shows for it, like [`SendNowRejection::message`].
    pub fn message(self) -> &'static str {
        match self {
            Self::AwaitingApproval => {
                "Could not send now: the agent is waiting for approval on its machine"
            }
            Self::Unreachable => "Could not send now: the agent could not be reached",
            Self::ChannelUnavailable => "Could not send now: the agent's channel is not available",
            Self::Uncertain => {
                "It may have been sent. Delivery is not confirmed, so do not send it again."
            }
        }
    }
}

/// What a send-now decided, with its hold released and the request ready to type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendNowOutcome {
    /// The Digest hold is released and the request marked urgent, so the ordinary
    /// delivery path applies; the caller types it.
    Release { recipient_identity_id: String },
    /// It had already reached the agent: nothing changed, nothing is resent.
    Already(DeliveryState),
}

/// How far the message itself has got, whichever route carried it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Handover {
    Waiting,
    InFlight,
    Reached,
    Unreachable,
    Uncertain,
    Refused,
}

fn handover(facts: &DeliveryFacts) -> Handover {
    if facts.inbox {
        return match facts.wake {
            WakeState::NotAttempted => Handover::Waiting,
            WakeState::Claimed => Handover::InFlight,
            WakeState::Sent => Handover::Reached,
            WakeState::Unavailable => Handover::Unreachable,
            WakeState::Uncertain => Handover::Uncertain,
        };
    }
    match facts.status {
        AttemptStatus::Prepared | AttemptStatus::Sending | AttemptStatus::Queued => {
            Handover::Waiting
        }
        AttemptStatus::Sent => Handover::Reached,
        AttemptStatus::Uncertain => Handover::Uncertain,
        AttemptStatus::DefinitelyFailed => Handover::Refused,
    }
}

pub fn delivery_view(facts: &DeliveryFacts, now_ms: u64) -> DeliveryView {
    use DeliveryReason as Why;
    use DeliveryState as State;
    // What came back outranks how the hand-over went: a reply from the inbox answers a message
    // whose paste never happened, and a turn that ended is not "working" however busy the agent is.
    match facts.answer {
        ReturnedAnswer::None => {}
        ReturnedAnswer::Final => return DeliveryView::of(State::Answered, Why::AnsweredByReply),
        ReturnedAnswer::Turn { folded } => {
            let why = if folded {
                Why::AnsweredByFoldedTurn
            } else {
                Why::AnsweredByTurn
            };
            return DeliveryView::of(State::Answered, why);
        }
        ReturnedAnswer::NoCapture { bound } => {
            let why = if bound {
                Why::TurnEndedWithoutAnswer
            } else {
                Why::NoTurnStarted
            };
            return DeliveryView::of(State::Unconfirmed, why);
        }
    }
    if facts.recipient_is_user
        && (facts.status == AttemptStatus::Sent
            || (facts.inbox && facts.status == AttemptStatus::Queued))
    {
        // Inbox publication delivers to a person. Old holds and agent observations
        // neither delay that delivery nor turn it into an agent's running turn.
        return DeliveryView::of(State::Delivered, Why::WaitingForPerson);
    }
    let waiting_approval = facts
        .recipient
        .is_some_and(|activity| activity.phase == ActivityPhase::Waiting);
    // A turn counts as the message's own only when it began after the message left
    // its sender (or was handed over, for a pane): a turn already running then is
    // still the one the message waits behind.
    let since = facts.settled_at_ms.unwrap_or(facts.prepared_at_ms);
    let busy = facts.recipient_start_refusal.is_none()
        && facts.recipient.is_some_and(|activity| {
            activity.phase != ActivityPhase::Idle && activity.since_ms >= since
        });
    // The turn the message waits on, once it began, is over when its end was recorded, when the
    // agent has gone idle without one, or when it has run past the window; and a turn that never
    // began leaves the activity as the only evidence.
    let working_since = match facts.turn {
        HandoverTurn::Started { at_ms } => Some(at_ms),
        _ => facts
            .recipient
            .filter(|_| busy)
            .map(|activity| activity.since_ms),
    };
    let recipient_idle = facts
        .recipient
        .is_some_and(|activity| activity.phase == ActivityPhase::Idle);
    let turn_over = match facts.turn {
        HandoverTurn::Ended => true,
        HandoverTurn::Started { at_ms } => {
            recipient_idle || now_ms.saturating_sub(at_ms) > WORKING_WINDOW_MS
        }
        HandoverTurn::Unrecorded => false,
    };
    let started = busy
        && !turn_over
        && working_since.is_some_and(|since| now_ms.saturating_sub(since) <= WORKING_WINDOW_MS);
    let mut view = match (handover(facts), facts.hold) {
        (Handover::Uncertain, _) | (_, Some(DigestHold::Uncertain | DigestHold::Unconfirmed)) => {
            DeliveryView::of(State::Unconfirmed, Why::HandedOver)
        }
        (Handover::Refused, _) => DeliveryView::of(State::NotSent, Why::Refused),
        (Handover::Unreachable, _) => DeliveryView {
            can_send_now: true,
            ..DeliveryView::of(State::NotSent, Why::Unreachable)
        },
        (Handover::Reached, _) if turn_over => {
            DeliveryView::of(State::Unconfirmed, Why::TurnEndedWithoutReply)
        }
        (Handover::Reached, _) if started => DeliveryView::of(State::Working, Why::AgentWorking),
        (Handover::Reached, _) | (_, Some(DigestHold::Delivered)) => {
            DeliveryView::of(State::Delivered, Why::ReachedTheAgent)
        }
        (Handover::InFlight, _) | (_, Some(DigestHold::Claimed)) => {
            DeliveryView::of(State::Sent, Why::WaitingForDelivery)
        }
        (Handover::Waiting, Some(DigestHold::Open { since_ms })) => {
            let latest_ms = facts.max_hold_ms.map(|hold| {
                facts
                    .oldest_held_at_ms
                    .unwrap_or(since_ms)
                    .saturating_add(hold)
            });
            DeliveryView {
                latest_ms,
                remaining_ms: latest_ms.map(|latest| latest.saturating_sub(now_ms)),
                can_send_now: !waiting_approval,
                ..DeliveryView::of(State::Batched, Why::HeldToSaveUsage)
            }
        }
        (Handover::Waiting, None) if waiting_approval => {
            DeliveryView::of(State::Sent, Why::AwaitingApproval)
        }
        (Handover::Waiting, None) => DeliveryView {
            can_send_now: true,
            ..DeliveryView::of(State::Sent, Why::WaitingForDelivery)
        },
    };
    if view.state == State::Delivered
        && let Some(refusal) = facts.recipient_start_refusal
    {
        view.reason = Why::SessionNotBound;
        if refusal == StartRefusalReason::SessionNotLearned {
            view.reason_text = format!(
                "session not bound · {}",
                facts
                    .recipient_session_recovery
                    .as_deref()
                    .unwrap_or("inspect the recipient's identity for the next step")
            );
            if view.reason_text.len() > 256 || view.reason_text.chars().any(char::is_control) {
                view.reason_text =
                    "session not bound · inspect the recipient's identity for the next step".into();
            }
            return view;
        }
        let (step, command) = if refusal == StartRefusalReason::PreviousSessionLive {
            (
                "the old session is still running; move it with:",
                "tmt resume --move",
            )
        } else {
            ("exit the agent in its pane, then run:", "tmt resume")
        };
        let command = facts.recipient_name.as_deref().map_or_else(
            || command.to_owned(),
            |name| format!("{command} -- {}", crate::names::command_word(name)),
        );
        view.reason_text = format!("session not bound · {step} {command}");
        if view.reason_text.len() >= 256
            || view
                .reason_text
                .contains(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'])
        {
            view.reason_text = if refusal == StartRefusalReason::PreviousSessionLive {
                "session not bound · the old session is still running; run tmt resume --move in its bound pane"
            } else {
                "session not bound · exit the agent in its pane, then run tmt resume there"
            }.into();
        }
    }
    view
}

#[cfg(test)]
mod tests;
