//! The rules for typing a person's own message into an agent's pane. One owner for
//! `talk --passthrough` and `dispatch.create` in `passthrough` mode, so neither
//! restates a rule or its wording.

use crate::storage::{PassthroughFacts, Storage, StorageError};
use tmt_core::binding::session::{RuntimeState, activity::ActivityPhase};

/// Why a recipient cannot take typed input now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// No identity binding to a pane.
    NoPane,
    /// Bound, but its agent has ended. A runtime not yet observed does not count: the
    /// command types into such a pane too, and the send verifies it.
    NotRunning,
    /// The agent is waiting on its user, so the delivery would be refused.
    AwaitingApproval,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoPane => "no_pane",
            Self::NotRunning => "not_running",
            Self::AwaitingApproval => "awaiting_approval",
        }
    }
}

/// `reason` is set exactly when `available` is false.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Can {
    pub available: bool,
    pub reason: Option<Reason>,
}

impl Can {
    pub const YES: Self = Self {
        available: true,
        reason: None,
    };
    pub fn no(reason: Reason) -> Self {
        Self {
            available: false,
            reason: Some(reason),
        }
    }
    pub fn to_json(self) -> serde_json::Value {
        match self.reason {
            Some(reason) => serde_json::json!({"available": false, "reason": reason.as_str()}),
            None => serde_json::json!({"available": true}),
        }
    }
}

/// From stored facts only: a hint to offer the action, not a lease. The send still
/// verifies the pane fresh.
pub fn can(facts: PassthroughFacts) -> Can {
    if !facts.bound {
        Can::no(Reason::NoPane)
    } else if facts.runtime == RuntimeState::Ended {
        Can::no(Reason::NotRunning)
    } else if facts.phase == Some(ActivityPhase::Waiting) {
        Can::no(Reason::AwaitingApproval)
    } else {
        Can::YES
    }
}

/// `None` when the identity is unknown or retired.
pub fn can_identity(storage: &Storage, identity: &str) -> Result<Option<Can>, StorageError> {
    Ok(storage.passthrough_facts(identity)?.map(can))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotAllowed,
    Unavailable(Reason),
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotAllowed => "PASSTHROUGH_NOT_ALLOWED",
            Self::Unavailable(_) => "PASSTHROUGH_UNAVAILABLE",
        }
    }

    pub fn message(self, target: &str) -> String {
        match self {
            Self::NotAllowed => "Passthrough types a person's own message into a pane, and the sender is not marked as a person. Nothing was sent.".into(),
            Self::Unavailable(Reason::NoPane) => {
                format!("{target} has no live pane to type into. Nothing was sent.")
            }
            Self::Unavailable(Reason::NotRunning) => {
                format!("{target} has no running agent in its pane. Nothing was sent.")
            }
            Self::Unavailable(Reason::AwaitingApproval) => {
                format!("{target} is waiting on its user. Nothing was sent.")
            }
        }
    }

    /// What to do instead, when there is something.
    pub fn suggestion(self) -> Option<&'static str> {
        match self {
            Self::NotAllowed => None,
            Self::Unavailable(_) => {
                Some("Send it as a request instead: run the command again without --passthrough.")
            }
        }
    }
}

/// The two rules, in the order both callers report them: the sender must be a person,
/// then the recipient must be able to take typed input.
pub fn gate(sender_is_person: bool, recipient: Can) -> Result<(), Refusal> {
    if !sender_is_person {
        return Err(Refusal::NotAllowed);
    }
    match recipient.reason {
        Some(reason) => Err(Refusal::Unavailable(reason)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(bound: bool, runtime: RuntimeState, phase: Option<ActivityPhase>) -> PassthroughFacts {
        PassthroughFacts {
            bound,
            runtime,
            phase,
        }
    }

    #[test]
    fn a_recipient_can_take_typed_input_when_bound_not_ended_and_not_waiting() {
        let running = RuntimeState::Running;
        assert_eq!(
            can(facts(true, running, Some(ActivityPhase::Working))),
            Can::YES
        );
        assert_eq!(can(facts(true, running, None)), Can::YES);
        assert_eq!(
            can(facts(true, running, Some(ActivityPhase::Idle))),
            Can::YES
        );
        assert_eq!(
            can(facts(false, running, None)),
            Can::no(Reason::NoPane),
            "no binding is checked before anything else"
        );
        assert_eq!(
            can(facts(
                true,
                RuntimeState::Ended,
                Some(ActivityPhase::Waiting)
            )),
            Can::no(Reason::NotRunning),
            "a stopped agent keeps its last phase, which is not evidence"
        );
        assert_eq!(
            can(facts(true, RuntimeState::Unknown, None)),
            Can::YES,
            "a runtime nobody observed yet is offered, as the command does"
        );
        assert_eq!(
            can(facts(true, running, Some(ActivityPhase::Waiting))),
            Can::no(Reason::AwaitingApproval)
        );
    }

    #[test]
    fn the_sender_rule_comes_first_and_each_refusal_keeps_its_code_and_text() {
        let gone = Can::no(Reason::NoPane);
        assert_eq!(gate(false, gone), Err(Refusal::NotAllowed));
        assert_eq!(gate(true, gone), Err(Refusal::Unavailable(Reason::NoPane)));
        assert_eq!(gate(true, Can::YES), Ok(()));
        assert_eq!(Refusal::NotAllowed.code(), "PASSTHROUGH_NOT_ALLOWED");
        assert_eq!(
            Refusal::Unavailable(Reason::NoPane).message("worker"),
            "worker has no live pane to type into. Nothing was sent."
        );
        assert!(Refusal::NotAllowed.suggestion().is_none());
        assert!(
            Refusal::Unavailable(Reason::NotRunning)
                .suggestion()
                .is_some()
        );
    }

    #[test]
    fn the_json_names_a_reason_exactly_when_unavailable() {
        assert_eq!(Can::YES.to_json(), serde_json::json!({"available": true}));
        assert_eq!(
            Can::no(Reason::AwaitingApproval).to_json(),
            serde_json::json!({"available": false, "reason": "awaiting_approval"})
        );
    }
}
