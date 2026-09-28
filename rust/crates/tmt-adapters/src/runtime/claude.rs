//! Claude hook wire mapping. Payloads describe observations, never bindings.

use serde::Deserialize;
use serde_json::{Value, json};
use tmt_core::binding::session::{
    BindingSessionState, ObservedSessionKey, ProviderSessionId, RuntimeIncarnation,
    RuntimeLiveness, RuntimeState, SessionTransition,
};

mod evidence;
pub use evidence::observe_in_pane;

pub const HOOK_INPUT_LIMIT: usize = 64 * 1024;
pub const CONTEXT_LIMIT: usize = 4096;
pub const HOOK_TIMEOUT_SECONDS: u64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeObservation {
    pub session: ProviderSessionId,
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
        transition,
        starting,
    })
}

/// Only a verified, successfully processed start may call this encoder. End
/// hooks, missing context and failures have no stdout, not malformed raw text.
/// No permission decision, veto or user-facing spinner is part of this output.
pub fn encode_context(context: &str) -> Option<String> {
    if context.is_empty() || context.len() > CONTEXT_LIMIT {
        return None;
    }
    Some(
        json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": context
            }
        })
        .to_string(),
    )
}

/// The launcher is selected and validated by setup, retaining its stable symlink
/// rather than canonicalizing it into an immutable release directory.
pub fn hook_entry(launcher: &str) -> Value {
    let quoted = format!("'{}'", launcher.replace('\'', "'\\''"));
    json!({"hooks": [{
        "type": "command",
        "command": format!("{quoted} __hook claude"),
        "timeout": HOOK_TIMEOUT_SECONDS
    }]})
}

#[cfg(test)]
mod tests;
