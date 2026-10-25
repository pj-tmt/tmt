use super::{RefreshStatus, refresh_recorded};
use crate::{drivers::Registry, setup, test_support::TestDirectory};
use std::{fs, os::unix::fs::symlink};

fn installation(
    root: &TestDirectory,
    provider: &str,
    usage: Option<bool>,
) -> setup::record::RecordedHooks {
    let entry = setup::record::RecordedHooks {
        driver: provider.into(),
        settings: root.path.join(provider).join("settings.json"),
        launcher: "/private/stable/bin/tmt".into(),
        usage,
    };
    let driver = Registry::builtin().find(provider).unwrap();
    let plan = setup::plan(
        driver,
        entry.settings.clone(),
        None,
        entry.launcher.clone(),
        false,
        match usage {
            Some(true) => setup::UsageHook::Install,
            Some(false) => setup::UsageHook::Remove,
            None => setup::UsageHook::Keep,
        },
    )
    .unwrap();
    setup::apply(&plan).unwrap();
    setup::record::remember(&root.path, entry.clone()).unwrap();
    entry
}

#[test]
fn stale_recorded_hooks_refresh_once_and_preserve_user_bytes_and_usage_choice() {
    let root = TestDirectory::new();
    let entry = installation(&root, "claude", Some(false));
    let text = fs::read_to_string(&entry.settings).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["hooks"]
        .as_object_mut()
        .unwrap()
        .remove("PostToolUseFailure");
    let text = format!(
        "{{\n  \"user\" : {{\"opaque\":1e999, \"text\":\"keep me\"}},\n  \"hooks\":{}\n}}",
        value["hooks"]
    );
    fs::write(&entry.settings, text).unwrap();
    let report = refresh_recorded(&root.path);
    assert_eq!(report.integrations.len(), 1);
    assert_eq!(report.integrations[0].status, RefreshStatus::Updated);
    let after = fs::read_to_string(&entry.settings).unwrap();
    assert!(after.contains("\"user\" : {\"opaque\":1e999, \"text\":\"keep me\"}"));
    assert!(after.contains("PostToolUseFailure"));
    assert_eq!(setup::record::read(&root.path).unwrap(), [entry]);
    assert_eq!(
        refresh_recorded(&root.path).integrations[0].status,
        RefreshStatus::Unchanged
    );
    assert_eq!(
        fs::read_to_string(root.path.join("claude/settings.json")).unwrap(),
        after
    );
}

#[test]
fn symlink_and_user_edited_hooks_are_preserved_without_blocking_other_providers() {
    let root = TestDirectory::new();
    let claude = installation(&root, "claude", Some(true));
    let codex = installation(&root, "codex", Some(false));
    let target = root.path.join("untouched.json");
    fs::write(&target, "user bytes").unwrap();
    fs::remove_file(&claude.settings).unwrap();
    symlink(&target, &claude.settings).unwrap();
    let before = fs::read_to_string(&codex.settings)
        .unwrap()
        .replace("\"timeout\":3", "\"timeout\":30");
    fs::write(&codex.settings, &before).unwrap();
    let report = refresh_recorded(&root.path);
    assert_eq!(report.integrations[0].status, RefreshStatus::Skipped);
    assert_eq!(report.integrations[1].status, RefreshStatus::NeedsApproval);
    assert_eq!(
        report.integrations[1].next_step.as_deref(),
        Some("tmt setup codex")
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "user bytes");
    assert_eq!(fs::read_to_string(&codex.settings).unwrap(), before);
    assert!(fs::symlink_metadata(&claude.settings).unwrap().is_symlink());
}

#[test]
fn no_record_or_missing_settings_never_installs_a_provider() {
    let root = TestDirectory::new();
    assert!(refresh_recorded(&root.path).integrations.is_empty());
    assert!(!root.path.join("setup-record.json").exists());
    let entry = installation(&root, "claude", None);
    fs::remove_file(&entry.settings).unwrap();
    let report = refresh_recorded(&root.path);
    assert_eq!(report.integrations[0].status, RefreshStatus::NeedsApproval);
    assert!(!entry.settings.exists());
    assert!(!root.path.join("codex").exists());
}

