//! Shared command failure and table presentation.

pub use tmt_command_output::{Failure, after_cleanup, identity_document, identity_missing};

/// One shell word for a command a hint prints: plain when safe, otherwise
/// single-quoted, so a name or ID pastes back exactly.
pub fn shell_word(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// The remembered session projection shared by `identity show` and
/// `ls --json`. It is additive: the `resume` key appears only while a session
/// is remembered, and `usage` only while its driver recorded one (#519). The
/// model and usage come from the session's own driver; core never parses
/// driver state.
pub fn resume_document(
    preferences: &tmt_core::binding::session::SessionPreferences,
    registry: &tmt_adapters::runtime::RuntimeRegistry,
) -> Option<serde_json::Value> {
    preferences.remembered.as_ref().map(|session| {
        let mut resume = serde_json::json!({
            "driver": session.harness.as_str(),
            "mode": session.mode.as_str(),
            "session": session.provider_session.as_str(),
            "model": registry.remembered_model(session),
            "staleAtMs": session.stale_at_ms,
        });
        if let Some(usage) = registry.remembered_usage(session) {
            resume["usage"] = usage.document();
        }
        resume
    })
}

#[cfg(test)]
mod tests {
    use super::shell_word;

    #[test]
    fn shell_words_preserve_exact_opaque_values() {
        assert_eq!(shell_word("request-id"), "request-id");
        assert_eq!(shell_word("identity with space"), "'identity with space'");
        assert_eq!(shell_word("owner's"), "'owner'\\''s'");
    }
}
