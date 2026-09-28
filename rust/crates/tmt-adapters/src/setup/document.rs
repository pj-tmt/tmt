//! Surgical JSON edits: preserve unrelated values and all document bytes outside
//! the edited value. RawValue borrows retain opaque numbers and user hook text.

use super::{PlanError, SETTINGS_LIMIT};
use crate::runtime::claude::hook_entry;
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use serde_json::{Value, value::RawValue};
use std::fmt;

struct Object<'a>(Vec<(String, &'a RawValue)>);

impl<'de> Deserialize<'de> for Object<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = Object<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object without duplicate keys")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut fields = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, &'de RawValue>()? {
                    if fields.iter().any(|(previous, _)| previous == &key) {
                        return Err(serde::de::Error::custom("duplicate settings key"));
                    }
                    fields.push((key, value));
                }
                Ok(Object(fields))
            }
        }
        deserializer.deserialize_map(Fields)
    }
}

fn object(text: &str) -> Result<Object<'_>, PlanError> {
    serde_json::from_str(text).map_err(|_| PlanError::InvalidSettings)
}

fn field<'a>(object: &'a Object<'a>, key: &str) -> Option<&'a str> {
    object
        .0
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.get())
}

fn set(text: &str, key: &str, value: &str) -> Result<String, PlanError> {
    let fields = object(text)?;
    if let Some(old) = field(&fields, key) {
        let start = old.as_ptr() as usize - text.as_ptr() as usize;
        let mut result = text.to_owned();
        result.replace_range(start..start + old.len(), value);
        return Ok(result);
    }
    let end = text.rfind('}').ok_or(PlanError::InvalidSettings)?;
    let comma = if fields.0.is_empty() { "" } else { "," };
    let key = serde_json::to_string(key).map_err(|_| PlanError::InvalidSettings)?;
    let mut result = text.to_owned();
    result.insert_str(end, &format!("{comma}\n  {key}: {value}\n"));
    Ok(result)
}

/// Only the exact generated command shape establishes ownership. A matching
/// marker in user text is not enough; edited entries are never overwritten.
fn owned(entry: &RawValue) -> Result<bool, PlanError> {
    let fields = object(entry.get())?;
    let handlers: Vec<&RawValue> =
        serde_json::from_str(field(&fields, "hooks").ok_or(PlanError::InvalidSettings)?)
            .map_err(|_| PlanError::InvalidSettings)?;
    let mut candidates = Vec::new();
    for handler in handlers {
        let fields = object(handler.get())?;
        if let Some(command) = field(&fields, "command") {
            let command: String =
                serde_json::from_str(command).map_err(|_| PlanError::InvalidSettings)?;
            if command.contains("__hook claude") {
                candidates.push(command);
            }
        }
    }
    if candidates.is_empty() {
        return Ok(false);
    }
    if candidates.len() != 1 {
        return Err(PlanError::EditedHook);
    }
    // Only a candidate owned entry needs semantic comparison. All other
    // provider data remains raw, including numbers outside f64's range.
    let value: Value = serde_json::from_str(entry.get()).map_err(|_| PlanError::EditedHook)?;
    let quoted = candidates[0]
        .strip_suffix(" __hook claude")
        .and_then(|value| value.strip_prefix('\''))
        .and_then(|value| value.strip_suffix('\''))
        .ok_or(PlanError::EditedHook)?;
    let launcher = quoted.replace("'\\''", "'");
    if !std::path::Path::new(&launcher).is_absolute() || value != hook_entry(&launcher) {
        return Err(PlanError::EditedHook);
    }
    Ok(true)
}

fn event_array(text: &str, launcher: &str, removing: bool) -> Result<String, PlanError> {
    let entries: Vec<&RawValue> =
        serde_json::from_str(text).map_err(|_| PlanError::InvalidSettings)?;
    let mut count = 0;
    let desired = hook_entry(launcher).to_string();
    let mut result = Vec::new();
    let mut changed = false;
    for entry in entries {
        if owned(entry)? {
            count += 1;
            if count > 1 {
                return Err(PlanError::EditedHook);
            }
            if removing {
                changed = true;
            } else {
                let old: Value =
                    serde_json::from_str(entry.get()).map_err(|_| PlanError::InvalidSettings)?;
                if old == hook_entry(launcher) {
                    result.push(entry.get().to_owned());
                } else {
                    changed = true;
                    result.push(desired.clone());
                }
            }
        } else {
            result.push(entry.get().to_owned());
        }
    }
    if count == 0 && !removing {
        changed = true;
        result.push(desired);
    }
    Ok(if changed {
        format!("[{}]", result.join(","))
    } else {
        text.to_owned()
    })
}

