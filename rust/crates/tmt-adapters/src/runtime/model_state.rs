//! First-party driver state: the model a provider hook reported, the only
//! resume detail kept. Never inferred from transcripts, arguments or files.

use serde_json::{Map, Value, json};
use tmt_core::binding::session::DriverState;

pub const MODEL_STATE_VERSION: u16 = 1;

/// A provider model slug, safe to pass as one argv value: bounded, no
/// whitespace or controls, and never option-like.
fn valid_model(model: &str) -> bool {
    (1..=128).contains(&model.len())
        && !model.starts_with('-')
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._:/[]@".contains(&byte))
}

/// The model stored in a readable first-party state document, if any.
pub fn state_model(state: &DriverState) -> Option<String> {
    if state.version() != MODEL_STATE_VERSION {
        return None;
    }
    let document: Map<String, Value> = serde_json::from_str(state.document()).ok()?;
    match (document.len(), document.get("model")) {
        (1, Some(Value::String(model))) if valid_model(model) => Some(model.clone()),
        _ => None,
    }
}

/// A reported model replaces the stored one. When none is reported, the
/// previous readable state stays; nothing is guessed.
pub fn next_state(reported: Option<&str>, previous: Option<&DriverState>) -> Option<DriverState> {
    match reported.filter(|model| valid_model(model)) {
        Some(model) => {
            DriverState::new(MODEL_STATE_VERSION, &json!({ "model": model }).to_string()).ok()
        }
        None => previous
            .filter(|state| state_model(state).is_some())
            .cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reported_model_replaces_and_an_absent_one_keeps_the_previous() {
        let first = next_state(Some("claude-opus-5"), None).unwrap();
        assert_eq!(first.document(), r#"{"model":"claude-opus-5"}"#);
        assert_eq!(state_model(&first).as_deref(), Some("claude-opus-5"));
        assert_eq!(next_state(None, Some(&first)), Some(first.clone()));
        let second = next_state(Some("claude-sonnet-4-5[1m]"), Some(&first)).unwrap();
        assert_eq!(
            state_model(&second).as_deref(),
            Some("claude-sonnet-4-5[1m]")
        );
        assert_eq!(next_state(None, None), None);
    }

    #[test]
    fn unsafe_models_and_unreadable_documents_are_never_used() {
        let kept = next_state(Some("gpt-5.2-codex"), None).unwrap();
        for model in [
            "",
            "--dangerously-skip-permissions",
            "a b",
            "x\n",
            &"m".repeat(129),
        ] {
            assert_eq!(
                next_state(Some(model), Some(&kept)),
                Some(kept.clone()),
                "{model:?}"
            );
        }
        for (version, document) in [
            (2, r#"{"model":"opus"}"#),
            (1, r#"{"model":"opus","effort":"high"}"#),
            (1, r#"{"model":7}"#),
            (1, r#"{"model":"-x"}"#),
            (1, "not json"),
        ] {
            let state = DriverState::new(version, document).unwrap();
            assert_eq!(state_model(&state), None, "{document}");
            assert_eq!(next_state(None, Some(&state)), None, "{document}");
        }
    }
}
