use super::provider_mod::{self, ModOutcome, ModStep};
use crate::test_support::TestDirectory;
use std::{fs, os::unix::fs::symlink, path::PathBuf};

fn settings(root: &TestDirectory) -> PathBuf {
    root.path.join(".claude/settings.json")
}

fn mod_dir(root: &TestDirectory) -> PathBuf {
    root.path.join(".claude/skills/tmt-allowance")
}

fn install(root: &TestDirectory) {
    let pending = provider_mod::pending("claude", &settings(root), true)
        .unwrap()
        .unwrap();
    assert_eq!(pending.apply().unwrap(), ModOutcome::Installed);
}

#[test]
fn only_a_driver_with_a_mod_has_one() {
    assert!(provider_mod::for_driver("claude").is_some());
    for other in ["codex", "agy", "nobody"] {
        assert!(provider_mod::for_driver(other).is_none(), "{other}");
    }
    let root = TestDirectory::new();
    assert!(
        provider_mod::pending("codex", &root.path.join(".codex/config.toml"), true)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_consented_setup_installs_the_marked_copy_once_and_then_has_nothing_to_do() {
    let root = TestDirectory::new();
    let pending = provider_mod::pending("claude", &settings(&root), true)
        .unwrap()
        .unwrap();
    assert_eq!(pending.step, ModStep::Install);
    assert_eq!(pending.target(), mod_dir(&root));
    // Planning changes nothing.
    assert!(!mod_dir(&root).exists());
    assert_eq!(pending.apply().unwrap(), ModOutcome::Installed);
    for file in [
        ".claude-plugin/plugin.json",
        "hooks/hooks.json",
        "hooks/register.mjs",
        ".tmt-managed.json",
    ] {
        assert!(mod_dir(&root).join(file).is_file(), "{file}");
    }
    let register = fs::read_to_string(mod_dir(&root).join("hooks/register.mjs")).unwrap();
    assert!(register.contains("__allowance"));
    assert!(
        provider_mod::pending("claude", &settings(&root), true)
            .unwrap()
            .is_none()
    );
    // No staging leftovers beside it.
    let names: Vec<_> = fs::read_dir(root.path.join(".claude/skills"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["tmt-allowance"]);
}

#[test]
fn a_changed_or_older_copy_is_refreshed_and_an_upgrade_never_creates_a_missing_one() {
    let root = TestDirectory::new();
    assert!(
        provider_mod::pending_refresh("claude", &settings(&root), true)
            .unwrap()
            .is_none()
    );
    install(&root);
    let register = mod_dir(&root).join("hooks/register.mjs");
    let shipped = fs::read(&register).unwrap();
    fs::write(&register, b"// edited").unwrap();
    let refresh = provider_mod::pending_refresh("claude", &settings(&root), true)
        .unwrap()
        .unwrap();
    assert_eq!(refresh.step, ModStep::Refresh);
    assert_eq!(refresh.apply().unwrap(), ModOutcome::Refreshed);
    assert_eq!(fs::read(&register).unwrap(), shipped);
    assert!(
        provider_mod::pending_refresh("claude", &settings(&root), true)
            .unwrap()
            .is_none()
    );
}

#[test]
fn turning_usage_off_or_uninstalling_removes_only_the_marked_copy() {
    let root = TestDirectory::new();
    install(&root);
    let off = provider_mod::pending("claude", &settings(&root), false)
        .unwrap()
        .unwrap();
    assert_eq!(off.step, ModStep::Remove);
    assert_eq!(off.apply().unwrap(), ModOutcome::Removed);
    assert!(!mod_dir(&root).exists());
    assert!(
        provider_mod::pending_removal("claude", &settings(&root))
            .unwrap()
            .is_none()
    );

    install(&root);
    let uninstall = provider_mod::pending_removal("claude", &settings(&root))
        .unwrap()
        .unwrap();
    assert_eq!(uninstall.apply().unwrap(), ModOutcome::Removed);
    assert!(!mod_dir(&root).exists());
}

#[test]
fn a_directory_that_is_not_tmts_is_blocked_and_never_changed_or_removed() {
    let root = TestDirectory::new();
    let foreign = mod_dir(&root);
    fs::create_dir_all(foreign.join("hooks")).unwrap();
    fs::write(foreign.join("hooks/mine.mjs"), b"mine").unwrap();
    let blocked = provider_mod::pending("claude", &settings(&root), true)
        .unwrap()
        .unwrap();
    assert_eq!(blocked.step, ModStep::Blocked);
    assert!(!blocked.changes());
    assert_eq!(blocked.apply().unwrap(), ModOutcome::Blocked);
    // Neither usage off nor uninstall touches it.
    assert!(
        provider_mod::pending("claude", &settings(&root), false)
            .unwrap()
            .is_none()
    );
    assert!(
        provider_mod::pending_removal("claude", &settings(&root))
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(foreign.join("hooks/mine.mjs")).unwrap(), b"mine");
    assert_eq!(fs::read_dir(&foreign).unwrap().count(), 1);

    // A marker that does not name TMT does not make it TMT's.
    fs::write(
        foreign.join(".tmt-managed.json"),
        br#"{"owner":"someone","digest":"x"}"#,
    )
    .unwrap();
    assert_eq!(
        provider_mod::pending("claude", &settings(&root), true)
            .unwrap()
            .unwrap()
            .step,
        ModStep::Blocked
    );
}

#[test]
fn a_link_in_the_way_is_blocked_and_followed_nowhere() {
    let root = TestDirectory::new();
    let elsewhere = root.path.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(root.path.join(".claude/skills")).unwrap();
    symlink(&elsewhere, mod_dir(&root)).unwrap();
    let blocked = provider_mod::pending("claude", &settings(&root), true)
        .unwrap()
        .unwrap();
    assert_eq!(blocked.step, ModStep::Blocked);
    assert_eq!(blocked.apply().unwrap(), ModOutcome::Blocked);
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);

    // A linked skills root is not written through either.
    let other = TestDirectory::new();
    let real = other.path.join("real-skills");
    fs::create_dir_all(&real).unwrap();
    fs::create_dir_all(other.path.join(".claude")).unwrap();
    symlink(&real, other.path.join(".claude/skills")).unwrap();
    let blocked = provider_mod::pending("claude", &settings(&other), true)
        .unwrap()
        .unwrap();
    assert_eq!(blocked.step, ModStep::Blocked);
    assert_eq!(fs::read_dir(&real).unwrap().count(), 0);
}

mod with_recorded_hooks {
    use super::{mod_dir, settings};
    use crate::{
        drivers::Registry,
        setup::{
            self, RefreshStatus,
            provider_mod::{self, ModOutcome},
            record, refresh_recorded, removal,
        },
        skill_installation::ProviderEnvironment,
        test_support::TestDirectory,
    };
    use std::fs;

    /// Hooks and their record for Claude under `root/.claude`, as a consenting setup leaves them.
    fn setup_claude(root: &TestDirectory, usage: bool) {
        let entry = record::RecordedHooks {
            driver: "claude".into(),
            settings: settings(root),
            launcher: "/private/stable/bin/tmt".into(),
            usage: Some(usage),
        };
        let plan = setup::plan(
            Registry::builtin().find("claude").unwrap(),
            entry.settings.clone(),
            None,
            entry.launcher.clone(),
            false,
            if usage {
                setup::UsageHook::Install
            } else {
                setup::UsageHook::Remove
            },
        )
        .unwrap();
        setup::apply(&plan).unwrap();
        record::remember(&root.path, entry).unwrap();
    }

    fn install_mod(root: &TestDirectory) {
        let pending = provider_mod::pending("claude", &settings(root), true)
            .unwrap()
            .unwrap();
        assert_eq!(pending.apply().unwrap(), ModOutcome::Installed);
    }

    #[test]
    fn an_upgrade_refreshes_an_installed_mod_and_never_creates_a_missing_one() {
        let root = TestDirectory::new();
        setup_claude(&root, true);
        let report = refresh_recorded(&root.path);
        assert!(report.error.is_none());
        assert!(!mod_dir(&root).exists(), "an upgrade does not install it");

        install_mod(&root);
        let register = mod_dir(&root).join("hooks/register.mjs");
        let shipped = fs::read(&register).unwrap();
        fs::write(&register, b"// older build").unwrap();
        let report = refresh_recorded(&root.path);
        assert_eq!(report.integrations[0].status, RefreshStatus::Updated);
        assert_eq!(fs::read(&register).unwrap(), shipped);
        let report = refresh_recorded(&root.path);
        assert_eq!(report.integrations[0].status, RefreshStatus::Unchanged);
    }

    #[test]
    fn an_upgrade_removes_a_mod_left_behind_a_recorded_usage_opt_out() {
        let root = TestDirectory::new();
        setup_claude(&root, false);
        install_mod(&root);
        let report = refresh_recorded(&root.path);
        assert_eq!(report.integrations[0].status, RefreshStatus::Updated);
        assert!(!mod_dir(&root).exists());
    }

    #[test]
    fn uninstall_removes_the_mod_with_the_hooks_and_lists_it() {
        let root = TestDirectory::new();
        setup_claude(&root, true);
        install_mod(&root);
        let environment = ProviderEnvironment::from_parts(
            root.path.join("home"),
            root.path.clone(),
            Vec::new(),
            [],
        );
        let steps =
            removal::plan_hook_removal(&root.path, &environment, &Registry::builtin()).unwrap();
        let step = steps
            .iter()
            .find(|step| removal::mod_target(step).is_some())
            .expect("a step that also removes the mod");
        assert_eq!(removal::mod_target(step).unwrap(), mod_dir(&root));
        for step in &steps {
            removal::remove_hooks(&root.path, step).unwrap();
        }
        assert!(!mod_dir(&root).exists());
        assert!(record::read(&root.path).unwrap().is_empty());
    }

    #[test]
    fn uninstall_keeps_a_directory_that_is_not_tmts() {
        let root = TestDirectory::new();
        setup_claude(&root, true);
        fs::create_dir_all(mod_dir(&root)).unwrap();
        fs::write(mod_dir(&root).join("mine"), b"mine").unwrap();
        let environment = ProviderEnvironment::from_parts(
            root.path.join("home"),
            root.path.clone(),
            Vec::new(),
            [],
        );
        let steps =
            removal::plan_hook_removal(&root.path, &environment, &Registry::builtin()).unwrap();
        assert!(steps.iter().all(|step| removal::mod_target(step).is_none()));
        for step in &steps {
            removal::remove_hooks(&root.path, step).unwrap();
        }
        assert_eq!(fs::read(mod_dir(&root).join("mine")).unwrap(), b"mine");
    }
}
