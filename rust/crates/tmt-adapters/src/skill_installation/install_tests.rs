use super::{
    InstallFailure, ProviderEnvironment, bundled_skill, bundled_skill_named, install,
    install_office,
};
use crate::test_support::TestDirectory;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn fixture() -> (TestDirectory, ProviderEnvironment, PathBuf, PathBuf) {
    let directory = TestDirectory::new();
    let home = directory.path.join("home");
    let cwd = directory.path.join("cwd");
    let global = directory.path.join("global");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&cwd).unwrap();
    let home_for_return = home.clone();
    (
        directory,
        ProviderEnvironment::from_parts(home, cwd, Vec::new(), []),
        global,
        home_for_return,
    )
}

fn assert_link(path: &Path) -> PathBuf {
    assert!(fs::symlink_metadata(path).unwrap().is_symlink());
    fs::read_link(path).unwrap()
}

fn assert_failure_preserves(failure: &InstallFailure, expected: &str) {
    assert!(failure.to_string().contains(expected), "{failure}");
    assert!(failure.report.installed.is_empty());
    assert!(failure.pending_backup.is_none());
}

fn office_target(target: &Path) -> PathBuf {
    target.parent().unwrap().join("tmt-office")
}

fn prop_create_target(target: &Path) -> PathBuf {
    target.parent().unwrap().join("tmt-prop-create")
}

fn avatar_create_target(target: &Path) -> PathBuf {
    target.parent().unwrap().join("tmt-avatar-create")
}

#[test]
fn neutral_install_is_exact_repeat_noop_and_records_one_target() {
    let (_directory, environment, global, home) = fixture();
    let first = install(&environment, &global, None, None, false).unwrap();
    assert_eq!(first.installed.len(), 2);
    assert_eq!(first.installed[0].agent, None);
    assert!(first.installed[0].changed);
    let universal = home.join(".agents/skills/tmt");
    let inbox = home.join(".agents/skills/tmt-inbox");
    let source = assert_link(&universal);
    assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), bundled_skill());
    assert_eq!(
        fs::read(assert_link(&inbox).join("SKILL.md")).unwrap(),
        super::bundled_skill_named("tmt-inbox").unwrap()
    );

    let registry = fs::read(global.join("skill-installations.json")).unwrap();
    let second = install(&environment, &global, None, None, false).unwrap();
    assert_eq!(second.installed.len(), 2);
    assert!(!second.installed[0].changed);
    assert_eq!(second.installed[0].backup, None);
    assert_eq!(assert_link(&universal), source);
    assert_eq!(
        fs::read(global.join("skill-installations.json")).unwrap(),
        registry
    );
    assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), bundled_skill());
}

#[test]
fn custom_install_uses_exact_target_and_does_not_invent_provider() {
    let (directory, environment, global, _home) = fixture();
    let custom_root = PathBuf::from("custom skills");
    let report = install(&environment, &global, None, Some(&custom_root), false).unwrap();
    assert_eq!(report.installed.len(), 2);
    assert_eq!(report.installed[0].agent, None);
    assert!(report.installed[0].changed);
    let custom = directory.path.join("cwd/custom skills/tmt");
    assert_eq!(report.installed[0].target, custom);
    let source = assert_link(&custom);
    assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), bundled_skill());
    assert_eq!(fs::read_dir(custom.parent().unwrap()).unwrap().count(), 2);
}

