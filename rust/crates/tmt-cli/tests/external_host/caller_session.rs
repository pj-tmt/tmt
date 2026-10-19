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

/// How many times a scenario may start over after its worker ran out of budget.
const ATTEMPTS: usize = 5;

/// What one run of a scenario came to.
enum Attempt {
    Passed,
    /// A command's worker used up its fixed 500 ms budget under machine load, so
    /// the scenario never reached the refusal or admission it checks.
    OutOfBudget,
}

/// Run a scenario again, whole and on a fresh fixture, while a command in it
/// ran out of the worker's budget (#3860). Running out is the product's
/// correct best-effort answer to load, remembered as the `budget` layer, not a
/// result of the scenario. Any other mismatch fails inside the run at once, so
/// a refusal that is wrong is never retried. Nothing sleeps and the product
/// budget is unchanged.
fn under_load(scenario: &str, mut run: impl FnMut(&str) -> Attempt) {
    for attempt in 1..=ATTEMPTS {
        match run(&format!("{scenario}-{attempt}")) {
            Attempt::Passed => return,
            Attempt::OutOfBudget => {
                eprintln!(
                    "{scenario}: attempt {attempt} of {ATTEMPTS} ran out of the worker budget"
                );
            }
        }
    }
    panic!("{scenario}: the worker ran out of its budget in all {ATTEMPTS} attempts");
}

