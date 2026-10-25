//! Exact generated hook forms TMT has shipped. Keep this historical list
//! separate from the current generator so updates can recognize old owned data.
use crate::drivers::{DriverDefinition, HookFormat};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
enum ShippedForm {
    SessionCommandV1,
    NamedCommandV1,
}

// SessionCommandV1: Claude #344 and Codex #346. NamedCommandV1: agy #3007.
// Both shipped a three-second timeout and the same shell-quoted launcher.
const SHIPPED_FORMS: &[ShippedForm] = &[ShippedForm::SessionCommandV1, ShippedForm::NamedCommandV1];

pub(super) fn matches(driver: &DriverDefinition, value: &Value, launcher: &str) -> bool {
    let quoted = format!("'{}'", launcher.replace('\'', "'\\''"));
    let command = json!({"type":"command", "command":format!("{quoted} __hook {}", driver.name()), "timeout":3});
    SHIPPED_FORMS
        .iter()
        .any(|form| match (form, driver.descriptor.hooks) {
            (ShippedForm::SessionCommandV1, Some(HookFormat::SessionHooksJson)) => {
                value == &json!({"hooks":[command.clone()]})
            }
            (ShippedForm::NamedCommandV1, Some(HookFormat::NamedHooksJson)) => value == &command,
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::Registry;

    #[test]
    fn every_shipped_form_is_explicit_and_edits_are_not_owned() {
        // Frozen document shapes from the setup generator history, not values
        // produced by the implementation under test.
        for (provider, fixture) in [
            (
                "claude",
                r#"{"hooks":[{"type":"command","command":"'/old/bin/tmt' __hook claude","timeout":3}]}"#,
            ),
            (
                "codex",
                r#"{"hooks":[{"type":"command","command":"'/old/bin/tmt' __hook codex","timeout":3}]}"#,
            ),
            (
                "agy",
                r#"{"type":"command","command":"'/old/bin/tmt' __hook agy","timeout":3}"#,
            ),
        ] {
            let driver = Registry::builtin().find(provider).unwrap();
            let original: Value = serde_json::from_str(fixture).unwrap();
            assert!(matches(driver, &original, "/old/bin/tmt"));
            assert_eq!(
                super::super::hook_entry(driver, "/old/bin/tmt").unwrap(),
                original
            );
            for edited in [
                fixture.replace(":3", ":30"),
                fixture.replace("__hook", "other-command"),
                fixture.replace(
                    "\"type\":\"command\"",
                    "\"type\":\"command\",\"extra\":true",
                ),
            ] {
                assert!(!matches(
                    driver,
                    &serde_json::from_str(&edited).unwrap(),
                    "/old/bin/tmt"
                ));
            }
        }
    }
}
