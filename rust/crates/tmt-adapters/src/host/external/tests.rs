//! A real spawned driver in a private test directory: a small shell script
//! that answers each operation from a file the test wrote, so approval, the
//! calls, the trust checks and the bounds run through actual processes.

use super::{CallError, DriverProcess};
use crate::driver_protocol::registry::{self, RegistryError};
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
/// It keeps each request in `<itself>.<op>.in`, and sleeps `<itself>.<op>.sleep`
/// seconds after reading it when present.
const SCRIPT: &str = r#"#!/bin/sh
echo "$3 ${TMT_DRIVER_CALL:-}" >> "$0.calls"
if [ -f "$0.sleep" ]; then sleep "$(cat "$0.sleep")"; fi
if [ "$1" != "__tmt-driver" ] || [ "$2" != "1" ] || [ -z "$3" ]; then
  printf '{"error":{"code":"bad_request","message":"usage"}}'
  exit 2
fi
input=$(cat)
printf '%s\n' "$input" >> "$0.$3.in"
if [ -f "$0.$3.sleep" ]; then sleep "$(cat "$0.$3.sleep")"; fi
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
    // This call retains the actual 1 s protocol deadline and cleanup owner.
    let timed_driver = DriverProcess::open(record, ScriptRunner).unwrap();
    assert_eq!(
        driver.call::<ClearResponse>(clear(), soon()).unwrap(),
        Ok(ClearResponse { cleared: false })
    );

    fs::write(installed.beside("sleep"), "2.5").unwrap();
    let started = Instant::now();
    let result = timed_driver.call::<ClearResponse>(clear(), soon());
    assert!(
        matches!(&result, Err(CallError::Process { error, .. })
            if error.kind == CommandFailure::Timeout && !error.cleanup_failed()),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(2000),
        "stopped at clear's 1 s deadline, took {:?}",
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
#[derive(Clone)]
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
        assert!(driver.clear(&binding, &identity()).unwrap());
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

    /// A driver that also declares `name-agent`.
    fn naming(pane_pid: u64) -> Installed {
        let installed = installed(pane_pid);
        installed.answer(
            "capabilities",
            &json!({"ok": {"protocols": [1], "kind": "host", "name": "fake",
                "version": "0.0.0-test",
                "ops": ["caller", "server", "resolve-target", "snapshot", "probe", "publish",
                        "clear", "capture", "focus", "name-agent"],
                "paneId": {"prefix": "fake-"}, "target": "f{n}", "callerEnv": []}})
            .to_string(),
        );
        installed
    }

    fn refusal(code: &str) -> String {
        json!({"error": {"code": code, "message": "refused"}}).to_string()
    }

    /// Whether a host offers agent names comes from the approval alone: no
    /// driver runs to answer it, and tmux never offers them.
    #[test]
    fn offering_agent_names_reads_the_approval_and_runs_no_driver() {
        use tmt_driver_protocol::Op;
        let (pid, start) = live();
        for (installed, offers) in [(installed(pid), false), (naming(pid), true)] {
            let drivers = drivers(&installed);
            let before = installed.calls().len();
            assert_eq!(drivers.offers(fake(), Op::NameAgent), offers);
            assert_eq!(installed.calls().len(), before);
        }
        let mut bound = binding(server(pid, &start), pid);
        bound.server.host = HostKind::Tmux;
        let host =
            crate::host::Host::for_server_with(&bound.server, crate::process::UnixCommandRunner);
        assert!(!host.offers_agent_names(&bound));
    }

    #[test]
    fn an_agent_name_goes_through_name_agent_only_when_the_driver_declares_it() {
        use crate::host::{AgentNameRefusal, AgentNaming};
        let (pid, start) = live();

        // A driver without `name-agent` is not linked, and is never asked.
        let old = installed(pid);
        let old_drivers = drivers(&old);
        let mut session = crate::host::external::Session::new(&old_drivers);
        session.begin_coordination();
        let driver = session.driver(fake());
        let bound = binding(server(pid, &start), pid);
        let before = old.calls().len();
        assert_eq!(
            driver.name_agent(&bound, "worker").unwrap(),
            AgentNaming::NotLinked
        );
        assert_eq!(old.calls().len(), before);

        let installed = naming(pid);
        installed.answer("name-agent", r#"{"ok": {}}"#);
        let drivers = drivers(&installed);
        let mut session = crate::host::external::Session::new(&drivers);
        session.begin_coordination();
        let driver = session.driver(fake());
        assert_eq!(
            driver.name_agent(&bound, "worker").unwrap(),
            AgentNaming::Done
        );
        assert_eq!(installed.calls().last().unwrap(), "name-agent 1");
        for (code, class) in [
            ("refused", AgentNameRefusal::Refused),
            ("no_agent", AgentNameRefusal::NoAgent),
            ("not_ready", AgentNameRefusal::NotReady),
            ("not_found", AgentNameRefusal::Uncertain),
            ("failed", AgentNameRefusal::Uncertain),
        ] {
            installed.answer("name-agent", &refusal(code));
            let error = driver.name_agent(&bound, "worker").unwrap_err();
            assert!(matches!(error, HostError::Refused { .. }), "{error:?}");
            assert_eq!(AgentNameRefusal::of(&error), class, "{code}");
        }
        // A code that answers only `prompt` breaks the protocol here.
        installed.answer("name-agent", &refusal("blocked"));
        let error = driver.name_agent(&bound, "worker").unwrap_err();
        assert!(matches!(error, HostError::Driver(_)), "{error:?}");
    }

    /// Driver calls with their real deadlines, unlike [`HostRunner`].
    #[derive(Clone)]
    struct TimedRunner;

    impl CommandRunner for TimedRunner {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            if request.args.first().map(OsString::as_os_str)
                == Some(OsStr::new("TMT_DRIVER_CALL=1"))
            {
                ScriptRunner.execute(request)
            } else {
                UnixCommandRunner.execute(request)
            }
        }
    }

    /// A caller's deadline caps the whole call, the driver's start included:
    /// a slow driver ends at that deadline, not at the operation's own 2 s.
    #[test]
    fn a_callers_deadline_caps_naming_an_agent() {
        let (pid, start) = live();
        let installed = naming(pid);
        installed.answer("name-agent", r#"{"ok": {}}"#);
        let host = crate::host::Host::for_server_with(&server(pid, &start), TimedRunner)
            .with_drivers(vec![installed.approve().unwrap()]);
        let bound = binding(server(pid, &start), pid);
        fs::write(installed.beside("sleep"), "3").unwrap();
        let started = Instant::now();
        let error = host
            .name_agent(
                &bound,
                &identity(),
                Some(Instant::now() + Duration::from_secs(1)),
            )
            .unwrap_err();
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(900) && took < Duration::from_millis(1900),
            "{took:?}: {error:?}"
        );
        assert_eq!(
            crate::host::AgentNameRefusal::of(&error),
            crate::host::AgentNameRefusal::Uncertain
        );
    }

    /// The requests the driver received for `op`.
    fn requests(installed: &Installed, op: &str) -> Vec<Value> {
        fs::read_to_string(installed.beside(&format!("{op}.in")))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// The exact `name-agent` request: the binding's socket, pane, pane
    /// process and binding ID, with the name; before a clear, `null` and the
    /// identity's current name as `expected`, also after a rename.
    #[test]
    fn name_agent_sends_the_bindings_exact_request() {
        let (pid, start) = live();
        let installed = naming(pid);
        installed.answer("name-agent", r#"{"ok": {}}"#);
        installed.answer("clear", r#"{"ok": {"cleared": true}}"#);
        let drivers = drivers(&installed);
        let bound = binding(server(pid, &start), pid);
        let mut session = crate::host::external::Session::new(&drivers);
        session.begin_coordination();
        session.driver(fake()).name_agent(&bound, "worker").unwrap();
        let renamed = Identity {
            name: "lead".into(),
            canonical_name: "lead".into(),
            ..identity()
        };
        assert!(session.driver(fake()).clear(&bound, &renamed).unwrap());
        let request = |name: Value| {
            json!({"socket": bound.server.socket_path, "paneId": bound.pane_id,
                   "panePid": pid, "bindingId": bound.id, "name": name})
        };
        let mut clear = request(Value::Null);
        clear["expected"] = json!("lead");
        let mut sent = requests(&installed, "name-agent");
        // The envelope's deadline: the operation's own 2 s for a set, at
        // most half of the 3 s session budget before a clear.
        let deadlines: Vec<_> = sent
            .iter_mut()
            .map(|request| request.as_object_mut().unwrap().remove("deadlineMs"))
            .map(|deadline| deadline.and_then(|ms| ms.as_u64()).unwrap())
            .collect();
        assert!(
            deadlines[0] <= 2000 && deadlines[1] <= 1500,
            "{deadlines:?}"
        );
        assert_eq!(sent, [request(json!("worker")), clear]);
    }

    /// A slow `name-agent` before a clear ends at half of the budget left,
    /// and the clear still runs in the rest.
    #[test]
    fn a_slow_name_agent_leaves_the_clear_its_half_of_the_budget() {
        let (pid, start) = live();
        let installed = naming(pid);
        installed.answer("name-agent", r#"{"ok": {}}"#);
        installed.answer("clear", r#"{"ok": {"cleared": true}}"#);
        let drivers = Drivers::new(TimedRunner, vec![installed.approve().unwrap()]);
        let bound = binding(server(pid, &start), pid);
        let mut session = crate::host::external::Session::new(&drivers);
        session.begin_coordination();
        let driver = session.driver(fake());
        fs::write(installed.beside("name-agent.sleep"), "5").unwrap();
        let started = Instant::now();
        assert!(driver.clear(&bound, &identity()).unwrap());
        let took = started.elapsed();
        assert!(took < Duration::from_secs(3), "{took:?}");
        assert!(
            installed
                .calls()
                .ends_with(&["name-agent 1".to_owned(), "clear 1".to_owned()]),
            "{:?}",
            installed.calls()
        );
    }

    /// Answers every driver call through the fixture, except a `name-agent`
    /// whose process could not be cleaned up.
    struct UnreapedNaming;

    impl CommandRunner for UnreapedNaming {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            if request.args.iter().any(|arg| arg == "name-agent") {
                let mut error = CommandError::new(CommandFailure::Timeout);
                error.cleanup_error = Some(std::io::Error::other("not reaped"));
                return Err(error);
            }
            TimedRunner.execute(request)
        }
    }

    /// A driver process that could not be cleaned up stops the clear: TMT
    /// does not start another driver process past a leaked one.
    #[test]
    fn a_name_agent_cleanup_failure_stops_the_clear() {
        let (pid, start) = live();
        let installed = naming(pid);
        installed.answer("clear", r#"{"ok": {"cleared": true}}"#);
        let drivers = Drivers::new(UnreapedNaming, vec![installed.approve().unwrap()]);
        let bound = binding(server(pid, &start), pid);
        let mut session = crate::host::external::Session::new(&drivers);
        session.begin_coordination();
        let error = session
            .driver(fake())
            .clear(&bound, &identity())
            .unwrap_err();
        assert!(error.cleanup_failed(), "{error:?}");
        assert!(
            !installed
                .calls()
                .iter()
                .any(|call| call.starts_with("clear")),
            "{:?}",
            installed.calls()
        );
    }

    /// Unbind: the agent name goes before the marker, because the driver acts
    /// only on a pane that carries the binding. Its answer,
    /// whatever it is, never stops the clear; a driver process that could not
    /// be cleaned up does, which this test does not cover.
    #[test]
    fn clearing_takes_tmts_agent_name_away_first_and_clears_whatever_it_answers() {
        let (pid, start) = live();
        let bound = binding(server(pid, &start), pid);
        for answer in [
            r#"{"ok": {}}"#.to_owned(),
            refusal("refused"),
            refusal("not_found"),
            "not json".to_owned(),
        ] {
            let installed = naming(pid);
            installed.answer("clear", r#"{"ok": {"cleared": true}}"#);
            installed.answer("name-agent", &answer);
            let drivers = drivers(&installed);
            let mut session = crate::host::external::Session::new(&drivers);
            session.begin_coordination();
            assert!(
                session.driver(fake()).clear(&bound, &identity()).unwrap(),
                "{answer}"
            );
            assert!(
                installed
                    .calls()
                    .ends_with(&["name-agent 1".to_owned(), "clear 1".to_owned()]),
                "{answer}: {:?}",
                installed.calls()
            );
        }
        // A driver without `name-agent` only clears.
        let installed = installed(pid);
        installed.answer("clear", r#"{"ok": {"cleared": true}}"#);
        let drivers = drivers(&installed);
        let mut session = crate::host::external::Session::new(&drivers);
        session.begin_coordination();
        assert!(session.driver(fake()).clear(&bound, &identity()).unwrap());
        assert!(
            !installed
                .calls()
                .iter()
                .any(|call| call.starts_with("name-agent"))
        );
    }

    #[test]
    fn a_rename_rewrites_only_this_bindings_stale_marker() {
        use crate::host::{Host, PaneCosmetics, PaneRefresh};
        let (pid, start) = live();
        let installed = installed(pid);
        installed.answer("publish", r#"{"ok": {}}"#);
        let host = Host::for_server_with(&server(pid, &start), HostRunner)
            .with_drivers(vec![installed.approve().unwrap()]);
        let binding = binding(server(pid, &start), pid);
        let refresh = |identity: &Identity, binding: &Binding| {
            host.update_binding_cosmetics_until(
                binding,
                PaneCosmetics::Bound {
                    identity,
                    badge: true,
                    border_hint: false,
                },
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap()
        };
        let publishes = || {
            installed
                .calls()
                .iter()
                .filter(|call| call.starts_with("publish"))
                .count()
        };

        // The marker already carries the name: nothing to write.
        assert_eq!(refresh(&identity(), &binding), PaneRefresh::Updated);
        assert_eq!(publishes(), 0);
        // Renamed: the marker is rewritten.
        let renamed = Identity {
            name: "Lead".into(),
            canonical_name: "lead".into(),
            ..identity()
        };
        assert_eq!(refresh(&renamed, &binding), PaneRefresh::Updated);
        assert_eq!(publishes(), 1);
        // A pane another binding owns is left alone.
        let other = Binding {
            id: "44444444-4444-4444-8444-444444444444".into(),
            ..binding.clone()
        };
        assert_eq!(refresh(&renamed, &other), PaneRefresh::Absent);
        assert_eq!(publishes(), 1);
        // A driver that can't write reports the refresh failed, not an error.
        installed.answer(
            "publish",
            r#"{"error": {"code": "not_found", "message": "the pane closed"}}"#,
        );
        assert_eq!(refresh(&renamed, &binding), PaneRefresh::Failed);
        // An ended binding has nothing to show.
        assert_eq!(
            host.update_binding_cosmetics(&binding, PaneCosmetics::Ended)
                .unwrap(),
            PaneRefresh::Absent
        );
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

    /// Fake tmux for a caller in pane `%1` of `/tmp/nest.sock`, server pid 1,
    /// whose pane shell is `pane`; with `None` every tmux command fails, as
    /// for stale variables. Driver calls run the fixture and `ps` is real.
    #[derive(Clone)]
    struct Nested {
        pane: Option<u64>,
    }

    impl CommandRunner for Nested {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            if request.program != OsStr::new("tmux") {
                return HostRunner.execute(request);
            }
            let format = request.args.last().unwrap().to_string_lossy();
            let listing = request.args.iter().any(|arg| arg == "list-panes");
            let stdout = match self.pane {
                Some(_) if format.contains("#{socket_path}") && !listing => {
                    ["%1", "/tmp/nest.sock", "1"].join("__TMT_CALLER_PANE_4f1c__")
                }
                Some(pane) if format == "#{pane_pid}" => pane.to_string(),
                _ => {
                    return Err(CommandError::new(CommandFailure::Exit {
                        code: Some(1),
                        signal: None,
                    }));
                }
            };
            Ok(CommandOutput {
                stdout: stdout.into_bytes(),
                stderr: Vec::new(),
            })
        }
    }

    enum Pane {
        Shell(u64),
        Missing,
        Fails,
        /// The approved executable changed after approval, so it is not run.
        Changed(u64),
        /// The caller variable is set but is not text a driver can be given.
        NotText(u64),
    }

    #[derive(Clone, Copy)]
    enum TmuxEnv {
        Full,
        PaneOnly,
        TmuxOnly,
        Absent,
    }

    /// Which host `for_verified_caller` picks for this caller: the fake
    /// driver's answer, the tmux pane's shell and the tmux variables set.
    fn chosen(pane: Pane, tmux: Option<u64>, env: TmuxEnv) -> Result<HostKind, HostError> {
        let (pid, _) = live();
        choose(pane, env, pid, Nested { pane: tmux }, false)
    }

    /// `chosen` for a caller `process_id`, observed through `runner`; with
    /// `second`, another approved host's variable (whose pane is this test
    /// process) is set too.
    fn choose<R: CommandRunner + Clone>(
        pane: Pane,
        env: TmuxEnv,
        process_id: u64,
        runner: R,
        second: bool,
    ) -> Result<HostKind, HostError> {
        admitted(pane, env, process_id, runner, second, &mut Ids::default())
            .map(|host| host.primary)
    }

    /// The handle `choose` describes, with the recorded servers `ids`.
    fn admitted<R: CommandRunner + Clone>(
        pane: Pane,
        env: TmuxEnv,
        process_id: u64,
        runner: R,
        second: bool,
        ids: &mut impl HostServerIds,
    ) -> Result<crate::host::Host<R>, HostError> {
        with_caller(pane, env, process_id, second, |environment, records| {
            crate::host::Host::for_verified_caller(environment, runner, records, ids)
        })
    }

    /// `use_caller` given the caller `choose` describes and the approved
    /// drivers, while their executables are installed.
    fn with_caller<T>(
        pane: Pane,
        env: TmuxEnv,
        process_id: u64,
        second: bool,
        use_caller: impl FnOnce(
            &crate::host::CallerEnvironment,
            Vec<crate::driver_protocol::registry::DriverRecord>,
        ) -> T,
    ) -> T {
        use crate::host::CallerEnvironment;
        use std::os::unix::ffi::OsStringExt;
        let (pid, start) = live();
        let mut answers = answers("fake", "fake-", Some("f{n}"));
        answers[0].1["ok"]["callerEnv"] = json!(["FAKE_PANE_ID"]);
        let installed = Installed::new("fake", answers);
        installed.answer(
            "server",
            &json!({"ok": {"server": {"socket": SOCKET, "pid": pid, "startTime": start}}})
                .to_string(),
        );
        let shell = |shell: u64| json!({"ok": {"pane": {"id": "fake-1", "socket": SOCKET, "shellPid": shell}}});
        let caller = match pane {
            Pane::Shell(pid) | Pane::Changed(pid) | Pane::NotText(pid) => shell(pid),
            Pane::Missing => json!({"ok": {"pane": null}}),
            Pane::Fails => json!({"error": {"code": "failed", "message": "host is wedged"}}),
        };
        installed.answer("caller", &caller.to_string());
        let record = installed.approve().unwrap();
        if matches!(pane, Pane::Changed(_)) {
            let mut script = fs::read_to_string(&installed.executable).unwrap();
            script.push_str("# changed after approval\n");
            fs::write(&installed.executable, script).unwrap();
        }
        let value = if matches!(pane, Pane::NotText(_)) {
            OsString::from_vec(vec![b'f', 0xff])
        } else {
            "f1".into()
        };
        let mut records = vec![record];
        let mut driver_env: std::collections::BTreeMap<_, _> =
            [("FAKE_PANE_ID".to_owned(), value)].into();
        let other = second.then(|| {
            let mut declared = super::answers("other", "other-", Some("o{n}"));
            declared[0].1["ok"]["callerEnv"] = json!(["OTHER_PANE_ID"]);
            let other = Installed::new("other", declared);
            other.answer("caller", &shell(pid).to_string());
            records.push(other.approve().unwrap());
            driver_env.insert("OTHER_PANE_ID".to_owned(), "o1".into());
            other
        });
        let environment = CallerEnvironment {
            tmux: matches!(env, TmuxEnv::Full | TmuxEnv::TmuxOnly)
                .then(|| "/tmp/nest.sock,1,0".into()),
            pane: matches!(env, TmuxEnv::Full | TmuxEnv::PaneOnly).then(|| "%1".into()),
            process_id,
            driver_env,
        };
        let chosen = use_caller(&environment, records);
        drop(other);
        chosen
    }

    /// This test process and its parent: the caller's own pane shell and an
    /// outer one.
    fn ancestors() -> Vec<u64> {
        crate::process::ancestry::chain(&UnixCommandRunner, u64::from(std::process::id()), soon())
            .unwrap()
    }

    fn refused(result: Result<HostKind, HostError>) {
        assert!(
            matches!(result, Err(HostError::Unavailable(_))),
            "{result:?}"
        );
    }

    /// Refused because the locators name more than one host, not for a check.
    fn ambiguous(result: Result<HostKind, HostError>) {
        assert!(
            matches!(&result, Err(HostError::Unavailable(reason)) if reason.contains("more than one host")),
            "{result:?}"
        );
    }

    #[test]
    fn a_verified_caller_named_by_two_external_hosts_refuses_whatever_the_panes_say() {
        let chain = ancestors();
        let (pid, _) = live();
        // Two approved external hosts whose variables are both set, with or
        // without tmux variables: whatever each pane check says, refused.
        // tmux and ONE external host is a nested caller (`nested`).
        for env in [TmuxEnv::Absent, TmuxEnv::Full] {
            for pane in [Pane::Shell(chain[0]), Pane::Missing, Pane::Fails] {
                ambiguous(choose(
                    pane,
                    env,
                    pid,
                    Nested {
                        pane: Some(chain[1]),
                    },
                    true,
                ));
            }
        }
    }

    #[test]
    fn a_verified_caller_named_by_one_host_admits_only_that_hosts_proof() {
        let chain = ancestors();
        let only_external = chosen(Pane::Shell(chain[0]), None, TmuxEnv::Absent);
        assert_eq!(only_external.unwrap(), HostKind::External(fake()));
        // The named host's negative or failing check never tries tmux.
        refused(chosen(Pane::Missing, Some(chain[0]), TmuxEnv::Absent));
        refused(chosen(Pane::Fails, Some(chain[0]), TmuxEnv::Absent));
        // A pane that is no ancestor of the caller is no proof either.
        refused(chosen(Pane::Shell(999_999), None, TmuxEnv::Absent));
        // An approved executable that changed, or a set variable that is not
        // text, is not a verification.
        refused(chosen(Pane::Changed(chain[0]), None, TmuxEnv::Absent));
        refused(chosen(Pane::NotText(chain[0]), None, TmuxEnv::Absent));
    }

    #[test]
    fn a_caller_without_external_locators_keeps_the_tmux_host() {
        use crate::host::{CallerEnvironment, Host};
        let caller = |tmux: bool| CallerEnvironment {
            tmux: tmux.then(|| "/tmp/nest.sock,1,0".into()),
            pane: tmux.then(|| "%1".into()),
            process_id: u64::from(std::process::id()),
            driver_env: Default::default(),
        };
        // tmux variables alone and no locators at all are unchanged: the
        // built-in host, whose own caller check decides later.
        for tmux in [true, false] {
            let host = Host::for_verified_caller(
                &caller(tmux),
                Nested { pane: None },
                Vec::new(),
                &mut Ids::default(),
            )
            .unwrap();
            assert_eq!(host.primary, HostKind::Tmux);
        }
    }

    /// The probe itself, with no tmux involved: a driver that was not run, or
    /// not given its set variable, leaves the answer inconclusive, while a
    /// driver that ran and found no pane is a conclusive "no pane".
    #[test]
    fn an_unrun_or_ungiven_driver_leaves_the_caller_probe_inconclusive() {
        use crate::host::CallerEnvironment;
        use std::os::unix::ffi::OsStringExt;
        let probe = |changed: bool, value: OsString| {
            let mut declared = answers("fake", "fake-", Some("f{n}"));
            declared[0].1["ok"]["callerEnv"] = json!(["FAKE_PANE_ID"]);
            let installed = Installed::new("fake", declared);
            installed.answer("caller", &json!({"ok": {"pane": null}}).to_string());
            let drivers = Drivers::new(HostRunner, vec![installed.approve().unwrap()]);
            if changed {
                let mut script = fs::read_to_string(&installed.executable).unwrap();
                script.push_str("# changed after approval\n");
                fs::write(&installed.executable, script).unwrap();
            }
            let environment = CallerEnvironment {
                tmux: None,
                pane: None,
                process_id: u64::from(std::process::id()),
                driver_env: [("FAKE_PANE_ID".to_owned(), value)].into(),
            };
            let callers = drivers.callers(&environment, None, soon()).unwrap();
            (callers.conclusive, callers.found.len(), installed.calls())
        };
        let (conclusive, found, calls) = probe(false, "f1".into());
        assert!(conclusive && found == 0, "{calls:?}");
        assert!(
            calls.iter().any(|call| call.starts_with("caller")),
            "{calls:?}"
        );
        let (conclusive, found, calls) = probe(true, "f1".into());
        assert!(!conclusive && found == 0, "changed executable: {calls:?}");
        assert!(
            !calls.iter().any(|call| call.starts_with("caller")),
            "{calls:?}"
        );
        let (conclusive, found, calls) = probe(false, OsString::from_vec(vec![b'f', 0xff]));
        assert!(!conclusive && found == 0, "non-text variable: {calls:?}");
        assert!(
            !calls.iter().any(|call| call.starts_with("caller")),
            "{calls:?}"
        );
    }

    mod discovery;
    mod nested;
}
