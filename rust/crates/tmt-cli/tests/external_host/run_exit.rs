//! What `tmt run` says when it cannot record its command's exit (#4008): a
//! real `tmt run` owns a waiting command in the fake pane, the test changes
//! the fixture's store after admission, then lets the command exit.
//!
//! A newer tmt that migrated the store, a launch that lost its binding and
//! any other storage failure each get their own warning, and the exit status
//! never changes. The store is read with raw SQL: `Storage::open` refuses an
//! upgraded store, as it should.
use super::*;
use rusqlite::OptionalExtension;

const NEWER: &str = "warning: tmt: command exited, but a newer tmt upgraded identity storage \
    while it ran, so this exit was not recorded; no command was retried\n\
    hint: to continue this pane's remembered session, use the current tmt: tmt resume\n";
const REPLACED: &str = "warning: tmt: command exited, but its final state was not stored \
    because this launch no longer owns the identity's binding; no command was retried\n";
const UNSTORED: &str = "warning: tmt: command exited, but its final state could not be stored: ";

/// The worker binding's session columns, or `None` without a binding.
#[derive(Debug, PartialEq)]
struct Row {
    state: String,
    transition: Option<String>,
    runtime_pid: Option<i64>,
    owner_pid: Option<i64>,
}

fn row(fixture: &Fixture) -> Option<Row> {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.state().join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection
        .query_row(
            "SELECT b.runtime_state, b.last_transition, b.runtime_pid, b.launch_owner_pid \
             FROM bindings AS b JOIN identities AS i ON i.id = b.identity_id \
             WHERE i.name = 'worker'",
            [],
            |row| {
                Ok(Row {
                    state: row.get(0)?,
                    transition: row.get(1)?,
                    runtime_pid: row.get(2)?,
                    owner_pid: row.get(3)?,
                })
            },
        )
        .optional()
        .unwrap()
}

/// Starts `tmt run worker` and waits until its launch is admitted: the row
/// is running with a launch owner, so the run will try its final write.
fn admitted_run(name: &str) -> (Fixture, Launched, Row) {
    register();
    let fixture = Fixture::new(name);
    fixture.approve();
    let run = run_agent(&fixture);
    let mut admitted = None;
    wait_until("the launch is admitted", || {
        admitted = row(&fixture).filter(|row| row.state == "running" && row.owner_pid.is_some());
        admitted.is_some()
    });
    (fixture, run, admitted.unwrap())
}

/// A write to the fixture's own store, as another binary would make it.
fn mutate(fixture: &Fixture, sql: &str) {
    let connection = rusqlite::Connection::open(fixture.state().join("tmt.db")).unwrap();
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    connection.execute_batch(sql).unwrap();
}

