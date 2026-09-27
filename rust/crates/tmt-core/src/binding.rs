//! Identity binding policy. SQLite, tmux and monotonic clocks remain adapters.

mod observation;
mod operations;
pub mod session;

#[cfg(test)]
mod evidence_tests;

pub use observation::{
    current_name_presence, evaluate_binding, list_presence, name_presence, pane_presence,
};
pub use operations::{
    bind_identity, bind_identity_at, bind_identity_with_creation, bind_identity_with_creation_at,
    remove_identity, unbind_identity,
};

use crate::{
    endpoint::{BindingMarker, EndpointProbe, EndpointSnapshot, PaneObservation, ServerEvidence},
    identity::{Identity, IdentityReader, IdentityRepository},
    names::NameError,
};
use std::{error::Error, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub id: String,
    pub identity_id: String,
    pub server: ServerEvidence,
    pub pane_id: String,
    pub pane_pid: u64,
    pub session: session::BindingSessionState,
}

/// Endpoint evidence frozen before a binding operation begins.
///
/// Explicit selectors such as a tmux mark can change independently of the
/// pane they identified. Keeping this evidence separate from the selector
/// lets binding policy reject pane-id reuse or server drift without resolving
/// the selector again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingTargetEvidence {
    pub server: ServerEvidence,
    pub pane_id: String,
    pub pane_pid: u64,
}

impl Binding {
    pub fn marker(&self, identity: &Identity) -> BindingMarker {
        BindingMarker {
            name: identity.name.clone(),
            canonical_name: identity.canonical_name.clone(),
            identity_id: identity.id.clone(),
            binding_id: self.id.clone(),
            server_id: self.server.server_id.clone(),
            pane_pid: self.pane_pid,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingEntry {
    pub identity: Identity,
    pub binding: Option<Binding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingEvidence {
    Active(Box<PaneObservation>),
    EndpointLost,
    MarkerMismatch,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Active,
    Offline,
    Unknown,
}

impl Presence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Offline => "offline",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityPresence {
    pub identity: Identity,
    pub presence: Presence,
    pub pane: Option<PaneObservation>,
    pub binding: Option<Binding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundIdentity {
    pub presence: IdentityPresence,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneIdentity {
    pub server: ServerEvidence,
    pub pane: PaneObservation,
    pub identity: Option<Identity>,
    pub binding: Option<Binding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnboundIdentity {
    pub identity: Identity,
    pub retired: bool,
    pub binding: Option<Binding>,
}

pub trait BindingRecords: IdentityReader {
    fn session_preferences(
        &self,
        identity_id: &str,
    ) -> Result<session::SessionPreferences, Self::Error>;
    /// Replace the complete preference record; partial updates read-modify-write
    /// within this same immediate transaction.
    fn set_session_preferences(
        &mut self,
        identity_id: &str,
        preferences: &session::SessionPreferences,
    ) -> Result<bool, Self::Error>;
    fn set_session_state(
        &mut self,
        binding_id: &str,
        expected: &session::BindingSessionState,
        state: &session::BindingSessionState,
    ) -> Result<bool, Self::Error>;
    fn entry_by_id(&self, id: &str) -> Result<Option<BindingEntry>, Self::Error>;
    fn entry_by_pane(&self, pane: &str, server: &str) -> Result<Option<BindingEntry>, Self::Error>;
    fn binding_entries(&self) -> Result<Vec<BindingEntry>, Self::Error>;
    fn insert_binding(
        &mut self,
        identity: &Identity,
        server: &ServerEvidence,
        pane: &PaneObservation,
    ) -> Result<Binding, Self::Error>;
    fn touch_binding(&mut self, id: &str) -> Result<(), Self::Error>;
    fn detach_binding(&mut self, id: &str) -> Result<(), Self::Error>;
    /// Application policy chooses retirement and whether explicit removal also
    /// erases profile/preamble. Retained exchanges/cadence must never be erased.
    fn retire_identity(
        &mut self,
        identity: &Identity,
        remove_content: bool,
    ) -> Result<(), Self::Error>;
}

pub trait BindingRepository: IdentityRepository {
    fn with_binding_transaction<T, E: From<Self::Error>>(
        &mut self,
        operation: impl FnOnce(&mut dyn BindingRecords<Error = Self::Error>) -> Result<T, E>,
    ) -> Result<T, E>;
}

/// One invocation-owned adapter, reset only after the repository acquires its
/// immediate lock. It owns the shared three-second budget, not core wall time.
pub trait BindingEndpoint {
    type Error;
    fn begin_coordination(&mut self);
    fn budget_available(&self) -> bool;
    fn current_snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, Self::Error>;
    fn probe_binding(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, Self::Error>;
    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), Self::Error>;
    fn clear(&mut self, binding: &Binding) -> Result<bool, Self::Error>;
}

#[derive(Debug)]
pub enum BindingError<R, O> {
    InvalidName(NameError),
    NameNotFound(String),
    PaneNotFound(String),
    TargetChanged(String),
    PaneAlreadyBound,
    NameAlreadyActive,
    ConfirmationRequired,
    Unverified,
    Deadline,
    Repository(R),
    Endpoint(O),
}

impl<R, O> From<R> for BindingError<R, O> {
    fn from(error: R) -> Self {
        Self::Repository(error)
    }
}

impl<R, O: fmt::Display> fmt::Display for BindingError<R, O> {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(error) => error.fmt(output),
            Self::NameNotFound(name) => write!(output, "Identity '{name}' was not found."),
            Self::PaneNotFound(pane) => write!(output, "Pane '{pane}' was not found."),
            Self::TargetChanged(pane) => write!(
                output,
                "Pane '{pane}' changed before binding completed. No successful binding change is claimed."
            ),
            Self::PaneAlreadyBound => output.write_str("Pane is already bound to another name."),
            Self::NameAlreadyActive => output.write_str("Name is already active on another pane."),
            Self::ConfirmationRequired => output.write_str("Saved identity removal requires --force. Its role and preamble will be removed; exchanges are retained."),
            Self::Unverified => output.write_str("Could not verify the identity binding. No successful binding change is claimed."),
            Self::Deadline => output.write_str("Identity coordination deadline exceeded."),
            Self::Repository(_) => output.write_str("Could not update identity state."),
            Self::Endpoint(error) => error.fmt(output),
        }
    }
}

impl<R: Error + 'static, O: Error + 'static> Error for BindingError<R, O> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidName(error) => Some(error),
            Self::Repository(error) => Some(error),
            Self::Endpoint(error) => Some(error),
            _ => None,
        }
    }
}
