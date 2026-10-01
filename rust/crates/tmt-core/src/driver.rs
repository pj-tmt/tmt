//! Optional driver actions and observation-only lifecycle hooks.
//!
//! The caller resolves identity and interface authority before invoking a driver.
//! Unsupported is the only automatic fall-through outcome at this boundary.
//! A failed action, including a proven unsent action, requires an explicit routing
//! decision by the caller. Drivers do not own request storage or retry policy.

pub mod caller;
pub mod descriptor;
pub mod detection;
pub mod routing;

pub use descriptor::ALL;

use crate::binding::session::{
    DriverState, HarnessId, ObservedSessionKey, ProviderSessionId, RuntimeMode, RuntimeState,
    SessionTransition,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionResult<T, E> {
    Unsupported,
    Completed(T),
    Failed(E),
}

impl<T, E> ActionResult<T, E> {
    pub fn or_unsupported(self, next: impl FnOnce() -> Self) -> Self {
        match self {
            Self::Unsupported => next(),
            terminal => terminal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryAcceptance {
    /// Submitted to the interface, not proof of model execution or a final reply.
    Submitted,
    /// Accepted by a provider queue, not yet necessarily submitted to its runtime.
    Queued,
    /// Handed to a one-way channel that has no provider receipt: the message may
    /// or may not have been seen. It is terminal for routing (no fallback, no
    /// resend), and callers must not report it as delivered.
    Unacknowledged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendFailure<E> {
    NotSent(E),
    Uncertain(E),
    Denied(E),
    AwaitingApproval(E),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfacePresence {
    Present,
    Gone,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceStatus {
    pub presence: InterfacePresence,
    pub runtime: RuntimeState,
}

/// Typed intent, not caller-supplied shell fragments. A runtime adapter owns the
/// resulting argv/context encoding; the host owns its bounded process lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessStart<'a> {
    pub harness: &'a HarnessId,
    pub context: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessResume<'a> {
    pub start: HarnessStart<'a>,
    pub session: &'a ProviderSessionId,
    pub mode: &'a RuntimeMode,
    /// The driver's own stored resume details, if any; a driver ignores state
    /// it cannot read rather than guessing.
    pub state: Option<&'a DriverState>,
}

/// A completed focus: the host interface now shown to the invoking user, the
/// interface it showed before, so a caller can return without host
/// knowledge, and the user view that moved. IDs are host-owned (a tmux pane
/// ID and client name, for example).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focused {
    pub interface: String,
    pub previous: Option<String>,
    pub viewer: String,
}

/// Implement only supported actions. Concrete adapters must bound I/O using the
/// existing process owner. This port neither installs code nor bypasses consent.
pub trait Driver {
    type Target: ?Sized;
    type Error;
    type Launch;

    /// Pure command recognition. No argument mutation, process or network I/O.
    /// A claim describes runtime handling, not authority over an identity.
    fn claims(&self, _command: &str) -> Option<HarnessId> {
        None
    }

    /// Recognize this invocation's runtime conversation. Concrete drivers own
    /// environment/process observations; core never interprets provider markers.
    /// Completed evidence still needs correlation with an active binding.
    fn identify_caller(&mut self) -> ActionResult<caller::RuntimeCaller, Self::Error> {
        ActionResult::Unsupported
    }

    fn send(
        &mut self,
        _target: &Self::Target,
        _message: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<Self::Error>> {
        ActionResult::Unsupported
    }

    fn status(&mut self, _target: &Self::Target) -> ActionResult<InterfaceStatus, Self::Error> {
        ActionResult::Unsupported
    }

    fn launch(&mut self, _start: HarnessStart<'_>) -> ActionResult<Self::Launch, Self::Error> {
        ActionResult::Unsupported
    }

    fn resume(&mut self, _resume: HarnessResume<'_>) -> ActionResult<Self::Launch, Self::Error> {
        ActionResult::Unsupported
    }

    fn inject_context(
        &mut self,
        _target: &Self::Target,
        _context: &str,
    ) -> ActionResult<(), Self::Error> {
        ActionResult::Unsupported
    }

    /// Show the verified target's interface to the invoking user. It changes
    /// only what the user is looking at: never input, bindings or runtime state.
    fn focus(&mut self, _target: &Self::Target) -> ActionResult<Focused, Self::Error> {
        ActionResult::Unsupported
    }
}

/// IDs refer to already resolved canonical records. Observations cannot grant
/// ownership or mutate the message. Durable retirement receipts retain their
/// separate identity_hooks owner and must not be replaced by best-effort hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent<'a> {
    SessionStarted {
        binding_id: &'a str,
        key: &'a ObservedSessionKey,
        transition: SessionTransition,
    },
    SessionEnded {
        binding_id: &'a str,
        key: &'a ObservedSessionKey,
    },
    BindingChanged {
        identity_id: &'a str,
        binding_id: &'a str,
    },
    BeforeMessage {
        request_id: &'a str,
    },
    MessageDelivered {
        request_id: &'a str,
        acceptance: DeliveryAcceptance,
    },
    ReplyReceived {
        request_id: &'a str,
    },
}

pub trait HookObserver {
    type Error;

    /// Adapters enforce their invocation budget; this cannot veto an operation.
    fn observe(&mut self, event: &HookEvent<'_>) -> Result<(), Self::Error>;
}

/// Retain the diagnostic separately from an operation's result. There is no
/// subscription loop, executable discovery or retry worker at this boundary.
#[must_use]
pub fn observe_driver_hook<O: HookObserver>(
    observer: &mut O,
    event: &HookEvent<'_>,
) -> Option<O::Error> {
    observer.observe(event).err()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Unsupported;
    impl Driver for Unsupported {
        type Target = str;
        type Error = &'static str;
        type Launch = ();
    }

    #[test]
    fn absent_action_falls_through_once() {
        let mut attempts = 0;
        let result = Unsupported
            .send("resolved-interface", "message")
            .or_unsupported(|| {
                attempts += 1;
                ActionResult::Completed(DeliveryAcceptance::Queued)
            });
        assert_eq!(result, ActionResult::Completed(DeliveryAcceptance::Queued));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn acceptance_uncertainty_and_policy_failures_never_implicitly_fall_through() {
        let terminal = [
            ActionResult::Completed(DeliveryAcceptance::Submitted),
            ActionResult::Completed(DeliveryAcceptance::Queued),
            ActionResult::Completed(DeliveryAcceptance::Unacknowledged),
            ActionResult::Failed(SendFailure::NotSent("unsent")),
            ActionResult::Failed(SendFailure::Uncertain("unknown")),
            ActionResult::Failed(SendFailure::Denied("denied")),
            ActionResult::Failed(SendFailure::AwaitingApproval("approval")),
        ];
        for result in terminal {
            let expected = result.clone();
            assert_eq!(
                result.or_unsupported(|| panic!("must not resend")),
                expected
            );
        }
    }

    #[test]
    fn unsupported_runtime_actions_do_not_invent_success() {
        let harness = HarnessId::new("claude").unwrap();
        let mode = RuntimeMode::new("default").unwrap();
        let session = ProviderSessionId::new("history").unwrap();
        let start = HarnessStart {
            harness: &harness,
            context: None,
        };
        assert_eq!(Unsupported.status("interface"), ActionResult::Unsupported);
        assert_eq!(Unsupported.launch(start.clone()), ActionResult::Unsupported);
        assert_eq!(
            Unsupported.resume(HarnessResume {
                start,
                session: &session,
                mode: &mode,
                state: None,
            }),
            ActionResult::Unsupported
        );
        assert_eq!(
            Unsupported.inject_context("interface", "context"),
            ActionResult::Unsupported
        );
    }

    #[test]
    fn observer_failure_is_a_diagnostic_not_a_veto() {
        struct FailingObserver;
        impl HookObserver for FailingObserver {
            type Error = &'static str;
            fn observe(&mut self, _: &HookEvent<'_>) -> Result<(), Self::Error> {
                Err("subscriber unavailable")
            }
        }
        let operation_result = DeliveryAcceptance::Submitted;
        let diagnostic = observe_driver_hook(
            &mut FailingObserver,
            &HookEvent::MessageDelivered {
                request_id: "request",
                acceptance: operation_result,
            },
        );
        assert_eq!(diagnostic, Some("subscriber unavailable"));
        assert_eq!(operation_result, DeliveryAcceptance::Submitted);
    }
}
