//! The setup record: which provider settings files hold TMT's lifecycle hooks,
//! and for which launcher. Uninstall reverses exactly this. A hook entry is
//! fully determined by its driver and launcher (`super::hook_entry`), so the
//! record stores those, not the JSON. Hooks written before the record existed
//! are adopted when they exactly match what setup generates.

use crate::{bounded_file, skill_installation::files};
use serde_json::{Value, json};
use std::{
    io,
    path::{Path, PathBuf},
};

const MAXIMUM_BYTES: usize = 65_536;
const MAXIMUM_HOOKS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RecordedHooks {
    pub driver: String,
    /// The provider settings file the hooks are in.
    pub settings: PathBuf,
    /// The stable launcher the hook commands run.
    pub launcher: PathBuf,
    /// None is a legacy record, which did not retain explicit usage choices.
    pub usage: Option<bool>,
}

/// Resolve the default from the same installation record used by uninstall.
/// Legacy lifecycle-only records cannot distinguish an old opt-out, so keep
/// them disabled until the user explicitly enables collection.
pub fn usage_policy(
    entries: &[RecordedHooks],
    driver: &str,
    settings: &Path,
    requested: super::UsageHook,
) -> super::UsageHook {
    use super::UsageHook;
    if requested != UsageHook::Default {
        return requested;
    }
    match entries
        .iter()
        .find(|entry| entry.driver == driver && entry.settings == settings)
    {
        Some(entry) if entry.usage == Some(false) => UsageHook::Remove,
        Some(entry) if entry.usage.is_none() => UsageHook::Keep,
        _ => UsageHook::Install,
    }
}

pub fn path(global: &Path) -> PathBuf {
    global.join("setup-record.json")
}

fn invalid(file: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "The setup record is invalid; it was preserved for inspection: {}",
            file.display()
        ),
    )
}

