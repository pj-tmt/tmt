//! Shared command failure and table presentation.

pub use tmt_command_output::{Failure, after_cleanup, identity_document, identity_missing};

/// The remembered session projection shared by `identity show` and
/// `ls --json`. It is additive: the `resume` key appears only while a session
/// is remembered. The model comes from the session's own driver; core never
/// parses driver state.
pub fn resume_document(
    preferences: &tmt_core::binding::session::SessionPreferences,
    registry: &tmt_adapters::runtime::RuntimeRegistry,
) -> Option<serde_json::Value> {
    preferences.remembered.as_ref().map(|session| {
        serde_json::json!({
            "driver": session.harness.as_str(),
            "mode": session.mode.as_str(),
            "session": session.provider_session.as_str(),
            "model": registry.remembered_model(session),
            "staleAtMs": session.stale_at_ms,
        })
    })
}

pub mod table {
    pub use tmt_command_output::table::write;
}
