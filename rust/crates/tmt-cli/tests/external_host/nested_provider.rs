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
        let (status, _, stderr) = codex.hook(codex_event("SessionStart", None));
        assert_eq!((status, stderr.as_str()), (0, ""), "{words:?}");
        assert_eq!(
            runtime(&fixture).1,
            process.incarnation.pid() as i64,
            "{words:?}"
        );
        drop(codex);
        assert_eq!(process.seen(), Seen::Ended, "{words:?}");
        assert_drivers_ended(&fixture);
    }
}
