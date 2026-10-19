//! Caller-session discovery after an ordinary command on an external host
//! (#3580): the host's caller variable holds a target, not a pane ID, so the
//! precheck has no pane locator. It only checks that the host has a binding
//! and starts the worker, which resolves the pane through the driver.
//!
//! Each scenario runs `tmt config show --json`, which reads only storage,
//! from a live stand-in Claude, then reads the database and the refusal
//! cache read only while that provider still runs.
use super::*;

/// Canonical session UUIDs, as Claude sets `CLAUDE_CODE_SESSION_ID`.
const SESSION: &str = "6a0f3c1e-5b2d-4e8a-9c7f-1d2e3f4a5b6c";
const CONTROL: &str = "0b9e8d7c-6a5f-4e3d-8c2b-1a0f9e8d7c6b";

/// The command an agent runs, with its Claude coordinates.
fn run_command(claude: &mut SteppedProvider, session: &str, pid: u64) -> (i32, String, String) {
    claude.command(
        &[
            ("CLAUDE_CODE_SESSION_ID", session),
            ("CLAUDE_PID", &pid.to_string()),
        ],
        &["config", "show", "--json"],
    )
}

/// The session state a binding records, read only.
fn recorded(fixture: &Fixture, pane: &str) -> Vec<String> {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.state().join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection
        .query_row(
            "SELECT coalesce(observed_provider_session_id, ''), coalesce(runtime_pid, 0), \
             runtime_state FROM bindings WHERE pane_id = ?1",
            [pane],
            |row| {
                Ok(vec![
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?.to_string(),
                    row.get::<_, String>(2)?,
                ])
            },
        )
        .unwrap()
}

#[test]
fn a_caller_in_an_external_pane_records_its_session_after_a_command() {
    register();
    within_budget("caller-session-records", |fixture| {
        let record = fixture.approve();
        bind(fixture, record);
        let expected = fixture.json(&["config", "show", "--json"]);
        let mut claude = SteppedProvider::start(fixture, "claude", "claude-steps");
        let provider = live_provider(&claude);
        let pid = provider.incarnation.pid();
        let rows = stored_rows(fixture);
        let before = Before::of(fixture);
        let (status, stdout, stderr) = run_command(&mut claude, SESSION, pid);
        let own = before.own(fixture);
        assert_eq!((status, stderr.as_str()), (0, ""));
        assert_eq!(serde_json::from_str::<Value>(&stdout).unwrap(), expected);
        let load = own.budget(fixture, &rows, Some(("fake-1", SESSION, pid)));
        if !load {
            assert_eq!(own.hints, Vec::<String>::new());
            assert_eq!(own.calls, ["caller", "server", "snapshot"]);
            assert_eq!(
                recorded(fixture, "fake-1"),
                [SESSION.to_owned(), pid.to_string(), "running".into()]
            );
        }
        assert_eq!(provider.seen(), Seen::Live);
        drop(claude);
        assert_eq!(provider.seen(), Seen::Ended);
        assert_drivers_ended(fixture);
        load
    });
}

