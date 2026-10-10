//! Surgical JSON edits: preserve unrelated values and all document bytes outside
//! the edited value. RawValue borrows retain opaque numbers and user hook text.

use super::{PlanError, SETTINGS_LIMIT};
use crate::drivers::DriverDefinition;
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

/// Compose ephemeral hooks using setup's exact ownership rule. Existing owned
/// observation hooks at either level remain the sole recorder for their event.
/// All unrelated JSON, including opaque number spellings, stays raw.
pub(crate) fn compose_launch_hooks(
    provider: &DriverDefinition,
    text: &str,
    global: &str,
    observations: &[(String, Value)],
    digest: &Value,
) -> Result<String, PlanError> {
    for document in [text, global] {
        let root = object(document)?;
        for key in ["disableAllHooks", "allowManagedHooksOnly"] {
            if let Some(raw) = field(&root, key) {
                let enabled: bool =
                    serde_json::from_str(raw).map_err(|_| PlanError::InvalidSettings)?;
                if enabled {
                    return Err(PlanError::UnsupportedProvider);
                }
            }
        }
    }
    let root = object(text)?;
    let mut hooks = field(&root, "hooks").unwrap_or("{}").to_owned();
    object(&hooks)?;
    for (event, entry) in observations {
        // Validate ownership even when it is ambiguous: never double-record an
        // edited or duplicated setup entry by treating it as merely absent.
        let local_owned = owned_event(provider, text, event)?;
        let global_owned = owned_event(provider, global, event)?;
        if !local_owned && !global_owned {
            hooks = append_event(&hooks, event, entry)?;
        }
    }
    hooks = append_event(&hooks, "Stop", digest)?;
    let result = set(text, "hooks", &hooks)?;
    if result.len() > SETTINGS_LIMIT {
        return Err(PlanError::TooLarge);
    }
    Ok(result)
}

fn append_event(hooks: &str, event: &str, entry: &Value) -> Result<String, PlanError> {
    let fields = object(hooks)?;
    let entries: Vec<&RawValue> = serde_json::from_str(field(&fields, event).unwrap_or("[]"))
        .map_err(|_| PlanError::InvalidSettings)?;
    let mut values: Vec<String> = entries.iter().map(|entry| entry.get().to_owned()).collect();
    values.push(entry.to_string());
    set(hooks, event, &format!("[{}]", values.join(",")))
}

pub(crate) fn owned_event(
    provider: &DriverDefinition,
    text: &str,
    event: &str,
) -> Result<bool, PlanError> {
    let root = object(text)?;
    let Some(raw) = field(&root, "hooks") else {
        return Ok(false);
    };
    let hooks = object(raw)?;
    let Some(raw) = field(&hooks, event) else {
        return Ok(false);
    };
    let entries: Vec<&RawValue> =
        serde_json::from_str(raw).map_err(|_| PlanError::InvalidSettings)?;
    let mut count = 0;
    for entry in entries {
        count += usize::from(owned(provider, entry)?);
    }
    if count > 1 {
        return Err(PlanError::EditedHook);
    }
    Ok(count == 1)
}

