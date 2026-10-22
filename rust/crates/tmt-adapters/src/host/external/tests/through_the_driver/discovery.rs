//! First-run inventory of an external host (#4063): a spawned script driver
//! answers `server` and `snapshot`, and core reads every start token itself.
//! Nothing here records a server except the guarded recording step, whose
//! refusals are counted against a recording `HostServerIds` fake.

use super::*;
use tmt_core::discovery::{DiscoveryObservation, DiscoverySource, DiscoveryTarget};

/// A driver with one pane `fake-1` whose shell is `pane_pid`, on a server
/// that core sees as `server_pid`.
fn inventory(server_pid: u64, pane_pid: u64) -> Installed {
    let installed = Installed::new("fake", answers("fake", "fake-", Some("f{n}")));
    installed.answer(
        "server",
        &json!({"ok": {"server": {"socket": SOCKET, "pid": server_pid, "startTime": "driver-says"}}})
            .to_string(),
    );
    installed.answer("snapshot", &panes(&[("fake-1", pane_pid)]));
    installed
}

fn panes(rows: &[(&str, u64)]) -> String {
    let panes: Vec<Value> = rows
        .iter()
        .map(|(id, pid)| {
            json!({"id": id, "target": "f1", "cwd": "/src", "command": "claude",
                "panePid": pid, "suggestedName": null, "agentName": "Ada", "marker": null})
        })
        .collect();
    json!({"ok": {"panes": panes}}).to_string()
}

fn scan(
    drivers: &Drivers<impl CommandRunner>,
    socket: Option<&str>,
    pane: Option<&str>,
) -> Result<DiscoveryObservation, HostError> {
    drivers.discover(fake(), socket, pane, soon())
}

