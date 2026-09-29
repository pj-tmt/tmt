//! Codex lifecycle mapping. Shared server ancestry never selects an identity.
//! The verified 0.157.1 contract reports SessionEnd reason `other`; unsupported
//! reasons leave state unchanged rather than guessing that a client ended a thread.

pub use super::hook_protocol::encode_context;
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
    super::evidence::observe_named_in_pane(runner, caller_pid, pane_pid, deadline, "codex")
}

#[derive(Debug, Clone)]
pub struct CodexObservation {
    pub session: ProviderSessionId,
    pub starting: bool,
    pub transition: SessionTransition,
}

#[derive(Deserialize)]
struct Payload {
    hook_event_name: String,
    session_id: String,
    source: Option<String>,
    reason: Option<String>,
}

pub fn decode_hook(bytes: &[u8]) -> Option<CodexObservation> {
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let payload: Payload = serde_json::from_slice(bytes).ok()?;
    let (starting, transition) = match payload.hook_event_name.as_str() {
        "SessionStart" => (
            true,
            match payload.source.as_deref()? {
                "startup" => SessionTransition::Started,
                "resume" => SessionTransition::Resumed,
                "clear" => SessionTransition::Cleared,
                "compact" => SessionTransition::Compacted,
                _ => return None,
            },
        ),
        "SessionEnd" if payload.reason.as_deref() == Some("other") => {
            (false, SessionTransition::Ended)
        }
        _ => return None,
    };
    Some(CodexObservation {
        session: ProviderSessionId::new(&payload.session_id).ok()?,
        starting,
        transition,
    })
}

impl CodexObservation {
    /// Independent mode has fresh pane/process evidence. Shared mode must first
    /// select a unique existing exact-thread mapping, never an ambient pane.
    pub fn propose(
        &self,
        current: &BindingSessionState,
        process: &RuntimeIncarnation,
        previous: RuntimeLiveness,
        shared: bool,
    ) -> Option<BindingSessionState> {
        self.propose_with_resume(current, process, previous, shared, false)
    }

    /// owned_resume is supplied only after a live foreground owner and its
    /// exact remembered shared-mode coordinates have been verified.
    pub fn propose_with_resume(
        &self,
        current: &BindingSessionState,
        process: &RuntimeIncarnation,
        previous: RuntimeLiveness,
        shared: bool,
        owned_resume: bool,
    ) -> Option<BindingSessionState> {
        let owned_resume = shared
            && owned_resume
            && current.launch_owner.is_some()
            && current.last_transition == Some(SessionTransition::Resumed)
            && current.state != RuntimeState::Ended;
        let key = ObservedSessionKey {
            incarnation: process.clone(),
            provider_session: Some(self.session.clone()),
        };
        if shared {
            let old = current.key.as_ref()?;
            if old.provider_session.as_ref() != Some(&self.session) {
                return None;
            }
            // A resumed thread can move from an ended embedded process to a
            // shared server, but cannot steal an independently live attachment.
            if old.incarnation != *process && previous != RuntimeLiveness::Gone && !owned_resume {
                return None;
            }
        }
        if let Some(old) = &current.key {
            if old.incarnation == *process {
                if !self.starting || matches!(self.transition, SessionTransition::Compacted) {
                    return current.transition(&key, self.transition, None);
                }
                if current.state == RuntimeState::Ended {
                    return None;
                }
                if matches!(
                    self.transition,
                    SessionTransition::Cleared | SessionTransition::Resumed
                ) {
                    // Codex clear-start is the provider's context transition;
                    // unlike Claude it need not precede it with a clear-end.
                    return current.transition(old, self.transition, Some(self.session.clone()));
                }
            } else if previous != RuntimeLiveness::Gone && !owned_resume {
                return None;
            }
        }
        if !self.starting {
            return None;
        }
        let mut next = current.admit(key, self.transition, RuntimeLiveness::Alive)?;
        if owned_resume {
            next.launch_owner = current.launch_owner.clone();
        }
        Some(next)
    }
}