/// Every row that `pane`'s identity or binding owns, in every table, so a
/// scenario can show that it changed nothing of that agent's state.
fn owned_rows(fixture: &Fixture, pane: &str) -> Vec<String> {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.state().join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (identity, binding): (String, String) = connection
        .query_row(
            "SELECT identity_id, id FROM bindings WHERE pane_id = ?1",
            [pane],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut rows = Vec::new();
    for table in tables {
        let columns: Vec<String> = connection
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let mut owners = Vec::new();
        for (column, owner) in [("identity_id", &identity), ("binding_id", &binding)] {
            if columns.iter().any(|name| name == column) {
                owners.push(format!("\"{column}\" = '{owner}'"));
            }
        }
        match table.as_str() {
            "identities" => owners.push(format!("id = '{identity}'")),
            "bindings" => owners.push(format!("id = '{binding}'")),
            _ => {}
        }
        if owners.is_empty() {
            continue;
        }
        let mut select = connection
            .prepare(&format!(
                "SELECT * FROM \"{table}\" WHERE {}",
                owners.join(" OR ")
            ))
            .unwrap();
        let count = select.column_count();
        let mut found = select.query([]).unwrap();
        while let Some(row) = found.next().unwrap() {
            let values: Vec<String> = (0..count)
                .map(|index| format!("{:?}", row.get_ref(index).unwrap()))
                .collect();
            rows.push(format!("{table}: {}", values.join(" | ")));
        }
    }
    rows.sort();
    rows
}

/// The refusal hints the precheck kept: each entry's layer, read only.
fn hints(fixture: &Fixture) -> Vec<String> {
    let Ok(text) = fs::read_to_string(fixture.state().join("caller-session-refusals.json")) else {
        return Vec::new();
    };
    let document: Value = serde_json::from_str(&text).unwrap();
    document["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["layer"].as_str().unwrap().to_owned())
        .collect()
}

/// The worker's budget is fixed at 500 ms, so a loaded host can refuse any
/// command's worker with `budget` (#3860). That is load, not the behaviour
/// under test. A scenario asserts each command at once. For a command whose
/// own hint is `budget`, it asserts only what load cannot change and reports
/// it; then the whole scenario runs again in a fresh fixture, at most five
/// times, and each such attempt is reported. Cleanup runs in every attempt.
fn within_budget(name: &str, scenario: impl Fn(&Fixture) -> bool) {
    for attempt in 1..=5 {
        let fixture = Fixture::new(&format!("{name}-{attempt}"));
        if !scenario(&fixture) {
            return;
        }
        eprintln!("{name}: attempt {attempt} had a command refused for its budget");
    }
    panic!("{name}: every attempt had a command refused for its budget");
}

/// The driver calls and refusal hints a fixture had before one command.
struct Before {
    calls: usize,
    hints: usize,
}

/// What one command added: its own driver calls and its own refusal hint.
struct Own {
    calls: Vec<String>,
    hints: Vec<String>,
}

impl Before {
    fn of(fixture: &Fixture) -> Self {
        Self {
            calls: fixture.calls().len(),
            hints: hints(fixture).len(),
        }
    }

    fn own(&self, fixture: &Fixture) -> Own {
        Own {
            calls: fixture.calls()[self.calls..].to_vec(),
            hints: hints(fixture)[self.hints..].to_vec(),
        }
    }
}

/// The session that an admissible command records: pane, session and PID.
type Admission<'a> = (&'a str, &'a str, u64);

impl Own {
    /// Whether load refused this command: its only hint is `budget`. Then it
    /// still must have changed no stored row, unless the command is
    /// admissible: the supervisor's deadline can pass after its worker's
    /// commit, so the hint can follow that whole admission. Then only the
    /// pane's binding, its preferences and the change cursor change, to
    /// exactly that session.
    fn budget(&self, fixture: &Fixture, rows: &[String], admitted: Option<Admission<'_>>) -> bool {
        let load = self.hints == ["budget"];
        if !load {
            return false;
        }
        let now = stored_rows(fixture);
        match admitted {
            Some((pane, session, pid)) if now != rows => {
                assert_eq!(
                    recorded(fixture, pane),
                    [session.to_owned(), pid.to_string(), "running".into()],
                    "a budget refusal after its commit recorded another session"
                );
                let others = |rows: &[String]| -> Vec<String> {
                    rows.iter()
                        .filter(|row| {
                            ![
                                "bindings: ",
                                "identity_session_preferences: ",
                                "change_cursor: ",
                            ]
                            .iter()
                            .any(|table| row.starts_with(table))
                        })
                        .cloned()
                        .collect()
                };
                assert_eq!(
                    others(&now),
                    others(rows),
                    "a budget refusal after its commit changed another row"
                );
            }
            _ => assert_eq!(now, rows, "a budget refusal changed a row"),
        }
        true
    }
}

/// A second pane, `fake-2`, with a provider whose pane shell the test owns.
/// The pane shell's own end is checked when the provider is dropped.
fn second_pane(fixture: &Fixture) -> (SteppedProvider, Owned) {
    fs::write(fixture.root.join("driver/two-panes"), "").unwrap();
    let claude = SteppedProvider::start_in(fixture, "claude", "f2-steps", "f2", true);
    let shell = live_process(u64::from(claude.child.id()));
    fs::write(
        fixture.root.join("driver/pid-2"),
        claude.child.id().to_string(),
    )
    .unwrap();
    (claude, shell)
}

/// With two agents on one server, the caller in `f2` records only its own
/// session: every row of the agent in `f1` stays as it was.
#[test]
fn two_agents_on_one_external_server_never_record_for_each_other() {
    register();
    within_budget("caller-session-two-agents", |fixture| {
        let record = fixture.approve();
        bind(fixture, record.clone());
        let (mut claude, shell) = second_pane(fixture);
        bind_pane(fixture, record, "fake-2", "other");
        let provider = live_provider(&claude);
        let pid = provider.incarnation.pid();
        let first = owned_rows(fixture, "fake-1");
        let rows = stored_rows(fixture);
        let before = Before::of(fixture);
        let (status, _, stderr) = run_command(&mut claude, SESSION, pid);
        let own = before.own(fixture);
        assert_eq!((status, stderr.as_str()), (0, ""));
        assert_eq!(
            owned_rows(fixture, "fake-1"),
            first,
            "the agent in f1 is untouched"
        );
        let load = own.budget(fixture, &rows, Some(("fake-2", SESSION, pid)));
        if !load {
            assert_eq!(own.hints, Vec::<String>::new());
            assert_eq!(own.calls, ["caller", "server", "snapshot"]);
            assert_eq!(
                recorded(fixture, "fake-2"),
                [SESSION.to_owned(), pid.to_string(), "running".into()]
            );
        }
        assert_eq!(provider.seen(), Seen::Live);
        drop(claude);
        assert_eq!((provider.seen(), shell.seen()), (Seen::Ended, Seen::Ended));
        assert_drivers_ended(fixture);
        load
    });
}

/// The behaviour this precheck adds: a caller in an unbound pane, on a host
/// where another pane is bound, starts one refused attempt. It changes
/// nothing, and its kept hint suppresses the next one.
#[test]
fn an_unbound_external_caller_makes_one_refused_attempt_then_a_hint_suppresses_it() {
    register();
    within_budget("caller-session-unbound", |fixture| {
        let record = fixture.approve();
        bind(fixture, record);
        let (mut claude, shell) = second_pane(fixture);
        let provider = live_provider(&claude);
        let pid = provider.incarnation.pid();
        let rows = stored_rows(fixture);
        let before = Before::of(fixture);
        let (status, _, stderr) = run_command(&mut claude, SESSION, pid);
        let own = before.own(fixture);
        assert_eq!((status, stderr.as_str()), (0, ""));
        assert_eq!(stored_rows(fixture), rows);
        let load = own.budget(fixture, &rows, None);
        if !load {
            assert_eq!(own.hints, ["native-admission"]);
            assert_eq!(own.calls, ["caller", "server", "snapshot"]);
        }
        // Whatever the first attempt's layer, its kept hint suppresses the
        // second: no driver call, no new hint, nothing changed.
        let before = Before::of(fixture);
        let (status, _, stderr) = run_command(&mut claude, SESSION, pid);
        let own = before.own(fixture);
        assert_eq!((status, stderr.as_str()), (0, ""));
        assert_eq!((own.calls, own.hints), (Vec::new(), Vec::new()));
        assert_eq!(stored_rows(fixture), rows);
        assert_eq!(provider.seen(), Seen::Live);
        drop(claude);
        assert_eq!((provider.seen(), shell.seen()), (Seen::Ended, Seen::Ended));
        assert_drivers_ended(fixture);
        load
    });
}

/// How a refusal scenario breaks the fixture, and restores it.
#[derive(Clone, Copy, Debug)]
enum Break {
    /// The driver's `server` names another live process.
    MissingServer,
    /// The pane's marker names another binding.
    StaleMarker,
    /// `CLAUDE_PID` names a live process that is not the provider.
    WrongProvider,
    /// `tmt driver rm fake`.
    Unapproved,
}

/// Each refusal changes no stored row and keeps a hint at its layer. After
/// the fixture is restored, a new session is admitted in the same setup, so
/// the refusal is not only an attempt that never ran.
#[test]
fn external_caller_refusals_change_nothing_and_an_admitted_control_follows() {
    register();
    for (index, broken, layer, calls) in [
        (
            0,
            Break::MissingServer,
            &["native-admission"][..],
            &["caller", "server"][..],
        ),
        (
            1,
            Break::StaleMarker,
            &["binding"],
            &["caller", "server", "snapshot"],
        ),
        (
            2,
            Break::WrongProvider,
            &["native-admission"],
            &["caller", "server", "snapshot"],
        ),
        (3, Break::Unapproved, &[], &[]),
    ] {
        within_budget(&format!("caller-session-refused-{index}"), |fixture| {
            refusal_then_control(fixture, broken, Refusal::Expected { layer, calls }, true)
        });
    }
}

/// What the refused command must show when load does not refuse it.
#[derive(Clone, Copy)]
enum Refusal<'a> {
    Expected {
        layer: &'a [&'a str],
        calls: &'a [&'a str],
    },
    /// Its worker is held past its budget, so its own hint is `budget`.
    Held,
}

