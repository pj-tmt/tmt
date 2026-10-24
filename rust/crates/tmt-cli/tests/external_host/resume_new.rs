//! `tmt resume --new` keeps the identity and starts its driver with a fresh
//! session handed off from the notebook. Provider trees and executables live
//! only under the fixture's private root.
use super::driver_switch::{expected_launch, missing_launch_evidence, switch_provider};
use super::move_session::{destination, remembered, start};
use super::*;

const FRESH: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

/// The old agent has ended and its pane is released, so a resume here binds
/// and launches. The provider tree exists without the old session's file.
fn ended_member(label: &str, provider: &'static str) -> Fixture {
    let fixture = Fixture::delivering(label);
    destination(&fixture);
    fs::write(fixture.root.join("driver/agent"), "ok").unwrap();
    let (old, process) = start(&fixture, provider, fixture.approve());
    drop(old);
    wait_until("the old provider ended", || process.seen() == Seen::Ended);
    fs::remove_file(fixture.root.join("driver/marker")).unwrap();
    let home = fixture.root.join("home");
    fs::create_dir_all(if provider == "claude" {
        home.join(".claude/projects/-src")
    } else {
        home.join(".codex/sessions/2026/10/23")
    })
    .unwrap();
    let notes = fixture
        .state()
        .join("notes")
        .join(remembered(&fixture).entry.identity.id);
    fs::create_dir_all(&notes).unwrap();
    fs::write(
        notes.join("notes.md"),
        "Current work: keep the identity. Next: start fresh.",
    )
    .unwrap();
    fixture
}

fn fresh_command(fixture: &Fixture, provider_on_path: bool) -> std::process::Command {
    let mut command =
        fixture.command_in(Some("f2"), &["resume", "--new", "--no-channel", "worker"]);
    let path = "/usr/bin:/bin:/usr/sbin:/sbin";
    command.env(
        "PATH",
        if provider_on_path {
            format!("{}:{path}", fixture.root.join("bin").display())
        } else {
            path.into()
        },
    );
    command
}

/// The fresh session starts with the handoff and replaces the remembered one.
fn assert_fresh_launch(fixture: &Fixture, provider: &str) {
    switch_provider(fixture, provider, FRESH, "startup");
    let mut launched = Launched::start(fixture, fresh_command(fixture, true));
    wait_until("the fresh provider starts or the command ends", || {
        fixture.root.join("agent.pid").exists()
            || launched
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_some()
    });
    assert!(
        fixture.root.join("agent.pid").exists(),
        "{}",
        fs::read_to_string(fixture.root.join("launched.err")).unwrap()
    );
    launched.adopt("agent.pid", false);
    let expected = expected_launch(&launched, provider, FRESH);
    wait_until("the fresh session is committed", || {
        missing_launch_evidence(&remembered(fixture), &expected).is_none()
    });
    let args = fs::read(fixture.root.join("switch.args")).unwrap();
    let args = args
        .split(|byte| *byte == 0)
        .filter(|word| !word.is_empty())
        .map(|word| String::from_utf8(word.to_vec()).unwrap())
        .collect::<Vec<_>>();
    assert!(
        !args
            .iter()
            .any(|word| word == "--resume" || word == "resume"),
        "{args:?}"
    );
    let context = args.last().unwrap();
    assert!(context.contains("untrusted context"), "{context}");
    assert!(
        context.contains("Current work: keep the identity."),
        "{context}"
    );
    assert!(!context.contains("--receipt"), "{context}");
    // The old agent is gone, so nothing is asked of it.
    assert!(fixture.requests("prompted").is_empty());
    let (status, stderr) = launched.finish();
    assert_eq!(status, Some(7), "{stderr}");
}

