//! A provider hook in a pane records only the pane's main provider (#3638):
//! the provider nearest the hook, with no other built-in runtime provider
//! between it and the pane. A nested one, such as a daemon-hosted Claude
//! worker below the terminal Claude, is refused before any write. The shared
//! exact-thread branch, which has no pane ancestry, is not part of this rule.
//!
//! Every tree is synthetic: links to `sh` named after a provider, under this
//! test process, which stands for the pane shell.
use super::*;

/// Every start below runs on an empty binding, so on its own it would be
/// admitted: only the provider's place in the pane decides.
fn start(source: &str) -> Value {
    serde_json::json!({"hook_event_name": "SessionStart", "session_id": "s-1",
        "source": source, "cwd": "/src", "model": "claude-test"})
}

/// The binding's runtime, read only: provider session, PID, start identity
/// and state.
fn runtime(fixture: &Fixture) -> (String, i64, String, String) {
    rusqlite::Connection::open_with_flags(
        fixture.state().join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT coalesce(observed_provider_session_id, ''), coalesce(runtime_pid, 0), \
         coalesce(runtime_start_identity, ''), runtime_state FROM bindings WHERE pane_id = 'fake-1'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .unwrap()
}

/// A refused hook: the fixed line, no context, and no stored row changed.
fn assert_refused(fixture: &Fixture, rows: &[String], output: (i32, String, String), case: &str) {
    assert_eq!(
        output,
        (0, String::new(), HOOK_UNAVAILABLE.to_owned()),
        "{case}"
    );
    assert_eq!(
        stored_rows(fixture),
        rows,
        "{case}: a refused hook changed a row"
    );
}

/// An admitted start: its context, and exactly this incarnation recorded
/// with the start's session.
fn assert_recorded(fixture: &Fixture, provider: &Owned, output: (i32, String, String), case: &str) {
    let (status, stdout, stderr) = output;
    assert_eq!((status, stderr.as_str()), (0, ""), "{case}");
    let context: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        context["hookSpecificOutput"]["hookEventName"], "SessionStart",
        "{case}"
    );
    assert_eq!(
        runtime(fixture),
        (
            "s-1".to_owned(),
            provider.incarnation.pid() as i64,
            provider.incarnation.start_identity().to_owned(),
            "running".to_owned()
        ),
        "{case}"
    );
}

fn pid_of(provider: &Owned) -> Vec<u8> {
    provider.incarnation.pid().to_string().into_bytes()
}

/// The incident's tree: pane → Claude (terminal) → Claude (daemon) →
/// Claude (pty host) → Claude (worker). Neither the worker's own start, nor
/// the spare-worker shape, nor a background fork records the worker. The
/// terminal, the pane's main provider, records the same start.
#[test]
fn a_daemon_hosted_worker_never_records_itself_and_the_terminal_still_does() {
    register();
    let mut spare = start("startup");
    spare["agent_type"] = "claude".into();
    for (case, payload, job) in [
        ("startup", start("startup"), false),
        ("spare", spare, true),
        ("fork", start("fork"), true),
    ] {
        let fixture = Fixture::new(&format!("nested-worker-{case}"));
        let record = fixture.approve();
        bind(&fixture, record);
        let mut worker = SteppedProvider::start_nested(
            &fixture,
            "claude",
            "worker-steps",
            &["claude", "claude", "claude"],
        );
        let process = live_provider(&worker);
        let rows = stored_rows(&fixture);
        let mut env = vec![("CLAUDE_PID", pid_of(&process))];
        if job {
            env.push(("CLAUDE_JOB_DIR", b"/tmp/jobs/test".to_vec()));
        }
        let env: Vec<_> = env
            .iter()
            .map(|(name, value)| (*name, &value[..]))
            .collect();
        let output = worker.hook_with_env(payload.clone(), &env);
        assert_refused(&fixture, &rows, output, case);
        assert_eq!(process.seen(), Seen::Live);
        drop(worker);
        assert_eq!(process.seen(), Seen::Ended);

        // The same start from the pane's main provider is recorded.
        let mut terminal = SteppedProvider::start(&fixture, "claude", "terminal-steps");
        let main = live_provider(&terminal);
        let output = terminal.hook_with_env(payload, &[("CLAUDE_PID", &pid_of(&main))]);
        assert_recorded(&fixture, &main, output, case);
        drop(terminal);
        assert_eq!(main.seen(), Seen::Ended);
        assert_drivers_ended(&fixture);
    }
}