#[test]
fn office_install_adds_optional_guidance_to_detected_and_managed_custom_roots() {
    let (directory, environment, global, home) = fixture();
    let custom_root = PathBuf::from("custom skills");
    install(&environment, &global, None, Some(&custom_root), false).unwrap();
    let custom_core = directory.path.join("cwd/custom skills/tmt");
    assert!(!office_target(&custom_core).exists());
    assert!(!home.join(".agents/skills/tmt").exists());

    let first = install_office(&environment, &global, false).unwrap();
    let office_targets = first
        .installed
        .iter()
        .map(|item| item.target.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        office_targets,
        [
            home.join(".agents/skills/tmt-office"),
            home.join(".agents/skills/tmt-prop-create"),
            home.join(".agents/skills/tmt-avatar-create"),
            office_target(&custom_core),
            prop_create_target(&custom_core),
            avatar_create_target(&custom_core),
        ]
        .into_iter()
        .collect()
    );
    assert!(first.installed.iter().all(|item| {
        matches!(
            item.name,
            "tmt-office" | "tmt-prop-create" | "tmt-avatar-create"
        ) && item.changed
            && item.backup.is_none()
    }));
    for target in office_targets {
        let name = target.file_name().unwrap().to_str().unwrap();
        let installed = fs::read(assert_link(&target).join("SKILL.md")).unwrap();
        assert_eq!(installed, bundled_skill_named(name).unwrap());
        let guidance = String::from_utf8(installed).unwrap();
        if name == "tmt-prop-create" {
            assert!(!guidance.contains("extensions/tmt-office/contracts/"));
            for required in [
                "\"formatVersion\": 1",
                "tmt office prop validate --file",
                "tmt office prop ls --local",
                "tmt office prop install --local",
                "tmt office layout show",
                "tmt office layout apply",
                "tmt office prop rm --local",
                "`.layout`",
            ] {
                assert!(
                    guidance.contains(required),
                    "missing creator guidance: {required}"
                );
            }
        } else if name == "tmt-avatar-create" {
            assert!(!guidance.contains("extensions/tmt-office/contracts/"));
            for required in [
                "`formatVersion: 1` for 16×24 pixels",
                "`2` for 32×48 pixels",
                "V2 rows contain exactly 64 characters",
                "tmt office avatar validate --file",
                "tmt office avatar ls --local",
                "tmt office avatar install --local",
                "tmt office profile show --local",
                "tmt office profile apply --local",
                "tmt office avatar rm --local",
                "maintained `Avatar` composition",
            ] {
                assert!(
                    guidance.contains(required),
                    "missing avatar creator guidance: {required}"
                );
            }
        } else {
            for required in [
                "`tmt-prop-create`",
                "`tmt-avatar-create`",
                "tmt office prop ls --local",
                "tmt office prop install --local",
                "tmt office prop rm --local",
                "`.layout`",
                "{\"version\":1,\"map\":{...},\"objects\":[...]}",
                "tmt office layout show",
                "tmt office layout apply",
            ] {
                assert!(
                    guidance.contains(required),
                    "missing Office guidance: {required}"
                );
            }
        }
    }
    assert!(custom_core.exists());
    assert!(!home.join(".agents/skills/tmt").exists());

    let second = install_office(&environment, &global, false).unwrap();
    assert_eq!(second.installed.len(), 6);
    assert!(second.installed.iter().all(|item| !item.changed));
}

#[test]
fn office_install_replaces_an_existing_published_name_without_force() {
    let (_directory, environment, global, home) = fixture();
    let target = home.join(".agents/skills/tmt-avatar-create");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"old copy").unwrap();
    let report = install_office(&environment, &global, false).unwrap();
    assert_eq!(report.installed.len(), 3);
    assert!(report.installed.iter().all(|item| item.backup.is_none()));
    assert_eq!(
        fs::read(target.join("SKILL.md")).unwrap(),
        bundled_skill_named("tmt-avatar-create").unwrap()
    );
}

#[test]
fn office_install_uses_the_detected_provider_root() {
    let (directory, _environment, global, home) = fixture();
    fs::create_dir(home.join(".claude")).unwrap();
    let environment =
        ProviderEnvironment::from_parts(home.clone(), directory.path.join("cwd"), Vec::new(), []);

    let report = install_office(&environment, &global, false).unwrap();
    assert_eq!(report.installed.len(), 3);
    assert!(
        report
            .installed
            .iter()
            .all(|item| item.agent == Some(&crate::drivers::claude::DRIVER))
    );
    let targets = report
        .installed
        .iter()
        .map(|item| item.target.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        targets,
        [
            home.join(".claude/skills/tmt-avatar-create"),
            home.join(".claude/skills/tmt-office"),
            home.join(".claude/skills/tmt-prop-create"),
        ]
        .into_iter()
        .collect()
    );
    assert!(!home.join(".agents/skills/tmt-office").exists());
}