/// Only the exact generated command shape establishes ownership. A matching
/// marker in user text is not enough; edited entries are never overwritten.
fn owned(provider: &DriverDefinition, entry: &RawValue) -> Result<bool, PlanError> {
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
            if command.contains(&format!("__hook {}", provider.name())) {
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
        .strip_suffix(&format!(" __hook {}", provider.name()))
        .and_then(|value| value.strip_prefix('\''))
        .and_then(|value| value.strip_suffix('\''))
        .ok_or(PlanError::EditedHook)?;
    let launcher = quoted.replace("'\\''", "'");
    if !std::path::Path::new(&launcher).is_absolute()
        || value != super::hook_entry(provider, &launcher)?
    {
        return Err(PlanError::EditedHook);
    }
    Ok(true)
}

fn event_array(
    provider: &DriverDefinition,
    text: &str,
    launcher: &str,
    removing: bool,
) -> Result<String, PlanError> {
    let entries: Vec<&RawValue> =
        serde_json::from_str(text).map_err(|_| PlanError::InvalidSettings)?;
    let mut count = 0;
    let desired = super::hook_entry(provider, launcher)?.to_string();
    let mut result = Vec::new();
    let mut changed = false;
    for entry in entries {
        if owned(provider, entry)? {
            count += 1;
            if count > 1 {
                return Err(PlanError::EditedHook);
            }
            if removing {
                changed = true;
            } else {
                let old: Value =
                    serde_json::from_str(entry.get()).map_err(|_| PlanError::InvalidSettings)?;
                if old == super::hook_entry(provider, launcher)? {
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

/// Whether the settings hold exactly one TMT-owned hook for the provider on
/// `event`. Read-only; anything unparsable or edited counts as absent.
pub(super) fn has_owned_hook(provider: &DriverDefinition, text: &str, event: &str) -> bool {
    (|| {
        let root = object(text).ok()?;
        let hooks = object(field(&root, "hooks")?).ok()?;
        let entries: Vec<&RawValue> = serde_json::from_str(field(&hooks, event)?).ok()?;
        let mut owned_entries = 0;
        for entry in entries {
            owned_entries += usize::from(owned(provider, entry).ok()?);
        }
        Some(owned_entries == 1)
    })()
    .unwrap_or(false)
}

/// The lifecycle events, in the order setup inserts them.
const LIFECYCLE: [&str; 3] = ["SessionStart", "SessionEnd", "UserPromptSubmit"];
/// The opt-in turn-end event that records context usage (#519).
pub(super) const USAGE: &str = "Stop";

/// What setup does to one event's TMT hook.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Ensure,
    Remove,
    /// Not read at all, so an unrelated user entry can never fail setup.
    Skip,
}

pub(super) fn settings(
    provider: &DriverDefinition,
    text: &str,
    launcher: &str,
    removing: bool,
    usage: super::UsageHook,
) -> Result<String, PlanError> {
    use super::UsageHook;
    if text.len() > SETTINGS_LIMIT {
        return Err(PlanError::TooLarge);
    }
    let root = object(text)?;
    let existing = field(&root, "hooks");
    if existing.is_none() && removing {
        return Ok(text.to_owned());
    }
    let lifecycle = if removing {
        Action::Remove
    } else {
        Action::Ensure
    };
    let turn_end = match usage {
        _ if removing => Action::Remove,
        UsageHook::Default | UsageHook::Install => Action::Ensure,
        UsageHook::Remove => Action::Remove,
        UsageHook::Keep if has_owned_hook(provider, text, USAGE) => Action::Ensure,
        UsageHook::Keep => Action::Skip,
    };
    let mut hooks = existing.unwrap_or("{}").to_owned();
    let actions = LIFECYCLE
        .map(|event| (event, lifecycle))
        .into_iter()
        .chain([(USAGE, turn_end)]);
    for (event, action) in actions {
        let fields = object(&hooks)?;
        let old = field(&fields, event);
        if action == Action::Skip || (old.is_none() && action == Action::Remove) {
            continue;
        }
        let edited = event_array(
            provider,
            old.unwrap_or("[]"),
            launcher,
            action == Action::Remove,
        )?;
        hooks = set(&hooks, event, &edited)?;
    }
    let result = set(text, "hooks", &hooks)?;
    if result.len() > SETTINGS_LIMIT {
        return Err(PlanError::TooLarge);
    }
    Ok(result)
}

/// Removes the provider's TMT-owned hooks, then the keys setup inserted to
/// hold them when removal left them empty, so a file setup only added to
/// returns to its exact earlier bytes. `None` when there was nothing to remove.
pub(super) fn removed(
    provider: &DriverDefinition,
    text: &str,
) -> Result<Option<String>, PlanError> {
    let result = settings(provider, text, "/", true, super::UsageHook::Keep)?;
    if result == text {
        return Ok(None);
    }
    trim_inserted(
        text,
        result,
        &[USAGE, LIFECYCLE[2], LIFECYCLE[1], LIFECYCLE[0]],
    )
    .map(Some)
}

/// Removes only the usage hook, restoring the bytes it was installed into.
pub(super) fn usage_removed(
    provider: &DriverDefinition,
    text: &str,
    launcher: &str,
) -> Result<String, PlanError> {
    let result = settings(provider, text, launcher, false, super::UsageHook::Remove)?;
    if result == text {
        return Ok(result);
    }
    trim_inserted(text, result, &[USAGE])
}

/// Drops each of `events` (latest inserted first) that removal emptied, and
/// then `hooks` itself, where setup inserted them.
fn trim_inserted(text: &str, mut result: String, events: &[&str]) -> Result<String, PlanError> {
    let root = object(text)?;
    let Some(original) = field(&root, "hooks") else {
        return Ok(result);
    };
    let original_hooks = object(original)?;
    let mut hooks = field(&object(&result)?, "hooks").unwrap_or("{}").to_owned();
    for &event in events {
        let emptied = field(&original_hooks, event).is_some_and(|value| value != "[]")
            && field(&object(&hooks)?, event) == Some("[]");
        if emptied && let Some(shorter) = unset_inserted(&hooks, event) {
            hooks = shorter;
        }
    }
    result = set(&result, "hooks", &hooks)?;
    if hooks == "{}"
        && original != "{}"
        && let Some(shorter) = unset_inserted(&result, "hooks")
    {
        result = shorter;
    }
    Ok(result)
}

/// The exact inverse of [`set`] adding `key` last: removes it only when it sits
/// exactly where and how `set` inserts it. Otherwise the key stays.
fn unset_inserted(text: &str, key: &str) -> Option<String> {
    let fields = object(text).ok()?;
    let value = field(&fields, key)?;
    let start = value.as_ptr() as usize - text.as_ptr() as usize;
    let end = start + value.len();
    let last = text.rfind('}')?;
    let head = format!("\n  {}: ", serde_json::to_string(key).ok()?);
    if !text[..start].ends_with(&head) || text.get(end..last) != Some("\n") {
        return None;
    }
    let mut cut = start - head.len();
    if text[..cut].ends_with(',') {
        cut -= 1;
    } else if fields.0.len() != 1 {
        return None;
    }
    Some(format!("{}{}", &text[..cut], &text[last..]))
}

#[cfg(test)]
mod tests {
    #[test]
    fn removal_restores_a_file_setup_only_added_to() {
        let user_hook = r#"{ "hooks" : [{ "type": "command", "command": "user-command" }] }"#;
        let originals = [
            "{}".to_owned(),
            "{\n \"permissions\": {\"allow\": []}\n}\n".to_owned(),
            format!("{{\n \"hooks\": {{\"SessionStart\": [{user_hook}]}}\n}}\n"),
            format!("{{\"hooks\":{{\"SessionStart\":[{user_hook}],\"SessionEnd\":[]}}}}"),
            format!("{{\"hooks\":{{\"UserPromptSubmit\":[{user_hook}]}}}}"),
        ];
        for original in originals {
            for driver in [&claude::DRIVER, &codex::DRIVER] {
                let installed = settings(
                    driver,
                    &original,
                    "/stable/tmt",
                    false,
                    crate::setup::UsageHook::Keep,
                )
                .unwrap();
                assert_ne!(installed, original);
                assert_eq!(
                    removed(driver, &installed).unwrap().as_deref(),
                    Some(original.as_str()),
                    "{original}"
                );
                assert_eq!(removed(driver, &original).unwrap(), None);
            }
        }
    }

    #[test]
    fn the_usage_hook_is_opt_in_kept_on_rerun_and_removed_exactly() {
        use crate::setup::UsageHook::{Install, Keep};
        let user_hook = r#"{ "hooks" : [{ "type": "command", "command": "user-command" }] }"#;
        let originals = [
            "{}".to_owned(),
            format!("{{\n \"hooks\": {{\"SessionStart\": [{user_hook}]}}\n}}\n"),
            format!("{{\"hooks\":{{\"Stop\":[{user_hook}]}}}}"),
        ];
        for original in originals {
            for driver in [&claude::DRIVER, &codex::DRIVER] {
                let lifecycle = settings(driver, &original, "/stable/tmt", false, Keep).unwrap();
                assert!(!has_owned_hook(driver, &lifecycle, USAGE), "off by default");
                let usage = settings(driver, &original, "/stable/tmt", false, Install).unwrap();
                assert!(has_owned_hook(driver, &usage, USAGE));
                assert!(has_owned_hook(driver, &usage, "SessionStart"));
                // A re-run without a choice keeps it, and refreshes its launcher.
                assert_eq!(
                    settings(driver, &usage, "/stable/tmt", false, Keep).unwrap(),
                    usage
                );
                let moved = settings(driver, &usage, "/new/tmt", false, Keep).unwrap();
                assert!(has_owned_hook(driver, &moved, USAGE));
                assert!(moved.contains("/new/tmt") && !moved.contains("/stable/tmt"));
                // --no-usage restores the lifecycle-only bytes; --remove all.
                assert_eq!(
                    usage_removed(driver, &usage, "/stable/tmt").unwrap(),
                    lifecycle
                );
                assert_eq!(
                    usage_removed(driver, &lifecycle, "/stable/tmt").unwrap(),
                    lifecycle
                );
                assert_eq!(
                    removed(driver, &usage).unwrap().as_deref(),
                    Some(original.as_str()),
                    "{original}"
                );
            }
        }
    }

    #[test]
    fn without_a_usage_choice_setup_never_reads_stop_entries() {
        use crate::setup::UsageHook::{Install, Keep};
        // A user's own edit of a TMT Stop hook fails only an explicit choice.
        let edited = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"'/x/tmt' __hook claude --mine"}]}]}}"#;
        let planned = settings(&claude::DRIVER, edited, "/stable/tmt", false, Keep).unwrap();
        assert!(planned.contains("--mine"));
        assert_eq!(
            settings(&claude::DRIVER, edited, "/stable/tmt", false, Install),
            Err(PlanError::EditedHook)
        );
    }

    #[test]
    fn removal_keeps_keys_it_did_not_insert() {
        let installed = settings(
            &claude::DRIVER,
            "{}",
            "/stable/tmt",
            false,
            crate::setup::UsageHook::Keep,
        )
        .unwrap();
        // A user who reformatted the file keeps empty keys; nothing else moves.
        let reformatted = installed.replace("\n  ", "\n    ");
        let after = removed(&claude::DRIVER, &reformatted).unwrap().unwrap();
        assert!(!after.contains("__hook"));
        assert!(after.contains("\"SessionStart\""));
        let edited = installed.replace("__hook claude", "__hook claude --edited");
        assert!(removed(&claude::DRIVER, &edited).is_err());
    }

    use super::*;
    use crate::drivers::{claude, codex};

    fn hook_entry(launcher: &str) -> serde_json::Value {
        super::super::hook_entry(&claude::DRIVER, launcher).unwrap()
    }

    fn claude_settings(text: &str, launcher: &str, removing: bool) -> Result<String, PlanError> {
        settings(
            &claude::DRIVER,
            text,
            launcher,
            removing,
            crate::setup::UsageHook::Keep,
        )
    }

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

    #[test]
    fn async_activity_hooks_are_edited_owned_entries() {
        for event in ["UserPromptSubmit", "Stop"] {
            let mut entry = hook_entry("/tmt");
            entry["hooks"][0]["async"] = true.into();
            let input = serde_json::json!({"hooks":{event:[entry]}}).to_string();
            assert!(!has_owned_hook(&claude::DRIVER, &input, event));
            assert_eq!(
                settings(
                    &claude::DRIVER,
                    &input,
                    "/tmt",
                    false,
                    super::super::UsageHook::Install
                ),
                Err(PlanError::EditedHook)
            );
        }
    }

    #[test]
    fn installed_start_hooks_are_recognized_only_in_their_owned_shape() {
        let installed = claude_settings("{}", "/stable/tmt", false).unwrap();
        assert!(has_owned_hook(&claude::DRIVER, &installed, "SessionStart"));
        assert!(
            !has_owned_hook(&codex::DRIVER, &installed, "SessionStart"),
            "another provider's hook"
        );
        let removed = claude_settings(&installed, "/stable/tmt", true).unwrap();
        assert!(!has_owned_hook(&claude::DRIVER, &removed, "SessionStart"));
        for text in [
            "",
            "not json",
            "{}",
            r#"{"hooks":{}}"#,
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"user-hook"}]}]}}"#,
        ] {
            assert!(
                !has_owned_hook(&claude::DRIVER, text, "SessionStart"),
                "{text}"
            );
        }
        let edited = installed.replace("__hook claude", "__hook claude --edited");
        assert!(
            !has_owned_hook(&claude::DRIVER, &edited, "SessionStart"),
            "edited hooks count as absent"
        );
    }
}