/// Nesting of the same provider and of different providers: each nested
/// start is refused, and the same start from that provider as the pane's
/// main provider is admitted.
#[test]
fn nested_providers_of_either_kind_are_refused() {
    register();
    let claude = start("startup");
    let codex = codex_event("SessionStart", None);
    for (case, provider, ancestors, payload) in [
        (
            "claude-in-claude-tool-shell",
            "claude",
            &["claude", "sh"][..],
            &claude,
        ),
        ("claude-in-codex", "claude", &["codex"], &claude),
        ("codex-in-claude", "codex", &["claude"], &codex),
        ("codex-in-codex", "codex", &["codex"], &codex),
    ] {
        let fixture = Fixture::new(&format!("nested-{case}"));
        let record = fixture.approve();
        bind(&fixture, record);
        let mut nested =
            SteppedProvider::start_nested(&fixture, provider, "nested-steps", ancestors);
        let process = live_provider(&nested);
        let rows = stored_rows(&fixture);
        let output = nested.hook(payload.clone());
        assert_refused(&fixture, &rows, output, case);
        drop(nested);
        assert_eq!(process.seen(), Seen::Ended, "{case}");

        let mut main = SteppedProvider::start(&fixture, provider, "main-steps");
        let process = live_provider(&main);
        let (status, _, stderr) = main.hook(payload.clone());
        assert_eq!((status, stderr.as_str()), (0, ""), "{case}");
        let recorded = runtime(&fixture);
        assert_eq!(
            (recorded.0.as_str(), recorded.1, recorded.2.as_str()),
            (
                "s-1",
                process.incarnation.pid() as i64,
                process.incarnation.start_identity()
            ),
            "{case}"
        );
        drop(main);
        assert_eq!(process.seen(), Seen::Ended, "{case}");
        assert_drivers_ended(&fixture);
    }
}

/// When Claude sets `CLAUDE_PID`, it must name the main provider. Any other
/// live process, or a malformed value, refuses; without it (older Claude)
/// the main provider is still admitted. Each malformed value that can hold a
/// number holds the main provider's own PID, so only its form can refuse it.
#[test]
fn claude_pid_must_name_the_main_provider_when_present() {
    /// Builds one `CLAUDE_PID` value from the main provider's own PID.
    type Form = fn(&[u8]) -> Vec<u8>;
    register();
    let other = std::process::id().to_string().into_bytes();
    let refused: [(&str, Form); 9] = [
        ("another live process", |_| Vec::new()),
        ("empty", |_| Vec::new()),
        ("zero", |_| b"0".to_vec()),
        ("negative", |pid| [b"-".as_slice(), pid].concat()),
        ("text", |pid| [pid, b"x".as_slice()].concat()),
        ("over u32", |_| b"4294967296".to_vec()),
        ("leading space", |pid| [b" ".as_slice(), pid].concat()),
        ("trailing space", |pid| [pid, b" ".as_slice()].concat()),
        ("not UTF-8", |pid| [pid, b"\xff".as_slice()].concat()),
    ];
    for (case, form) in refused {
        let fixture = Fixture::new(&format!("claude-pid-refused-{}", case.replace(' ', "-")));
        let record = fixture.approve();
        bind(&fixture, record);
        let mut claude = SteppedProvider::start(&fixture, "claude", "claude-steps");
        let process = live_provider(&claude);
        let value = if case == "another live process" {
            other.clone()
        } else {
            form(&pid_of(&process))
        };
        let rows = stored_rows(&fixture);
        let output = claude.hook_with_env(start("startup"), &[("CLAUDE_PID", &value)]);
        assert_refused(&fixture, &rows, output, case);
        drop(claude);
        assert_eq!(process.seen(), Seen::Ended, "{case}");
        assert_drivers_ended(&fixture);
    }
    for case in ["absent", "equal"] {
        let fixture = Fixture::new(&format!("claude-pid-{case}"));
        let record = fixture.approve();
        bind(&fixture, record);
        let mut claude = SteppedProvider::start(&fixture, "claude", "claude-steps");
        let process = live_provider(&claude);
        let pid = pid_of(&process);
        let env: &[(&str, &[u8])] = if case == "equal" {
            &[("CLAUDE_PID", &pid)]
        } else {
            &[]
        };
        let output = claude.hook_with_env(start("startup"), env);
        assert_recorded(&fixture, &process, output, case);
        drop(claude);
        assert_eq!(process.seen(), Seen::Ended, "{case}");
        assert_drivers_ended(&fixture);
    }
}