#[test]
fn legacy_usage_stays_conservative_and_invalid_record_writes_nothing() {
    let root = TestDirectory::new();
    let entry = installation(&root, "claude", None);
    assert_eq!(
        refresh_recorded(&root.path).integrations[0].status,
        RefreshStatus::Updated
    );
    assert_eq!(
        setup::record::read(&root.path).unwrap()[0].usage,
        Some(false)
    );
    assert!(
        fs::read_to_string(&entry.settings)
            .unwrap()
            .contains("\"Stop\"")
    );
    let before = fs::read(&entry.settings).unwrap();
    fs::write(setup::record::path(&root.path), "bad record").unwrap();
    assert!(refresh_recorded(&root.path).error.is_some());
    assert_eq!(fs::read(&entry.settings).unwrap(), before);
    assert_eq!(
        fs::read_to_string(setup::record::path(&root.path)).unwrap(),
        "bad record"
    );
}

#[test]
fn a_changed_consent_record_refuses_before_the_settings_action() {
    let root = TestDirectory::new();
    let mut entry = installation(&root, "claude", Some(false));
    let captured = entry.clone();
    entry.usage = Some(true);
    setup::record::remember(&root.path, entry.clone()).unwrap();
    let before = fs::read(&entry.settings).unwrap();
    let called = std::cell::Cell::new(false);
    assert!(
        setup::record::refresh(&root.path, &captured, false, || {
            called.set(true);
            Ok(())
        })
        .is_err()
    );
    assert!(!called.get());
    assert_eq!(fs::read(&entry.settings).unwrap(), before);
    assert_eq!(setup::record::read(&root.path).unwrap(), [entry]);
}

#[test]
fn duplicated_hooks_refuse_while_a_later_recorded_provider_is_refreshed() {
    let root = TestDirectory::new();
    let claude = installation(&root, "claude", Some(true));
    let agy = installation(&root, "agy", Some(false));
    let mut edited: serde_json::Value =
        serde_json::from_slice(&fs::read(&claude.settings).unwrap()).unwrap();
    let starts = edited["hooks"]["SessionStart"].as_array_mut().unwrap();
    starts.push(starts[0].clone());
    let before = serde_json::to_vec(&edited).unwrap();
    fs::write(&claude.settings, &before).unwrap();
    let mut stale: serde_json::Value =
        serde_json::from_slice(&fs::read(&agy.settings).unwrap()).unwrap();
    stale["tmt"]
        .as_object_mut()
        .unwrap()
        .remove("PreInvocation");
    fs::write(&agy.settings, serde_json::to_vec(&stale).unwrap()).unwrap();
    let report = refresh_recorded(&root.path);
    assert_eq!(
        report
            .integrations
            .iter()
            .find(|row| row.provider == "claude")
            .unwrap()
            .status,
        RefreshStatus::NeedsApproval
    );
    assert_eq!(
        report
            .integrations
            .iter()
            .find(|row| row.provider == "agy")
            .unwrap()
            .status,
        RefreshStatus::Updated
    );
    assert_eq!(fs::read(&claude.settings).unwrap(), before);
    assert!(
        fs::read_to_string(&agy.settings)
            .unwrap()
            .contains("PreInvocation")
    );
}

#[test]
fn existing_unrecorded_provider_files_are_not_inspected_or_rewritten() {
    let root = TestDirectory::new();
    let path = root.path.join("codex/hooks.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "unrecorded user bytes").unwrap();
    installation(&root, "claude", Some(false));
    assert_eq!(refresh_recorded(&root.path).integrations.len(), 1);
    assert_eq!(fs::read_to_string(path).unwrap(), "unrecorded user bytes");
}
