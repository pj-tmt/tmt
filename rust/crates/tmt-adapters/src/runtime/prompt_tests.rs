//! Provider hook wire contract; fixture provenance lives beside the payloads.
use super::lifecycle::RuntimeLifecycle;
use super::{RuntimeRegistry, hook_protocol, lifecycle::NoLifecycle};
use serde_json::{Value, json};
use tmt_core::binding::session::HarnessId;

#[test]
fn prompt_hooks_decode_only_submissions_and_encode_their_own_event() {
    let registry = RuntimeRegistry::first_party();
    for (name, fixture) in [
        (
            "claude",
            include_bytes!("fixtures/claude-prompt-submit.json").as_slice(),
        ),
        (
            "codex",
            include_bytes!("fixtures/codex-prompt-submit.json").as_slice(),
        ),
    ] {
        let driver = registry.lifecycle(&HarnessId::new(name).unwrap()).unwrap();
        let session = driver.decode_prompt(fixture).unwrap();
        assert_eq!(session.as_str(), "11111111-1111-4111-8111-111111111111");
        let text = "Extension fixture (informational): \"next turn\"\n";
        let encoded: Value =
            serde_json::from_str(&driver.encode_prompt_context(text).unwrap()).unwrap();
        assert_eq!(
            encoded,
            json!({"hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit", "additionalContext": text
            }})
        );
        let startup: Value = serde_json::from_str(&driver.encode_context(text).unwrap()).unwrap();
        assert_eq!(
            startup["hookSpecificOutput"]["hookEventName"],
            "SessionStart"
        );
        assert!(driver.encode_prompt_context("").is_none());
        assert!(
            driver
                .encode_prompt_context(&"x".repeat(hook_protocol::CONTEXT_LIMIT + 1))
                .is_none()
        );
        let original: Value = serde_json::from_slice(fixture).unwrap();
        for event in ["Stop", "SubagentStop", "SessionStart", "future-event"] {
            let mut bad = original.clone();
            bad["hook_event_name"] = event.into();
            assert!(driver.decode_prompt(bad.to_string().as_bytes()).is_none());
        }
        for bad in [
            b"{}".as_slice(),
            b"not json",
            &vec![b'x'; hook_protocol::HOOK_INPUT_LIMIT + 1],
        ] {
            assert!(driver.decode_prompt(bad).is_none());
        }
        let mut invalid = original;
        invalid["session_id"] = "".into();
        assert!(
            driver
                .decode_prompt(invalid.to_string().as_bytes())
                .is_none()
        );
        assert!(NoLifecycle.decode_prompt(fixture).is_none());
    }
}

#[test]
fn activity_uses_only_main_turn_events_and_codex_requires_a_turn_id() {
    use tmt_core::binding::session::activity::ActivityPhase;
    let registry = RuntimeRegistry::first_party();
    for name in ["claude", "codex"] {
        let driver = registry.lifecycle(&HarnessId::new(name).unwrap()).unwrap();
        for (event, phase) in [
            ("UserPromptSubmit", ActivityPhase::Working),
            ("Stop", ActivityPhase::Idle),
        ] {
            let mut payload =
                json!({"hook_event_name":event,"session_id":"session","turn_id":"turn"});
            let decoded = driver
                .decode_activity(payload.to_string().as_bytes())
                .unwrap();
            assert_eq!(decoded.event.kind.phase(), phase);
            assert_eq!(decoded.event.turn.is_some(), name == "codex");
            payload.as_object_mut().unwrap().remove("turn_id");
            assert_eq!(
                driver
                    .decode_activity(payload.to_string().as_bytes())
                    .is_some(),
                name == "claude"
            );
        }
        for event in [
            "SubagentStop",
            "SessionStart",
            "Notification",
            "StopFailure",
        ] {
            assert!(
                driver
                    .decode_activity(
                        json!({"hook_event_name":event,"session_id":"s","turn_id":"t"})
                            .to_string()
                            .as_bytes()
                    )
                    .is_none()
            );
        }
        for bytes in [
            b"{}".as_slice(),
            b"invalid",
            &vec![b'x'; hook_protocol::HOOK_INPUT_LIMIT + 1],
        ] {
            assert!(driver.decode_activity(bytes).is_none());
        }
    }
}

