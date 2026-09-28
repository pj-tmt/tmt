//! Shared command-hook envelope; decoding and lifecycle stay provider-owned.
use serde_json::{Value, json};

pub const HOOK_INPUT_LIMIT: usize = 64 * 1024;
pub const CONTEXT_LIMIT: usize = 4096;
pub const HOOK_TIMEOUT_SECONDS: u64 = 3;

pub fn encode_context(context: &str) -> Option<String> {
    if context.is_empty() || context.len() > CONTEXT_LIMIT {
        return None;
    }
    Some(json!({"hookSpecificOutput": {"hookEventName":"SessionStart", "additionalContext":context}}).to_string())
}

pub fn command_entry(provider: &str, launcher: &str) -> Value {
    let quoted = format!("'{}'", launcher.replace('\'', "'\\''"));
    json!({"hooks":[{"type":"command", "command":format!("{quoted} __hook {provider}"), "timeout":HOOK_TIMEOUT_SECONDS}]})
}
