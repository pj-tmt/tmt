//! Runtime observations attached to a binding, never identity ownership.
//!
//! A live container can contain an ended runtime. Conversely, disconnecting a
//! client need not end a shared runtime. Drivers supply those distinctions;
//! neither a provider session ID nor inherited hook text authorizes a binding.
//! Foreground launch callers use `admit_launched`, not direct owner assignment.

use std::{error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceKind {
    Container,
    Session,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RuntimeState {
    #[default]
    Unknown,
    Running,
    Ended,
}

impl RuntimeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Running => "running",
            Self::Ended => "ended",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTransition {
    Started,
    Resumed,
    Cleared,
    Compacted,
    Forked,
    Ended,
}

/// A driver identifier, not an executable name, shell command or config path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessId(String);

impl HarnessId {
    pub fn new(value: &str) -> Result<Self, SessionValueError> {
        if !(1..=64).contains(&value.len())
            || !value.as_bytes()[0].is_ascii_lowercase()
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
        {
            return Err(SessionValueError::Harness);
        }
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque provider-owned value. Do not normalize it or assume all providers use UUIDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSessionId(String);

impl ProviderSessionId {
    pub fn new(value: &str) -> Result<Self, SessionValueError> {
        if !(1..=256).contains(&value.len())
            || value.trim().is_empty()
            || value.chars().any(char::is_control)
        {
            return Err(SessionValueError::ProviderSession);
        }
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque driver-owned mode identifier, using the same bounded token alphabet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMode(HarnessId);

impl RuntimeMode {
    pub fn new(value: &str) -> Result<Self, SessionValueError> {
        HarnessId::new(value)
            .map(Self)
            .map_err(|_| SessionValueError::RuntimeMode)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Remembered identity preferences survive losing or replacing an interface.
/// Keeping them does not assert that the referenced runtime still exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionPreferences {
    pub preferred_harness: Option<HarnessId>,
    pub remembered: Option<RememberedSession>,
}

/// Exact resume coordinates stay paired even when the preferred harness changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberedSession {
    pub harness: HarnessId,
    pub mode: RuntimeMode,
    pub provider_session: ProviderSessionId,
}

/// Evidence belongs to one binding, unlike the identity's remembered preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingSessionState {
    pub last_transition: Option<SessionTransition>,
    pub state: RuntimeState,
    pub key: Option<ObservedSessionKey>,
    /// The foreground wrapper is binding evidence, not provider session identity.
    pub launch_owner: Option<RuntimeIncarnation>,
}

/// Drivers supply verified process identity, never a PID or hook environment alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIncarnation {
    pid: u64,
    start_identity: ProviderSessionId,
}

impl RuntimeIncarnation {
    pub fn new(pid: u64, start_identity: &str) -> Result<Self, SessionValueError> {
        if !crate::endpoint::valid_process_id(pid) {
            return Err(SessionValueError::Incarnation);
        }
        Ok(Self {
            pid,
            start_identity: ProviderSessionId::new(start_identity)
                .map_err(|_| SessionValueError::Incarnation)?,
        })
    }
    pub fn pid(&self) -> u64 {
        self.pid
    }
    pub fn start_identity(&self) -> &str {
        self.start_identity.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedSessionKey {
    pub incarnation: RuntimeIncarnation,
    /// Codex may not disclose its thread ID at launch. The process is still fenced.
    pub provider_session: Option<ProviderSessionId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLiveness {
    Alive,
    Gone,
    Unknown,
}

impl BindingSessionState {
    /// Record an already-exited, directly owned and unreaped launch. The caller
    /// must retain actual start evidence for that child and its launch owner.
    /// Unlike admission, this never constructs a deliverable Running state.
    pub fn record_launched_exit(
        &self,
        key: ObservedSessionKey,
        owner: RuntimeIncarnation,
    ) -> Option<Self> {
        if self.key.as_ref().is_some_and(|old| {
            old.incarnation == key.incarnation
                && (self
                    .launch_owner
                    .as_ref()
                    .is_some_and(|current| current != &owner)
                    || (old.provider_session.is_some()
                        && old.provider_session != key.provider_session))
        }) {
            return None;
        }
        Some(Self {
            key: Some(key),
            launch_owner: Some(owner),
            state: RuntimeState::Ended,
            last_transition: Some(SessionTransition::Ended),
        })
    }

    /// A foreground launch needs live evidence for both processes. Hooks use
    /// `admit` instead and cannot replace ownership on an existing incarnation.
    pub fn admit_launched(
        &self,
        key: ObservedSessionKey,
        owner: RuntimeIncarnation,
        transition: SessionTransition,
        child_liveness: RuntimeLiveness,
        owner_liveness: RuntimeLiveness,
    ) -> Option<Self> {
        if owner_liveness != RuntimeLiveness::Alive
            || !matches!(
                transition,
                SessionTransition::Started | SessionTransition::Resumed
            )
            || self.key.as_ref().is_some_and(|old| {
                old.incarnation == key.incarnation
                    && self
                        .launch_owner
                        .as_ref()
                        .is_some_and(|current| current != &owner)
            })
        {
            return None;
        }
        let mut next = self.admit(key, transition, child_liveness)?;
        next.launch_owner = Some(owner);
        Some(next)
    }

    /// Admission requires fresh driver evidence. Uncertain admission leaves a
    /// known-ended observation intact rather than enabling unverified delivery.
    pub fn admit(
        &self,
        key: ObservedSessionKey,
        transition: SessionTransition,
        liveness: RuntimeLiveness,
    ) -> Option<Self> {
        if liveness != RuntimeLiveness::Alive
            || !matches!(
                transition,
                SessionTransition::Started | SessionTransition::Resumed | SessionTransition::Forked
            )
            || self.key.as_ref().is_some_and(|old| {
                old.incarnation == key.incarnation
                    && (self.state == RuntimeState::Ended
                        || (old.provider_session.is_some()
                            && old.provider_session != key.provider_session))
            })
        {
            return None;
        }
        Some(Self {
            launch_owner: self
                .key
                .as_ref()
                .filter(|old| old.incarnation == key.incarnation)
                .and(self.launch_owner.clone()),
            key: Some(key),
            last_transition: Some(transition),
            state: RuntimeState::Running,
        })
    }

    /// Clear's preliminary end notification maps to Cleared, not terminal Ended.
    /// A subsequent clear-start can replace the session ID in this incarnation.
    pub fn transition(
        &self,
        key: &ObservedSessionKey,
        transition: SessionTransition,
        next_session: Option<ProviderSessionId>,
    ) -> Option<Self> {
        if self.key.as_ref() != Some(key) || self.state == RuntimeState::Ended {
            return None;
        }
        if !matches!(
            transition,
            SessionTransition::Ended
                | SessionTransition::Compacted
                | SessionTransition::Cleared
                | SessionTransition::Resumed
        ) || (next_session.is_some()
            && !matches!(
                transition,
                SessionTransition::Cleared | SessionTransition::Resumed
            ))
        {
            return None;
        }
        let mut next = self.clone();
        if let Some(session) = next_session {
            next.key.as_mut()?.provider_session = Some(session);
        }
        next.last_transition = Some(transition);
        next.state = if transition == SessionTransition::Ended {
            RuntimeState::Ended
        } else {
            RuntimeState::Running
        };
        Some(next)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionValueError {
    Harness,
    ProviderSession,
    RuntimeMode,
    Incarnation,
}

impl fmt::Display for SessionValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Harness => "Invalid harness identifier.",
            Self::ProviderSession => "Invalid provider session identifier.",
            Self::RuntimeMode => "Invalid runtime mode identifier.",
            Self::Incarnation => "Invalid runtime process incarnation.",
        })
    }
}

impl Error for SessionValueError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binding_without_runtime_evidence_is_unknown() {
        assert_eq!(
            BindingSessionState::default(),
            BindingSessionState {
                last_transition: None,
                state: RuntimeState::Unknown,
                key: None,
                launch_owner: None,
            }
        );
    }

    #[test]
    fn harness_identifiers_are_bounded_not_commands() {
        for value in ["codex", "claude", "community_driver-2", &"a".repeat(64)] {
            assert_eq!(HarnessId::new(value).unwrap().as_str(), value);
        }
        for value in [
            "",
            "Codex",
            "2codex",
            "codex --resume",
            "../codex",
            &"a".repeat(65),
        ] {
            assert_eq!(HarnessId::new(value), Err(SessionValueError::Harness));
        }
    }

    #[test]
    fn provider_session_ids_preserve_exact_bytes_without_uuid_assumptions() {
        for value in [
            "opaque-session:42",
            "11111111-1111-7111-8111-111111111111",
            &"x".repeat(256),
        ] {
            assert_eq!(ProviderSessionId::new(value).unwrap().as_str(), value);
        }
        for value in ["", "  ", "a\nb", "a\0b", &"x".repeat(257)] {
            assert_eq!(
                ProviderSessionId::new(value),
                Err(SessionValueError::ProviderSession)
            );
        }
    }

    #[test]
    fn provider_metadata_does_not_implicitly_claim_a_running_agent() {
        let preferences = SessionPreferences {
            preferred_harness: Some(HarnessId::new("codex").unwrap()),
            remembered: Some(RememberedSession {
                harness: HarnessId::new("codex").unwrap(),
                mode: RuntimeMode::new("shared").unwrap(),
                provider_session: ProviderSessionId::new("retained-history").unwrap(),
            }),
        };
        let state = BindingSessionState::default();
        assert!(preferences.remembered.is_some());
        assert_eq!(state.state, RuntimeState::Unknown);
        assert_eq!(state.last_transition, None);
    }

    #[test]
    fn runtime_modes_are_driver_owned_bounded_tokens() {
        for value in ["shared", "embedded", "default", "community-mode_2"] {
            assert_eq!(RuntimeMode::new(value).unwrap().as_str(), value);
        }
        for value in ["", "../config", "shared --approval=never", &"x".repeat(65)] {
            assert_eq!(RuntimeMode::new(value), Err(SessionValueError::RuntimeMode));
        }
    }

    fn key(start: &str, session: &str) -> ObservedSessionKey {
        ObservedSessionKey {
            incarnation: RuntimeIncarnation::new(42, start).unwrap(),
            provider_session: Some(ProviderSessionId::new(session).unwrap()),
        }
    }

    #[test]
    fn launched_admission_requires_two_live_processes_and_preserves_owner_authority() {
        let original = key("child-start", "session-a");
        let owner = RuntimeIncarnation::new(41, "owner-start").unwrap();
        let initial = BindingSessionState::default();
        for child in [
            RuntimeLiveness::Alive,
            RuntimeLiveness::Gone,
            RuntimeLiveness::Unknown,
        ] {
            for wrapper in [
                RuntimeLiveness::Alive,
                RuntimeLiveness::Gone,
                RuntimeLiveness::Unknown,
            ] {
                let admitted = initial.admit_launched(
                    original.clone(),
                    owner.clone(),
                    SessionTransition::Started,
                    child,
                    wrapper,
                );
                assert_eq!(
                    admitted.is_some(),
                    child == RuntimeLiveness::Alive && wrapper == RuntimeLiveness::Alive
                );
            }
        }
        let admitted = initial
            .admit_launched(
                original.clone(),
                owner.clone(),
                SessionTransition::Resumed,
                RuntimeLiveness::Alive,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        assert_eq!(admitted.launch_owner, Some(owner.clone()));
        assert!(
            admitted
                .admit_launched(
                    original.clone(),
                    RuntimeIncarnation::new(41, "different-start").unwrap(),
                    SessionTransition::Resumed,
                    RuntimeLiveness::Alive,
                    RuntimeLiveness::Alive
                )
                .is_none()
        );
        assert!(
            initial
                .admit_launched(
                    original,
                    owner,
                    SessionTransition::Forked,
                    RuntimeLiveness::Alive,
                    RuntimeLiveness::Alive
                )
                .is_none()
        );
    }

    #[test]
    fn fast_launch_completion_never_admits_running_or_replaces_same_child_authority() {
        let key = key("fast-child", "session-a");
        let owner = RuntimeIncarnation::new(41, "owner-start").unwrap();
        let ended = BindingSessionState::default()
            .record_launched_exit(key.clone(), owner.clone())
            .unwrap();
        assert_eq!(ended.state, RuntimeState::Ended);
        assert_eq!(ended.last_transition, Some(SessionTransition::Ended));
        assert_eq!(ended.key, Some(key.clone()));
        assert_eq!(ended.launch_owner, Some(owner.clone()));
        assert!(
            ended
                .admit(
                    key.clone(),
                    SessionTransition::Started,
                    RuntimeLiveness::Alive
                )
                .is_none()
        );
        assert!(
            ended
                .record_launched_exit(
                    key.clone(),
                    RuntimeIncarnation::new(41, "other-owner").unwrap()
                )
                .is_none()
        );
        let mut stale = key;
        stale.provider_session = None;
        assert!(ended.record_launched_exit(stale, owner).is_none());
    }

    #[test]
    fn hook_events_preserve_launch_ownership_until_a_new_incarnation_is_admitted() {
        let original = key("child-start", "session-a");
        let owner = RuntimeIncarnation::new(41, "owner-start").unwrap();
        let state = BindingSessionState::default()
            .admit_launched(
                original.clone(),
                owner.clone(),
                SessionTransition::Started,
                RuntimeLiveness::Alive,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        let repeated = state
            .admit(
                original.clone(),
                SessionTransition::Started,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        assert_eq!(repeated.launch_owner, Some(owner.clone()));
        let cleared = repeated
            .transition(
                &original,
                SessionTransition::Cleared,
                Some(ProviderSessionId::new("session-b").unwrap()),
            )
            .unwrap();
        assert_eq!(cleared.launch_owner, Some(owner.clone()));
        let compacted = cleared
            .transition(
                cleared.key.as_ref().unwrap(),
                SessionTransition::Compacted,
                None,
            )
            .unwrap();
        assert_eq!(compacted.launch_owner, Some(owner.clone()));
        let ended = compacted
            .transition(
                compacted.key.as_ref().unwrap(),
                SessionTransition::Ended,
                None,
            )
            .unwrap();
        assert_eq!(ended.launch_owner, Some(owner));
        let replacement = ended
            .admit(
                key("new-child-start", "session-c"),
                SessionTransition::Started,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        assert_eq!(replacement.launch_owner, None);
    }

    #[test]
    fn clear_changes_context_without_ending_the_runtime_and_rejects_old_events() {
        let original = key("process-start", "session-a");
        let running = BindingSessionState::default()
            .admit(
                original.clone(),
                SessionTransition::Started,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        let clearing = running
            .transition(&original, SessionTransition::Cleared, None)
            .unwrap();
        assert_eq!(clearing.state, RuntimeState::Running);
        let cleared = clearing
            .transition(
                &original,
                SessionTransition::Cleared,
                Some(ProviderSessionId::new("session-b").unwrap()),
            )
            .unwrap();
        assert_eq!(cleared.state, RuntimeState::Running);
        assert_eq!(
            cleared.key.as_ref().unwrap().incarnation,
            original.incarnation
        );
        assert!(
            cleared
                .transition(&original, SessionTransition::Ended, None)
                .is_none()
        );
        assert!(
            cleared
                .transition(&original, SessionTransition::Compacted, None)
                .is_none()
        );
        assert!(
            cleared
                .admit(original, SessionTransition::Started, RuntimeLiveness::Alive)
                .is_none()
        );
        let current = cleared.key.as_ref().unwrap();
        assert_eq!(
            cleared
                .transition(current, SessionTransition::Compacted, None)
                .unwrap()
                .state,
            RuntimeState::Running
        );
        assert_eq!(
            cleared
                .transition(current, SessionTransition::Ended, None)
                .unwrap()
                .state,
            RuntimeState::Ended
        );
    }

    #[test]
    fn ended_incarnation_cannot_restart_but_a_verified_new_process_can() {
        let original = key("old-start", "session-a");
        let running = BindingSessionState::default()
            .admit(
                original.clone(),
                SessionTransition::Started,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        let ended = running
            .transition(&original, SessionTransition::Ended, None)
            .unwrap();
        assert!(
            ended
                .admit(original, SessionTransition::Resumed, RuntimeLiveness::Alive)
                .is_none()
        );
        let next = key("new-start-same-pid", "session-b");
        for evidence in [RuntimeLiveness::Unknown, RuntimeLiveness::Gone] {
            assert!(
                ended
                    .admit(next.clone(), SessionTransition::Started, evidence)
                    .is_none()
            );
            assert_eq!(ended.state, RuntimeState::Ended);
        }
        let restarted = ended
            .admit(
                next.clone(),
                SessionTransition::Started,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        assert_eq!(restarted.key, Some(next));
        assert_eq!(restarted.state, RuntimeState::Running);
    }

    #[test]
    fn correlated_in_process_resume_can_end_with_the_new_session_key() {
        let old = key("same-process", "session-a");
        let running = BindingSessionState::default()
            .admit(
                old.clone(),
                SessionTransition::Started,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        let resumed = running
            .transition(
                &old,
                SessionTransition::Resumed,
                Some(ProviderSessionId::new("session-b").unwrap()),
            )
            .unwrap();
        let new = resumed.key.as_ref().unwrap();
        assert_eq!(new.incarnation, old.incarnation);
        assert_eq!(new.provider_session.as_ref().unwrap().as_str(), "session-b");
        assert!(
            resumed
                .transition(&old, SessionTransition::Ended, None)
                .is_none()
        );
        assert_eq!(
            resumed
                .transition(new, SessionTransition::Ended, None)
                .unwrap()
                .state,
            RuntimeState::Ended
        );
    }
}
