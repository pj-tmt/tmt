//! Shared command failure and table presentation.

pub use tmt_command_output::{Failure, after_cleanup, identity_document, identity_missing};

pub use tmt_core::names::command_word as shell_word;

/// The remembered session projection shared by `identity show` and
/// `ls --json`. It is additive: the `resume` key appears only while a session
/// is remembered, and `usage` only while its driver recorded one (#519). The
/// model, the last observed model, usage and optional consumption come from
/// the session's own driver;
/// core never parses
/// driver state.
pub fn resume_document(
    preferences: &tmt_core::binding::session::SessionPreferences,
    registry: &tmt_adapters::runtime::RuntimeRegistry,
    sampled: Option<&tmt_adapters::storage::SampledAccount>,
) -> Option<serde_json::Value> {
    preferences.remembered.as_ref().map(|session| {
        let mut resume = serde_json::json!({
            "driver": session.harness.as_str(),
            "mode": session.mode.as_str(),
            "session": session.provider_session.as_str(),
            "model": registry.remembered_model(session),
            "staleAtMs": session.stale_at_ms,
        });
        if let Some(observed) = registry.remembered_observed_model(session) {
            resume["observedModel"] = observed.document();
        }
        if let Some(usage) = registry.remembered_usage(session) {
            resume["usage"] = usage.document();
        }
        if let Some(mut consumption) = registry.remembered_consumption(session) {
            // The remembered state is capped at 1024 bytes and carries no account; the newest
            // sample names it only when it is the same observation (#5274).
            if let (Some(limits), Some(sampled)) = (consumption.rate_limits.as_mut(), sampled)
                && sampled.driver == session.harness.as_str()
                && sampled.session == session.provider_session.as_str()
                && sampled.provider == limits.provider
                && sampled.observed_at_ms == limits.observed_at_ms
            {
                limits.account = Some(sampled.account.clone());
            }
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
        // A start document as a pre-v5 binary wrote it: no observed model.
        let original = tmt_core::binding::session::DriverState::new(
            driver_state::USAGE_STATE_VERSION,
            &serde_json::json!({"model":"model-a","usage":usage.document()}).to_string(),
        )
        .unwrap();
        let mut preferences = SessionPreferences {
            retained: Vec::new(),
            channel: None,
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
        let legacy = resume_document(&preferences, &registry, None).unwrap();
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
            "observedAtMs":100,"complete":false,"gap":true,
            // Optional Codex limits ride the same public document, unchanged.
            "rateLimits":{"provider":"codex","observedAtMs":90,"windows":[{"windowMinutes":10080,"usedPercent":78.5,"resetsAtMs":1_791_948_558_000u64}]}
        });
        let counters = State::read(&serde_json::json!({"value":public})).unwrap();
        preferences.remembered.as_mut().unwrap().state =
            driver_state::after_observation(Some(usage), Some(counters), Some(&original));
        let mut projected = resume_document(&preferences, &registry, None).unwrap();
        assert_eq!(projected["consumption"], public);
        projected.as_object_mut().unwrap().remove("consumption");
        assert_eq!(
            projected, legacy,
            "existing fields are byte-equivalent values"
        );
        assert!(resume_document(&SessionPreferences::default(), &registry, None).is_none());
    }

    #[test]
    fn a_sampled_account_names_only_the_limits_of_the_same_observation() {
        use tmt_adapters::{
            runtime::{
                RuntimeRegistry,
                consumption::{RateAccount, State},
                driver_state,
            },
            storage::SampledAccount,
        };
        use tmt_core::binding::session::{
            HarnessId, ProviderSessionId, RememberedSession, RuntimeMode, SessionPreferences,
        };
        let public = serde_json::json!({
            "inputTokens":10,"outputTokens":2,"cachedInputTokens":5,
            "epoch":"00000000-0000-4000-8000-000000000001","sequence":1,
            "observedAtMs":100,"complete":false,"gap":true,
            "rateLimits":{"provider":"claude","observedAtMs":90,"windows":[{"windowMinutes":10080,"usedPercent":78.5,"resetsAtMs":1_791_948_558_000u64}]}
        });
        let counters = State::read(&serde_json::json!({"value":public})).unwrap();
        let preferences = SessionPreferences {
            retained: Vec::new(),
            channel: None,
            preferred_harness: None,
            remembered: Some(RememberedSession {
                harness: HarnessId::new("claude").unwrap(),
                mode: RuntimeMode::new("independent").unwrap(),
                provider_session: ProviderSessionId::new("s").unwrap(),
                state: driver_state::after_observation(None, Some(counters), None),
                stale_at_ms: None,
                resume_pending_at_ms: None,
            }),
        };
        let registry = RuntimeRegistry::first_party();
        let account = RateAccount::derive("claude", "account-a", ".claude-work");
        let sampled = |driver: &str, session: &str, provider: &str, observed: u64| SampledAccount {
            driver: driver.into(),
            session: session.into(),
            provider: provider.into(),
            observed_at_ms: observed,
            account: account.clone(),
        };
        let limits = |sampled: Option<&SampledAccount>| {
            resume_document(&preferences, &registry, sampled).unwrap()["consumption"]["rateLimits"]
                .clone()
        };
        // Without a sample, or with one of another session, driver, provider or observation (a
        // later reading may belong to another account), nothing is added.
        assert!(limits(None).get("account").is_none());
        for other in [
            sampled("claude", "other", "claude", 90),
            sampled("codex", "s", "claude", 90),
            sampled("claude", "s", "codex", 90),
            sampled("claude", "s", "claude", 91),
            sampled("claude", "s", "claude", 89),
        ] {
            assert!(limits(Some(&other)).get("account").is_none(), "{other:?}");
        }
        let named = limits(Some(&sampled("claude", "s", "claude", 90)));
        assert_eq!(
            named["account"],
            serde_json::json!({"key": account.key, "label": "claude-work"})
        );
        // Everything else in the limits is the stored value, unchanged.
        let mut bare = named;
        bare.as_object_mut().unwrap().remove("account");
        assert_eq!(bare, public["rateLimits"]);
    }

    #[test]
    fn the_observed_model_is_additive_and_absent_when_unrecorded() {
        use tmt_adapters::runtime::{RuntimeRegistry, driver_state};
        use tmt_core::binding::session::{
            HarnessId, ProviderSessionId, RememberedSession, RuntimeMode, SessionPreferences,
        };
        let registry = RuntimeRegistry::first_party();
        let project = |state| {
            let preferences = SessionPreferences {
                retained: Vec::new(),
                channel: None,
                preferred_harness: None,
                remembered: Some(RememberedSession {
                    harness: HarnessId::new("codex").unwrap(),
                    mode: RuntimeMode::new("embedded").unwrap(),
                    provider_session: ProviderSessionId::new("s").unwrap(),
                    state,
                    stale_at_ms: None,
                    resume_pending_at_ms: None,
                }),
            };
            resume_document(&preferences, &registry, None).unwrap()
        };
        let started = driver_state::after_start(Some("gpt-5.2-codex"), None, None, 100);
        assert_eq!(
            project(started.clone()),
            serde_json::json!({
                "driver":"codex", "mode":"embedded", "session":"s", "model":"gpt-5.2-codex",
                "observedModel":{"model":"gpt-5.2-codex","observedAtMs":100}, "staleAtMs":null
            })
        );
        // Without a recorded observation the key is absent, never null.
        let cleared = driver_state::after_start(None, None, started.as_ref(), 200);
        let document = project(cleared);
        assert!(document.get("observedModel").is_none());
        assert_eq!(document["model"], "gpt-5.2-codex");
        assert!(project(None).get("observedModel").is_none());
    }
}
