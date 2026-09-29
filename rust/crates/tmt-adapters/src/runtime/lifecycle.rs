//! Runtime-owned lifecycle policy. CLI callers only coordinate evidence and CAS.

use std::time::Instant;
use tmt_core::binding::session::{
    BindingSessionState, DriverState, ObservedSessionKey, ProviderSessionId, RuntimeIncarnation,
    RuntimeLiveness, RuntimeMode, SessionPreferences,
};

#[derive(Debug, Clone, Copy)]
pub enum HostEvidence {
    Independent { runtime_pid: Option<u32> },
    Ambiguous { runtime_pid: u32 },
    Unsupported,
}

impl HostEvidence {
    pub fn shared(self) -> bool {
        matches!(self, Self::Ambiguous { .. })
    }

    pub fn runtime_pid(self) -> Option<u32> {
        match self {
            Self::Independent { runtime_pid } => runtime_pid,
            Self::Ambiguous { runtime_pid } => Some(runtime_pid),
            Self::Unsupported => None,
        }
    }
}

#[derive(Debug)]
pub struct LifecycleUnavailable;

pub trait LifecycleObservation {
    fn session(&self) -> &ProviderSessionId;
    fn starting(&self) -> bool;
    /// Optional persistence: the driver state to keep after this starting
    /// event, given the same driver's previous state. Only fields the provider
    /// reported count; without them the driver returns the previous state.
    fn driver_state(&self, _previous: Option<&DriverState>) -> Option<DriverState> {
        None
    }
    fn propose(
        &self,
        current: &BindingSessionState,
        process: &RuntimeIncarnation,
        previous: RuntimeLiveness,
        host: HostEvidence,
        owned_resume: bool,
    ) -> Option<BindingSessionState>;
}

pub trait RuntimeLifecycle {
    fn decode(&self, _payload: &[u8]) -> Option<Box<dyn LifecycleObservation>> {
        None
    }

    fn encode_context(&self, text: &str) -> Option<String> {
        super::hook_protocol::encode_context(text)
    }

    fn host_evidence(&self) -> Result<HostEvidence, LifecycleUnavailable> {
        Ok(HostEvidence::Unsupported)
    }

    fn observe_in_pane(
        &self,
        _caller_pid: u64,
        _pane_pid: u64,
        _deadline: Instant,
    ) -> Option<RuntimeIncarnation> {
        None
    }

    fn observe_replacement(
        &self,
        _pane_pid: u64,
        _deadline: Instant,
    ) -> Option<RuntimeIncarnation> {
        None
    }

    fn mode(&self, _host: HostEvidence) -> Option<RuntimeMode> {
        None
    }

    /// A driver may recognize exact resume coordinates; the coordinator must
    /// independently prove the launch owner is still alive before admitting it.
    fn permits_owned_resume(
        &self,
        _preferences: &SessionPreferences,
        _session: &ProviderSessionId,
        _host: HostEvidence,
    ) -> bool {
        false
    }

    fn client_exit(
        &self,
        current: &BindingSessionState,
        key: ObservedSessionKey,
        owner: RuntimeIncarnation,
        _preferences: &SessionPreferences,
    ) -> Option<BindingSessionState> {
        current.record_launched_exit(key, owner)
    }

    /// The driver-state version this driver reads. A driver without the
    /// persistence interface reads none, so any stored state is discarded.
    fn state_version(&self) -> Option<u16> {
        None
    }

    /// The model recorded in this driver's own state, for display only.
    fn state_model(&self, _state: &DriverState) -> Option<String> {
        None
    }

    /// An admitted runtime may outlive its owned foreground client. None keeps
    /// the generic exact-child exit path; a value is the driver's disconnect state.
    fn disconnected(
        &self,
        _current: &BindingSessionState,
        _preferences: &SessionPreferences,
    ) -> Option<BindingSessionState> {
        None
    }
}

pub struct NoLifecycle;
impl RuntimeLifecycle for NoLifecycle {}