pub(super) fn claude_settings(
    text: &str,
    launcher: &str,
    removing: bool,
) -> Result<String, PlanError> {
    if text.len() > SETTINGS_LIMIT {
        return Err(PlanError::TooLarge);
    }
    let root = object(text)?;
    let existing = field(&root, "hooks");
    if existing.is_none() && removing {
        return Ok(text.to_owned());
    }
    let mut hooks = existing.unwrap_or("{}").to_owned();
    for event in ["SessionStart", "SessionEnd"] {
        let fields = object(&hooks)?;
        let old = field(&fields, event);
        if old.is_none() && removing {
            continue;
        }
        let edited = event_array(old.unwrap_or("[]"), launcher, removing)?;
        hooks = set(&hooks, event, &edited)?;
    }
    let result = set(text, "hooks", &hooks)?;
    if result.len() > SETTINGS_LIMIT {
        return Err(PlanError::TooLarge);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_user_bytes_and_repairs_only_stale_owned_launchers() {
        let user = r#"{ "matcher" : "startup", "future": 1e999, "hooks": [{"type":"command", "command":"user-hook", "timeout":123, "opaque": 1e999}] }"#;
        let input = format!(
            "{{\n  \"permissions\": {{ \"allow\": [\"Bash(git *)\"] }},\n  \"opaque\": 1.234000e+99,\n  \"hooks\": {{\"SessionStart\": [{user}], \"Stop\": []}}\n}}\n"
        );
        let first = claude_settings(&input, "/stable/tmt", false).unwrap();
        assert!(first.contains(user));
        assert!(first.contains("\"opaque\": 1.234000e+99"));
        assert!(first.contains("\"permissions\": { \"allow\": [\"Bash(git *)\"] }"));
        assert_eq!(
            claude_settings(&first, "/stable/tmt", false).unwrap(),
            first
        );
        let repaired = claude_settings(&first, "/new stable/tmt", false).unwrap();
        assert!(!repaired.contains("/stable/tmt"));
        assert!(repaired.contains(user));
        let removed = claude_settings(&repaired, "/new stable/tmt", true).unwrap();
        assert!(removed.contains(user));
        assert!(!removed.contains("__hook"));
        assert_eq!(
            claude_settings(&removed, "/new stable/tmt", true).unwrap(),
            removed
        );
    }

    #[test]
    fn refuses_invalid_duplicate_and_edited_settings_without_guessing() {
        for input in [
            "{",
            "[]",
            r#"{"hooks":null}"#,
            r#"{"hooks":{},"hooks":{}}"#,
            r#"{"hooks":{"SessionStart":{},"SessionEnd":[]}}"#,
            r#"{"hooks":{"SessionStart":[{"hooks":[],"hooks":[]}]}}"#,
            r#"{"hooks":{"SessionStart":[{"hooks":[{"command":"keep","command":"replace"}]}]}}"#,
        ] {
            assert_eq!(
                claude_settings(input, "/tmt", false),
                Err(PlanError::InvalidSettings)
            );
        }
        let owned = hook_entry("/tmt");
        let duplicated =
            serde_json::json!({"hooks":{"SessionStart":[owned.clone(),owned.clone()]}}).to_string();
        assert_eq!(
            claude_settings(&duplicated, "/tmt", false),
            Err(PlanError::EditedHook)
        );
        let mut edited = owned;
        edited["hooks"][0]["timeout"] = 30.into();
        let input = serde_json::json!({"hooks":{"SessionStart":[edited]}}).to_string();
        assert_eq!(
            claude_settings(&input, "/tmt", true),
            Err(PlanError::EditedHook)
        );
        assert_eq!(claude_settings("{\n}\n", "/tmt", true).unwrap(), "{\n}\n");
    }
}