/// One refusal scenario in `fixture`: break it, run the refused command,
/// restore it (or not), then run the control with a new session. Each
/// command is asserted at once; returns whether load refused one of them.
fn refusal_then_control(
    fixture: &Fixture,
    broken: Break,
    refusal: Refusal<'_>,
    restore: bool,
) -> bool {
    let record = fixture.approve();
    bind(fixture, record);
    let mut claude = SteppedProvider::start(fixture, "claude", "claude-steps");
    let provider = live_provider(&claude);
    let pid = provider.incarnation.pid();
    // The commands' assertions are caught, so that cleanup and its checks
    // below run outside any unwinding, and a cleanup failure fails on its
    // own; then a caught failure is raised again.
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let server = fs::read_to_string(fixture.root.join("driver/server")).unwrap();
        let marker = fs::read_to_string(fixture.root.join("driver/marker")).unwrap();
        let mut claimed = pid;
        match broken {
            Break::MissingServer => {
                let parent = std::os::unix::process::parent_id();
                fs::write(
                    fixture.root.join("driver/server"),
                    format!(r#"{{"ok":{{"server":{{"socket":"/tmp/fake.sock","pid":{parent},"startTime":"driver-says"}}}}}}"#),
                )
                .unwrap();
            }
            Break::StaleMarker => {
                let mut stale: Value = serde_json::from_str(&marker).unwrap();
                stale["bindingId"] = "7d6c5b4a-3f2e-4d1c-8b0a-9f8e7d6c5b4a".into();
                fs::write(fixture.root.join("driver/marker"), stale.to_string()).unwrap();
            }
            Break::WrongProvider => claimed = u64::from(std::process::id()),
            Break::Unapproved => {
                let removed = fixture.tmt(&["driver", "rm", "fake"]);
                assert!(removed.status.success(), "{removed:?}");
            }
        }
        let gate = matches!(refusal, Refusal::Held).then(|| Gate::hold(fixture));
        let rows = stored_rows(fixture);
        let before = Before::of(fixture);
        let (status, _, stderr) = run_command(&mut claude, SESSION, claimed);
        let own = before.own(fixture);
        if let Some(gate) = gate {
            gate.release_after_its_worker_ended();
        }
        assert_eq!((status, stderr.as_str()), (0, ""), "{broken:?}");
        assert_eq!(recorded(fixture, "fake-1")[0], "", "{broken:?}");
        assert_eq!(
            stored_rows(fixture),
            rows,
            "{broken:?}: the refusal changed a row"
        );
        let mut load = own.budget(fixture, &rows, None);
        match refusal {
            Refusal::Held => assert!(
                load,
                "{broken:?}: the held worker was not refused for its budget"
            ),
            Refusal::Expected { layer, calls } if !load => {
                assert_eq!(
                    (own.hints, own.calls),
                    (
                        layer
                            .iter()
                            .map(|layer| (*layer).to_owned())
                            .collect::<Vec<_>>(),
                        calls
                            .iter()
                            .map(|call| (*call).to_owned())
                            .collect::<Vec<_>>()
                    ),
                    "{broken:?}"
                );
            }
            Refusal::Expected { .. } => {}
        }
        if restore {
            fs::write(fixture.root.join("driver/server"), &server).unwrap();
            fs::write(fixture.root.join("driver/marker"), &marker).unwrap();
            if let Break::Unapproved = broken {
                fixture.approve();
            }
        }
        let rows = stored_rows(fixture);
        let before = Before::of(fixture);
        let (status, _, stderr) = run_command(&mut claude, CONTROL, pid);
        let own = before.own(fixture);
        assert_eq!((status, stderr.as_str()), (0, ""), "{broken:?}");
        if !own.budget(fixture, &rows, Some(("fake-1", CONTROL, pid))) {
            assert_eq!(
                (own.hints, recorded(fixture, "fake-1")[0].clone()),
                (Vec::new(), CONTROL.to_owned()),
                "{broken:?}: the control"
            );
        } else {
            load = true;
        }
        load
    }));
    assert_eq!(provider.seen(), Seen::Live, "{broken:?}");
    drop(claude);
    assert_eq!(provider.seen(), Seen::Ended, "{broken:?}");
    assert_drivers_ended(fixture);
    checked.unwrap_or_else(|failure| std::panic::resume_unwind(failure))
}