/// Whether a command in this fixture's scenario left a `budget` hint.
fn out_of_budget(fixture: &Fixture) -> bool {
    hints(fixture).iter().any(|layer| layer == "budget")
}

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
    under_load("caller-session-records", |name| {
        let fixture = Fixture::new(name);
        let record = fixture.approve();
        bind(&fixture, record);
        let expected = fixture.json(&["config", "show", "--json"]);
        let mut claude = SteppedProvider::start(&fixture, "claude", "claude-steps");
        let provider = live_provider(&claude);
        let pid = provider.incarnation.pid();
        let before = fixture.calls().len();
        let (status, stdout, stderr) = run_command(&mut claude, SESSION, pid);
        let calls = fixture.calls()[before..].to_vec();
        let session = recorded(&fixture, "fake-1");
        let live = provider.seen();
        let starved = out_of_budget(&fixture);
        if !starved {
            assert_eq!((status, stderr.as_str()), (0, ""));
            assert_eq!(serde_json::from_str::<Value>(&stdout).unwrap(), expected);
            assert_eq!(calls, ["caller", "server", "snapshot"]);
            assert_eq!(
                session,
                [SESSION.to_owned(), pid.to_string(), "running".into()]
            );
            assert_eq!(live, Seen::Live);
        }
        drop(claude);
        assert_eq!(provider.seen(), Seen::Ended);
        assert_drivers_ended(&fixture);
        if starved {
            Attempt::OutOfBudget
        } else {
            Attempt::Passed
        }
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
    under_load("caller-session-two-agents", |name| {
        let fixture = Fixture::new(name);
        let record = fixture.approve();
        bind(&fixture, record.clone());
        let (mut claude, shell) = second_pane(&fixture);
        bind_pane(&fixture, record, "fake-2", "other");
        let provider = live_provider(&claude);
        let first = owned_rows(&fixture, "fake-1");
        let before = fixture.calls().len();
        let (status, _, stderr) = run_command(&mut claude, SESSION, provider.incarnation.pid());
        let calls = fixture.calls()[before..].to_vec();
        let second = recorded(&fixture, "fake-2");
        let after = owned_rows(&fixture, "fake-1");
        let live = provider.seen();
        let starved = out_of_budget(&fixture);
        if !starved {
            assert_eq!((status, stderr.as_str()), (0, ""));
            assert_eq!(calls, ["caller", "server", "snapshot"]);
            assert_eq!(
                second,
                [
                    SESSION.to_owned(),
                    provider.incarnation.pid().to_string(),
                    "running".into()
                ]
            );
            assert_eq!(after, first, "the agent in f1 is untouched");
            assert_eq!(live, Seen::Live);
        }
        drop(claude);
        assert_eq!((provider.seen(), shell.seen()), (Seen::Ended, Seen::Ended));
        assert_drivers_ended(&fixture);
        if starved {
            Attempt::OutOfBudget
        } else {
            Attempt::Passed
        }
    });
}

/// The behaviour this precheck adds: a caller in an unbound pane, on a host
/// where another pane is bound, starts one refused attempt. It changes
/// nothing, and its kept hint suppresses the next one.
#[test]
fn an_unbound_external_caller_makes_one_refused_attempt_then_a_hint_suppresses_it() {
    register();
    under_load("caller-session-unbound", |name| {
        let fixture = Fixture::new(name);
        let record = fixture.approve();
        bind(&fixture, record);
        let (mut claude, shell) = second_pane(&fixture);
        let provider = live_provider(&claude);
        let rows = stored_rows(&fixture);
        let mut attempts = Vec::new();
        for _ in 0..2 {
            let before = fixture.calls().len();
            let (status, _, stderr) = run_command(&mut claude, SESSION, provider.incarnation.pid());
            attempts.push((
                status,
                stderr,
                fixture.calls()[before..].to_vec(),
                hints(&fixture),
                stored_rows(&fixture) == rows,
            ));
        }
        let starved = out_of_budget(&fixture);
        if !starved {
            assert_eq!(
                attempts,
                [
                    (
                        0,
                        String::new(),
                        vec!["caller".to_owned(), "server".into(), "snapshot".into()],
                        vec!["native-admission".to_owned()],
                        true
                    ),
                    (
                        0,
                        String::new(),
                        vec![],
                        vec!["native-admission".to_owned()],
                        true
                    ),
                ]
            );
            assert_eq!(provider.seen(), Seen::Live);
        }
        drop(claude);
        assert_eq!((provider.seen(), shell.seen()), (Seen::Ended, Seen::Ended));
        assert_drivers_ended(&fixture);
        if starved {
            Attempt::OutOfBudget
        } else {
            Attempt::Passed
        }
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
            Some("native-admission"),
            &["caller", "server"][..],
        ),
        (
            1,
            Break::StaleMarker,
            Some("binding"),
            &["caller", "server", "snapshot"],
        ),
        (
            2,
            Break::WrongProvider,
            Some("native-admission"),
            &["caller", "server", "snapshot"],
        ),
        (3, Break::Unapproved, None, &[]),
    ] {
        under_load(&format!("caller-session-refused-{index}"), |name| {
            let fixture = Fixture::new(name);
            let record = fixture.approve();
            bind(&fixture, record);
            let mut claude = SteppedProvider::start(&fixture, "claude", "claude-steps");
            let provider = live_provider(&claude);
            let pid = provider.incarnation.pid();
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
            let rows = stored_rows(&fixture);
            let before = fixture.calls().len();
            let (status, _, stderr) = run_command(&mut claude, SESSION, claimed);
            let refused = (
                status,
                stderr,
                fixture.calls()[before..].to_vec(),
                hints(&fixture),
                stored_rows(&fixture) == rows,
                recorded(&fixture, "fake-1")[0].clone(),
            );
            let mut starved = out_of_budget(&fixture);
            if !starved {
                assert_eq!(
                    refused,
                    (
                        0,
                        String::new(),
                        calls.iter().map(|call| (*call).to_owned()).collect(),
                        layer.iter().map(|layer| (*layer).to_owned()).collect(),
                        true,
                        String::new()
                    ),
                    "{broken:?}"
                );
                // Restored, the same setup admits a new session.
                fs::write(fixture.root.join("driver/server"), &server).unwrap();
                fs::write(fixture.root.join("driver/marker"), &marker).unwrap();
                if let Break::Unapproved = broken {
                    fixture.approve();
                }
                let (status, _, stderr) = run_command(&mut claude, CONTROL, pid);
                starved = out_of_budget(&fixture);
                if !starved {
                    assert_eq!((status, stderr.as_str()), (0, ""), "{broken:?}");
                    assert_eq!(recorded(&fixture, "fake-1")[0], CONTROL, "{broken:?}");
                    assert_eq!(provider.seen(), Seen::Live);
                }
            }
            drop(claude);
            assert_eq!(provider.seen(), Seen::Ended);
            assert_drivers_ended(&fixture);
            if starved {
                Attempt::OutOfBudget
            } else {
                Attempt::Passed
            }
        });
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
    let [driver_pid, worker_pid, group]: [i32; 3] = held
        .split_whitespace()
        .map(|field| field.parse().unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    // A generous bound: the worker is stopped at its deadline, and the
    // process owner then has its own cleanup budget.
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