fn last_migration(fixture: &Fixture) -> i64 {
    rusqlite::Connection::open_with_flags(
        fixture.state().join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row("SELECT max(version) FROM _migrations", [], |row| row.get(0))
    .unwrap()
}

/// The control: an unchanged store records the exit without a warning.
#[test]
fn an_exit_is_recorded_without_a_warning() {
    let (fixture, run, admitted) = admitted_run("exit-recorded");
    let (code, stderr) = run.finish();
    assert_eq!(code, Some(7), "{stderr}");
    assert!(!stderr.contains("warning:"), "{stderr}");
    assert_eq!(
        row(&fixture),
        Some(Row {
            state: "ended".into(),
            transition: Some("ended".into()),
            ..admitted
        })
    );
}

/// A newer tmt migrated the store while the run was live: the run says so,
/// points to `tmt resume`, and writes nothing.
#[test]
fn an_exit_after_a_newer_tmt_upgraded_storage_says_so_and_writes_nothing() {
    let (fixture, run, _) = admitted_run("exit-newer");
    let next = last_migration(&fixture) + 1;
    mutate(
        &fixture,
        &format!(
            "INSERT INTO _migrations (version, name, applied_at) \
             VALUES ({next}, 'a future migration', '2026-10-20T00:00:00.000Z')"
        ),
    );
    let baseline = row(&fixture);
    let (code, stderr) = run.finish();
    assert_eq!(code, Some(7), "{stderr}");
    assert_eq!(stderr.matches(NEWER).count(), 1, "{stderr}");
    assert!(!stderr.contains(REPLACED), "{stderr}");
    assert!(!stderr.contains(UNSTORED), "{stderr}");
    assert_eq!(
        baseline.as_ref().map(|row| row.state.as_str()),
        Some("running")
    );
    assert_eq!(row(&fixture), baseline);
}

/// The binding went away while the run was live: the run says that it no
/// longer owns it, not that storage failed.
#[test]
fn an_exit_after_the_binding_was_replaced_says_so() {
    let (fixture, run, _) = admitted_run("exit-replaced");
    mutate(&fixture, "DELETE FROM bindings");
    let (code, stderr) = run.finish();
    assert_eq!(code, Some(7), "{stderr}");
    assert_eq!(stderr.matches(REPLACED).count(), 1, "{stderr}");
    assert!(!stderr.contains("upgraded identity storage"), "{stderr}");
    assert!(!stderr.contains(UNSTORED), "{stderr}");
    assert_eq!(row(&fixture), None);
}

/// Storage that fails for another reason: the warning names the cause.
#[test]
fn an_exit_with_unreadable_storage_names_the_cause() {
    let (fixture, run, _) = admitted_run("exit-unreadable");
    let last = last_migration(&fixture);
    mutate(
        &fixture,
        &format!("UPDATE _migrations SET name = 'changed' WHERE version = {last}"),
    );
    let baseline = row(&fixture);
    let (code, stderr) = run.finish();
    assert_eq!(code, Some(7), "{stderr}");
    let cause = format!("{UNSTORED}Migration {last} has changed from changed to ");
    assert_eq!(stderr.matches(&cause).count(), 1, "{stderr}");
    assert!(!stderr.contains("upgraded identity storage"), "{stderr}");
    assert!(!stderr.contains(REPLACED), "{stderr}");
    assert_eq!(row(&fixture), baseline);
}

/// The worker row's activity state as `tmt ls --json` shows it.
fn listed_activity(fixture: &Fixture) -> Value {
    worker(&fixture.json(&["ls", "--json"]))["session"]["activity"]["state"].clone()
}

/// After an exit the old binary could not record, a reader that knows the
/// schema shows the row as ended once both of its processes are gone: the
/// runtime state comes from fresh process evidence, not from the stale row,
/// and the read leaves the stored session observations as they were.
#[test]
fn a_reader_shows_an_unrecorded_exit_as_ended_once_its_processes_are_gone() {
    let (fixture, run, _) = admitted_run("exit-read");
    assert_ne!(listed_activity(&fixture), "ended");
    let next = last_migration(&fixture) + 1;
    let future = format!(
        "INSERT INTO _migrations (version, name, applied_at) \
         VALUES ({next}, 'a future migration', '2026-10-20T00:00:00.000Z')"
    );
    mutate(&fixture, &future);
    let (code, stderr) = run.finish();
    assert_eq!(code, Some(7), "{stderr}");
    assert_eq!(stderr.matches(NEWER).count(), 1, "{stderr}");
    // Simulates the newer binary as the reader: it knows every recorded
    // migration. This is not a two-version compatibility test.
    mutate(
        &fixture,
        &format!("DELETE FROM _migrations WHERE version = {next}"),
    );
    let stale = row(&fixture);
    assert_eq!(
        stale.as_ref().map(|row| row.state.as_str()),
        Some("running")
    );
    let listed = worker(&fixture.json(&["ls", "--json"])).clone();
    assert_eq!(listed["session"]["activity"]["state"], "ended", "{listed}");
    // Supplementary: a plain command never has a running driver; binding
    // presentation tests own the Running-versus-Ended proof of this key.
    assert!(listed.get("runningDriver").is_none(), "{listed}");
    assert_eq!(row(&fixture), stale);
}

/// A native-shaped peer, invoked only by this test's private npm launcher.
#[test]
fn native_peer() {
    if std::env::var_os("TMT_TEST_NPM_NATIVE_PEER").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("TMT_TEST_ROOT").unwrap());
    assert!(root.is_absolute() && std::env::temp_dir().starts_with(&root));
    fs::write(
        root.join("native.staged"),
        format!("{}\n", std::process::id()),
    )
    .unwrap();
    fs::rename(root.join("native.staged"), root.join("native.pid")).unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !root.join("release").exists() {
        assert!(std::time::Instant::now() < end, "native release timed out");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::process::exit(7);
}

#[test]
fn wrapper_exit_does_not_end_a_live_or_stopped_native_or_stale_its_resume() {
    use nix::sys::signal::{Signal, kill};
    use tmt_core::binding::{
        BindingRepository,
        session::{HarnessId, ProviderSessionId, RuntimeMode},
    };
    use tmt_core::identity::IdentityReader;
    for (resuming, stopped, wrapper_exits) in [
        (false, false, false),
        (false, false, true),
        (false, true, true),
        (true, false, true),
        (true, true, true),
    ] {
        register();
        let fixture = Fixture::new(&format!("npm-exit-{resuming}-{stopped}-{wrapper_exits}"));
        fixture.approve();
        let native = fixture.root.join("native/codex");
        fs::create_dir(native.parent().unwrap()).unwrap();
        tmt_test_support::write_executable(
            &native,
            &fs::read(std::env::current_exe().unwrap()).unwrap(),
            0o755,
        )
        .unwrap();
        let package = fixture.root.join("node_modules/@openai/codex");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(
            package.join("package.json"),
            br#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#,
        )
        .unwrap();
        let script = format!(
            r#"#!/usr/bin/env node
const fs = require('node:fs');
const {{spawn}} = require('node:child_process');
const child = spawn('{native}', ['--exact', 'run_exit::native_peer', '--nocapture'], {{stdio:'inherit'}});
process.on('SIGUSR1', () => process.exit(23));
fs.writeFileSync('{root}/launcher.pid', String(process.pid)+'\n');
child.on('exit', (code, signal) => {{ if (signal) process.kill(process.pid,signal); else process.exit(code ?? 1); }});
"#,
            native = native.display(),
            root = fixture.root.display()
        );
        tmt_test_support::write_executable(&package.join("bin/codex.js"), script.as_bytes(), 0o755)
            .unwrap();
        fs::create_dir(fixture.root.join("bin")).unwrap();
        std::os::unix::fs::symlink(package.join("bin/codex.js"), fixture.root.join("bin/codex"))
            .unwrap();
        let node = tmt_adapters::skill_installation::ProviderEnvironment::capture()
            .unwrap()
            .find_command("node")
            .unwrap();
        std::os::unix::fs::symlink(node, fixture.root.join("bin/node")).unwrap();
        let path = std::env::join_paths([
            fixture.root.join("bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
        ])
        .unwrap();
        if resuming {
            fixture.json(&["identity", "create", "worker", "--json"]);
            let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
            let identity = storage.find_identity("worker").unwrap().unwrap();
            storage
                .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|records| {
                    let mut prefs = records.session_preferences(&identity.id)?;
                    prefs.remember(
                        HarnessId::new("codex").unwrap(),
                        RuntimeMode::new(tmt_adapters::drivers::codex::MODE_EMBEDDED).unwrap(),
                        ProviderSessionId::new("11111111-1111-4111-8111-111111111111").unwrap(),
                    );
                    records.set_session_preferences(&identity.id, &prefs)?;
                    Ok(())
                })
                .unwrap();
            storage.close().unwrap();
            let settings = fixture.root.join("home/.codex/hooks.json");
            fs::create_dir_all(settings.parent().unwrap()).unwrap();
            fs::write(settings,serde_json::json!({"hooks":{"SessionStart":[tmt_adapters::runtime::hook_protocol::command_entry("codex",env!("CARGO_BIN_EXE_tmt"))]}}).to_string()).unwrap();
        }
        let args = if resuming {
            vec!["resume", "--no-channel", "--retry", "worker"]
        } else {
            vec!["run", "--no-channel", "worker", "codex"]
        };
        let mut command = fixture.command_in(Some("f1"), &args);
        command
            .env("PATH", path)
            .env("TMT_TEST_NPM_NATIVE_PEER", "1")
            .env("NODE_DISABLE_COMPILE_CACHE", "1");
        let mut run = Launched::start(&fixture, command);
        run.adopt("native.pid", false);
        run.adopt("launcher.pid", false);
        let native_pid = run.owned[0].incarnation.pid();
        wait_until("the exact native foreground is admitted", || {
            row(&fixture)
                .is_some_and(|r| r.state == "running" && r.runtime_pid == Some(native_pid as i64))
        });
        let baseline = row(&fixture);
        if !wrapper_exits {
            let (status, stderr) = run.finish();
            assert_eq!(status, Some(7), "{stderr}");
            assert_eq!(row(&fixture).unwrap().state, "ended");
            continue;
        }
        if stopped {
            assert_eq!(run.owned[0].seen(), Seen::Live);
            kill(run.owned[0].pid(), Signal::SIGSTOP).unwrap();
            wait_until("native stopped", || run.owned[0].seen() == Seen::Stopped);
        }
        assert_eq!(run.owned[1].seen(), Seen::Live);
        kill(run.owned[1].pid(), Signal::SIGUSR1).unwrap();
        let status = support::wait(&mut run.child, std::time::Duration::from_secs(20))
            .wait()
            .unwrap();
        assert_eq!(status.code(), Some(23));
        assert_eq!(
            run.owned[0].seen(),
            if stopped { Seen::Stopped } else { Seen::Live }
        );
        assert_eq!(
            row(&fixture),
            baseline,
            "wrapper exit must not end native state"
        );
        if resuming {
            let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
            let identity = storage.find_identity("worker").unwrap().unwrap();
            let remembered = storage
                .session_preferences(&identity.id)
                .unwrap()
                .remembered
                .unwrap();
            assert!(remembered.stale_at_ms.is_none());
            assert!(remembered.resume_pending_at_ms.is_some());
            storage.close().unwrap();
        }
        if stopped {
            kill(run.owned[0].pid(), Signal::SIGCONT).unwrap();
        }
        fs::write(fixture.root.join("release"), "").unwrap();
        run.assert_owned_gone();
    }
}