/// A command refused for its budget never hides another failure in the same
/// attempt: the first command's worker is held past its budget, and the
/// control then runs in a fixture left broken. That attempt fails at once.
#[test]
fn a_semantic_failure_beside_a_budget_refusal_fails_at_once() {
    register();
    let attempts = std::cell::Cell::new(0);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        within_budget("caller-session-mixed", |fixture| {
            attempts.set(attempts.get() + 1);
            refusal_then_control(fixture, Break::MissingServer, Refusal::Held, false)
        });
    }))
    .unwrap_err();
    let message = failed.downcast_ref::<String>().cloned().unwrap_or_default();
    assert_eq!(attempts.get(), 1, "{message}");
    assert!(message.contains("the control"), "{message}");
}

/// The fake driver's `hold-caller` gate: a worker's `caller` call blocks
/// until it is removed. It is removed on every path.
struct Gate(PathBuf);

impl Gate {
    fn hold(fixture: &Fixture) -> Self {
        let gate = fixture.root.join("driver/hold-caller");
        fs::write(&gate, "").unwrap();
        Self(gate)
    }

    /// Waits until the held driver, its worker and their group have ended
    /// (the precheck stops them at the worker's deadline), then opens.
    fn release_after_its_worker_ended(self) {
        let held = fs::read_to_string(self.0.with_file_name("held")).unwrap();
        held_ended(&held);
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Waits, within a generous bound, until the processes a held driver call
/// names (`driver worker group`) are gone: the worker is stopped at its
/// deadline, and the process owner then has its own cleanup budget.
fn held_ended(held: &str) {
    let [driver_pid, worker_pid, group]: [i32; 3] = held
        .split_whitespace()
        .map(|field| field.parse().unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let watchdog = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !(gone(driver_pid)
        && gone(worker_pid)
        && matches!(
            nix::sys::signal::killpg(nix::unistd::Pid::from_raw(group), None),
            Err(nix::errno::Errno::ESRCH)
        ))
    {
        assert!(
            std::time::Instant::now() < watchdog,
            "driver {driver_pid}, worker {worker_pid} or group {group} still runs"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Whether `pid` is gone: `kill(pid, 0)` finds no process. A refused or
/// failed inspection is not absence.
fn gone(pid: i32) -> bool {
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Err(nix::errno::Errno::ESRCH) => true,
        Ok(()) => false,
        Err(error) => panic!("process {pid} could not be inspected: {error}"),
    }
}

/// A worker whose driver call outlives the precheck's budget is stopped
/// with its driver, changes nothing and leaves a `budget` hint. The driver
/// holds its `caller` call only after the command's whole output is
/// written, so the budget is the post-output one.
#[test]
fn an_external_worker_over_its_budget_is_stopped_and_changes_nothing() {
    register();
    let fixture = Fixture::new("caller-session-budget");
    let record = fixture.approve();
    bind(&fixture, record);
    let expected = fixture.json(&["config", "show", "--json"]);
    let mut claude = SteppedProvider::start(&fixture, "claude", "claude-steps");
    let provider = live_provider(&claude);
    let driver = fixture.root.join("driver");
    fs::write(
        driver.join("watch"),
        claude
            .steps
            .join(format!("{}.out", claude.next))
            .to_str()
            .unwrap(),
    )
    .unwrap();
    fs::write(driver.join("hold-caller"), "").unwrap();
    // The gate is released on every path, so a held driver ends by itself.
    struct Release(PathBuf);
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let release = Release(driver.join("hold-caller"));
    let rows = stored_rows(&fixture);
    let (status, stdout, stderr) = run_command(&mut claude, SESSION, provider.incarnation.pid());
    let held = fs::read_to_string(driver.join("held")).unwrap();
    let watched = fs::read_to_string(driver.join("watched")).unwrap();
    let after = (hints(&fixture), stored_rows(&fixture) == rows);
    held_ended(&held);
    drop(release);
    assert_eq!((status, stderr.as_str()), (0, ""));
    assert_eq!(serde_json::from_str::<Value>(&stdout).unwrap(), expected);
    assert_eq!(
        serde_json::from_str::<Value>(&watched).unwrap(),
        expected,
        "the driver was held after the command's whole output"
    );
    assert_eq!(after, (vec!["budget".to_owned()], true));
    assert_eq!(recorded(&fixture, "fake-1")[0], "");
    assert_eq!(provider.seen(), Seen::Live);
    drop(claude);
    assert_eq!(provider.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

/// The pane shell's cleanup does not need the provider's help: a provider
/// that ignores its stop file and SIGHUP still ends, with its pane shell
/// and every other member of the group the test owns.
#[test]
fn a_stuck_provider_and_its_pane_shell_end_with_their_owned_group() {
    use nix::sys::signal::killpg;
    register();
    let fixture = Fixture::new("caller-session-stuck");
    fs::create_dir_all(fixture.root.join("stuck-steps")).unwrap();
    fs::write(fixture.root.join("stuck-steps/hang"), "").unwrap();
    let claude = SteppedProvider::start_in(&fixture, "claude", "stuck-steps", "f1", true);
    let provider = live_provider(&claude);
    let shell = live_process(u64::from(claude.child.id()));
    let group = nix::unistd::Pid::from_raw(i32::try_from(claude.child.id()).unwrap());
    drop(claude);
    assert_eq!(
        (shell.seen(), killpg(group, None)),
        (Seen::Ended, Err(nix::errno::Errno::ESRCH))
    );
    // The killed provider was its shell's child; its new parent reaps it.
    wait_until("the provider ends", || provider.seen() == Seen::Ended);
}
