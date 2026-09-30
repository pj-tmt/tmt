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