pub fn hook_entry(launcher: &str) -> Value {
    super::hook_protocol::command_entry("codex", launcher)
}

/// A foreground client exiting does not prove a shared thread ended. Require
/// exact driver-owned coordinates; never reinterpret a later provider end.
pub fn disconnected(
    current: &BindingSessionState,
    preferences: &tmt_core::binding::session::SessionPreferences,
) -> Option<BindingSessionState> {
    if !is_shared_session(current.key.as_ref()?, preferences)
        || current.state == RuntimeState::Ended
    {
        return None;
    }
    let mut next = current.clone();
    next.state = RuntimeState::Unknown;
    Some(next)
}

pub fn is_shared_session(
    key: &ObservedSessionKey,
    preferences: &tmt_core::binding::session::SessionPreferences,
) -> bool {
    preferences.remembered.as_ref().is_some_and(|remembered| {
        remembered.harness.as_str() == "codex"
            && remembered.mode.as_str() == super::CODEX_MODE_SHARED
            && key.provider_session.as_ref() == Some(&remembered.provider_session)
    })
}

/// A reaped client is not a shared-thread end, but a hook's matching terminal
/// observation must remain terminal even when it arrived before launch probing.
pub fn record_client_exit(
    current: &BindingSessionState,
    key: ObservedSessionKey,
    owner: RuntimeIncarnation,
    preferences: &tmt_core::binding::session::SessionPreferences,
) -> Option<BindingSessionState> {
    let provider_ended = current.state == RuntimeState::Ended && current.key.as_ref() == Some(&key);
    let shared = is_shared_session(&key, preferences);
    let mut next = current.record_launched_exit(key, owner)?;
    if shared && !provider_ended {
        next.state = RuntimeState::Unknown;
        next.last_transition = Some(SessionTransition::Resumed);
    }
    Some(next)
}

pub struct CodexLifecycle;

impl super::lifecycle::RuntimeLifecycle for CodexLifecycle {
    fn observe_replacement(
        &self,
        pane_pid: u64,
        deadline: std::time::Instant,
    ) -> Option<RuntimeIncarnation> {
        let process = super::evidence::observe_replacement(
            &crate::process::SupervisedProbeRunner,
            pane_pid,
            deadline,
            "codex",
        )?;
        let observed = crate::runtime_caller::codex::CodexCaller::new(
            &crate::process::SupervisedProbeRunner,
            crate::runtime_caller::codex::CallerEnvironment {
                thread_id: None,
                process_id: u32::try_from(process.pid()).ok()?,
            },
        )
        .observe_host()
        .ok()??;
        (observed.0 == tmt_core::driver::caller::HostAttribution::Independent
            && u64::from(observed.1) == process.pid())
        .then_some(process)
    }
    fn decode(&self, payload: &[u8]) -> Option<Box<dyn super::lifecycle::LifecycleObservation>> {
        decode_hook(payload).map(|value| Box::new(value) as _)
    }

    fn host_evidence(
        &self,
    ) -> Result<super::lifecycle::HostEvidence, super::lifecycle::LifecycleUnavailable> {
        use super::lifecycle::{HostEvidence, LifecycleUnavailable};
        use crate::runtime_caller::codex::{CallerEnvironment, CodexCaller};
        use tmt_core::driver::caller::HostAttribution;
        Ok(
            match CodexCaller::new(
                &crate::process::SupervisedProbeRunner,
                CallerEnvironment::current(),
            )
            .observe_host()
            .map_err(|_| LifecycleUnavailable)?
            {
                Some((HostAttribution::Independent, pid)) => HostEvidence::Independent {
                    runtime_pid: Some(pid),
                },
                Some((HostAttribution::Ambiguous, pid)) => {
                    HostEvidence::Ambiguous { runtime_pid: pid }
                }
                None => HostEvidence::Unsupported,
            },
        )
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
        host: super::lifecycle::HostEvidence,
    ) -> Option<tmt_core::binding::session::RuntimeMode> {
        tmt_core::binding::session::RuntimeMode::new(if host.shared() {
            super::CODEX_MODE_SHARED
        } else {
            super::CODEX_MODE_EMBEDDED
        })
        .ok()
    }

