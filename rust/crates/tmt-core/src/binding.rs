//! Identity binding policy. SQLite, tmux and monotonic clocks remain adapters.

mod observation;
mod operations;
pub mod session;

#[cfg(test)]
mod evidence_tests;

pub use observation::{
    current_identity_presence, current_name_presence, evaluate_binding, list_presence,
    name_presence, pane_presence,
};
pub use operations::{
    bind_auto_identity, bind_existing_identity, bind_guarded, bind_identity, bind_identity_at,
    bind_identity_with_creation, bind_identity_with_creation_at,
    bind_runtime_identity_with_creation, move_identity, name_auto_identity, remove_identity,
    remove_identity_by_id, rename_identity, rename_identity_by_id, retire_failed_auto_launch,
    unbind_identity,
};

use crate::{
    endpoint::{
        BindingMarker, EndpointProbe, EndpointSnapshot, PaneObservation, ProcessIncarnation,
        ServerEvidence, incarnations_differ,
    },
    identity::{Identity, IdentityReader, IdentityRepository},
    names::{NameError, ValidatedName},
};
use std::{error::Error, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub id: String,
    pub identity_id: String,
    pub server: ServerEvidence,
    pub pane_id: String,
    pub pane_pid: u64,
    /// The start token core observed for `pane_pid` when the binding was
    /// made; `None` for bindings made before it was recorded, or when that
    /// observation failed.
    pub pane_incarnation: Option<String>,
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
    pub pane_incarnation: Option<String>,
    pub discovery: Option<crate::discovery::DiscoveryTarget>,
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

/// A target reserved before its marker is published. Active bindings live
/// separately; moves keep their old binding active until activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingReservation {
    pub target: Binding,
    pub old_binding_id: Option<String>,
    pub expected_binding_id: Option<String>,
    pub publisher: ProcessIncarnation,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

impl BindingReservation {
    pub fn is_stale(&self, now_ms: u64, liveness: session::RuntimeLiveness) -> bool {
        now_ms >= self.expires_at_ms || liveness == session::RuntimeLiveness::Gone
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

/// An identity under a new name, with its UUID and everything keyed by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedIdentity {
    pub previous: Identity,
    pub identity: Identity,
    /// The stored binding, whose pane may still show the previous name.
    pub binding: Option<Binding>,
}

impl RenamedIdentity {
    pub fn changed(&self) -> bool {
        self.previous.name != self.identity.name
    }
}

/// Exact UUID/name compare-and-set result, captured inside the binding transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameOutcome {
    Applied(Box<RenamedIdentity>),
    Conflict(Identity),
    NotFound,
}

/// Exact UUID/name compare-and-set result of a guarded removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveOutcome {
    Removed(Box<BindingEntry>),
    Conflict(Identity),
    NotFound,
}

/// Why an identity is not removed: its bound agent may still be running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRunning {
    /// The recorded agent process is alive.
    Alive,
    /// The recorded agent process is suspended or could not be observed.
    Unproven,
    /// The binding's host could not be observed.
    HostUnobserved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnboundIdentity {
    pub identity: Identity,
    pub retired: bool,
    pub binding: Option<Binding>,
}

