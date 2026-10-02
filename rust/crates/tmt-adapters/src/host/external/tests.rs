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
    ffi::{OsStr, OsString},
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

/// Keep the production env/guard request while reading the fixture through its shell.
struct ScriptRunner;

impl CommandRunner for ScriptRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        assert_eq!(request.program, OsStr::new("/usr/bin/env"));
        assert_eq!(request.args[0], OsStr::new("TMT_DRIVER_CALL=1"));
        // A concurrent fork can inherit the written script's fd; only read that inode.
        let mut args = request.args.to_vec();
        args.insert(1, "/bin/sh".into());
        UnixCommandRunner.execute(CommandRequest {
            args: &args,
            ..request
        })
    }
}

/// Allow success fixtures their existing CI scheduling budget, while ScriptRunner
/// retains real process cleanup and unchanged wire/output bounds.
struct FixtureRunner;

impl CommandRunner for FixtureRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        ScriptRunner.execute(CommandRequest {
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
        let mut shell_args = vec![installed.executable.clone().into_os_string()];
        shell_args.extend_from_slice(&args);
        let result = UnixCommandRunner.execute(CommandRequest {
            program: OsStr::new("/bin/sh"),
            args: &shell_args,
            input: request,
            deadline: Instant::now() + Duration::from_secs(30),
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
    let timed_driver = DriverProcess::open(record, ScriptRunner).unwrap();
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
    // A built-in host's name.
    let installed = Installed::new("candidate", answers("tmux", "tm-", None));
    let result = installed.approve();
    assert!(
        matches!(result, Err(RegistryError::Refused(_))),
        "{result:?}"
    );
    assert_eq!(registry::read(&installed.global()).unwrap(), []);
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

// ---- Stored bindings on an external host, through its driver (3b-2a-2) ----

/// Runs driver calls through the fixture's shell and everything else (the
/// core's own `ps`) as production does.
struct HostRunner;

impl CommandRunner for HostRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        if request.args.first().map(OsString::as_os_str) == Some(OsStr::new("TMT_DRIVER_CALL=1")) {
            FixtureRunner.execute(request)
        } else {
            UnixCommandRunner.execute(request)
        }
    }
}

mod through_the_driver {
    use super::*;
    use crate::host::{HostError, external::Drivers};
    use tmt_core::{
        binding::{Binding, session::BindingSessionState},
        endpoint::{EndpointProbe, ServerEvidence},
        host::{HostKind, HostName, HostServerIds, HostServerIncarnation},
        identity::{Identity, Lifetime},
    };

    const SERVER_ID: &str = "11111111-1111-4111-8111-111111111111";
    const BINDING_ID: &str = "22222222-2222-4222-8222-222222222222";
    const IDENTITY_ID: &str = "33333333-3333-4333-8333-333333333333";

    fn fake() -> HostName {
        HostName::new("fake").unwrap()
    }

    /// This test process: a live pid with a start core can observe.
    fn live() -> (u64, String) {
        let pid = u64::from(std::process::id());
        let start = crate::process::runtime::observe_start(&UnixCommandRunner, pid, soon())
            .unwrap()
            .expect("the test process is observable");
        (pid, start)
    }

    /// A pid whose process exited and was reaped.
    fn gone() -> u64 {
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = u64::from(child.id());
        child.wait().unwrap();
        pid
    }

    fn server(pid: u64, start: &str) -> ServerEvidence {
        ServerEvidence {
            host: HostKind::External(fake()),
            server_id: SERVER_ID.into(),
            socket_path: SOCKET.into(),
            server_pid: pid,
            server_start_time: start.into(),
        }
    }

    fn identity() -> Identity {
        Identity {
            id: IDENTITY_ID.into(),
            name: "worker".into(),
            canonical_name: "worker".into(),
            lifetime: Lifetime::Saved,
            created_at: "created".into(),
            updated_at: "updated".into(),
        }
    }

    fn binding(server: ServerEvidence, pane_pid: u64) -> Binding {
        Binding {
            id: BINDING_ID.into(),
            identity_id: IDENTITY_ID.into(),
            server,
            pane_id: "fake-1".into(),
            pane_pid,
            pane_incarnation: None,
            session: BindingSessionState::default(),
        }
    }

    /// A driver whose one pane runs `pane_pid` and carries the binding's
    /// marker.
    fn installed(pane_pid: u64) -> Installed {
        let installed = Installed::new("fake", answers("fake", "fake-", Some("f{n}")));
        let marker = binding(server(1, "s"), pane_pid).marker(&identity());
        installed.answer(
            "snapshot",
            &json!({"ok": {"panes": [{"id": "fake-1", "target": "f1", "cwd": "/src",
                "command": "claude", "panePid": pane_pid, "suggestedName": null,
                "marker": {"name": marker.name, "canonicalName": marker.canonical_name,
                    "identityId": marker.identity_id, "bindingId": marker.binding_id,
                    "serverId": SERVER_ID, "panePid": pane_pid}}]}})
            .to_string(),
        );
        installed
    }

    fn drivers(installed: &Installed) -> Drivers<HostRunner> {
        Drivers::new(HostRunner, vec![installed.approve().unwrap()])
    }

    fn probe(drivers: &Drivers<HostRunner>, server: &ServerEvidence) -> EndpointProbe {
        let mut session = crate::host::external::Session::new(drivers);
        session.begin_coordination();
        session
            .driver(fake())
            .probe(server, &["fake-1".to_owned()])
            .unwrap()
    }

    #[test]
    fn core_proves_an_external_server_live_or_dead_from_its_own_process_check() {
        let (pid, start) = live();
        let installed = installed(pid);
        let drivers = drivers(&installed);

        // The same server process, reached by the driver: live, with core's
        // own start of each pane shell and the driver's marker.
        let EndpointProbe::Live(snapshot) = probe(&drivers, &server(pid, &start)) else {
            panic!("the same process is live");
        };
        assert_eq!(snapshot.server, server(pid, &start));
        let [pane] = snapshot.panes.as_slice() else {
            panic!("one pane");
        };
        assert_eq!(pane.id, "fake-1");
        assert_eq!(pane.pane_incarnation.as_deref(), Some(start.as_str()));
        assert_eq!(pane.marker.as_ref().unwrap().binding_id, BINDING_ID);
        assert_eq!(pane.suggested_name.as_deref(), Some("claude"));

        // Another process holds the recorded pid: the server is gone.
        assert_eq!(
            probe(&drivers, &server(pid, "ps-v1:Thu Jan 1 00:00:00 1970")),
            EndpointProbe::Dead
        );
        // The recorded process exited.
        assert_eq!(
            probe(&drivers, &server(gone(), &start)),
            EndpointProbe::Dead
        );

        // The driver can't reach a server core still sees: unknown, never dead.
        installed.answer(
            "snapshot",
            r#"{"error": {"code": "unavailable", "message": "socket gone"}}"#,
        );
        assert_eq!(
            probe(&drivers, &server(pid, &start)),
            EndpointProbe::Unknown
        );
        // A driver's own claim never retires anything: core doesn't call probe.
        assert!(
            !installed
                .calls()
                .iter()
                .any(|call| call.starts_with("probe"))
        );
    }

    #[test]
    fn a_missing_or_changed_driver_never_retires_a_binding() {
        let (pid, start) = live();
        let installed = installed(pid);

        // No approved driver: the host reads, and nothing is known.
        let none = Drivers::new(HostRunner, Vec::new());
        assert_eq!(
            probe(&none, &server(gone(), &start)),
            EndpointProbe::Unknown
        );
        let mut session = crate::host::external::Session::new(&none);
        session.begin_coordination();
        let error = session
            .driver(fake())
            .publish(&binding(server(pid, &start), pid), &identity())
            .unwrap_err();
        assert_eq!(error.to_string(), "Host driver fake is not installed.");

        // An executable changed since approval is never run again.
        let drivers = drivers(&installed);
        fs::write(&installed.executable, format!("{SCRIPT}# changed\n")).unwrap();
        let before = installed.calls().len();
        assert_eq!(
            probe(&drivers, &server(gone(), &start)),
            EndpointProbe::Unknown
        );
        assert_eq!(installed.calls().len(), before, "not run");
    }

    #[test]
    fn publish_clear_and_input_go_through_the_driver_or_the_inbox() {
        let (pid, start) = live();
        let installed = installed(pid);
        installed.answer("publish", r#"{"ok": {}}"#);
        installed.answer("clear", r#"{"ok": {"cleared": true}}"#);
        let drivers = drivers(&installed);
        let mut session = crate::host::external::Session::new(&drivers);
        session.begin_coordination();
        let driver = session.driver(fake());
        let binding = binding(server(pid, &start), pid);

        driver.publish(&binding, &identity()).unwrap();
        assert!(driver.clear(&binding).unwrap());
        // No input until 3b-2b: core uses the inbox.
        assert!(!driver.has_input());
        assert_eq!(
            driver.pane_incarnation(pid).unwrap().as_deref(),
            Some(start.as_str())
        );
        assert!(
            installed
                .calls()
                .ends_with(&["publish 1".to_owned(), "clear 1".to_owned()])
        );

        // A driver's refusal reaches the caller as its own message.
        installed.answer(
            "publish",
            r#"{"error": {"code": "not_found", "message": "the pane closed"}}"#,
        );
        let error = driver.publish(&binding, &identity()).unwrap_err();
        assert!(matches!(error, HostError::Refused { .. }), "{error:?}");
        assert_eq!(error.to_string(), "Host driver fake: the pane closed");
    }

    #[derive(Default)]
    struct Ids(Vec<(String, u64, String)>);

    impl HostServerIds for Ids {
        type Error = std::io::Error;

        fn server_id(&mut self, server: &HostServerIncarnation<'_>) -> Result<String, Self::Error> {
            self.0.push((
                server.socket_path.into(),
                server.server_pid,
                server.server_start_time.into(),
            ));
            Ok(SERVER_ID.into())
        }
    }

    #[test]
    fn an_external_server_is_core_observed_and_snapshots_only_that_incarnation() {
        let (pid, start) = live();
        let installed = installed(pid);
        installed.answer(
            "server",
            &json!({"ok": {"server": {"socket": SOCKET, "pid": pid, "startTime": "driver-says"}}})
                .to_string(),
        );
        let drivers = drivers(&installed);
        let mut ids = Ids::default();
        drivers.resolve_server(fake(), &mut ids).unwrap();
        // Core's start token is the identity; the driver's startTime is advisory.
        assert_eq!(ids.0, [(SOCKET.to_owned(), pid, start.clone())]);
        assert_eq!(drivers.resolved(), Some(&server(pid, &start)));

        let mut session = crate::host::external::Session::new(&drivers);
        session.begin_coordination();
        let snapshot = session
            .driver(fake())
            .snapshot(&["fake-1".to_owned()])
            .unwrap();
        assert_eq!(snapshot.server, server(pid, &start));
        assert_eq!(
            snapshot.panes[0].pane_incarnation.as_deref(),
            Some(start.as_str())
        );

        // A handle without a resolved server, or resolved to another
        // incarnation, never snapshots a substitute.
        let unresolved = Drivers::new(HostRunner, vec![installed.approve().unwrap()]);
        let mut session = crate::host::external::Session::new(&unresolved);
        session.begin_coordination();
        assert!(matches!(
            session.driver(fake()).snapshot(&[]),
            Err(HostError::Evidence { .. })
        ));
        let other = Drivers::new(HostRunner, vec![installed.approve().unwrap()]);
        other.set_resolved(server(pid, "ps-v1:Thu Jan 1 00:00:00 1970"));
        let mut session = crate::host::external::Session::new(&other);
        session.begin_coordination();
        assert!(matches!(
            session.driver(fake()).snapshot(&[]),
            Err(HostError::Evidence { .. })
        ));
    }
}