/// Every recorded hook installation, sorted. A missing record is empty; an
/// unreadable or invalid one is an error and is never overwritten.
pub fn read(global: &Path) -> io::Result<Vec<RecordedHooks>> {
    let file = path(global);
    if !files::exists(&file)? {
        return Ok(Vec::new());
    }
    if !std::fs::symlink_metadata(&file)?.is_file() {
        return Err(invalid(&file));
    }
    let bytes = bounded_file::read(&file, MAXIMUM_BYTES).map_err(|_| invalid(&file))?;
    let document: Value = serde_json::from_slice(&bytes).map_err(|_| invalid(&file))?;
    if document.as_object().is_none_or(|object| object.len() != 2) || document["version"] != 1 {
        return Err(invalid(&file));
    }
    let entries = document["hooks"].as_array().ok_or_else(|| invalid(&file))?;
    if entries.len() > MAXIMUM_HOOKS {
        return Err(invalid(&file));
    }
    let mut hooks = entries
        .iter()
        .map(|entry| {
            let fields = entry.as_object().filter(|fields| {
                fields.len() == 3 || (fields.len() == 4 && fields.contains_key("usage"))
            });
            let text = |key: &str| {
                fields
                    .and_then(|fields| fields.get(key))
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid(&file))
            };
            let absolute = |key: &str| {
                let value = PathBuf::from(text(key)?);
                if value.is_absolute() {
                    Ok(value)
                } else {
                    Err(invalid(&file))
                }
            };
            Ok(RecordedHooks {
                driver: text("driver")?.to_owned(),
                settings: absolute("settings")?,
                launcher: absolute("launcher")?,
                usage: entry
                    .get("usage")
                    .map(|value| value.as_bool().ok_or_else(|| invalid(&file)))
                    .transpose()?,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    hooks.sort();
    hooks.dedup();
    Ok(hooks)
}

/// Record `hooks`, replacing any entry for the same driver and settings file.
/// Recording what is already recorded leaves the file untouched.
pub fn remember(global: &Path, hooks: RecordedHooks) -> io::Result<()> {
    update(global, |entries| {
        entries
            .retain(|entry| (&entry.driver, &entry.settings) != (&hooks.driver, &hooks.settings));
        entries.push(hooks);
    })
}

/// Drop the entry for this driver and settings file, if any.
pub fn forget(global: &Path, driver: &str, settings: &Path) -> io::Result<()> {
    update(global, |entries| {
        entries.retain(|entry| {
            (entry.driver.as_str(), entry.settings.as_path()) != (driver, settings)
        });
    })
}

fn update(global: &Path, change: impl FnOnce(&mut Vec<RecordedHooks>)) -> io::Result<()> {
    std::fs::create_dir_all(global)?;
    let _lock = crate::file_lock::exclusive(&global.join("setup-record.lock"))?;
    let before = read(global)?;
    let mut entries = before.clone();
    change(&mut entries);
    entries.sort();
    entries.dedup();
    if entries == before {
        return Ok(());
    }
    let file = path(global);
    if entries.len() > MAXIMUM_HOOKS {
        return Err(invalid(&file));
    }
    let hooks = entries
        .iter()
        .map(|entry| {
            let mut document = json!({
                "driver": entry.driver,
                "settings": entry.settings.to_str().ok_or_else(|| invalid(&file))?,
                "launcher": entry.launcher.to_str().ok_or_else(|| invalid(&file))?,
            });
            if let Some(usage) = entry.usage {
                document["usage"] = json!(usage);
            }
            Ok(document)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let bytes = serde_json::to_vec_pretty(&json!({"version": 1, "hooks": hooks}))
        .map_err(io::Error::other)?;
    files::atomic_write(&file, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use std::fs;

    fn hooks(driver: &str, settings: &str) -> RecordedHooks {
        RecordedHooks {
            driver: driver.into(),
            settings: settings.into(),
            launcher: "/stable/bin/tmt".into(),
            usage: None,
        }
    }

    #[test]
    fn remembering_is_idempotent_and_forgetting_removes_one_entry() {
        let root = TestDirectory::new();
        assert!(read(&root.path).unwrap().is_empty());
        remember(&root.path, hooks("claude", "/home/.claude/settings.json")).unwrap();
        remember(&root.path, hooks("codex", "/home/.codex/hooks.json")).unwrap();
        let bytes = fs::read(path(&root.path)).unwrap();
        let modified = fs::metadata(path(&root.path)).unwrap().modified().unwrap();
        remember(&root.path, hooks("claude", "/home/.claude/settings.json")).unwrap();
        assert_eq!(fs::read(path(&root.path)).unwrap(), bytes);
        assert_eq!(
            fs::metadata(path(&root.path)).unwrap().modified().unwrap(),
            modified
        );
        let moved = RecordedHooks {
            launcher: "/new/bin/tmt".into(),
            ..hooks("claude", "/home/.claude/settings.json")
        };
        remember(&root.path, moved.clone()).unwrap();
        assert_eq!(
            read(&root.path).unwrap(),
            [moved, hooks("codex", "/home/.codex/hooks.json")]
        );
        forget(
            &root.path,
            "claude",
            Path::new("/home/.claude/settings.json"),
        )
        .unwrap();
        assert_eq!(
            read(&root.path).unwrap(),
            [hooks("codex", "/home/.codex/hooks.json")]
        );
    }

    #[test]
    fn usage_choice_is_scoped_persisted_and_explicitly_overridden() {
        use crate::setup::UsageHook::{Default, Install, Keep, Remove};
        let root = TestDirectory::new();
        let mut entry = hooks("claude", "/home/.claude/settings.json");
        assert_eq!(
            usage_policy(&[], &entry.driver, &entry.settings, Default),
            Install
        );
        assert_eq!(
            usage_policy(&[entry.clone()], &entry.driver, &entry.settings, Default),
            Keep
        );
        entry.usage = Some(false);
        remember(&root.path, entry.clone()).unwrap();
        let entries = read(&root.path).unwrap();
        assert_eq!(
            usage_policy(&entries, &entry.driver, &entry.settings, Default),
            Remove
        );
        assert_eq!(
            usage_policy(&entries, &entry.driver, &entry.settings, Install),
            Install
        );
        assert_eq!(
            usage_policy(&entries, "codex", &entry.settings, Default),
            Install
        );
        assert_eq!(
            usage_policy(&entries, &entry.driver, Path::new("/other"), Default),
            Install
        );
        entry.usage = Some(true);
        remember(&root.path, entry.clone()).unwrap();
        assert_eq!(
            usage_policy(
                &read(&root.path).unwrap(),
                &entry.driver,
                &entry.settings,
                Default
            ),
            Install
        );
    }

    #[test]
    fn an_invalid_record_fails_closed_and_is_preserved() {
        let root = TestDirectory::new();
        for document in [
            "not json",
            r#"{"version":1,"hooks":[{"driver":"claude","settings":"/s","launcher":"/t","usage":null}]}"#,
            r#"{"version":2,"hooks":[]}"#,
            r#"{"version":1,"hooks":[{"driver":"claude","settings":"relative","launcher":"/t"}]}"#,
            r#"{"version":1,"hooks":[{"driver":"claude","settings":"/s"}]}"#,
        ] {
            fs::write(path(&root.path), document).unwrap();
            assert!(read(&root.path).is_err(), "{document}");
            assert!(remember(&root.path, hooks("claude", "/s")).is_err());
            assert_eq!(fs::read_to_string(path(&root.path)).unwrap(), document);
        }
    }
}
