//! Public resume moves use only owned fake providers and a private host driver.
use super::*;

pub(super) const SESSION: &str = "4f8c2a7e-0d3b-4c61-9a52-7b1e6d0c9f13";

pub(super) fn destination(fixture: &Fixture) {
    fs::write(fixture.root.join("driver/two-panes"), "").unwrap();
    // The test process is the destination shell and the new CLI's ancestor.
    fs::write(
        fixture.root.join("driver/pid-2"),
        std::process::id().to_string(),
    )
    .unwrap();
}

pub(super) fn remembered(fixture: &Fixture) -> tmt_adapters::storage::IdentityContextSnapshot {
    let database = fixture.state().join("tmt.db");
    // Preferences belong to the identity even between a move and child admission.
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let identity: String = connection
        .query_row(
            "SELECT id FROM identities WHERE name = 'worker' AND retired_at_ms IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);
    Storage::context_by_identity(&database, &identity, 0)
        .unwrap()
        .unwrap()
}

pub(super) fn start(
    fixture: &Fixture,
    provider: &'static str,
    record: registry::DriverRecord,
) -> (SteppedProvider, Owned) {
    let mut agent = SteppedProvider::start_in(fixture, provider, "old-steps", "f1", true);
    // The source has its own shell ancestor; it is never the destination or server PID.
    let shell = agent.child.id().to_string();
    fs::write(fixture.root.join("driver/pid"), &shell).unwrap();
    fs::write(fixture.root.join("driver/shell"), &shell).unwrap();
    bind(fixture, record);
    let process = live_provider(&agent);
    let mut payload = codex_event("SessionStart", None);
    payload["session_id"] = SESSION.into();
    let pid = process.incarnation.pid().to_string().into_bytes();
    let environment: &[(&str, &[u8])] = if provider == "claude" {
        &[("CLAUDE_PID", &pid)]
    } else {
        &[]
    };
    let (code, _, stderr) = agent.hook_with_env(payload, environment);
    assert_eq!((code, stderr.as_str()), (0, ""));
    let stored = remembered(fixture);
    assert_eq!(
        stored
            .entry
            .binding
            .unwrap()
            .session
            .key
            .unwrap()
            .incarnation,
        process.incarnation
    );
    (agent, process)
}

fn command(fixture: &Fixture, pane: &str) -> std::process::Command {
    command_for(fixture, pane, "worker")
}

/// The move command naming its member by `selector`: a display name or the identity UUID.
fn command_for(fixture: &Fixture, pane: &str, selector: &str) -> std::process::Command {
    let mut command = fixture.command_in(Some(pane), &["resume", "--move", selector]);
    command.env(
        "PATH",
        format!(
            "{}:/usr/bin:/bin:/usr/sbin:/sbin",
            fixture.root.join("bin").display()
        ),
    );
    command
}

#[test]
fn resume_move_stops_only_the_recorded_agent_and_preserves_the_identity_and_session() {
    register();
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(&format!("move-{provider}"));
        destination(&fixture);
        let (old, process) = start(&fixture, provider, fixture.approve());
        let before = remembered(&fixture);
        let executable = fixture.root.join("bin").join(provider);
        // The running stand-in already loaded sh; replace only our own link.
        fs::remove_file(&executable).unwrap();
        tmt_test_support::write_executable(
            &executable,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > resume.args\n{AGENT}\n").as_bytes(),
            0o755,
        )
        .unwrap();
        // Ops hands members off by UUID, so one provider moves by it.
        let selector = if provider == "codex" {
            before.entry.identity.id.clone()
        } else {
            "worker".to_owned()
        };
        let mut resumed = Launched::start(&fixture, command_for(&fixture, "f2", &selector));
        wait_until("move launches or refuses", || {
            fixture.root.join("agent.pid").exists()
                || resumed
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
        resumed.adopt("agent.pid", false);
        assert_eq!(
            process.seen(),
            Seen::Ended,
            "old agent must end before the new launch"
        );
        let after = remembered(&fixture);
        assert_eq!(after.entry.identity, before.entry.identity);
        assert_ne!(
            after.entry.binding.as_ref().unwrap().id,
            before.entry.binding.as_ref().unwrap().id
        );
        assert_eq!(after.entry.binding.as_ref().unwrap().pane_id, "fake-2");
        assert_eq!(
            after.preferences.preferred_harness,
            before.preferences.preferred_harness
        );
        let session = after.preferences.remembered.unwrap();
        assert_eq!(session.provider_session.as_str(), SESSION);
        assert_eq!(session.harness.as_str(), provider);
        assert!(!fixture.root.join("driver/marker").exists());
        let marker: Value =
            serde_json::from_slice(&fs::read(fixture.root.join("driver/marker-2")).unwrap())
                .unwrap();
        assert_eq!(marker["identityId"], before.entry.identity.id);
        let args = fs::read_to_string(fixture.root.join("resume.args")).unwrap();
        let args: Vec<_> = args.lines().collect();
        if provider == "claude" {
            assert_eq!(&args[..2], &["--resume", SESSION]);
        } else {
            assert!(
                args.first() == Some(&"resume")
                    && args.windows(2).any(|pair| pair == [SESSION, "--no-daemon"]),
                "{args:?}"
            );
            assert!(args.contains(&"--no-daemon"));
        }
        let (code, stderr) = resumed.finish();
        assert_eq!(code, Some(7), "{stderr}");
        drop(old);
        assert_drivers_ended(&fixture);
    }
}

#[test]
fn resume_move_refuses_an_occupied_or_identical_destination_without_stopping() {
    register();
    for occupied in [false, true] {
        let fixture = Fixture::new(&format!("move-refused-{occupied}"));
        let record = fixture.approve();
        destination(&fixture);
        let (mut old, process) = start(&fixture, "claude", record.clone());
        if occupied {
            bind_pane(&fixture, record, "fake-2", "other");
        }
        let before = remembered(&fixture);
        let markers = (
            fs::read(fixture.root.join("driver/marker")).unwrap(),
            fs::read(fixture.root.join("driver/marker-2")).ok(),
        );
        let (code, error) = if occupied {
            let output = command(&fixture, "f2").output().unwrap();
            (
                output.status.code().unwrap(),
                String::from_utf8(output.stderr).unwrap(),
            )
        } else {
            let (code, _, stderr) = old.command(&[], &["resume", "--move", "worker"]);
            (code, stderr)
        };
        assert_ne!(code, 0, "{error}");
        assert!(error.contains("No new agent was launched"), "{error}");
        assert!(error.contains("tmt resume"), "{error}");
        assert_eq!(process.seen(), Seen::Live);
        let after = remembered(&fixture);
        assert_eq!(after.entry, before.entry);
        assert_eq!(after.preferences, before.preferences);
        assert_eq!(
            (
                fs::read(fixture.root.join("driver/marker")).unwrap(),
                fs::read(fixture.root.join("driver/marker-2")).ok()
            ),
            markers
        );
        assert!(!fixture.root.join("agent.pid").exists());
        drop(old);
        assert_eq!(process.seen(), Seen::Ended);
        assert_drivers_ended(&fixture);
    }
}

#[test]
fn resume_move_sends_one_term_and_refuses_an_agent_that_does_not_exit() {
    register();
    let fixture = Fixture::new("move-ignores-term");
    destination(&fixture);
    fs::create_dir_all(fixture.root.join("old-steps")).unwrap();
    fs::write(fixture.root.join("old-steps/ignore-term"), "").unwrap();
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let before = remembered(&fixture);
    let output = command(&fixture, "f2").output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("No new agent was launched"), "{error}");
    assert!(error.contains("Exit the old agent in its pane"), "{error}");
    assert!(error.contains("tmt resume --move -- worker"), "{error}");
    assert_eq!(process.seen(), Seen::Live);
    assert_eq!(
        fs::read_to_string(fixture.root.join("old-steps/terms")).unwrap(),
        "TERM\n"
    );
    let after = remembered(&fixture);
    assert_eq!(after.entry, before.entry);
    assert_eq!(after.preferences, before.preferences);
    assert!(!fixture.root.join("driver/marker-2").exists());
    assert!(!fixture.root.join("agent.pid").exists());
    drop(old);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

#[test]
fn resume_move_refuses_a_shared_codex_server_without_a_signal_or_binding_change() {
    use tmt_core::binding::{BindingRepository, session::RuntimeMode};
    register();
    let fixture = Fixture::new("move-shared-codex");
    destination(&fixture);
    let (old, process) = start(&fixture, "codex", fixture.approve());
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let original = remembered(&fixture);
    storage
        .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|records| {
            let mut preferences = original.preferences.clone();
            preferences.remembered.as_mut().unwrap().mode = RuntimeMode::new("shared").unwrap();
            records.set_session_preferences(&original.entry.identity.id, &preferences)
        })
        .unwrap();
    storage.close().unwrap();
    let before = remembered(&fixture);
    let output = command(&fixture, "f2").output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("shared/channel servers are not stopped"),
        "{error}"
    );
    assert!(error.contains("No new agent was launched"), "{error}");
    assert!(error.contains("tmt resume --move -- worker"), "{error}");
    assert_eq!(process.seen(), Seen::Live);
    let after = remembered(&fixture);
    assert_eq!(after.entry, before.entry);
    assert_eq!(after.preferences, before.preferences);
    drop(old);
    assert_drivers_ended(&fixture);
}