/// A row that a nested worker already holds (recorded before this rule)
/// can no longer be ended by that worker's own later callback.
#[test]
fn a_nested_worker_cannot_end_a_row_it_already_holds() {
    register();
    let fixture = Fixture::new("nested-recorded");
    let record = fixture.approve();
    bind(&fixture, record);
    let mut worker = SteppedProvider::start_nested(
        &fixture,
        "claude",
        "worker-steps",
        &["claude", "claude", "claude"],
    );
    let process = live_provider(&worker);
    // The stored state a nested start left before this rule.
    rusqlite::Connection::open(fixture.state().join("tmt.db"))
        .unwrap()
        .execute(
            "UPDATE bindings SET runtime_state = 'running', last_transition = 'started', \
             runtime_pid = ?1, runtime_start_identity = ?2, observed_provider_session_id = 's-1' \
             WHERE pane_id = 'fake-1'",
            rusqlite::params![
                process.incarnation.pid() as i64,
                process.incarnation.start_identity()
            ],
        )
        .unwrap();
    let rows = stored_rows(&fixture);
    let mut end = start("startup");
    end["hook_event_name"] = "SessionEnd".into();
    end["reason"] = "other".into();
    let (status, stdout, _) = worker.hook_with_env(end, &[("CLAUDE_PID", &pid_of(&process))]);
    assert_eq!((status, stdout.as_str()), (0, ""));
    assert_eq!(
        stored_rows(&fixture),
        rows,
        "the nested worker changed a row"
    );
    drop(worker);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

/// In an unbound pane (here `f2`, on a host where `f1` is bound), a nested
/// start gets the fixed line, not the naming hint that the pane's main
/// provider gets. Both run below `f2`'s own pane shell.
#[test]
fn a_nested_start_in_an_unbound_pane_gets_no_naming_hint() {
    register();
    let fixture = Fixture::new("nested-unbound");
    let record = fixture.approve();
    bind(&fixture, record);
    fs::write(fixture.root.join("driver/two-panes"), "").unwrap();
    let in_f2 = |name: &str, ancestors: &[&'static str]| {
        let provider = SteppedProvider::start_nested_in(&fixture, "claude", name, "f2", ancestors);
        fs::write(
            fixture.root.join("driver/pid-2"),
            provider.child.id().to_string(),
        )
        .unwrap();
        provider
    };
    let mut worker = in_f2("worker-steps", &["sh", "claude", "claude"]);
    let process = live_provider(&worker);
    let rows = stored_rows(&fixture);
    let output = worker.hook(start("startup"));
    assert_refused(&fixture, &rows, output, "nested");
    drop(worker);
    assert_eq!(process.seen(), Seen::Ended);

    let mut main = in_f2("main-steps", &["sh"]);
    let process = live_provider(&main);
    let (status, stdout, stderr) = main.hook(start("startup"));
    assert_eq!((status, stderr.as_str()), (0, ""));
    let context: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        context["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    assert_eq!(stored_rows(&fixture), rows);
    drop(main);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

/// The pane's process can be the provider itself, after a shell `exec`.
#[test]
fn a_provider_that_is_the_pane_process_is_the_main_provider() {
    register();
    let fixture = Fixture::new("main-is-pane");
    let record = fixture.approve();
    bind(&fixture, record.clone());
    fs::write(fixture.root.join("driver/two-panes"), "").unwrap();
    let mut claude = SteppedProvider::start_in(&fixture, "claude", "f2-steps", "f2", false);
    fs::write(
        fixture.root.join("driver/pid-2"),
        claude.child.id().to_string(),
    )
    .unwrap();
    bind_pane(&fixture, record, "fake-2", "other");
    let process = live_provider(&claude);
    assert_eq!(process.incarnation.pid(), u64::from(claude.child.id()));
    let (status, _, stderr) =
        claude.hook_with_env(start("startup"), &[("CLAUDE_PID", &pid_of(&process))]);
    assert_eq!((status, stderr.as_str()), (0, ""));
    let recorded: (String, i64) = rusqlite::Connection::open_with_flags(
        fixture.state().join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT coalesce(observed_provider_session_id, ''), coalesce(runtime_pid, 0) \
         FROM bindings WHERE pane_id = 'fake-2'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap();
    assert_eq!(
        recorded,
        ("s-1".to_owned(), process.incarnation.pid() as i64)
    );
    drop(claude);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

/// A Codex launch's configuration options stay admitted: the main-provider
/// rule reads no command line.
#[test]
fn a_codex_launch_with_configuration_options_is_still_admitted() {
    register();
    for words in [
        &["--config", "model=x"][..],
        &["--config=model=x"],
        &["--profile", "p"],
        &["--profile=p"],
        &["-c", "model=x"],
        &["-p", "p"],
    ] {
        let fixture = Fixture::new(&format!(
            "codex-options-{}",
            words.join("").replace('=', "-")
        ));
        let record = fixture.approve();
        bind(&fixture, record);
        let mut codex = SteppedProvider::start_with_words(&fixture, "codex", "codex-steps", words);
        let process = live_provider(&codex);
        let output = codex.hook(codex_event("SessionStart", None));
        assert_options_admitted(&fixture, &process, output, &format!("{words:?}"));
        drop(codex);
        assert_eq!(process.seen(), Seen::Ended, "{words:?}");
        assert_drivers_ended(&fixture);
    }
}

/// Configuration options test provider admission, independently of context text.
fn assert_options_admitted(
    fixture: &Fixture,
    provider: &Owned,
    output: (i32, String, String),
    case: &str,
) {
    let (status, _, stderr) = output;
    assert_eq!(status, 0, "{case}: hook exit");
    assert!(
        stderr.is_empty() || stderr == HOOK_UNAVAILABLE,
        "{case}: hook stderr: {stderr:?}"
    );
    if !stderr.is_empty() {
        eprintln!("{case}: hook diagnostic: {stderr}");
    }
    assert_eq!(
        runtime(fixture),
        (
            "s-1".to_owned(),
            provider.incarnation.pid() as i64,
            provider.incarnation.start_identity().to_owned(),
            "running".to_owned(),
        ),
        "{case}: stored Codex runtime"
    );
}

#[test]
fn a_codex_options_context_notice_keeps_recorded_admission_required() {
    register();
    let fixture = Fixture::new("codex-options-context-notice");
    let record = fixture.approve();
    bind(&fixture, record);
    let mut codex =
        SteppedProvider::start_with_words(&fixture, "codex", "codex-steps", &["-c", "model=x"]);
    let provider = live_provider(&codex);
    let output = codex.hook(codex_event("SessionStart", None));
    assert_options_admitted(&fixture, &provider, output, "real hook");
    assert_options_admitted(
        &fixture,
        &provider,
        (0, String::new(), HOOK_UNAVAILABLE.to_owned()),
        "context unavailable",
    );
    drop(codex);
    assert_eq!(provider.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

fn assert_options_refused(
    fixture: &Fixture,
    provider: &Owned,
    output: (i32, String, String),
    reason: &str,
) {
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_options_admitted(fixture, provider, output, "negative control");
    }))
    .expect_err("invalid Codex options admission was accepted");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap();
    assert!(message.contains(reason), "unexpected refusal: {message}");
}

#[test]
fn codex_options_still_reject_failed_or_unexpected_hook_output() {
    register();
    let fixture = Fixture::new("codex-options-bad-output");
    let record = fixture.approve();
    bind(&fixture, record);
    let mut codex =
        SteppedProvider::start_with_words(&fixture, "codex", "codex-steps", &["-c", "model=x"]);
    let provider = live_provider(&codex);
    let output = codex.hook(codex_event("SessionStart", None));
    assert_options_admitted(&fixture, &provider, output, "real hook");
    let rows = stored_rows(&fixture);
    for (status, stderr, reason) in [
        (1, "".to_owned(), "hook exit"),
        (1, HOOK_UNAVAILABLE.to_owned(), "hook exit"),
        (0, "unexpected warning\n".to_owned(), "hook stderr"),
        (0, HOOK_UNAVAILABLE.trim_end().to_owned(), "hook stderr"),
        (0, HOOK_UNAVAILABLE.repeat(2), "hook stderr"),
    ] {
        assert_options_refused(&fixture, &provider, (status, String::new(), stderr), reason);
        assert_eq!(stored_rows(&fixture), rows);
    }
    drop(codex);
    assert_eq!(provider.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

#[test]
fn a_codex_options_context_notice_cannot_replace_session_admission() {
    register();
    let fixture = Fixture::new("codex-options-no-admission");
    let record = fixture.approve();
    bind(&fixture, record);
    let codex =
        SteppedProvider::start_with_words(&fixture, "codex", "codex-steps", &["-c", "model=x"]);
    let provider = live_provider(&codex);
    // Live and bound, but no SessionStart has been admitted.
    let rows = stored_rows(&fixture);
    assert_options_refused(
        &fixture,
        &provider,
        (0, String::new(), HOOK_UNAVAILABLE.to_owned()),
        "stored Codex runtime",
    );
    assert_eq!(stored_rows(&fixture), rows);
    drop(codex);
    assert_eq!(provider.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

/// A hand-started provider has no launch owner and missed SessionStart before
/// its pane was bound. Its first installed prompt/Stop supplies the exact ID.
fn assert_first_hook_adopts(provider: &'static str, event: &str) {
    const SESSION: &str = "4f8c2a7e-0d3b-4c61-9a52-7b1e6d0c9f13";
    register();
    let fixture = Fixture::new(&format!("first-hook-{provider}-{event}"));
    let record = fixture.approve();
    bind(&fixture, record);
    let mut agent = SteppedProvider::start(&fixture, provider, "agent-steps");
    let process = live_provider(&agent);
    assert_eq!(
        runtime(&fixture),
        (String::new(), 0, String::new(), "unknown".into())
    );
    let pid = pid_of(&process);
    let environment = if provider == "claude" {
        vec![("CLAUDE_PID", pid.as_slice())]
    } else {
        vec![]
    };
    let mut payload = codex_event(event, Some("turn-a"));
    payload["session_id"] = SESSION.into();
    let (status, _, stderr) = agent.hook_with_env(payload, &environment);
    assert_eq!(status, 0, "{stderr}");
    assert!(stderr.is_empty() || stderr == HOOK_UNAVAILABLE, "{stderr}");
    assert_eq!(
        runtime(&fixture),
        (
            SESSION.into(),
            process.incarnation.pid() as i64,
            process.incarnation.start_identity().into(),
            "running".into()
        )
    );
    let stored =
        Storage::context_by_provider_session(&fixture.state().join("tmt.db"), provider, SESSION, 1)
            .unwrap()
            .expect("first hook remembers the session");
    assert_eq!(stored.entry.binding.unwrap().session.launch_owner, None);
    let remembered = stored.preferences.remembered.unwrap();
    assert_eq!(remembered.harness.as_str(), provider);
    assert_eq!(remembered.provider_session.as_str(), SESSION);
    drop(agent);
    assert_eq!(process.seen(), Seen::Ended);
    // The public resume path consumes the coordinates learned by the hook.
    // Replace our ended shell link with a bounded stand-in, never a real agent.
    let executable = fixture.root.join("bin").join(provider);
    fs::remove_file(&executable).unwrap();
    fs::write(
        &executable,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > resume.args\n{AGENT}\n"),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = fixture.command_in(Some("f1"), &["resume", "worker"]);
    command.env(
        "PATH",
        format!(
            "{}:/usr/bin:/bin:/usr/sbin:/sbin",
            fixture.root.join("bin").display()
        ),
    );
    let mut resumed = Launched::start(&fixture, command);
    wait_until("resume starts the fake provider or refuses", || {
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
        "resume refused: {}",
        fs::read_to_string(fixture.root.join("launched.err")).unwrap()
    );
    resumed.adopt("agent.pid", false);
    let args = fs::read_to_string(fixture.root.join("resume.args")).unwrap();
    let args: Vec<_> = args.lines().collect();
    if provider == "claude" {
        assert_eq!(&args[..2], &["--resume", SESSION]);
    } else {
        assert_eq!(args.first(), Some(&"resume"));
        assert!(
            args.windows(2).any(|pair| pair == [SESSION, "--no-daemon"]),
            "{args:?}"
        );
    }
    let (code, stderr) = resumed.finish();
    assert_eq!(code, Some(7), "{stderr}");
    assert_drivers_ended(&fixture);
}

#[test]
fn first_hook_adopts_hand_started_claude_on_prompt() {
    assert_first_hook_adopts("claude", "UserPromptSubmit");
}
#[test]
fn first_hook_adopts_hand_started_claude_on_stop() {
    assert_first_hook_adopts("claude", "Stop");
}
#[test]
fn first_hook_adopts_hand_started_codex_on_prompt() {
    assert_first_hook_adopts("codex", "UserPromptSubmit");
}
#[test]
fn first_hook_adopts_hand_started_codex_on_stop() {
    assert_first_hook_adopts("codex", "Stop");
}

/// Optional first-hook learning grants nothing to nested or unverified callers.
#[test]
fn first_hook_adoption_requires_the_main_native_provider() {
    register();
    for provider in ["claude", "codex"] {
        for event in ["UserPromptSubmit", "Stop"] {
            for case in ["nested", "unverified"] {
                let fixture = Fixture::new(&format!("first-hook-{provider}-{event}-{case}"));
                let record = fixture.approve();
                bind(&fixture, record);
                let mut agent = if case == "nested" {
                    SteppedProvider::start_nested(&fixture, provider, "steps", &[provider])
                } else {
                    SteppedProvider::start(&fixture, provider, "steps")
                };
                let rows = stored_rows(&fixture);
                let payload = codex_event(event, Some("turn-a"));
                // A real main-provider ancestor grants nothing without a
                // verified bound pane for this hook invocation.
                let output = if case == "unverified" {
                    agent.hook_with_env(payload, &[("FAKE_PANE_ID", b"missing")])
                } else {
                    agent.hook(payload)
                };
                assert_eq!(output.0, 0, "{provider}/{event}/{case}: {output:?}");
                assert_eq!(stored_rows(&fixture), rows, "{provider}/{event}/{case}");
                drop(agent);
                assert_drivers_ended(&fixture);
            }
        }
    }
}

/// A prompt/Stop may fill only a never-observed binding. It neither swaps
/// conversations nor revives one whose SessionEnd was already admitted.
#[test]
fn first_hook_adoption_never_replaces_or_resurrects_an_observed_session() {
    register();
    for provider in ["claude", "codex"] {
        for event in ["UserPromptSubmit", "Stop"] {
            for case in ["different-session", "ended"] {
                let fixture = Fixture::new(&format!("first-hook-{provider}-{event}-{case}"));
                let record = fixture.approve();
                bind(&fixture, record);
                let mut agent = SteppedProvider::start(&fixture, provider, "steps");
                let output = agent.hook(codex_event("SessionStart", None));
                assert_eq!((output.0, output.2.as_str()), (0, ""));
                if case == "ended" {
                    let mut end = codex_event("SessionEnd", None);
                    end["reason"] = "other".into();
                    assert_eq!(agent.hook(end).0, 0);
                }
                let before = runtime(&fixture);
                let mut payload = codex_event(event, Some("turn-a"));
                if case == "different-session" {
                    payload["session_id"] = "other-session".into();
                }
                assert_eq!(agent.hook(payload).0, 0);
                assert_eq!(runtime(&fixture), before, "{provider}/{event}/{case}");
                drop(agent);
                assert_drivers_ended(&fixture);
            }
        }
    }
}
