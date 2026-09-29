//! Claude hook wire mapping. Payloads describe observations, never bindings.

pub use super::hook_protocol::{
    CONTEXT_LIMIT, HOOK_INPUT_LIMIT, HOOK_TIMEOUT_SECONDS, encode_context,
};
use serde::Deserialize;
use serde_json::Value;
use tmt_core::binding::session::{
    BindingSessionState, ObservedSessionKey, ProviderSessionId, RuntimeIncarnation,
    RuntimeLiveness, RuntimeState, SessionTransition,
};

pub fn observe_in_pane(
    runner: &impl crate::process::CommandRunner,
    caller_pid: u64,
    pane_pid: u64,
    deadline: std::time::Instant,
) -> Option<RuntimeIncarnation> {
    super::evidence::observe_named_in_pane(runner, caller_pid, pane_pid, deadline, "claude")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeObservation {
    pub session: ProviderSessionId,
    /// The model the provider reported for this event, unvalidated.
    pub model: Option<String>,
    pub transition: SessionTransition,
    pub starting: bool,
}

impl ClaudeObservation {
    /// Called only with a fresh driver-verified Claude process in the verified
    /// bound pane. The caller commits this proposal using the existing full CAS.
    pub fn propose(
        &self,
        current: &BindingSessionState,
        incarnation: &RuntimeIncarnation,
        previous_liveness: RuntimeLiveness,
    ) -> Option<BindingSessionState> {
        let key = ObservedSessionKey {
            incarnation: incarnation.clone(),
            provider_session: Some(self.session.clone()),
        };
        if !self.starting {
            let mut next = current.transition(&key, self.transition, None)?;
            if matches!(
                self.transition,
                SessionTransition::Cleared | SessionTransition::Resumed
            ) {
                // A conversation switch is not a terminal end, but input is
                // not deliverable until its matching start has been observed.
                next.state = RuntimeState::Unknown;
            }
            return Some(next);
        }
        if let Some(previous) = &current.key {
            if previous.incarnation != *incarnation {
                if previous_liveness != RuntimeLiveness::Gone {
                    return None;
                }
            } else {
                if current.state == RuntimeState::Ended {
                    return None;
                }
                match self.transition {
                    SessionTransition::Compacted => {
                        return current.transition(&key, self.transition, None);
                    }
                    SessionTransition::Cleared | SessionTransition::Resumed
                        if previous.provider_session != key.provider_session =>
                    {
                        // The matching preliminary end records continuation. A
                        // delayed start from an older conversation cannot jump
                        // over a completed newer start with no matching end.
                        if current.state != RuntimeState::Unknown
                            || current.last_transition != Some(self.transition)
                        {
                            return None;
                        }
                        return current.transition(
                            previous,
                            self.transition,
                            Some(self.session.clone()),
                        );
                    }
                    SessionTransition::Cleared | SessionTransition::Resumed => {
                        return current.transition(&key, self.transition, None);
                    }
                    _ => {}
                }
            }
        }
        current.admit(key, self.transition, RuntimeLiveness::Alive)
    }
}

#[derive(Deserialize)]
struct Payload {
    hook_event_name: String,
    session_id: String,
    /// Documented provider field: the active model slug.
    model: Option<String>,
    source: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookInputError {
    TooLarge,
    Invalid,
    Unsupported,
}

/// Unknown future events fail without a state guess. Additional provider fields
/// are ignored; in particular, transcript paths and cwd are never read or trusted.
pub fn decode_hook(bytes: &[u8]) -> Result<ClaudeObservation, HookInputError> {
    if bytes.len() > HOOK_INPUT_LIMIT {
        return Err(HookInputError::TooLarge);
    }
    let payload: Payload = serde_json::from_slice(bytes).map_err(|_| HookInputError::Invalid)?;
    let session =
        ProviderSessionId::new(&payload.session_id).map_err(|_| HookInputError::Invalid)?;
    let (starting, transition) = match payload.hook_event_name.as_str() {
        "SessionStart" => (
            true,
            match payload.source.as_deref() {
                Some("startup") => SessionTransition::Started,
                Some("resume") => SessionTransition::Resumed,
                Some("clear") => SessionTransition::Cleared,
                Some("compact") => SessionTransition::Compacted,
                Some("fork") => SessionTransition::Forked,
                _ => return Err(HookInputError::Unsupported),
            },
        ),
        "SessionEnd" => (
            false,
            match payload.reason.as_deref() {
                Some("clear") => SessionTransition::Cleared,
                Some("resume") => SessionTransition::Resumed,
                Some("logout" | "prompt_input_exit" | "other") => SessionTransition::Ended,
                _ => return Err(HookInputError::Unsupported),
            },
        ),
        _ => return Err(HookInputError::Unsupported),
    };
    Ok(ClaudeObservation {
        session,
        model: payload.model,
        transition,
        starting,
    })
}

/// The launcher is selected and validated by setup, retaining its stable symlink
/// rather than canonicalizing it into an immutable release directory.
pub fn hook_entry(launcher: &str) -> Value {
    super::hook_protocol::command_entry("claude", launcher)
}

pub struct ClaudeLifecycle;

impl super::lifecycle::RuntimeLifecycle for ClaudeLifecycle {
    fn state_version(&self) -> Option<u16> {
        Some(super::model_state::MODEL_STATE_VERSION)
    }

    fn observe_replacement(
        &self,
        pane_pid: u64,
        deadline: std::time::Instant,
    ) -> Option<RuntimeIncarnation> {
        super::evidence::observe_replacement(
            &crate::process::SupervisedProbeRunner,
            pane_pid,
            deadline,
            "claude",
        )
    }
    fn decode(&self, payload: &[u8]) -> Option<Box<dyn super::lifecycle::LifecycleObservation>> {
        decode_hook(payload).ok().map(|value| Box::new(value) as _)
    }

    fn host_evidence(
        &self,
    ) -> Result<super::lifecycle::HostEvidence, super::lifecycle::LifecycleUnavailable> {
        Ok(super::lifecycle::HostEvidence::Independent { runtime_pid: None })
    }

    fn observe_in_pane(
        &self,
        caller: u64,
        pane: u64,
        deadline: std::time::Instant,
    ) -> Option<RuntimeIncarnation> {
        observe_in_pane(
            &crate::process::SupervisedProbeRunner,
            caller,
            pane,
            deadline,
        )
    }

    fn mode(
        &self,
        _: super::lifecycle::HostEvidence,
    ) -> Option<tmt_core::binding::session::RuntimeMode> {
        tmt_core::binding::session::RuntimeMode::new(super::CLAUDE_MODE_DEFAULT).ok()
    }
}

impl super::lifecycle::LifecycleObservation for ClaudeObservation {
    fn session(&self) -> &ProviderSessionId {
        &self.session
    }
    fn driver_state(
        &self,
        previous: Option<&tmt_core::binding::session::DriverState>,
    ) -> Option<tmt_core::binding::session::DriverState> {
        super::model_state::next_state(self.model.as_deref(), previous)
    }
    fn starting(&self) -> bool {
        self.starting
    }
    fn propose(
        &self,
        current: &BindingSessionState,
        process: &RuntimeIncarnation,
        previous: RuntimeLiveness,
        _: super::lifecycle::HostEvidence,
        _: bool,
    ) -> Option<BindingSessionState> {
        ClaudeObservation::propose(self, current, process, previous)
    }
}

#[cfg(test)]
mod tests;