#[test]
fn resume_move_reports_a_failed_spawn_and_the_retry_resumes_here_without_moving_again() {
    register();
    let fixture = Fixture::new("move-spawn-fails");
    destination(&fixture);
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let before = remembered(&fixture);
    let executable = fixture.root.join("bin/claude");
    fs::remove_file(&executable).unwrap();
    let output = command(&fixture, "f2").output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("No new agent was launched"), "{error}");
    assert!(error.contains("The identity is now bound here"), "{error}");
    assert!(error.contains("retry: tmt resume -- worker"), "{error}");
    assert!(!error.contains("retry: tmt resume --move"), "{error}");
    assert_eq!(process.seen(), Seen::Ended);
    let moved = remembered(&fixture);
    assert_eq!(moved.entry.identity, before.entry.identity);
    let binding = moved.entry.binding.as_ref().unwrap();
    assert_eq!(binding.pane_id, "fake-2");
    assert_ne!(binding.id, before.entry.binding.unwrap().id);
    assert!(!fixture.root.join("agent.pid").exists());
    // Repair only our missing executable, then execute the suggested public retry.
    tmt_test_support::write_executable(
        &executable,
        format!("#!/bin/sh\n{AGENT}\n").as_bytes(),
        0o755,
    )
    .unwrap();
    let mut retry = fixture.command_in(Some("f2"), &["resume", "--", "worker"]);
    retry.env(
        "PATH",
        format!(
            "{}:/usr/bin:/bin:/usr/sbin:/sbin",
            fixture.root.join("bin").display()
        ),
    );
    let mut launched = Launched::start(&fixture, retry);
    wait_until("the suggested retry starts or refuses", || {
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
    assert_eq!(remembered(&fixture).entry.binding.unwrap().id, binding.id);
    let (code, stderr) = launched.finish();
    assert_eq!(code, Some(7), "{stderr}");
    drop(old);
    assert_drivers_ended(&fixture);
}

#[test]
fn ordinary_resume_names_the_live_old_pane_without_launching_or_mutating() {
    register();
    for (provider, stopped) in [("claude", false), ("codex", false), ("claude", true)] {
        let fixture = Fixture::new(&format!("resume-live-{provider}-{stopped}"));
        destination(&fixture);
        let (old, process) = start(&fixture, provider, fixture.approve());
        let before = remembered(&fixture);
        let marker = fs::read(fixture.root.join("driver/marker")).unwrap();
        if stopped {
            nix::sys::signal::kill(process.pid(), nix::sys::signal::Signal::SIGSTOP).unwrap();
            wait_until("owned provider stopped", || process.seen() == Seen::Stopped);
        }
        // Ordinary resume must not implicitly stop or move the old process.
        let mut resume = fixture.command_in(Some("f2"), &["resume", "worker"]);
        resume.env(
            "PATH",
            format!(
                "{}:/usr/bin:/bin:/usr/sbin:/sbin",
                fixture.root.join("bin").display()
            ),
        );
        let output = resume.output().unwrap();
        let error = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(5), "{error}");
        assert!(error.contains("recorded agent"), "{error}");
        assert!(error.contains("fake-1"), "{error}");
        assert!(error.contains("tmt resume --move -- worker"), "{error}");
        assert!(error.contains("No new agent was launched"), "{error}");
        assert_eq!(
            process.seen(),
            if stopped { Seen::Stopped } else { Seen::Live }
        );
        if stopped {
            nix::sys::signal::kill(process.pid(), nix::sys::signal::Signal::SIGCONT).unwrap();
            wait_until("owned provider continued", || process.seen() == Seen::Live);
        }
        let after = remembered(&fixture);
        assert_eq!(after.entry, before.entry);
        assert_eq!(after.preferences, before.preferences);
        assert_eq!(
            fs::read(fixture.root.join("driver/marker")).unwrap(),
            marker
        );
        assert!(!fixture.root.join("driver/marker-2").exists());
        assert!(!fixture.root.join("agent.pid").exists());
        drop(old);
        assert_eq!(process.seen(), Seen::Ended);
        assert_drivers_ended(&fixture);
    }
}
