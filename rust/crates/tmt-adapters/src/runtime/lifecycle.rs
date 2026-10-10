//! Runtime-owned lifecycle policy. CLI callers only coordinate evidence and CAS.

use crate::skill_installation::ProviderEnvironment;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
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
    /// The callback's own normalized transition, not the previously stored one.
    fn transition(&self) -> Option<tmt_core::binding::session::SessionTransition> {
        None
    }
    /// A binding locator held by a driver's private enrollment record for this
    /// exact session. Only such drivers may supply it. This selects stored
    /// evidence; fresh host/process proof and `propose` still authorize context.
    fn verified_binding(&self) -> Option<&str> {
        None
    }
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

/// Invocation-owned correlation locators. Native admission remains mandatory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerSession {
    pub session: ProviderSessionId,
    pub runtime_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerSessionRefusal {
    Configuration,
    IndexUnavailable,
    IndexShape,
    HeaderUnavailable,
    HeaderShape,
    NotRoot,
}

impl CallerSessionRefusal {
    pub fn layer(self) -> &'static str {
        match self {
            Self::Configuration => "provider-configuration",
            Self::IndexUnavailable => "provider-index-unavailable",
            Self::IndexShape => "provider-index-shape",
            Self::HeaderUnavailable => "provider-header-unavailable",
            Self::HeaderShape => "provider-header-shape",
            Self::NotRoot => "provider-not-root",
        }
    }
}

pub trait RuntimeLifecycle {
    fn launch_settings(
        &self,
        _args: &[std::ffi::OsString],
    ) -> super::launch_preset::LaunchSettings {
        super::launch_preset::LaunchSettings::default()
    }

    /// Compose settings using this provider's own resume grammar.
    fn resume_settings(
        &self,
        _command: &mut super::RuntimeCommand,
        settings: &super::launch_preset::LaunchSettings,
    ) -> bool {
        settings.model.is_none() && settings.effort.is_none()
    }

    fn caller_session(&self) -> Option<CallerSession> {
        None
    }

    /// An exact-ID provider header/index can corroborate a main conversation;
    /// it never establishes ownership and never reads transcript records.
    fn caller_session_observation(
        &self,
        _coordinates: &CallerSession,
        _environment: &ProviderEnvironment,
        _deadline: Instant,
    ) -> Result<Box<dyn LifecycleObservation>, CallerSessionRefusal> {
        Err(CallerSessionRefusal::HeaderShape)
    }

    fn observe_main_caller(
        &self,
        _runner: &dyn crate::process::CommandRunner,
        _caller: u64,
        _pane: u64,
        _deadline: Instant,
    ) -> Option<ProcessIncarnation> {
        None
    }

    /// Optional provider-owned, session-only hook installation. Global provider
    /// settings are read for composition/ownership, never written by a launch.
    fn prepare_launch_hooks(
        &self,
        _plan: &super::hook_protocol::LaunchHooks<'_>,
    ) -> std::io::Result<Option<super::RuntimeCommand>> {
        Ok(None)
    }

    /// Only a provider's main-agent continuation boundary may claim Digest.
    fn decode_digest_turn(&self, _payload: &[u8]) -> Option<ProviderSessionId> {
        None
    }

    fn encode_digest_turn(&self, _digest: &str) -> Option<String> {
        None
    }

    /// Provider-owned user-visible advisory output, merged into one decision.
    fn encode_hook_notice(&self, _message: &str, _decision: Option<&str>) -> Option<String> {
        None
    }

    /// Map a native hook caller to the exact launch incarnation. Shared callers
    /// need a driver-owned enrollment proof; the default admits only independent
    /// callers and never grants a shared server the foreground's authority.
    fn digest_process(
        &self,
        _current: &BindingSessionState,
        observed: &ProcessIncarnation,
        _session: &ProviderSessionId,
        host: HostEvidence,
        _deadline: Instant,
    ) -> Option<ProcessIncarnation> {
        matches!(host, HostEvidence::Independent { .. }).then(|| observed.clone())
    }

    /// Optional shared work budget for input, admission and the hook worker.
    /// The driver reserves process cleanup and provider timeout margin.
    fn hook_work_duration(&self) -> Option<Duration> {
        None
    }

    /// Driver-owned startup gate within the parent hook deadline.
    /// Drivers without deferred admission return immediately.
    fn wait_for_hook_admission(
        &self,
        _payload: &[u8],
        _deadline: Instant,
    ) -> Result<(), LifecycleUnavailable> {
        Ok(())
    }