/// Codex `Interrupt` (#3207) ends its own turn as idle. Without a turn ID, for
/// a subagent, or for Claude (which names no turn), it is no activity event.
#[test]
fn only_a_codex_main_turn_interrupt_with_its_turn_id_is_an_idle_end() {
    use tmt_core::binding::session::{ProviderSessionId, activity::ActivityPhase};
    let registry = RuntimeRegistry::first_party();
    let codex = registry
        .lifecycle(&HarnessId::new("codex").unwrap())
        .unwrap();
    let claude = registry
        .lifecycle(&HarnessId::new("claude").unwrap())
        .unwrap();
    let fixture = include_bytes!("fixtures/codex-interrupt.json").as_slice();
    let decoded = codex.decode_activity(fixture).unwrap();
    assert_eq!(decoded.event.kind.phase(), ActivityPhase::Idle);
    assert_eq!(
        decoded.event.turn,
        Some(ProviderSessionId::new("fixture-turn").unwrap())
    );
    assert!(claude.decode_activity(fixture).is_none());
    let payload: Value = serde_json::from_slice(fixture).unwrap();
    let variant = |edit: &dyn Fn(&mut serde_json::Map<String, Value>)| {
        let mut value = payload.clone();
        edit(value.as_object_mut().unwrap());
        codex.decode_activity(value.to_string().as_bytes())
    };
    assert!(variant(&|value| drop(value.remove("turn_id"))).is_none());
    for turn in [json!(""), json!(null), json!(7)] {
        assert!(variant(&|value| drop(value.insert("turn_id".into(), turn.clone()))).is_none());
    }
    // Any subagent field rejects an interrupt, an explicit null included.
    for field in ["agent_id", "agent_type"] {
        for value in [json!("agent-1"), json!(null)] {
            assert!(variant(&|v| drop(v.insert(field.into(), value.clone()))).is_none());
        }
    }
    // The subagent rule is the interrupt's alone: a prompt keeps today's decoding.
    let mut prompt = payload.clone();
    prompt["hook_event_name"] = "UserPromptSubmit".into();
    prompt["agent_id"] = "agent-1".into();
    assert_eq!(
        codex
            .decode_activity(prompt.to_string().as_bytes())
            .unwrap()
            .event
            .kind.phase(),
        ActivityPhase::Working
    );
}

