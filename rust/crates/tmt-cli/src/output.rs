//! Shared command failure and table presentation.

pub use tmt_command_output::{Failure, after_cleanup, identity_document, identity_missing};

/// The remembered session projection shared by `identity show` and
/// `ls --json`: additive, `null` when nothing is remembered. The model comes
/// from the session's own driver; core never parses driver state.
pub fn resume_document(
    preferences: &tmt_core::binding::session::SessionPreferences,
    registry: &tmt_adapters::runtime::RuntimeRegistry,
) -> serde_json::Value {
    preferences
        .remembered
        .as_ref()
        .map_or(serde_json::Value::Null, |session| {
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
