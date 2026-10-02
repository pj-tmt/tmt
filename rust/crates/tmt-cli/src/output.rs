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
/// model, usage and optional consumption come from the session's own driver;
/// core never parses
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
        if let Some(consumption) = registry.remembered_consumption(session) {
            resume["consumption"] = consumption.document();
        }
        resume
    })
}

/// Activity is meaningful only with caller-supplied runtime evidence. Storage-only
/// callers pass Unknown rather than claiming to have observed a live process.
pub fn activity_document(
    binding: Option<&tmt_core::binding::Binding>,
    remembered: Option<&tmt_core::binding::session::RememberedSession>,
    runtime: tmt_core::binding::session::RuntimeState,
    registry: &tmt_adapters::runtime::RuntimeRegistry,
) -> serde_json::Value {
    use tmt_core::binding::session::RuntimeState;
    let mut value = serde_json::json!({"state":"unknown", "sinceMs":null, "lastActivityMs":null, "providers":{}});
    if runtime == RuntimeState::Ended {
        value["state"] = "ended".into();
    }
    let Some((binding, remembered)) = binding.zip(remembered) else {
        return value;
    };
    let Some(key) = binding.session.key.as_ref() else {
        return value;
    };
    if key.provider_session.as_ref() != Some(&remembered.provider_session) {
        return value;
    }
    let Some(activity) = registry
        .lifecycle(&remembered.harness)
        .and_then(|driver| {
            remembered
                .state
                .as_ref()
                .and_then(|state| driver.state_activity(state))
        })
        .filter(|activity| activity.matches(&remembered.provider_session, &key.incarnation))
    else {
        return value;
    };
    if runtime == RuntimeState::Running {
        value["state"] = activity.phase.as_str().into();
        value["sinceMs"] = activity.since_ms.into();
    }
    value["lastActivityMs"] = activity.last_activity_ms.into();
    value
}

#[cfg(test)]
mod tests {
    use super::{resume_document, shell_word};

    #[test]
    fn shell_words_preserve_exact_opaque_values() {
        assert_eq!(shell_word("request-id"), "request-id");
        assert_eq!(shell_word("identity with space"), "'identity with space'");
        assert_eq!(shell_word("owner's"), "'owner'\\''s'");
    }
    #[test]
    fn consumption_is_additive_and_keeps_existing_usage_projection_exact() {
        use tmt_adapters::runtime::{RuntimeRegistry, consumption::State, driver_state};
        use tmt_core::binding::session::{
            HarnessId, ProviderSessionId, RememberedSession, RuntimeMode, SessionPreferences,
        };
        let usage = driver_state::Usage::new(195_664, None, 100).unwrap();
        let original = driver_state::after_start(Some("model-a"), Some(usage), None).unwrap();
        let mut preferences = SessionPreferences {
            preferred_harness: None,
            remembered: Some(RememberedSession {
                harness: HarnessId::new("claude").unwrap(),
                mode: RuntimeMode::new("independent").unwrap(),
                provider_session: ProviderSessionId::new("s").unwrap(),
                state: Some(original.clone()),
                stale_at_ms: None,
                resume_pending_at_ms: None,
            }),
        };
        let registry = RuntimeRegistry::first_party();
        let legacy = resume_document(&preferences, &registry).unwrap();
        assert_eq!(
            legacy,
            serde_json::json!({
                "driver":"claude", "mode":"independent", "session":"s", "model":"model-a", "staleAtMs":null,
                "usage":{"tokens":195664,"observedAtMs":100}
            })
        );
        let public = serde_json::json!({
            "inputTokens":10,"outputTokens":2,"cachedInputTokens":5,
            "epoch":"00000000-0000-4000-8000-000000000001","sequence":1,
            "observedAtMs":100,"complete":false,"gap":true
        });
        let counters = State::read(&serde_json::json!({"value":public})).unwrap();
        preferences.remembered.as_mut().unwrap().state =
            driver_state::after_observation(Some(usage), Some(counters), Some(&original));
        let mut projected = resume_document(&preferences, &registry).unwrap();
        assert_eq!(projected["consumption"], public);
        projected.as_object_mut().unwrap().remove("consumption");
        assert_eq!(
            projected, legacy,
            "existing fields are byte-equivalent values"
        );
        assert!(resume_document(&SessionPreferences::default(), &registry).is_none());
    }
}
