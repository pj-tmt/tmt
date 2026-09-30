//! A real spawned driver in a private test directory: a small shell script
//! that answers each operation from a file the test wrote, so approval, the
//! calls, the trust checks and the bounds run through actual processes.

use super::{
    CallError, DriverProcess,
    registry::{self, RegistryError},
};
use crate::{
    process::{
        CommandError, CommandFailure, CommandOutput, CommandRequest, CommandRunner,
        UnixCommandRunner,
    },
    test_support::TestDirectory,
};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};
use tmt_driver_protocol::{
    ClearRequest, ClearResponse, Op, SnapshotRequest, SnapshotResponse,
    conformance::{self, DriverOutput, Fixture},
};

const SOCKET: &str = "/tmp/fake-host.sock";

/// Success fixtures exercise real processes without charging CI scheduling to
/// the protocol budget. Only the runner deadline changes, not wire/output bounds.
struct FixtureRunner;

impl CommandRunner for FixtureRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        UnixCommandRunner.execute(CommandRequest {
            deadline: Instant::now() + Duration::from_secs(30),
            ..request
        })
    }
}

/// Answers `__tmt-driver 1 <op>` with the file `<itself>.<op>`, and
/// `unsupported` when there is none. It records `<op> <TMT_DRIVER_CALL>` for
/// every call, and sleeps for `<itself>.sleep` seconds first when present.
const SCRIPT: &str = r#"#!/bin/sh
echo "$3 ${TMT_DRIVER_CALL:-}" >> "$0.calls"
if [ -f "$0.sleep" ]; then sleep "$(cat "$0.sleep")"; fi
if [ "$1" != "__tmt-driver" ] || [ "$2" != "1" ] || [ -z "$3" ]; then
  printf '{"error":{"code":"bad_request","message":"usage"}}'
  exit 2
fi
input=$(cat)
if [ "$3" != capabilities ] && [ "$input" = "{" ]; then
  printf '{"error":{"code":"bad_request","message":"not JSON"}}'
  exit 0
fi
if [ -f "$0.$3" ]; then cat "$0.$3"; else printf '{"error":{"code":"unsupported","message":"no %s"}}' "$3"; fi
"#;

fn not_found() -> Value {
    json!({"error": {"code": "not_found", "message": "no such pane"}})
}

/// A host with a server and no panes, answering every operation correctly
/// for panes and targets that don't exist.
fn answers(name: &str, prefix: &str, target: Option<&str>) -> Vec<(&'static str, Value)> {
    vec![
        (
            "capabilities",
            json!({"ok": {"protocols": [1], "kind": "host", "name": name, "version": "0.0.0-test",
                "ops": ["caller", "server", "resolve-target", "snapshot", "probe", "publish",
                        "clear", "capture", "focus"],
                "paneId": {"prefix": prefix}, "target": target, "callerEnv": []}}),
        ),
        ("caller", json!({"ok": {"pane": null}})),
        (
            "server",
            json!({"ok": {"server": {"socket": SOCKET, "pid": 4242, "startTime": "t0"}}}),
        ),
        ("resolve-target", json!({"ok": {"paneId": null}})),
        ("snapshot", json!({"ok": {"panes": []}})),
        ("probe", json!({"ok": {"state": "dead"}})),
        ("clear", json!({"ok": {"cleared": false}})),
        ("publish", not_found()),
        ("capture", not_found()),
        ("focus", not_found()),
    ]
}

/// A private copy of the script with its own answers.
struct Installed {
    directory: TestDirectory,
    executable: PathBuf,
}