#[test]
fn all_install_deduplicates_shared_targets_but_reports_stable_provider_order() {
    let (_directory, environment, global, home) = fixture();
    let report = install(&environment, &global, Some("all"), None, false).unwrap();
    assert_eq!(
        report
            .installed
            .iter()
            .map(|item| item.agent.unwrap().name())
            .collect::<Vec<_>>(),
        vec![
            "claude", "claude", "codex", "codex", "gemini", "gemini", "agy", "agy", "pi", "pi",
            "opencode", "opencode"
        ]
    );
    assert_eq!(
        report.installed.iter().filter(|item| item.changed).count(),
        8
    );
    for provider in crate::drivers::Registry::builtin().iter() {
        assert!(
            fs::symlink_metadata(environment.target(provider))
                .unwrap()
                .is_symlink()
        );
    }
    let registry: serde_json::Value =
        serde_json::from_slice(&fs::read(global.join("skill-installations.json")).unwrap())
            .unwrap();
    assert_eq!(registry["version"], 1);
    let actual = registry["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| PathBuf::from(value.as_str().unwrap()))
        .collect::<BTreeSet<_>>();
    assert_eq!(registry["targets"].as_array().unwrap().len(), 8);
    let expected = [
        home.join(".claude/skills/tmt"),
        home.join(".agents/skills/tmt"),
        home.join(".gemini/config/skills/tmt"),
        home.join(".pi/agent/skills/tmt"),
        home.join(".claude/skills/tmt-inbox"),
        home.join(".agents/skills/tmt-inbox"),
        home.join(".gemini/config/skills/tmt-inbox"),
        home.join(".pi/agent/skills/tmt-inbox"),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn existing_file_directory_broken_and_foreign_links_are_replaced_without_force() {
    for kind in 0..4 {
        let (_directory, environment, global, home) = fixture();
        let target = home.join(".claude/skills/tmt");
        let personal = home.join(".claude/skills/personal");
        fs::create_dir_all(&personal).unwrap();
        fs::write(personal.join("SKILL.md"), "personal").unwrap();
        match kind {
            0 => fs::write(&target, b"old file").unwrap(),
            1 => {
                fs::create_dir(&target).unwrap();
                std::os::unix::fs::symlink(&personal, target.join("nested")).unwrap();
            }
            2 => std::os::unix::fs::symlink(home.join("missing"), &target).unwrap(),
            _ => std::os::unix::fs::symlink(&personal, &target).unwrap(),
        }
        let report = install(&environment, &global, Some("claude"), None, false).unwrap();
        assert!(report.installed[0].changed);
        assert!(report.installed.iter().all(|item| item.backup.is_none()));
        assert_eq!(fs::read(target.join("SKILL.md")).unwrap(), bundled_skill());
        assert_eq!(fs::read(personal.join("SKILL.md")).unwrap(), b"personal");
        assert!(!home.join(".claude/.tmt-skill-backups").exists());
    }
}

#[test]
fn claude_legacy_is_warned_then_force_backed_up_without_touching_content() {
    let (_directory, environment, global, home) = fixture();
    let legacy = home.join(".claude/commands/team.md");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, b"legacy command").unwrap();
    let warning = install(&environment, &global, Some("claude"), None, false).unwrap();
    assert_eq!(fs::read(&legacy).unwrap(), b"legacy command");
    assert_eq!(warning.warnings.len(), 1);
    assert!(warning.warnings[0].contains("--force"));
    let forced = install(&environment, &global, Some("claude"), None, true).unwrap();
    let backup = forced.installed[0].legacy_backups.first().unwrap();
    assert_eq!(fs::read(backup).unwrap(), b"legacy command");
    assert!(backup.starts_with(home.join(".claude/.tmt-skill-backups")));
    assert!(!backup.starts_with(home.join(".claude/commands")));
    assert!(!legacy.exists());
}

#[test]
fn overlap_guards_run_before_materialization_or_target_backup() {
    let (_directory, environment, global, home) = fixture();
    let source_root = global.join("skill-assets");
    let target = home.join(".claude/skills/tmt");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("user.md"), b"preserve").unwrap();
    let exact_root = source_root.clone();
    let exact = PathBuf::from("../global/skill-assets");
    let failure = install(&environment, &global, None, Some(&exact), true).unwrap_err();
    assert_failure_preserves(&failure, "overlaps bundled source");
    assert_eq!(fs::read(target.join("user.md")).unwrap(), b"preserve");
    assert!(!exact_root.exists());
}

#[test]
fn invalid_or_oversized_registry_is_preserved_and_prevents_materialization() {
    for bytes in [
        br#"{"version":2,"targets":[]}"#.to_vec(),
        vec![b'{'; 1_048_577],
    ] {
        let (_directory, environment, global, home) = fixture();
        fs::create_dir_all(&global).unwrap();
        let manifest = global.join("skill-installations.json");
        fs::write(&manifest, &bytes).unwrap();
        let failure = install(&environment, &global, Some("claude"), None, true).unwrap_err();
        assert_failure_preserves(&failure, "Invalid or oversized skill installation manifest");
        assert_eq!(fs::read(&manifest).unwrap(), bytes);
        assert!(global.join("skill-install.lock").exists());
        assert!(!home.join(".claude/skills/tmt").exists());
        assert!(!global.join("skill-assets").exists());
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bundle_digest(core: &[u8], inbox: &[u8]) -> String {
    let mut framed = Vec::new();
    framed.extend_from_slice(&(core.len() as u64).to_be_bytes());
    framed.extend_from_slice(core);
    framed.extend_from_slice(&(inbox.len() as u64).to_be_bytes());
    framed.extend_from_slice(inbox);
    digest(&framed)
}

fn old_bundle(global: &Path, core: &[u8], inbox: &[u8]) -> (PathBuf, PathBuf) {
    let version = super::files::resolved(global)
        .unwrap()
        .join("skill-assets")
        .join(bundle_digest(core, inbox));
    let main = version.join("tmt");
    let inbox_source = version.join("tmt-inbox");
    fs::create_dir_all(&main).unwrap();
    fs::create_dir(&inbox_source).unwrap();
    fs::write(main.join("SKILL.md"), core).unwrap();
    fs::write(inbox_source.join("SKILL.md"), inbox).unwrap();
    (main, inbox_source)
}

#[test]
fn valid_old_two_skill_bundle_upgrades_both_targets_without_backup() {
    let (_directory, environment, global, home) = fixture();
    let old_core = b"old core skill bytes\n";
    let old_inbox = b"old inbox skill bytes\n";
    let (old_main, old_inbox_source) = old_bundle(&global, old_core, old_inbox);
    let target = home.join(".claude/skills/tmt");
    let inbox_target = home.join(".claude/skills/tmt-inbox");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&old_main, &target).unwrap();
    std::os::unix::fs::symlink(&old_inbox_source, &inbox_target).unwrap();

    let report = install(&environment, &global, Some("claude"), None, false).unwrap();
    assert_eq!(report.installed.len(), 2);
    assert!(report.installed.iter().all(|item| item.changed));
    assert!(report.installed.iter().all(|item| item.backup.is_none()));
    assert_ne!(assert_link(&target), old_main);
    assert_ne!(assert_link(&inbox_target), old_inbox_source);
    assert_eq!(fs::read(target.join("SKILL.md")).unwrap(), bundled_skill());
    assert_eq!(
        fs::read(inbox_target.join("SKILL.md")).unwrap(),
        super::bundled_skill_named("tmt-inbox").unwrap()
    );
    assert_eq!(fs::read(old_main.join("SKILL.md")).unwrap(), old_core);
    assert_eq!(
        fs::read(old_inbox_source.join("SKILL.md")).unwrap(),
        old_inbox
    );
}

#[test]
fn tampered_old_two_skill_bundle_targets_are_replaced_and_sources_preserved() {
    let (_directory, environment, global, home) = fixture();
    let old_core = b"old core skill bytes\n";
    let old_inbox = b"old inbox skill bytes\n";
    let (old_main, old_inbox_source) = old_bundle(&global, old_core, old_inbox);
    fs::write(old_inbox_source.join("SKILL.md"), b"tampered inbox\n").unwrap();
    let target = home.join(".claude/skills/tmt");
    let inbox_target = home.join(".claude/skills/tmt-inbox");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&old_main, &target).unwrap();
    std::os::unix::fs::symlink(&old_inbox_source, &inbox_target).unwrap();

    install(&environment, &global, Some("claude"), None, false).unwrap();
    assert_ne!(assert_link(&target), old_main);
    assert_ne!(assert_link(&inbox_target), old_inbox_source);
    assert_eq!(fs::read(old_main.join("SKILL.md")).unwrap(), old_core);
    assert_eq!(
        fs::read(old_inbox_source.join("SKILL.md")).unwrap(),
        b"tampered inbox\n"
    );
}

#[test]
fn valid_old_digest_source_refreshes_to_current_source_without_backup() {
    let (_directory, environment, global, home) = fixture();
    let old = b"old canonical skill\n";
    let asset_root = super::files::resolved(&global)
        .unwrap()
        .join("skill-assets");
    let old_source = asset_root.join(digest(old)).join("tmt");
    fs::create_dir_all(&old_source).unwrap();
    fs::write(old_source.join("SKILL.md"), old).unwrap();
    let target = home.join(".claude/skills/tmt");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&old_source, &target).unwrap();

    let report = install(&environment, &global, Some("claude"), None, false).unwrap();
    assert!(report.installed[0].changed);
    assert_eq!(report.installed[0].backup, None);
    let current = assert_link(&target);
    assert_ne!(current, old_source);
    assert_eq!(fs::read(current.join("SKILL.md")).unwrap(), bundled_skill());
    assert_eq!(fs::read(old_source.join("SKILL.md")).unwrap(), old);
}