    fn decode(&self, _payload: &[u8]) -> Option<Box<dyn LifecycleObservation>> {
        None
    }

    /// Attribute activity to an already admitted process without changing the
    /// binding. The default preserves the observed incarnation without effects.
    fn activity_process(
        &self,
        _current: &BindingSessionState,
        observed: &ProcessIncarnation,
        _session: &ProviderSessionId,
        _host: HostEvidence,
        _deadline: Instant,
    ) -> Option<ProcessIncarnation> {
        Some(observed.clone())
    }

    /// A prompt submission does not establish or replace a session binding.
    fn decode_prompt(&self, _payload: &[u8]) -> Option<ProviderSessionId> {
        None
    }

    fn decode_activity(
        &self,
        _payload: &[u8],
    ) -> Option<tmt_core::binding::session::activity::Event> {
        None
    }

    /// Driver-owned persistence for a normalized admitted activity event.
    fn activity_state(
        &self,
        _event: &tmt_core::binding::session::activity::Event,
        _session: &ProviderSessionId,
        _process: &ProcessIncarnation,
        _previous: Option<&DriverState>,
        _now_ms: u64,
    ) -> Option<DriverState> {
        None
    }

    fn state_activity(
        &self,
        _state: &DriverState,
    ) -> Option<tmt_core::binding::session::activity::Activity> {
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
    /// tree in `environment`. Context and Codex consumption use bounded tails;
    /// Claude consumption streams bounded records within the hook deadline.
    fn turn_state(
        &self,
        _turn: &TurnEnd,
        _environment: &ProviderEnvironment,
        _previous: Option<&DriverState>,
        _now_ms: u64,
        _deadline: Instant,
    ) -> Option<DriverState> {
        None
    }

    /// Driver-private provider-relative locator from an already admitted hook.
    fn consumption_locator(
        &self,
        _payload: &[u8],
        _environment: &ProviderEnvironment,
    ) -> Option<String> {
        None
    }

    /// Resolve an exact remembered source for foreground-owned collection.
    fn sampling_turn(
        &self,
        _session: &ProviderSessionId,
        _locator: Option<&str>,
        _environment: &ProviderEnvironment,
        _deadline: Instant,
    ) -> Option<TurnEnd> {
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
        _runner: &dyn crate::process::CommandRunner,
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

    /// Cumulative completed-request counters, never context-window size.
    fn state_consumption(&self, _state: &DriverState) -> Option<super::consumption::Consumption> {
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

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn plain_and_claude_activity_preserve_observed_bytes_without_evidence() {
        let current = BindingSessionState::default();
        let observed = ProcessIncarnation::new(991, "byte-exact-incarnation").unwrap();
        let session = ProviderSessionId::new("session").unwrap();
        let registry = crate::runtime::RuntimeRegistry::first_party();
        let claude = registry
            .lifecycle(&tmt_core::binding::session::HarnessId::new("claude").unwrap())
            .unwrap();
        for lifecycle in [&NoLifecycle as &dyn RuntimeLifecycle, claude] {
            let mapped = lifecycle
                .activity_process(
                    &current,
                    &observed,
                    &session,
                    HostEvidence::Unsupported,
                    Instant::now(),
                )
                .unwrap();
            assert_eq!(mapped.pid().to_le_bytes(), observed.pid().to_le_bytes());
            assert_eq!(
                mapped.start_identity().as_bytes(),
                observed.start_identity().as_bytes()
            );
            assert_eq!(current, BindingSessionState::default());
        }
    }

    #[test]
    fn plain_and_claude_lifecycle_admission_is_a_noop() {
        // No locator, payload decoding, process probe or deadline is required
        // by the default gate. Codex is the only driver overriding this method.
        assert!(
            NoLifecycle
                .wait_for_hook_admission(b"not a payload", Instant::now())
                .is_ok()
        );
        assert_eq!(NoLifecycle.hook_work_duration(), None);
        let registry = crate::runtime::RuntimeRegistry::first_party();
        let claude = registry
            .lifecycle(&tmt_core::binding::session::HarnessId::new("claude").unwrap())
            .unwrap();
        assert_eq!(claude.hook_work_duration(), None);
        assert!(
            claude
                .wait_for_hook_admission(b"not a payload", Instant::now())
                .is_ok()
        );
    }
}
