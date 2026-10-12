//! Shared command-hook envelope; decoding and lifecycle stay provider-owned.
use serde_json::{Value, json};

pub const HOOK_INPUT_LIMIT: usize = 64 * 1024;
pub const CONTEXT_LIMIT: usize = 4096;
pub const HOOK_TIMEOUT_SECONDS: u64 = 3;

/// Coordinates select a launch; fresh host/process evidence grants admission.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookLaunch {
    pub identity_id: String,
    pub binding_id: String,
    pub owner_pid: u64,
    pub owner_start: String,
}

impl HookLaunch {
    pub fn owner(&self) -> Option<tmt_core::endpoint::ProcessIncarnation> {
        tmt_core::endpoint::ProcessIncarnation::new(self.owner_pid, &self.owner_start).ok()
    }
    pub fn valid(&self) -> bool {
        tmt_core::dispatch::canonical_id(&self.identity_id)
            && tmt_core::dispatch::canonical_id(&self.binding_id)
            && self.owner().is_some()
    }
}

pub struct LaunchHooks<'a> {
    pub command: &'a super::RuntimeCommand,
    pub launch: &'a HookLaunch,
    pub tmt: &'a std::path::Path,
    pub environment: &'a crate::skill_installation::ProviderEnvironment,
}

/// Internal worker result, never provider output or evidence of delivery.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestHandoff {
    pub launch: HookLaunch,
    pub checklist_id: String,
    pub attempt_token: String,
    pub digest: String,
}

pub fn encode_context(context: &str) -> Option<String> {
    encode_event_context("SessionStart", context)
}

pub fn encode_event_context(event: &str, context: &str) -> Option<String> {
    if context.is_empty() || context.len() > CONTEXT_LIMIT {
        return None;
    }
    Some(
        json!({"hookSpecificOutput": {"hookEventName":event, "additionalContext":context}})
            .to_string(),
    )
}

pub fn command_entry(provider: &str, launcher: &str) -> Value {
    let quoted = format!("'{}'", launcher.replace('\'', "'\\''"));
    json!({"hooks":[{"type":"command", "command":format!("{quoted} __hook {provider}"), "timeout":HOOK_TIMEOUT_SECONDS}]})
}

/// Decode only the common prompt envelope; provider lifecycle decoding stays separate.
pub fn decode_prompt(bytes: &[u8]) -> Option<tmt_core::binding::session::ProviderSessionId> {
    #[derive(serde::Deserialize)]
    struct Prompt {
        hook_event_name: String,
        session_id: String,
    }
    if bytes.len() > HOOK_INPUT_LIMIT {
        return None;
    }
    let payload: Prompt = serde_json::from_slice(bytes).ok()?;
    (payload.hook_event_name == "UserPromptSubmit").then_some(())?;
    tmt_core::binding::session::ProviderSessionId::new(&payload.session_id).ok()
}

/// First-party turn envelope. Claude ordering comes from setup's synchronous
/// command hooks; Codex additionally correlates its documented active turn ID.
pub fn decode_activity(
    bytes: &[u8],
    correlated: bool,
) -> Option<tmt_core::binding::session::activity::Event> {
    use tmt_core::binding::session::{
        ProviderSessionId,
        activity::{ActivityPhase, Event},
    };
    #[derive(serde::Deserialize)]
    struct Turn {
        hook_event_name: String,
        session_id: String,
        turn_id: Option<String>,
    }
    if bytes.len() > HOOK_INPUT_LIMIT {
        return None;
    }
    let payload: Turn = serde_json::from_slice(bytes).ok()?;
    ProviderSessionId::new(&payload.session_id).ok()?;
    let phase = match payload.hook_event_name.as_str() {
        "UserPromptSubmit" => ActivityPhase::Working,
        "Stop" => ActivityPhase::Idle,
        _ => return None,
    };
    let turn = if correlated {
        Some(ProviderSessionId::new(payload.turn_id.as_deref()?).ok()?)
    } else {
        None
    };
    Some(Event { phase, turn })
}
