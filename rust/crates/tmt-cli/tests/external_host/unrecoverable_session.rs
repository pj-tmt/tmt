//! A remembered session the provider no longer keeps is explained before any
//! launch. Provider trees live only under the fixture's private HOME.
use super::move_session::{SESSION, destination, remembered, start};
use super::*;
use std::os::unix::fs::PermissionsExt;

/// The fake provider stays off PATH: without the probe this resume would bind
/// and try to launch, and fail fast instead of starting an agent.
fn resume(fixture: &Fixture) -> Output {
    fixture
        .command_in(Some("f2"), &["resume", "worker"])
        .output()
        .unwrap()
}

#[test]
fn a_gone_or_unreadable_session_is_explained_and_nothing_is_launched() {
    register();
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(&format!("unrecoverable-{provider}"));
        destination(&fixture);
        let (old, process) = start(&fixture, provider, fixture.approve());
        drop(old);
        wait_until("the old provider ended", || process.seen() == Seen::Ended);
        // The old pane no longer carries the marker, so a resume here would
        // bind and launch; only the probe stops it.
        fs::remove_file(fixture.root.join("driver/marker")).unwrap();
        let home = fixture.root.join("home");
        let (tree, file) = if provider == "claude" {
            let tree = home.join(".claude/projects/-src");
            (tree.clone(), tree.join(format!("{SESSION}.jsonl")))
        } else {
            let tree = home.join(".codex/sessions/2026/10/23");
            (
                tree.clone(),
                tree.join(format!("rollout-2026-10-23T00-00-00-{SESSION}.jsonl")),
            )
        };
        fs::create_dir_all(&tree).unwrap();
        let before = remembered(&fixture);
        for (reason, text) in [
            ("missing", "is gone or expired"),
            ("unreadable", "cannot be read"),
        ] {
            if reason == "unreadable" {
                fs::write(&file, b"{}\n").unwrap();
                fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
            }
            let output = resume(&fixture);
            let error = String::from_utf8(output.stderr).unwrap();
            assert_eq!(
                output.status.code(),
                Some(1),
                "{provider} {reason}: {error}"
            );
            assert!(error.contains(text), "{provider} {reason}: {error}");
            assert!(error.contains("tmt resume --retry worker"), "{error}");
            assert!(
                error.contains("Start fresh from its notes: tmt resume --new worker"),
                "{error}"
            );
            assert!(
                !fixture.root.join("agent.pid").exists(),
                "{provider} launched"
            );
            let after = remembered(&fixture);
            assert_eq!(after.entry.identity, before.entry.identity);
            assert_eq!(
                after.preferences, before.preferences,
                "nothing is marked or forgotten"
            );
        }
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert_drivers_ended(&fixture);
    }
}

/// A replayed wrapper may keep its sessions under its own config root, so the
/// caller's tree without the session file does not stop the resume.
#[test]
fn a_replayed_wrapper_is_launched_without_the_probe() {
    register();
    let fixture = Fixture::new("unrecoverable-wrapper");
    destination(&fixture);
    let (old, process) = start(&fixture, "claude", fixture.approve());
    drop(old);
    wait_until("the old provider ended", || process.seen() == Seen::Ended);
    fs::remove_file(fixture.root.join("driver/marker")).unwrap();
    fs::create_dir_all(fixture.root.join("home/.claude/projects/-src")).unwrap();
    let launched = fixture.root.join("wrapper-launched");
    let wrapper = fixture.root.join("wrappers/claude-work");
    fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
    fs::write(
        &wrapper,
        format!("#!/bin/sh\n: > '{}'\n", launched.display()),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let identity = remembered(&fixture).entry.identity.id;
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let preset = tmt_adapters::runtime::launch_preset::LaunchPreset {
        executable: wrapper.to_str().unwrap().into(),
        model: None,
        effort: None,
        env: Default::default(),
        binding: "11111111-1111-4111-8111-111111111111".into(),
        owner_pid: process.incarnation.pid(),
        owner_start: process.incarnation.start_identity().into(),
        harness: Some("claude".into()),
        session: Some(SESSION.into()),
    };
    assert!(
        tmt_adapters::runtime::launch_preset::remember(&mut storage, &identity, &preset).unwrap()
    );
    drop(storage);
    let output = resume(&fixture);
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(!error.contains("gone or expired"), "{error}");
    assert!(launched.exists(), "the wrapper was not launched: {error}");
    assert_drivers_ended(&fixture);
}
