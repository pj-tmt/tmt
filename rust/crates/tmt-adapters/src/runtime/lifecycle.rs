//! Runtime-owned lifecycle policy. CLI callers only coordinate evidence and CAS.

use crate::skill_installation::ProviderEnvironment;
use std::{path::PathBuf, time::Instant};
use tmt_core::binding::session::{
    BindingSessionState, DriverState, ObservedSessionKey, ProviderSessionId, RuntimeLiveness,
    RuntimeMode, SessionPreferences,
};
use tmt_core::endpoint::ProcessIncarnation;

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
    /// event, given the same driver's previous state and the wall time. Only
    /// fields the provider reported count; without them the driver returns
    /// the previous state.
    fn driver_state(&self, _previous: Option<&DriverState>, _now_ms: u64) -> Option<DriverState> {
        None
    }
    fn propose(
        &self,
        current: &BindingSessionState,
        process: &ProcessIncarnation,
        previous: RuntimeLiveness,
        host: HostEvidence,
        owned_resume: bool,
    ) -> Option<BindingSessionState>;
}

/// A provider turn ended in a remembered session (#519). It is not a session
/// transition: it only lets the driver refresh its own state, and its hook
/// prints nothing. The transcript path is untrusted hook input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEnd {
    pub session: ProviderSessionId,
    pub transcript: Option<PathBuf>,
}

pub trait RuntimeLifecycle {
    fn decode(&self, _payload: &[u8]) -> Option<Box<dyn LifecycleObservation>> {
        None
    }

    /// A prompt submission does not establish or replace a session binding.
    fn decode_prompt(&self, _payload: &[u8]) -> Option<ProviderSessionId> {
        None
    }

    fn encode_prompt_context(&self, text: &str) -> Option<String> {
        super::hook_protocol::encode_event_context("UserPromptSubmit", text)
    }

    /// A turn-end event, recognized even when it carries nothing to read.
    fn decode_turn(&self, _payload: &[u8]) -> Option<TurnEnd> {
        None
    }

    /// The driver state after `turn`, given the remembered session's previous
    /// state; `None` leaves it unchanged. Reads stay under the driver's own
    /// tree in `environment` and within [`super::transcript::TAIL_LIMIT`].
    fn turn_state(
        &self,
        _turn: &TurnEnd,
        _environment: &ProviderEnvironment,
        _previous: Option<&DriverState>,
        _now_ms: u64,
    ) -> Option<DriverState> {
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
    ) -> Option<ProcessIncarnation> {
        None
    }

    fn observe_replacement(
        &self,
        _pane_pid: u64,
        _deadline: Instant,
    ) -> Option<ProcessIncarnation> {
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
        owner: ProcessIncarnation,
        _preferences: &SessionPreferences,
    ) -> Option<BindingSessionState> {
        current.record_launched_exit(key, owner)
    }

    /// Whether this driver reads a state document of this version. A driver
    /// without the persistence interface reads none, so any stored state is
    /// discarded.
    fn reads_state(&self, _version: u16) -> bool {
        false
    }

    /// The model recorded in this driver's own state, for display only.
    fn state_model(&self, _state: &DriverState) -> Option<String> {
        None
    }

    /// The context usage recorded in this driver's own state, for display only.
    fn state_usage(&self, _state: &DriverState) -> Option<super::driver_state::Usage> {
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