#[test]
fn modified_old_digest_source_is_untouched_when_its_target_is_replaced() {
    let (_directory, environment, global, home) = fixture();
    let original = b"old canonical skill\n";
    let modified = b"tampered old skill\n";
    let old_source = super::files::resolved(&global)
        .unwrap()
        .join("skill-assets")
        .join(digest(original))
        .join("tmt");
    fs::create_dir_all(&old_source).unwrap();
    fs::write(old_source.join("SKILL.md"), modified).unwrap();
    let target = home.join(".claude/skills/tmt");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&old_source, &target).unwrap();

    let report = install(&environment, &global, Some("claude"), None, false).unwrap();
    assert!(report.installed[0].backup.is_none());
    assert_ne!(fs::read_link(&target).unwrap(), old_source);
    assert_eq!(fs::read(old_source.join("SKILL.md")).unwrap(), modified);
    assert_eq!(fs::read(target.join("SKILL.md")).unwrap(), bundled_skill());
}

#[test]
fn modified_current_asset_blocks_force_before_target_backup() {
    let (_directory, env, global, home) = fixture();
    install(&env, &global, Some("claude"), None, false).unwrap();
    let target = home.join(".claude/skills/tmt");
    let source = assert_link(&target);
    fs::write(source.join("SKILL.md"), b"user-edited current source").unwrap();
    let failure = install(&env, &global, Some("claude"), None, true).unwrap_err();
    assert_failure_preserves(&failure, "Managed skill source has been modified");
    assert_eq!(assert_link(&target), source);
    assert_eq!(
        fs::read(source.join("SKILL.md")).unwrap(),
        b"user-edited current source"
    );
    assert!(!home.join(".claude/.tmt-skill-backups").exists());
}
