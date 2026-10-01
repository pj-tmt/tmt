//! One terminal host's operations behind one trait (#570), and the binding
//! policy over them written once.
//!
//! A [`HostDriver`] does what only its host can: list and probe panes, store
//! and clear the binding marker, observe the runtime in a pane, paste input,
//! and focus a pane. It decides nothing about bindings. [`status`], [`send`]
//! and [`focus`] hold that policy for every host: which evidence makes a
//! binding present, when a runtime blocks input, and what a failure means.
//! The operations follow the driver protocol's granularity
//! (`contracts/driver-protocol-v1.md`), so an out-of-process driver can
//! implement the same trait.

use super::{ActionError, DeliveryError, HostError};
use crate::process::CommandError;
use tmt_core::{
    binding::{Binding, BindingEntry, BindingEvidence, evaluate_binding, session::RuntimeState},
    driver::{
        ActionResult, DeliveryAcceptance, Focused, InterfacePresence, InterfaceStatus, SendFailure,
    },
    endpoint::{EndpointProbe, EndpointSnapshot, ServerEvidence},
    identity::Identity,
};

pub trait HostDriver {
    /// Starts one coordination budget for the operations that follow.
    fn begin_coordination(&mut self);
    fn budget_available(&self) -> bool;

    /// The given panes on this host's current server.
    fn snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, HostError>;

    /// Whether `server` still runs, with the given panes when it does. A
    /// server of another host, or a spent budget, is `Unknown`.
    fn probe(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, HostError>;

    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), HostError>;

    /// Removes the binding's marker if the pane still carries it.
    fn clear(&mut self, binding: &Binding) -> Result<bool, HostError>;

    /// Runtime liveness in a verified binding's pane.
    fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, CommandError>;

    /// The start token of a pane shell, from core's own process inspection,
    /// recorded when a binding is made.
    fn pane_incarnation(&mut self, pane_pid: u64) -> Result<Option<String>, HostError>;

    /// Whether the host can paste into a pane. Without it a send is
    /// `Unsupported` before any evidence is read, and core uses the inbox.
    fn has_input(&self) -> bool;

    /// Submits `message` to the agent the host recognizes in the binding's
    /// pane. `Unsupported` when the host has no agent-aware input or sees no
    /// agent there; [`send`] then pastes it as raw input. Every other result
    /// is final: an agent that is blocked or not ready refuses the message,
    /// and raw input must not go around that.
    fn prompt(
        &mut self,
        binding: &Binding,
        message: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>>;

    /// Pastes `message` into the binding's pane and submits it.
    fn input(&mut self, binding: &Binding, message: &str) -> Result<(), DeliveryError>;

    /// Whether this invocation could focus the binding's pane at all,
    /// decided before any evidence is read.
    fn focus_preflight(&self, binding: Option<&Binding>) -> Result<(), ActionError>;

    /// Focuses a pane whose binding was verified present.
    fn focus(&mut self, binding: &Binding) -> Result<Focused, ActionError>;
}

/// Presence from the host's evidence, and runtime from the pane's process
/// tree when present.
pub fn status(
    driver: &mut dyn HostDriver,
    entry: &BindingEntry,
) -> ActionResult<InterfaceStatus, ActionError> {
    let Some(binding) = &entry.binding else {
        return ActionResult::Completed(InterfaceStatus {
            presence: InterfacePresence::Gone,
            runtime: RuntimeState::Unknown,
        });
    };
    driver.begin_coordination();
    let probe = match driver.probe(&binding.server, std::slice::from_ref(&binding.pane_id)) {
        Ok(probe) => probe,
        Err(error) => return ActionResult::Failed(ActionError::Evidence(error)),
    };
    let presence = match evaluate_binding(entry, &probe) {
        BindingEvidence::Active(_) => InterfacePresence::Present,
        BindingEvidence::EndpointLost => InterfacePresence::Gone,
        BindingEvidence::MarkerMismatch | BindingEvidence::Unknown => InterfacePresence::Unknown,
    };
    let runtime = if presence == InterfacePresence::Present {
        match driver.observed_runtime(binding) {
            Ok(runtime) => runtime,
            Err(error) => return ActionResult::Failed(ActionError::Process(error)),
        }
    } else {
        RuntimeState::Unknown
    };
    ActionResult::Completed(InterfaceStatus { presence, runtime })
}

/// Input only into a present pane whose runtime, when one was recorded,
/// verified running.
pub fn send(
    driver: &mut dyn HostDriver,
    entry: &BindingEntry,
    message: &str,
) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
    if !driver.has_input() {
        return ActionResult::Unsupported;
    }
    match status(driver, entry) {
        ActionResult::Completed(InterfaceStatus {
            presence: InterfacePresence::Present,
            runtime: RuntimeState::Ended,
        }) => return ActionResult::Failed(SendFailure::NotSent(ActionError::Offline)),
        ActionResult::Completed(InterfaceStatus {
            presence: InterfacePresence::Present,
            runtime: RuntimeState::Unknown,
        }) if entry.binding.as_ref().is_some_and(|binding| {
            binding.session.key.is_some() || binding.session.state != RuntimeState::Unknown
        }) =>
        {
            // A failed verification of a recorded runtime is not permission
            // to downgrade to the legacy no-observation delivery path.
            return ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified));
        }
        ActionResult::Completed(InterfaceStatus {
            presence: InterfacePresence::Present,
            ..
        }) => {}
        ActionResult::Failed(error) => {
            return ActionResult::Failed(SendFailure::NotSent(error));
        }
        _ => return ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified)),
    }
    let Some(binding) = &entry.binding else {
        return ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified));
    };
    let message = &tmt_core::driver::pane_input_text(message);
    driver
        .prompt(binding, message)
        .or_unsupported(|| match driver.input(binding, message) {
            Ok(()) => ActionResult::Completed(DeliveryAcceptance::Submitted),
            Err(error) if error.uncertain() => {
                ActionResult::Failed(SendFailure::Uncertain(ActionError::Delivery(error)))
            }
            Err(error) => ActionResult::Failed(SendFailure::NotSent(ActionError::Delivery(error))),
        })
}