#[test]
fn new_after_a_gone_session_launches_fresh_and_a_failed_launch_keeps_the_record() {
    register();
    for provider in ["claude", "codex"] {
        let fixture = ended_member(&format!("resume-new-{provider}"), provider);
        let before = remembered(&fixture);
        // The refusal names the fresh start.
        let refused = fixture
            .command_in(Some("f2"), &["resume", "worker"])
            .output()
            .unwrap();
        let error = String::from_utf8(refused.stderr).unwrap();
        assert_eq!(refused.status.code(), Some(1), "{error}");
        assert!(error.contains("is gone or expired"), "{error}");
        assert!(
            error.contains("Start fresh from its notes: tmt resume --new -- worker"),
            "{error}"
        );
        // A launch that cannot start keeps the remembered session for --retry.
        let failed = fresh_command(&fixture, false).output().unwrap();
        let error = String::from_utf8(failed.stderr).unwrap();
        assert_eq!(failed.status.code(), Some(1), "{error}");
        assert!(
            error.contains("Could not start the requested command"),
            "{error}"
        );
        assert!(!fixture.root.join("agent.pid").exists());
        let after = remembered(&fixture);
        assert_eq!(after.entry.identity, before.entry.identity);
        assert_eq!(after.preferences.remembered, before.preferences.remembered);
        assert_fresh_launch(&fixture, provider);
        let after = remembered(&fixture);
        assert_eq!(after.entry.identity, before.entry.identity);
        assert_eq!(
            after.preferences.remembered.unwrap().harness.as_str(),
            provider
        );
        assert_drivers_ended(&fixture);
    }
}

#[test]
fn new_without_a_remembered_session_starts_the_preferred_driver() {
    register();
    let fixture = ended_member("resume-new-forgotten", "claude");
    let forgot = fixture.tmt_in(Some("f2"), &["resume", "--forget", "worker"]);
    assert!(forgot.status.success(), "{forgot:?}");
    assert!(remembered(&fixture).preferences.remembered.is_none());
    assert_fresh_launch(&fixture, "claude");
    assert_drivers_ended(&fixture);
}

/// A fresh start keeps the old session's launcher (a wrapper may own the
/// account). One that exits before its start hook keeps the old record.
#[test]
fn new_replays_the_wrapper_and_an_unconfirmed_start_keeps_the_record() {
    use std::os::unix::fs::PermissionsExt;
    register();
    let fixture = ended_member("resume-new-wrapper", "claude");
    let before = remembered(&fixture);
    let wrapper = fixture.root.join("wrappers/claude-work");
    let args = fixture.root.join("wrapper.args");
    fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
    fs::write(
        &wrapper,
        format!("#!/bin/sh\nprintf '%s\\0' \"$@\" > '{}'\n", args.display()),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let owner = before.entry.binding.as_ref().unwrap();
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let preset = tmt_adapters::runtime::launch_preset::LaunchPreset {
        executable: wrapper.to_str().unwrap().into(),
        model: Some("wrapped-model".into()),
        effort: None,
        env: Default::default(),
        binding: owner.id.clone(),
        owner_pid: u64::from(std::process::id()),
        owner_start: "start".into(),
        harness: Some("claude".into()),
        session: Some(super::move_session::SESSION.into()),
    };
    assert!(
        tmt_adapters::runtime::launch_preset::remember(
            &mut storage,
            &before.entry.identity.id,
            &preset
        )
        .unwrap()
    );
    drop(storage);
    let output = fresh_command(&fixture, false).output().unwrap();
    let error = String::from_utf8(output.stderr).unwrap();
    let words = fs::read(&args).unwrap_or_else(|_| panic!("the wrapper did not run: {error}"));
    let words = words
        .split(|byte| *byte == 0)
        .filter(|word| !word.is_empty())
        .map(|word| String::from_utf8(word.to_vec()).unwrap())
        .collect::<Vec<_>>();
    assert!(
        words.iter().any(|word| word == "wrapped-model"),
        "{words:?}"
    );
    assert!(!words.iter().any(|word| word == "--resume"), "{words:?}");
    assert!(
        words
            .last()
            .unwrap()
            .contains("Current work: keep the identity.")
    );
    let after = remembered(&fixture);
    assert_eq!(after.preferences.remembered, before.preferences.remembered);
    // `--retry` of the kept session still finds its wrapper.
    let storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let kept = tmt_adapters::runtime::launch_preset::read_for_session(
        &storage,
        &before.entry.identity.id,
        before.preferences.remembered.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(
        kept.map(|preset| preset.executable),
        Some(preset.executable),
        "the kept session lost its launch preset"
    );
    assert_drivers_ended(&fixture);
}