fn requests(installed: &Installed, op: &str) -> Vec<Value> {
    fs::read_to_string(installed.beside(&format!("{op}.in")))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_scan_fences_each_pane_with_cores_own_start_tokens() {
    let (pid, start) = live();
    let installed = inventory(pid, pid);
    let drivers = drivers(&installed);
    let observation = scan(&drivers, None, None).unwrap();
    assert_eq!(observation.host, HostKind::External(fake()));
    assert_eq!(observation.source, DiscoverySource::Ok);
    assert_eq!(observation.socket, SOCKET);
    assert!(!observation.truncated);
    let [candidate] = observation.candidates.as_slice() else {
        panic!("one pane: {:?}", observation.candidates);
    };
    assert_eq!(candidate.pane, "fake-1");
    assert_eq!(candidate.target.as_deref(), Some("f1"));
    assert_eq!(candidate.agent_name.as_deref(), Some("Ada"));
    assert!(candidate.adoptable(), "{candidate:?}");
    let fence = candidate.fence.as_ref().unwrap();
    // The server's start time is core's own token, never the driver's.
    assert_eq!(fence.server_start_time, start);
    assert_eq!(fence.server.start_identity(), start);
    assert_eq!(fence.process.start_identity(), start);
    assert_eq!(fence.server_id, None);
    assert_eq!(fence.runtime.as_deref(), Some("claude"));
    // Unscoped: the driver's default server and every pane.
    let field = |op: &str, n: usize, key: &str| requests(&installed, op)[n][key].clone();
    assert_eq!(field("server", 0, "socket"), Value::Null);
    assert_eq!(field("snapshot", 0, "socket"), SOCKET);
    assert_eq!(field("snapshot", 0, "panes"), Value::Null);
    // Scoped to one pane on an explicit socket.
    scan(&drivers, Some(SOCKET), Some("fake-1")).unwrap();
    assert_eq!(field("server", 1, "socket"), SOCKET);
    assert_eq!(field("snapshot", 1, "panes"), json!(["fake-1"]));
    // Approval asked for the capabilities once; discovery asks only for the
    // server and its panes, and records nothing.
    assert_eq!(
        installed.calls(),
        [
            "capabilities 1",
            "server 1",
            "snapshot 1",
            "server 1",
            "snapshot 1"
        ]
    );
}

#[test]
fn no_server_is_offline_but_a_failing_driver_is_never_evidence_of_absence() {
    let (pid, _) = live();
    let installed = inventory(pid, pid);
    installed.answer("server", r#"{"ok": {"server": null}}"#);
    let offline = scan(&drivers(&installed), None, None).unwrap();
    assert_eq!(offline.source, DiscoverySource::Offline);
    assert!(offline.candidates.is_empty());
    for (op, answer) in [
        (
            "server",
            r#"{"error": {"code": "unavailable", "message": "socket gone"}}"#,
        ),
        ("server", "not json"),
        (
            "snapshot",
            r#"{"error": {"code": "unavailable", "message": "socket gone"}}"#,
        ),
        ("snapshot", r#"{"ok": {"panes": [{"id": "fake-1"}]}}"#),
    ] {
        let installed = inventory(pid, pid);
        installed.answer(op, answer);
        let error = scan(&drivers(&installed), None, None).unwrap_err();
        assert!(!error.cleanup_failed(), "{op} {answer}: {error:?}");
    }
    // A pane listed twice fails the whole answer, and so does another pane
    // than the one asked for.
    let installed = inventory(pid, pid);
    installed.answer("snapshot", &panes(&[("fake-1", pid), ("fake-1", pid)]));
    assert!(scan(&drivers(&installed), None, None).is_err());
    let installed = inventory(pid, pid);
    installed.answer("snapshot", &panes(&[("fake-2", pid)]));
    assert!(matches!(
        scan(&drivers(&installed), None, Some("fake-1")),
        Err(HostError::Evidence { .. })
    ));
    // A late answer is a failure, not a partial inventory.
    let installed = inventory(pid, pid);
    installed.answer("snapshot.sleep", "3");
    let late = Drivers::new(TimedRunner, vec![installed.approve().unwrap()]).discover(
        fake(),
        None,
        None,
        Instant::now() + Duration::from_millis(500),
    );
    assert!(late.is_err(), "{late:?}");
}

#[test]
fn an_explicit_socket_must_be_the_one_the_driver_answers_for() {
    let (pid, _) = live();
    let installed = inventory(pid, pid);
    let error = scan(&drivers(&installed), Some("/tmp/other.sock"), None).unwrap_err();
    assert!(matches!(error, HostError::Evidence { .. }), "{error:?}");
    assert!(
        !installed
            .calls()
            .iter()
            .any(|call| call.starts_with("snapshot"))
    );
}

#[test]
fn a_host_without_its_driver_or_inventory_runs_nothing() {
    let (pid, _) = live();
    let installed = inventory(pid, pid);
    let none = Drivers::new(HostRunner, Vec::new());
    let observation = scan(&none, None, None).unwrap();
    assert_eq!(
        observation.source,
        DiscoverySource::Unsupported("driver_not_approved")
    );
    // A driver that lists no panes is asked nothing.
    installed.answer(
        "capabilities",
        &json!({"ok": {"protocols": [1], "kind": "host", "name": "fake", "version": "0.0.0-test",
            "ops": ["server", "publish", "clear"], "paneId": {"prefix": "fake-"},
            "target": "f{n}", "callerEnv": []}})
        .to_string(),
    );
    let drivers = drivers(&installed);
    let observation = scan(&drivers, None, None).unwrap();
    assert_eq!(
        observation.source,
        DiscoverySource::Unsupported("driver_lacks_inventory")
    );
    assert_eq!(installed.calls(), ["capabilities 1"]);
}

#[test]
fn an_unreadable_start_token_or_a_capped_scan_is_never_adoptable() {
    let (pid, _) = live();
    let installed = inventory(pid, gone());
    let observation = scan(&drivers(&installed), None, None).unwrap();
    let candidate = &observation.candidates[0];
    assert!(candidate.fence.is_none());
    assert_eq!(candidate.reason, Some("native_incarnation_unknown"));
    assert!(!candidate.adoptable());
    // 257 panes: the first 256 by pane ID, and none adoptable.
    let ids: Vec<String> = (0..257).map(|n| format!("fake-{n:03}")).collect();
    let rows: Vec<(&str, u64)> = ids.iter().rev().map(|id| (id.as_str(), pid)).collect();
    installed.answer("snapshot", &panes(&rows));
    let observation = scan(&drivers(&installed), None, None).unwrap();
    assert!(observation.truncated);
    assert_eq!(observation.candidates.len(), 256);
    assert_eq!(observation.candidates[0].pane, "fake-000");
    assert_eq!(observation.candidates[255].pane, "fake-255");
    assert!(
        observation
            .candidates
            .iter()
            .all(|candidate| !candidate.adoptable() && candidate.reason == Some("scan_truncated"))
    );
}

#[test]
fn a_malformed_marker_fails_the_whole_answer_not_one_row() {
    let (pid, _) = live();
    let installed = inventory(pid, pid);
    installed.answer(
        "snapshot",
        &json!({"ok": {"panes": [{"id": "fake-1", "target": "f1", "cwd": "/src",
            "command": "sh", "panePid": pid, "suggestedName": null,
            "marker": {"name": "w", "canonicalName": "w", "identityId": "not-a-uuid",
                "bindingId": BINDING_ID, "serverId": SERVER_ID, "panePid": pid}}]}})
        .to_string(),
    );
    let error = scan(&drivers(&installed), None, None).unwrap_err();
    assert!(matches!(error, HostError::Driver(_)), "{error:?}");
}

#[test]
fn a_driver_process_that_cannot_be_cleaned_up_fails_the_scan() {
    #[derive(Clone)]
    struct Unreaped;
    impl CommandRunner for Unreaped {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            if request.args.iter().any(|arg| arg == "snapshot") {
                let mut error = CommandError::new(CommandFailure::Timeout);
                error.cleanup_error = Some(std::io::Error::other("not reaped"));
                return Err(error);
            }
            HostRunner.execute(request)
        }
    }
    let (pid, _) = live();
    let installed = inventory(pid, pid);
    let drivers = Drivers::new(Unreaped, vec![installed.approve().unwrap()]);
    let error = scan(&drivers, None, None).unwrap_err();
    assert!(error.cleanup_failed(), "{error:?}");
}

/// Counts every request to record a server.
#[derive(Default)]
struct Recording(Vec<String>);

impl HostServerIds for Recording {
    type Error = std::io::Error;

    fn server_id(&mut self, server: &HostServerIncarnation<'_>) -> Result<String, Self::Error> {
        self.0.push(format!(
            "{} {} {}",
            server.socket_path, server.server_pid, server.server_start_time
        ));
        Ok(SERVER_ID.into())
    }
}

fn fenced(drivers: &Drivers<HostRunner>) -> DiscoveryTarget {
    scan(drivers, None, None).unwrap().candidates[0]
        .fence
        .clone()
        .unwrap()
}

#[test]
fn only_the_fenced_server_incarnation_is_recorded() {
    let (pid, start) = live();
    let installed = inventory(pid, pid);
    let drivers = drivers(&installed);
    let fence = fenced(&drivers);
    // Another socket, another server process, the same pid with another
    // start, or another host: refused before anything is recorded.
    let mut other_socket = fence.clone();
    other_socket.socket = "/tmp/other.sock".into();
    let mut other_pid = fence.clone();
    other_pid.server = tmt_core::endpoint::ProcessIncarnation::new(gone(), &start).unwrap();
    let mut restarted = fence.clone();
    restarted.server =
        tmt_core::endpoint::ProcessIncarnation::new(pid, "ps-v1:Thu Jan 1 00:00:00 1970").unwrap();
    let mut elsewhere = fence.clone();
    elsewhere.host = HostKind::parse("other").unwrap();
    for refused in [other_socket, other_pid, restarted, elsewhere] {
        let mut ids = Recording::default();
        let handle = super::drivers(&installed);
        let error = handle
            .resolve_fenced_server(fake(), &refused, &mut ids, soon())
            .unwrap_err();
        assert!(matches!(error, HostError::Evidence { .. }), "{error:?}");
        assert!(ids.0.is_empty(), "{refused:?}");
        assert_eq!(handle.resolved(), None);
    }
    // The fenced incarnation: recorded once, and the handle resolved to it.
    let mut ids = Recording::default();
    let id = drivers
        .resolve_fenced_server(fake(), &fence, &mut ids, soon())
        .unwrap();
    assert_eq!(id, SERVER_ID);
    assert_eq!(ids.0, [format!("{SOCKET} {pid} {start}")]);
    assert_eq!(drivers.resolved(), Some(&server(pid, &start)));
}

#[test]
fn a_caller_search_past_its_deadline_runs_no_driver() {
    let (pid, _) = live();
    let installed = inventory(pid, pid);
    let environment = crate::host::CallerEnvironment {
        tmux: None,
        pane: None,
        process_id: u64::from(std::process::id()),
        driver_env: [("FAKE_PANE_ID".to_owned(), "f1".into())].into(),
    };
    let record = installed.approve().unwrap();
    let calls = installed.calls().len();
    let host = crate::host::Host::for_caller_within(
        &environment,
        TimedRunner,
        vec![record],
        Instant::now(),
    )
    .unwrap();
    assert_eq!(host.kind(), HostKind::Tmux);
    assert_eq!(installed.calls().len(), calls, "{:?}", installed.calls());
}

#[test]
fn implicit_selection_refuses_a_gap_host_several_hosts_or_no_registry() {
    use crate::driver_protocol::registry::{Admitted, Gap};
    use crate::host::{CallerEnvironment, Unselectable};
    let installed = Installed::new("fake", answers("fake", "fake-", Some("f{n}")));
    let mut record = installed.approve().unwrap();
    record.capabilities = serde_json::from_value(json!({"protocols": [1], "kind": "host",
        "name": "fake", "version": "0.0.0-test", "ops": ["caller"],
        "paneId": {"prefix": "fake-"}, "target": "f{n}", "callerEnv": ["FAKE_PANE_ID"]}))
    .unwrap();
    let gap = |name: &str, variable: &str| Gap {
        name: name.into(),
        caller_env: vec![variable.into()],
        reason: "driver_approval_outdated",
    };
    let environment = |tmux: bool, variables: &[&str]| CallerEnvironment {
        tmux: tmux.then(|| "/tmp/nest.sock,1,0".into()),
        pane: tmux.then(|| "%1".into()),
        process_id: u64::from(std::process::id()),
        driver_env: variables
            .iter()
            .map(|name| ((*name).to_owned(), "f1".into()))
            .collect(),
    };
    let only_gap = Admitted {
        gaps: vec![gap("herdr", "HERDR_PANE_ID")],
        ..Admitted::default()
    };
    // A gap host named alone, or nested in tmux: that gap, with no fallback.
    for tmux in [false, true] {
        assert_eq!(
            environment(tmux, &["HERDR_PANE_ID"]).unselectable(&only_gap),
            Some(Unselectable::Gap(&only_gap.gaps[0]))
        );
    }
    // Not named: ordinary selection.
    assert_eq!(environment(true, &[]).unselectable(&only_gap), None);
    let both = Admitted {
        records: vec![record.clone()],
        gaps: vec![gap("herdr", "HERDR_PANE_ID")],
        unreadable: None,
    };
    assert_eq!(
        environment(false, &["FAKE_PANE_ID"]).unselectable(&both),
        None
    );
    assert_eq!(
        environment(false, &["FAKE_PANE_ID", "HERDR_PANE_ID"]).unselectable(&both),
        Some(Unselectable::Ambiguous(vec!["fake", "herdr"]))
    );
    let unreadable = Admitted {
        unreadable: Some("bad registry".into()),
        ..Admitted::default()
    };
    assert_eq!(
        environment(false, &[]).unselectable(&unreadable),
        Some(Unselectable::RegistryUnreadable)
    );
}