/// Requires present endpoint evidence (as before input) but no running
/// agent: a pane whose agent ended is still where the member worked.
pub fn focus(
    driver: &mut dyn HostDriver,
    entry: &BindingEntry,
) -> ActionResult<Focused, ActionError> {
    if let Err(error) = driver.focus_preflight(entry.binding.as_ref()) {
        return ActionResult::Failed(error);
    }
    let Some(binding) = &entry.binding else {
        return ActionResult::Failed(ActionError::Unverified);
    };
    match status(driver, entry) {
        ActionResult::Completed(InterfaceStatus {
            presence: InterfacePresence::Present,
            ..
        }) => {}
        ActionResult::Failed(error) => return ActionResult::Failed(error),
        _ => return ActionResult::Failed(ActionError::Unverified),
    }
    match driver.focus(binding) {
        Ok(focused) => ActionResult::Completed(focused),
        Err(error) => ActionResult::Failed(error),
    }
}

/// A host no driver serves in this process, such as a stored binding's host
/// whose driver isn't installed. It never claims evidence: a probe is
/// `Unknown`, so the binding is neither verified nor retired; it has no
/// input, so a send falls through to the inbox; and it can't be focused.
#[derive(Debug, Default)]
pub struct Unavailable {
    name: String,
}

impl Unavailable {
    pub fn of(host: tmt_core::host::HostKind) -> Self {
        Self {
            name: host.as_str().to_owned(),
        }
    }

    fn error(&self) -> HostError {
        HostError::Unavailable(self.name.clone())
    }
}

impl HostDriver for Unavailable {
    fn begin_coordination(&mut self) {}

    fn budget_available(&self) -> bool {
        true
    }

    fn snapshot(&mut self, _: &[String]) -> Result<EndpointSnapshot, HostError> {
        Err(self.error())
    }

    fn probe(&mut self, _: &ServerEvidence, _: &[String]) -> Result<EndpointProbe, HostError> {
        Ok(EndpointProbe::Unknown)
    }

    fn publish(&mut self, _: &Binding, _: &Identity) -> Result<(), HostError> {
        Err(self.error())
    }

    fn clear(&mut self, _: &Binding) -> Result<bool, HostError> {
        Err(self.error())
    }

    fn observed_runtime(&self, _: &Binding) -> Result<RuntimeState, CommandError> {
        Ok(RuntimeState::Unknown)
    }

    fn pane_incarnation(&mut self, _: u64) -> Result<Option<String>, HostError> {
        Ok(None)
    }

    fn has_input(&self) -> bool {
        false
    }

    fn prompt(
        &mut self,
        _: &Binding,
        _: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
        ActionResult::Unsupported
    }

    fn input(&mut self, _: &Binding, _: &str) -> Result<(), DeliveryError> {
        Err(DeliveryError::unsupported())
    }

    fn focus_preflight(&self, _: Option<&Binding>) -> Result<(), ActionError> {
        Err(ActionError::Evidence(self.error()))
    }

    fn focus(&mut self, _: &Binding) -> Result<Focused, ActionError> {
        Err(ActionError::Evidence(self.error()))
    }
}