    fn permits_owned_resume(
        &self,
        preferences: &tmt_core::binding::session::SessionPreferences,
        session: &ProviderSessionId,
        host: super::lifecycle::HostEvidence,
    ) -> bool {
        host.shared()
            && preferences.remembered.as_ref().is_some_and(|value| {
                value.harness.as_str() == "codex"
                    && value.mode.as_str() == super::CODEX_MODE_SHARED
                    && &value.provider_session == session
            })
    }

    fn client_exit(
        &self,
        current: &BindingSessionState,
        key: ObservedSessionKey,
        owner: RuntimeIncarnation,
        preferences: &tmt_core::binding::session::SessionPreferences,
    ) -> Option<BindingSessionState> {
        record_client_exit(current, key, owner, preferences)
    }

    fn disconnected(
        &self,
        current: &BindingSessionState,
        preferences: &tmt_core::binding::session::SessionPreferences,
    ) -> Option<BindingSessionState> {
        disconnected(current, preferences)
    }
}

impl super::lifecycle::LifecycleObservation for CodexObservation {
    fn session(&self) -> &ProviderSessionId {
        &self.session
    }
    fn starting(&self) -> bool {
        self.starting
    }
    fn propose(
        &self,
        current: &BindingSessionState,
        process: &RuntimeIncarnation,
        previous: RuntimeLiveness,
        host: super::lifecycle::HostEvidence,
        owned_resume: bool,
    ) -> Option<BindingSessionState> {
        self.propose_with_resume(current, process, previous, host.shared(), owned_resume)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn start(source: &str, session: &str) -> CodexObservation {
        decode_hook(
            json!({"hook_event_name":"SessionStart", "source":source, "session_id":session})
                .to_string()
                .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn owned_resume_preserves_owner_and_client_exit_is_not_thread_end() {
        use tmt_core::binding::session::{
            HarnessId, RememberedSession, RuntimeMode, SessionPreferences,
        };
        let client = RuntimeIncarnation::new(20, "client-start").unwrap();
        let owner = RuntimeIncarnation::new(10, "owner-start").unwrap();
        let server = RuntimeIncarnation::new(30, "server-start").unwrap();
        let session = ProviderSessionId::new("exact-thread").unwrap();
        let preferences = SessionPreferences {
            preferred_harness: Some(HarnessId::new("codex").unwrap()),
            remembered: Some(RememberedSession {
                harness: HarnessId::new("codex").unwrap(),
                mode: RuntimeMode::new(super::super::CODEX_MODE_SHARED).unwrap(),
                provider_session: session.clone(),
                state: None,
                stale_at_ms: None,
            }),
        };
        let launched = BindingSessionState::default()
            .admit_launched(
                ObservedSessionKey {
                    incarnation: client,
                    provider_session: Some(session),
                },
                owner.clone(),
                SessionTransition::Resumed,
                RuntimeLiveness::Alive,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        assert!(
            start("resume", "exact-thread")
                .propose(&launched, &server, RuntimeLiveness::Alive, true)
                .is_none()
        );
        let shared = start("resume", "exact-thread")
            .propose_with_resume(&launched, &server, RuntimeLiveness::Alive, true, true)
            .unwrap();
        assert_eq!(shared.launch_owner, Some(owner));
        assert_eq!(shared.key.as_ref().unwrap().incarnation, server);
        let lost = disconnected(&shared, &preferences).unwrap();
        assert_eq!(lost.state, RuntimeState::Unknown);
        assert_eq!(lost.key, shared.key);
        assert_eq!(lost.launch_owner, shared.launch_owner);
        let restored = start("resume", "exact-thread")
            .propose(&lost, &server, RuntimeLiveness::Alive, true)
            .unwrap();
        assert_eq!(restored.state, RuntimeState::Running);
        let ended = restored
            .transition(
                restored.key.as_ref().unwrap(),
                SessionTransition::Ended,
                None,
            )
            .unwrap();
        assert!(disconnected(&ended, &preferences).is_none());
        let fast = record_client_exit(
            &BindingSessionState::default(),
            launched.key.clone().unwrap(),
            launched.launch_owner.clone().unwrap(),
            &preferences,
        )
        .unwrap();
        assert_eq!(fast.state, RuntimeState::Unknown);
        let terminal = record_client_exit(
            &ended,
            ended.key.clone().unwrap(),
            ended.launch_owner.clone().unwrap(),
            &preferences,
        )
        .unwrap();
        assert_eq!(terminal.state, RuntimeState::Ended);
        assert_eq!(terminal.key, ended.key);
        let mut embedded = preferences.clone();
        embedded.remembered.as_mut().unwrap().mode =
            RuntimeMode::new(super::super::CODEX_MODE_EMBEDDED).unwrap();
        assert!(disconnected(&shared, &embedded).is_none());
        assert!(
            start("resume", "wrong-thread")
                .propose_with_resume(&launched, &server, RuntimeLiveness::Alive, true, true)
                .is_none()
        );
    }
    #[test]
    fn independent_lifecycle_and_shared_exact_mapping_are_separate() {
        let process = RuntimeIncarnation::new(20, "embedded-start").unwrap();
        let server = RuntimeIncarnation::new(30, "server-start").unwrap();
        let empty = BindingSessionState::default();
        assert!(
            start("startup", "a")
                .propose(&empty, &server, RuntimeLiveness::Gone, true)
                .is_none()
        );
        let running = start("startup", "a")
            .propose(&empty, &process, RuntimeLiveness::Unknown, false)
            .unwrap();
        let compact = start("compact", "a")
            .propose(&running, &process, RuntimeLiveness::Alive, false)
            .unwrap();
        assert_eq!(compact.key, running.key);
        let clear = start("clear", "b")
            .propose(&compact, &process, RuntimeLiveness::Alive, false)
            .unwrap();
        assert_eq!(
            clear
                .key
                .as_ref()
                .unwrap()
                .provider_session
                .as_ref()
                .unwrap()
                .as_str(),
            "b"
        );
        let end =
            decode_hook(br#"{"hook_event_name":"SessionEnd","session_id":"a","reason":"other"}"#)
                .unwrap();
        assert!(
            end.propose(&clear, &process, RuntimeLiveness::Alive, false)
                .is_none()
        );
        for previous in [RuntimeLiveness::Alive, RuntimeLiveness::Unknown] {
            assert!(
                start("resume", "b")
                    .propose(&clear, &server, previous, true)
                    .is_none()
            );
        }
        assert!(
            start("resume", "foreign")
                .propose(&clear, &server, RuntimeLiveness::Gone, true)
                .is_none()
        );
        let shared = start("resume", "b")
            .propose(&clear, &server, RuntimeLiveness::Gone, true)
            .unwrap();
        assert_eq!(shared.key.as_ref().unwrap().incarnation, server);
        let disconnected = decode_hook(
            br#"{"hook_event_name":"SessionEnd","session_id":"b","reason":"disconnect"}"#,
        );
        assert!(
            disconnected.is_none(),
            "a client disconnect is not a provider end"
        );
        let end =
            decode_hook(br#"{"hook_event_name":"SessionEnd","session_id":"b","reason":"other"}"#)
                .unwrap();
        let ended = end
            .propose(&shared, &server, RuntimeLiveness::Alive, true)
            .unwrap();
        assert_eq!(ended.state, RuntimeState::Ended);
        assert!(
            start("resume", "b")
                .propose(&ended, &server, RuntimeLiveness::Alive, true)
                .is_none()
        );
    }
}