impl Installed {
    fn new(file: &str, answers: Vec<(&'static str, Value)>) -> Self {
        let directory = TestDirectory::new();
        let bin = directory.path.join("bin");
        fs::create_dir(&bin).unwrap();
        let executable = bin.join(file);
        fs::write(&executable, SCRIPT).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let installed = Self {
            directory,
            executable,
        };
        for (op, answer) in answers {
            installed.answer(op, &answer.to_string());
        }
        installed
    }

    fn answer(&self, op: &str, text: &str) {
        fs::write(self.beside(op), text).unwrap();
    }

    fn beside(&self, extension: &str) -> PathBuf {
        let mut name = self.executable.file_name().unwrap().to_owned();
        name.push(".");
        name.push(extension);
        self.executable.with_file_name(name)
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.beside("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn global(&self) -> PathBuf {
        self.directory.path.join("global")
    }

    fn approve(&self) -> Result<registry::DriverRecord, RegistryError> {
        registry::approve(&self.global(), &self.executable, &FixtureRunner)
    }

    fn open(&self) -> DriverProcess<FixtureRunner> {
        DriverProcess::open(self.approve().unwrap(), FixtureRunner).unwrap()
    }
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn clear() -> ClearRequest {
    ClearRequest {
        socket: SOCKET.into(),
        pane_id: "fake-1".into(),
        binding_id: "00000000-0000-4000-8000-000000000000".into(),
    }
}

#[test]
fn a_spawned_driver_is_checked_for_conformance() {
    let installed = Installed::new("fake", answers("fake", "fake-", Some("f{n}")));
    // Elapsed time is an invoker input. Scripted timing separates conformance
    // checks from shell startup; the late-answer test below uses the real clock.
    let late = Cell::new(None::<Op>);
    let mut invoke = |args: &[&str], request: &[u8]| {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let result = FixtureRunner.execute(CommandRequest {
            program: installed.executable.as_os_str(),
            args: &args,
            input: request,
            deadline: soon(),
            max_output_bytes: 2 * 1024 * 1024,
        });
        DriverOutput {
            // A malformed invocation exits 2 with an error answer on stdout.
            stdout: match result {
                Ok(output) => output.stdout,
                Err(error) => error.output.map(|output| output.stdout).unwrap_or_default(),
            },
            elapsed: late
                .get()
                .filter(|op| {
                    args.get(2)
                        .is_some_and(|arg| arg == std::ffi::OsStr::new(op.as_str()))
                })
                .map_or(Duration::ZERO, |op| {
                    op.bounds().deadline + Duration::from_nanos(1)
                }),
        }
    };
    let fixture = Fixture {
        socket: SOCKET.into(),
    };
    assert_eq!(conformance::check(&mut invoke, &fixture), []);
    // The same harness finds a driver that clears a marker on a missing pane.
    installed.answer("clear", r#"{"ok": {"cleared": true}}"#);
    let findings = conformance::check(&mut invoke, &fixture);
    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert_eq!(findings[0].check, "clear");

    installed.answer("clear", r#"{"ok": {"cleared": false}}"#);
    for op in [Op::Capabilities, Op::Clear] {
        late.set(Some(op));
        let findings = conformance::check(&mut invoke, &fixture);
        let expected = if op == Op::Capabilities { 1 } else { 2 };
        assert_eq!(findings.len(), expected, "{findings:#?}");
        assert!(
            findings
                .iter()
                .all(|finding| finding.check == op.as_str() && finding.detail.contains("deadline")),
            "{findings:#?}"
        );
    }
}

#[test]
fn an_approved_driver_is_recorded_privately_and_called_with_the_guard() {
    let installed = Installed::new("fake", answers("fake", "fake-", Some("f{n}")));
    installed.answer(
        "snapshot",
        &json!({"ok": {"panes": [{"id": "fake-1", "target": "f1", "cwd": "/src",
            "command": "agent", "panePid": 77, "suggestedName": null, "marker": null}]}})
        .to_string(),
    );
    let record = installed.approve().unwrap();
    assert_eq!(record.name, "fake");
    assert_eq!(
        record.path,
        fs::canonicalize(&installed.executable).unwrap()
    );
    assert_eq!(
        registry::read(&installed.global()).unwrap(),
        std::slice::from_ref(&record)
    );
    let mode = fs::metadata(registry::registry_path(&installed.global()))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);

    let driver = DriverProcess::open(record, FixtureRunner).unwrap();
    assert!(driver.supports(Op::Snapshot) && !driver.supports(Op::Input));
    let snapshot: SnapshotResponse = driver
        .call(
            SnapshotRequest {
                socket: SOCKET.into(),
                panes: None,
            },
            soon(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.panes.len(), 1);
    assert_eq!(snapshot.panes[0].id, "fake-1");
    // Every call carries the guard, the approval's probe included.
    assert_eq!(installed.calls(), ["capabilities 1", "snapshot 1"]);

    assert!(registry::remove(&installed.global(), "fake").unwrap());
    assert!(!registry::remove(&installed.global(), "fake").unwrap());
    assert_eq!(registry::read(&installed.global()).unwrap(), []);
}

#[test]
fn a_changed_executable_is_not_run_until_approved_again() {
    let installed = Installed::new("fake", answers("fake", "fake-", None));
    let driver = installed.open();
    // Rewriting the file moves its modification and change times.
    fs::write(&installed.executable, format!("{SCRIPT}# changed\n")).unwrap();
    let result = driver.call::<ClearResponse>(clear(), soon());
    assert!(
        matches!(result, Err(CallError::Untrusted { .. })),
        "{result:?}"
    );
    assert_eq!(installed.calls(), ["capabilities 1"], "not run");

    // A record whose digest isn't the file's never opens.
    let mut tampered = installed.approve().unwrap();
    tampered.digest = "0".repeat(64);
    assert!(matches!(
        DriverProcess::open(tampered, UnixCommandRunner),
        Err(CallError::Untrusted { .. })
    ));
    // Neither is an executable anyone else can write approved.
    fs::set_permissions(&installed.executable, fs::Permissions::from_mode(0o757)).unwrap();
    assert!(matches!(installed.approve(), Err(RegistryError::Unsafe(_))));
}

#[test]
fn late_oversized_or_malformed_answers_fail() {
    let installed = Installed::new("fake", answers("fake", "fake-", None));
    let record = installed.approve().unwrap();
    let driver = DriverProcess::open(record.clone(), FixtureRunner).unwrap();
    // This call retains the actual 300 ms protocol deadline and cleanup owner.
    let timed_driver = DriverProcess::open(record, UnixCommandRunner).unwrap();
    assert_eq!(
        driver.call::<ClearResponse>(clear(), soon()).unwrap(),
        Ok(ClearResponse { cleared: false })
    );

    fs::write(installed.beside("sleep"), "1.5").unwrap();
    let started = Instant::now();
    let result = timed_driver.call::<ClearResponse>(clear(), soon());
    assert!(
        matches!(&result, Err(CallError::Process { error, .. })
            if error.kind == CommandFailure::Timeout && !error.cleanup_failed()),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(1200),
        "stopped at clear's 300 ms deadline, took {:?}",
        started.elapsed()
    );
    fs::remove_file(installed.beside("sleep")).unwrap();

    installed.answer("clear", &" ".repeat(5000));
    let result = driver.call::<ClearResponse>(clear(), soon());
    assert!(
        matches!(&result, Err(CallError::Process { error, .. })
            if error.kind == CommandFailure::OutputLimit && !error.cleanup_failed()),
        "over 4 KiB: {result:?}"
    );

    installed.answer("clear", r#"{"ok": {"cleared": true}} trailing"#);
    let result = driver.call::<ClearResponse>(clear(), soon());
    assert!(
        matches!(result, Err(CallError::Decode { .. })),
        "{result:?}"
    );
}

#[test]
fn a_driver_that_could_be_read_as_another_host_is_refused() {
    for (name, prefix, target, why) in [
        ("tmux", "tm-", None, "built-in name"),
        ("herdr", "hd-", None, "built-in name"),
        ("other", "term_", None, "Herdr's pane IDs"),
        ("other", "ot-", Some("w{n}:p{n}"), "Herdr's targets"),
    ] {
        let installed = Installed::new("candidate", answers(name, prefix, target));
        let result = installed.approve();
        assert!(
            matches!(result, Err(RegistryError::Refused(_))),
            "{why}: {result:?}"
        );
        assert_eq!(registry::read(&installed.global()).unwrap(), [], "{why}");
    }
    // Two drivers may not share a prefix; the first stays approved.
    let first = Installed::new("first", answers("first", "fx-", None));
    first.approve().unwrap();
    let second = Installed::new("second", answers("second", "fx-", None));
    let refused = registry::approve(&first.global(), &second.executable, &FixtureRunner);
    assert!(
        matches!(refused, Err(RegistryError::Refused(_))),
        "{refused:?}"
    );
    assert_eq!(registry::read(&first.global()).unwrap().len(), 1);
    // Something that isn't a driver is refused without a record.
    let not_driver = Installed::new("broken", vec![("capabilities", json!("hello"))]);
    assert!(matches!(
        not_driver.approve(),
        Err(RegistryError::Refused(_))
    ));
    assert_eq!(registry::read(&not_driver.global()).unwrap(), []);
}

#[test]
fn fixture_scheduling_does_not_change_the_capabilities_request() {
    // An expired command deadline models work dequeued after scheduler delay.
    // The production runner still refuses it; only the success fixture resets it.
    struct Queued<R>(R);
    impl<R: CommandRunner> CommandRunner for Queued<R> {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            let wire: Value = serde_json::from_slice(request.input).unwrap();
            assert_eq!(wire["deadlineMs"], 1000);
            assert_eq!(request.max_output_bytes, 4096);
            self.0.execute(CommandRequest {
                deadline: Instant::now(),
                ..request
            })
        }
    }
    let installed = Installed::new("fake", answers("fake", "fake-", None));
    let refused = registry::approve(
        &installed.global(),
        &installed.executable,
        &Queued(UnixCommandRunner),
    );
    assert!(
        matches!(refused, Err(RegistryError::Refused(ref reason)) if reason.contains("Timeout")),
        "{refused:?}"
    );
    assert!(
        installed.calls().is_empty(),
        "expired production runner never spawns"
    );
    let record = registry::approve(
        &installed.global(),
        &installed.executable,
        &Queued(FixtureRunner),
    )
    .unwrap();
    assert_eq!(record.name, "fake");
    assert_eq!(installed.calls(), ["capabilities 1"]);
}
