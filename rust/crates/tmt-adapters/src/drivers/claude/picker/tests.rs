use super::*;
use crate::{
    drivers::Registry,
    process::CommandFailure,
    scripted_runner::{ScriptedRunner, failure_with_kind},
    skill_installation::InstalledSkill,
    test_support::TestDirectory,
};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn environment_with_claude(directory: &TestDirectory) -> ProviderEnvironment {
    let bin = directory.path.join("bin");
    fs::create_dir(&bin).unwrap();
    let claude = bin.join("claude");
    fs::write(&claude, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    environment(directory, vec![bin])
}

fn environment(directory: &TestDirectory, path: Vec<std::path::PathBuf>) -> ProviderEnvironment {
    ProviderEnvironment::from_parts(&directory.path, &directory.path, path, [])
}

const WITH_PICKER: &str = r#"[
  {"id":"other@market","scope":"user","enabled":true},
  {"id":"tmt-picker@tmt","scope":"user","enabled":true,"version":"0.1.0"}
]"#;
const WITHOUT_PICKER: &str = r#"[{"id":"other@market","scope":"user","enabled":true}]"#;

fn hint(output: Result<&'static str, crate::process::CommandError>) -> Option<&'static str> {
    let directory = TestDirectory::new();
    let environment = environment_with_claude(&directory);
    let runner = ScriptedRunner::new([output]);
    let result = missing_hint_with(&environment, &runner);
    let calls = runner.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert!(Path::new(&calls[0].program).ends_with("bin/claude"));
    assert_eq!(calls[0].args, ["plugin", "list", "--json"]);
    assert!(calls[0].deadline <= Instant::now() + BUDGET);
    assert_eq!(calls[0].max_output_bytes, OUTPUT_LIMIT);
    result
}

#[test]
fn an_installed_picker_stays_silent() {
    assert_eq!(hint(Ok(WITH_PICKER)), None);
}

#[test]
fn a_missing_picker_gets_the_one_install_line() {
    assert_eq!(hint(Ok(WITHOUT_PICKER)), Some(MISSING_HINT));
    assert_eq!(hint(Ok("[]")), Some(MISSING_HINT));
    assert_eq!(MISSING_HINT.lines().count(), 1);
}

#[test]
fn the_picker_from_any_marketplace_counts_as_installed() {
    assert_eq!(
        hint(Ok(r#"[{"id":"tmt-picker@fork","enabled":false}]"#)),
        None
    );
    // The prefix is the plugin name, not a substring of another plugin's id.
    assert_eq!(
        hint(Ok(r#"[{"id":"not-tmt-picker@tmt"}]"#)),
        Some(MISSING_HINT)
    );
}

#[test]
fn unreadable_answers_stay_silent() {
    for text in ["", "not json", "{}", r#"{"id":"other@market"}"#, "[1,2]"] {
        assert_eq!(hint(Ok(text)), None, "{text:?}");
    }
    for kind in [
        CommandFailure::Timeout,
        CommandFailure::Exit {
            code: Some(1),
            signal: None,
        },
    ] {
        assert_eq!(hint(Err(failure_with_kind(kind, false))), None, "{kind:?}");
    }
    assert_eq!(
        hint(Err(failure_with_kind(CommandFailure::Timeout, true))),
        None
    );
}

#[test]
fn no_claude_on_path_never_starts_a_process() {
    let directory = TestDirectory::new();
    let empty = directory.path.join("empty");
    fs::create_dir(&empty).unwrap();
    let runner = ScriptedRunner::new([]);
    assert_eq!(
        missing_hint_with(&environment(&directory, vec![empty]), &runner),
        None
    );
    assert_eq!(
        missing_hint_with(&environment(&directory, Vec::new()), &runner),
        None
    );
    assert!(runner.calls.borrow().is_empty());
}

fn installed(agent: Option<&str>) -> InstallReport {
    InstallReport {
        installed: vec![InstalledSkill {
            name: "tmt",
            agent: agent.map(|name| Registry::builtin().find(name).expect("builtin driver")),
            target: "/skills/tmt".into(),
            changed: true,
            backup: None,
            legacy_backups: Vec::new(),
        }],
        warnings: Vec::new(),
    }
}

#[test]
fn only_an_installation_that_reached_claude_mentions_its_plugin() {
    let claude = super::super::DRIVER.descriptor.name;
    assert!(reached_claude(&installed(Some(claude))));
    // Another provider's root and a custom directory (no agent) never do.
    for other in Registry::builtin().iter().map(|driver| driver.name()) {
        assert_eq!(reached_claude(&installed(Some(other))), other == claude);
    }
    assert!(!reached_claude(&installed(None)));
    assert!(!reached_claude(&InstallReport::default()));
}