/// The observed model (#3207) comes only from a Codex main-session turn event.
/// A subagent-marked payload keeps its activity decoding but supplies no model,
/// and a malformed model never rejects the event.
#[test]
fn a_codex_turn_event_reports_its_main_session_model_only() {
    use tmt_core::binding::session::activity::ActivityPhase;
    let registry = RuntimeRegistry::first_party();
    let codex = registry
        .lifecycle(&HarnessId::new("codex").unwrap())
        .unwrap();
    let claude = registry
        .lifecycle(&HarnessId::new("claude").unwrap())
        .unwrap();
    for (fixture, phase) in [
        (
            include_bytes!("fixtures/codex-prompt-submit.json").as_slice(),
            ActivityPhase::Working,
        ),
        (
            include_bytes!("fixtures/codex-stop.json").as_slice(),
            ActivityPhase::Idle,
        ),
        (
            include_bytes!("fixtures/codex-interrupt.json").as_slice(),
            ActivityPhase::Idle,
        ),
    ] {
        let payload: Value = serde_json::from_slice(fixture).unwrap();
        let event = payload["hook_event_name"].as_str().unwrap();
        let decoded = codex.decode_activity(fixture).unwrap();
        assert_eq!(decoded.event.kind.phase(), phase, "{event}");
        assert_eq!(decoded.model.as_deref(), Some("gpt-5.2-codex"), "{event}");
        // Claude names no turn, so its events never report an observed model.
        assert!(
            claude
                .decode_activity(fixture)
                .is_none_or(|decoded| decoded.model.is_none()),
            "{event}"
        );
        for field in ["agent_id", "agent_type"] {
            for value in [json!("agent-1"), json!(null)] {
                let mut child = payload.clone();
                child[field] = value.clone();
                let decoded = codex.decode_activity(child.to_string().as_bytes());
                // Activity behavior is unchanged: only an interrupt is rejected.
                assert_eq!(decoded.is_some(), event != "Interrupt", "{event} {field}");
                assert!(
                    decoded.is_none_or(|decoded| decoded.model.is_none()),
                    "{event} {field}={value}"
                );
            }
        }
        for model in [json!(null), json!(7), json!("-x"), json!("a b"), json!("")] {
            let mut malformed = payload.clone();
            malformed["model"] = model.clone();
            let decoded = codex
                .decode_activity(malformed.to_string().as_bytes())
                .unwrap();
            assert_eq!(decoded.event.kind.phase(), phase, "{event} {model}");
            assert_eq!(decoded.model, None, "{event} {model}");
        }
        let mut missing = payload.clone();
        missing.as_object_mut().unwrap().remove("model");
        assert_eq!(
            codex
                .decode_activity(missing.to_string().as_bytes())
                .unwrap()
                .model,
            None
        );
    }
}
#[test]
fn claude_permission_notifications_report_waiting_without_matching_message_text() {
    let registry = RuntimeRegistry::first_party();
    let driver = registry
        .lifecycle(&HarnessId::new("claude").unwrap())
        .unwrap();
    let permission = json!({
        "hook_event_name": "Notification",
        "session_id": "session",
        "notification_type": "permission_prompt",
        "message": "Arbitrary provider presentation text"
    });
    let observed = driver.decode_activity(permission.to_string().as_bytes());
    assert_eq!(
        observed.as_ref().map(|event| event.event.kind.phase().as_str()),
        Some("waiting")
    );
    assert!(observed.unwrap().event.turn.is_none());
    for kind in [
        "idle_prompt",
        "elicitation_dialog",
        "auth_success",
        "future",
    ] {
        let mut other = permission.clone();
        other["notification_type"] = kind.into();
        other["message"] = "Permission needed".into();
        assert!(
            driver
                .decode_activity(other.to_string().as_bytes())
                .is_none(),
            "{kind}"
        );
    }
    for field in ["agent_id", "agent_type"] {
        let mut auxiliary = permission.clone();
        auxiliary[field] = "auxiliary".into();
        assert!(
            driver
                .decode_activity(auxiliary.to_string().as_bytes())
                .is_none(),
            "{field}"
        );
    }
}

#[test]
fn only_claude_main_tool_completion_clears_a_reported_approval_prompt() {
    use tmt_core::binding::session::activity::EventKind;
    let registry = RuntimeRegistry::first_party();
    for provider in ["claude", "codex"] {
        let driver = registry
            .lifecycle(&HarnessId::new(provider).unwrap())
            .unwrap();
        for event in [
            "Notification",
            "PostToolUse",
            "PostToolUseFailure",
            "PermissionRequest",
            "PermissionDenied",
        ] {
            let payload = json!({"hook_event_name": event, "session_id": "s", "turn_id": "t", "notification_type": "permission_prompt"});
            let decoded = driver.decode_activity(payload.to_string().as_bytes());
            if provider == "claude"
                && matches!(event, "Notification" | "PostToolUse" | "PostToolUseFailure")
            {
                assert_eq!(
                    decoded.unwrap().event.kind,
                    if event == "Notification" {
                        EventKind::ApprovalPrompt
                    } else {
                        EventKind::ToolFinished
                    }
                );
                assert_eq!(
                    driver
                        .decode_activity_session(payload.to_string().as_bytes())
                        .unwrap()
                        .as_str(),
                    "s"
                );
                for field in ["agent_id", "agent_type"] {
                    let mut auxiliary = payload.clone();
                    auxiliary[field] = Value::Null;
                    assert!(
                        driver
                            .decode_activity(auxiliary.to_string().as_bytes())
                            .is_none()
                    );
                    assert!(
                        driver
                            .decode_activity_session(auxiliary.to_string().as_bytes())
                            .is_none()
                    );
                }
            } else {
                assert!(decoded.is_none(), "{provider} {event}");
                assert!(
                    driver
                        .decode_activity_session(payload.to_string().as_bytes())
                        .is_none()
                );
            }
        }
    }
}