pub trait BindingRecords: IdentityReader {
    /// Provisional launch evidence grants no runtime/session authority.
    fn begin_launch_observation(
        &mut self,
        binding: &Binding,
        owner: &ProcessIncarnation,
        driver: &session::HarnessId,
        now_ms: u64,
    ) -> Result<bool, Self::Error>;
    fn launch_preferences_match(
        &self,
        observation: &session::launch::LaunchObservation,
    ) -> Result<bool, Self::Error>;
    fn launch_observation(
        &self,
        binding_id: &str,
        owner: &ProcessIncarnation,
    ) -> Result<Option<session::launch::LaunchObservation>, Self::Error>;
    fn set_launch_observation(
        &mut self,
        expected: &session::launch::LaunchObservation,
        next: &session::launch::LaunchObservation,
    ) -> Result<bool, Self::Error>;
    fn insert_identity(
        &mut self,
        name: &ValidatedName,
        lifetime: crate::identity::Lifetime,
    ) -> Result<Identity, Self::Error>;
    fn is_auto_named(&self, identity_id: &str) -> Result<bool, Self::Error>;
    fn save_identity(&mut self, identity: &Identity) -> Result<Identity, Self::Error>;

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
    /// Whether a marker's exact binding ID still has any durable row.
    fn binding_exists(&self, binding_id: &str) -> Result<bool, Self::Error>;
    fn entry_by_pane(
        &self,
        host: crate::host::HostKind,
        pane: &str,
        server: &str,
    ) -> Result<Option<BindingEntry>, Self::Error>;
    /// Candidate records captured before the current server is observed. Pane IDs
    /// may repeat across servers; reconciliation selects the observed server.
    fn entries_for_pane(
        &self,
        host: crate::host::HostKind,
        pane: &str,
    ) -> Result<Vec<BindingEntry>, Self::Error>;
    fn binding_entries(&self) -> Result<Vec<BindingEntry>, Self::Error>;
    fn insert_binding(
        &mut self,
        identity: &Identity,
        server: &ServerEvidence,
        pane: &PaneObservation,
    ) -> Result<Binding, Self::Error>;
    /// Reserve a future binding without creating an active binding row.
    fn reservation_by_target(
        &self,
        target_id: &str,
    ) -> Result<Option<BindingReservation>, Self::Error>;
    fn reservation_by_identity(
        &self,
        identity_id: &str,
    ) -> Result<Option<BindingReservation>, Self::Error>;
    fn reservation_by_pane(
        &self,
        host: crate::host::HostKind,
        server_id: &str,
        pane_id: &str,
    ) -> Result<Option<BindingReservation>, Self::Error>;
    fn begin_binding_reservation(
        &mut self,
        reservation: &BindingReservation,
    ) -> Result<(), Self::Error>;
    fn finish_binding_reservation(
        &mut self,
        reservation: &BindingReservation,
    ) -> Result<bool, Self::Error>;
    fn insert_reserved_binding(
        &mut self,
        identity: &Identity,
        reservation: &BindingReservation,
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
    /// Gives an unretired identity a new name in place; its UUID stays.
    fn rename_identity(
        &mut self,
        identity: &Identity,
        name: &ValidatedName,
    ) -> Result<Identity, Self::Error>;
}

pub trait BindingRepository: IdentityRepository {
    fn with_binding_transaction<T, E: From<Self::Error>>(
        &mut self,
        operation: impl FnOnce(&mut dyn BindingRecords<Error = Self::Error>) -> Result<T, E>,
    ) -> Result<T, E>;
}

/// One invocation-owned adapter. It owns the shared three-second budget, not
/// core wall time. Presence reads acquire host evidence outside write locks;
/// mutations reset the budget after acquiring their coordination lock.
pub trait BindingEndpoint {
    type Error;
    fn begin_coordination(&mut self);
    fn budget_available(&self) -> bool;
    /// A child that could not be reaped is never a best-effort failure.
    fn cleanup_failed(&self, _: &Self::Error) -> bool {
        false
    }
    /// Observe this publisher before acquiring the SQLite writer.
    fn publication_owner(&mut self) -> Result<Option<ProcessIncarnation>, Self::Error> {
        Ok(None)
    }
    /// Inspect an existing publisher outside the SQLite writer; unknown never proves death.
    fn publication_liveness(
        &mut self,
        _: &ProcessIncarnation,
    ) -> Result<session::RuntimeLiveness, Self::Error> {
        Ok(session::RuntimeLiveness::Unknown)
    }
    /// Wall-clock milliseconds for the durable publication lease.
    fn publication_time_ms(&self) -> Option<u64> {
        None
    }
    /// The host `current_snapshot` observes: the caller's own.
    fn current_host(&self) -> crate::host::HostKind;
    fn current_snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, Self::Error>;
    fn probe_binding(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, Self::Error>;
    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), Self::Error>;
    /// Publish only over the observed empty marker or exact stale binding ID;
    /// the new binding's own marker also permits idempotent retry.
    fn publish_if(
        &mut self,
        binding: &Binding,
        identity: &Identity,
        expected_binding_id: Option<&str>,
    ) -> Result<bool, Self::Error>;
    /// Capture an explicitly replaceable legacy marker outside the writer.
    fn legacy_marker(
        &mut self,
        _server: &ServerEvidence,
        _pane_id: &str,
    ) -> Result<Option<String>, Self::Error> {
        Ok(None)
    }
    /// Atomically publish only over the exact captured legacy marker bytes.
    fn publish_if_legacy(
        &mut self,
        _binding: &Binding,
        _identity: &Identity,
        _expected_raw: &str,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }
    /// Whether clearing this binding is an atomic exact-ID compare-and-swap.
    /// Unknown and legacy hosts conservatively use a reservation.
    fn conditional_clear(&mut self, _binding: &Binding) -> Result<bool, Self::Error> {
        Ok(false)
    }
    /// Clears this binding's marker. `identity` is the binding's identity, so
    /// a host with agent names takes away only the name that equals its name.
    fn clear(&mut self, binding: &Binding, identity: &Identity) -> Result<bool, Self::Error>;
    /// Conditionally clear this binding's marker after a failed activation.
    fn clear_if_owned(
        &mut self,
        binding: &Binding,
        identity: &Identity,
    ) -> Result<bool, Self::Error> {
        self.clear(binding, identity)
    }
    /// The start token of a pane shell's [`ProcessIncarnation`], observed by
    /// core when a binding is made; `None` when it can't be observed. Read
    /// paths don't ask: on macOS one `ps` costs about 110 ms.
    ///
    /// [`ProcessIncarnation`]: crate::endpoint::ProcessIncarnation
    fn pane_incarnation(
        &mut self,
        server: &ServerEvidence,
        pane_pid: u64,
    ) -> Result<Option<String>, Self::Error>;
}

#[derive(Clone, Copy)]
pub enum GuardedIdentity<'a> {
    Name(&'a str),
    Uuid(&'a str),
}

#[derive(Debug)]
pub struct GuardedBoundIdentity {
    pub bound: BoundIdentity,
    pub already_bound: bool,
}

pub trait GuardedBindingEndpoint: BindingEndpoint {
    fn discovery_target(
        &mut self,
        pane: &str,
    ) -> Result<Option<crate::discovery::DiscoveryTarget>, Self::Error>;
}

#[derive(Debug)]
pub enum BindingError<R, O> {
    InvalidName(NameError),
    NameNotFound(String),
    PaneNotFound(String),
    TargetChanged(String),
    PaneAlreadyBound,
    NameAlreadyActive,
    /// Another unretired identity already holds the requested name.
    NameTaken(String),
    ConfirmationRequired,
    /// The identity's bound agent may still be running; nothing was removed.
    AgentRunning(String, AgentRunning),
    /// The binding changed between the remove guard and the delete.
    Changed,
    Unverified,
    Deadline,
    Repository(R),
    Endpoint(O),
    /// Guarded admission completed; host effects may survive SQL rollback or a
    /// failed commit. Never claim a refusal without effects or automatic replay.
    Partial(Box<BindingError<R, O>>),
    /// The binding failed with `error`, and retiring the temporary identity
    /// this invocation had just created failed too (`cleanup`).
    CleanupFailed {
        error: Box<BindingError<R, O>>,
        cleanup: R,
    },
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
            Self::NameTaken(name) => write!(output, "Another identity is already named '{name}'."),
            Self::ConfirmationRequired => output.write_str("Saved identity removal requires --force. Its role and preamble will be removed; exchanges are retained."),
            Self::AgentRunning(name, AgentRunning::Alive) => write!(output, "{name}'s agent is still running; stop it first with tmt stop {name} or quit it in its pane, then remove. Nothing was removed."),
            Self::AgentRunning(name, AgentRunning::Unproven) => write!(output, "{name}'s agent is suspended or could not be observed, so it cannot be proven ended; exit it in its pane, then remove. Nothing was removed."),
            Self::AgentRunning(name, AgentRunning::HostUnobserved) => write!(output, "{name}'s host could not be observed, so TMT cannot tell whether its agent ended. Nothing was removed; retry once the host is reachable."),
            Self::Changed => output.write_str("The identity's binding changed before it was removed. Nothing was removed."),
            Self::Unverified => output.write_str("Could not verify the identity binding. No successful binding change is claimed."),
            Self::Deadline => output.write_str("Identity coordination deadline exceeded."),
            Self::Repository(_) => output.write_str("Could not update identity state."),
            Self::Endpoint(error) => error.fmt(output),
            Self::Partial(error) => write!(output,"Guarded binding may have partial host effects: {error}"),
            Self::CleanupFailed { error, .. } => error.fmt(output),
        }
    }
}

impl<R: Error + 'static, O: Error + 'static> Error for BindingError<R, O> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidName(error) => Some(error),
            Self::Repository(error) => Some(error),
            Self::Endpoint(error) => Some(error),
            Self::Partial(error) | Self::CleanupFailed { error, .. } => Some(error.as_ref()),
            _ => None,
        }
    }
}
