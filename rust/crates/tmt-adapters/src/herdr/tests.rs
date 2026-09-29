//! Herdr adapter tests over the scripted runner. The JSON shapes are the ones
//! a disposable Herdr 0.9.1 server printed (#498).

use super::*;
use crate::{
    host::{CallerEnvironment, Host},
    process::CommandOutput,
    scripted_runner::ScriptedRunner,
};
use serde_json::json;
use std::{convert::Infallible, rc::Rc};
use tmt_core::{
    binding::{Binding, BindingEndpoint, BindingEntry, BindingEvidence, evaluate_binding},
    endpoint::{BindingMarker, EndpointProbe},
    host::{HostKind, HostServerIds, HostServerIncarnation},
    identity::{Identity, Lifetime},
};

const SOCKET: &str = "/tmp/hdr/herdr.sock";
const SERVER_ID: &str = "123e4567-e89b-42d3-a456-426614174000";
const START: &str = "ps-v1:Sun Sep 27 10:00:00 2026";
/// A PID no process holds: `kill(pid, 0)` answers `ESRCH`.
const GONE_PID: u64 = 2_000_000_000;

/// The scripted runner shared by the host's two adapters.
#[derive(Clone, Default)]
struct Shared(Rc<ScriptedRunner>);

impl CommandRunner for Shared {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        self.0.execute(request)
    }
}

impl Shared {
    fn json(&self, value: serde_json::Value) -> &Self {
        self.0
            .push_output(value.to_string().into_bytes(), Vec::new());
        self
    }

    fn text(&self, text: &str) -> &Self {
        self.0.push_output(text.as_bytes().to_vec(), Vec::new());
        self
    }

    fn refuse(&self, code: &str) -> &Self {
        let mut error = CommandError::new(CommandFailure::Exit {
            code: Some(1),
            signal: None,
        });
        error.output = Some(CommandOutput {
            stdout: Vec::new(),
            stderr: json!({"error": {"code": code, "message": "refused"}, "id": "cli"})
                .to_string()
                .into_bytes(),
        });
        self.0.results.borrow_mut().push_back(Err(error));
        self
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.0
            .calls
            .borrow()
            .iter()
            .map(|call| {
                std::iter::once(call.program.clone())
                    .chain(call.args.iter().cloned())
                    .collect()
            })
            .collect()
    }

    fn done(&self) {
        assert!(
            self.0.results.borrow().is_empty(),
            "unused scripted results"
        );
    }
}

fn status(version: &str) -> serde_json::Value {
    json!({"status": "running", "running": true, "version": version, "protocol": 22,
           "socket": SOCKET, "session": null})
}

fn pane(pane_id: &str, terminal: &str, tokens: Option<serde_json::Value>) -> serde_json::Value {
    let mut pane = json!({"agent_status": "unknown", "cwd": "/repo", "focused": true,
        "foreground_cwd": "/repo/src", "pane_id": pane_id, "revision": 0,
        "tab_id": "w1:t1", "terminal_id": terminal, "workspace_id": "w1"});
    if let Some(tokens) = tokens {
        pane["tokens"] = tokens;
    }
    pane
}

fn list(panes: Vec<serde_json::Value>) -> serde_json::Value {
    json!({"id": "cli:pane:list", "result": {"panes": panes, "type": "pane_list"}})
}

fn process(pane_id: &str, shell: u64, command: &str) -> serde_json::Value {
    json!({"id": "cli:pane:process_info", "result": {"process_info": {
        "foreground_process_group_id": shell,
        "foreground_processes": [{"argv": [command], "argv0": command, "cwd": "/repo",
            "name": command, "pid": shell}],
        "pane_id": pane_id, "shell_pid": shell}, "type": "pane_process_info"}})
}

fn server(pid: u64) -> ServerEvidence {
    ServerEvidence {
        host: HostKind::Herdr,
        server_id: SERVER_ID.into(),
        socket_path: SOCKET.into(),
        server_pid: pid,
        server_start_time: START.into(),
    }
}

fn identity() -> Identity {
    Identity {
        id: "e27f6cd2-2ce7-4c40-8ef7-f156492e983b".into(),
        name: "Worker".into(),
        canonical_name: "worker".into(),
        lifetime: Lifetime::Saved,
        created_at: "created".into(),
        updated_at: "updated".into(),
    }
}

fn binding(pid: u64) -> Binding {
    Binding {
        id: "7a1f2e3d-4c5b-4a69-8877-665544332211".into(),
        identity_id: identity().id,
        server: server(pid),
        pane_id: "term_65ca1161edc141".into(),
        pane_pid: 90593,
        session: Default::default(),
    }
}

fn tokens(marker: &BindingMarker) -> serde_json::Value {
    let mut tokens = serde_json::Map::new();
    for token in marker::encode(marker).unwrap() {
        let (key, value) = token.split_once('=').unwrap();
        tokens.insert(key.into(), json!(value));
    }
    serde_json::Value::Object(tokens)
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(1)
}

/// A scoped observation of `w1:p1`: list, process-info, parents, server start.
fn observation(runner: &Shared, server_pid: u64, pane_tokens: Option<serde_json::Value>) {
    runner
        .json(list(vec![
            pane("w1:p1", "term_65ca1161edc141", pane_tokens),
            pane("w1:p2", "term_65ca1161edc142", None),
        ]))
        .json(process("w1:p1", 90593, "claude"))
        .text(&format!("90593 {server_pid}\n"))
        .text("Sun Sep 27 10:00:00 2026 S\n");
}

fn session(herdr: &Herdr<Shared>) -> Session<'_, Shared> {
    let mut session = Session::new(herdr);
    session.begin_coordination();
    session
}

#[test]
fn the_floor_is_0_9_1_and_newer_releases_are_trusted() {
    for version in [
        "0.9.1",
        "0.9.2",
        "0.10.0",
        "1.0.0",
        "0.9.1-preview.3",
        "0.9.1+build",
    ] {
        assert!(check_floor(Some(version)).is_ok(), "{version}");
    }
    for version in [
        Some("0.9.0"),
        Some("0.8.9"),
        Some("garbage"),
        Some("0.9"),
        None,
    ] {
        let error = check_floor(version).unwrap_err();
        assert_eq!(error.kind, HerdrFailure::Version, "{version:?}");
        assert!(error.to_string().contains("0.9.1 or later"));
    }
}

#[test]
fn every_call_selects_its_socket_and_maps_herdr_errors() {
    let runner = Shared::default();
    runner.refuse("pane_not_found").text("not json");
    let herdr = Herdr::new(runner.clone(), None);
    let error = herdr
        .call(Some(SOCKET), &["pane", "get", "w9:p9"], deadline())
        .unwrap_err();
    assert_eq!(error.code(), Some("pane_not_found"));
    assert_eq!(error.kind, HerdrFailure::Api);
    let error = herdr
        .call(Some(SOCKET), &["pane", "list"], deadline())
        .unwrap_err();
    assert_eq!(error.kind, HerdrFailure::Evidence);
    assert_eq!(
        runner.calls()[0],
        [
            "/usr/bin/env",
            &format!("HERDR_SOCKET_PATH={SOCKET}"),
            "herdr",
            "pane",
            "get",
            "w9:p9"
        ]
    );
    runner.done();
}

#[test]
fn a_snapshot_names_panes_by_terminal_and_the_server_by_its_process() {
    let runner = Shared::default();
    let marker = binding(90316).marker(&identity());
    observation(&runner, 90316, Some(tokens(&marker)));
    let herdr = Herdr::new(runner.clone(), None);
    herdr.set_resolved(server(90316));
    let snapshot = session(&herdr)
        .snapshot(Some(&["term_65ca1161edc141".to_owned()]))
        .unwrap();
    assert_eq!(snapshot.server, server(90316));
    let [pane] = snapshot.panes.as_slice() else {
        panic!("one scoped pane: {:?}", snapshot.panes);
    };
    assert_eq!(pane.id, "term_65ca1161edc141");
    assert_eq!(pane.target.as_deref(), Some("w1:p1"));
    assert_eq!(pane.cwd.as_deref(), Some("/repo/src"));
    assert_eq!(pane.command, "claude");
    assert_eq!(pane.suggested_name.as_deref(), Some("claude"));
    assert_eq!(pane.pane_pid, 90593);
    assert_eq!(pane.marker.as_ref(), Some(&marker));
    let calls = runner.calls();
    assert_eq!(calls[1][3..], ["pane", "process-info", "--pane", "w1:p1"]);
    assert!(calls[2].contains(&"pid=,ppid=".to_owned()));
    runner.done();

    // A restarted server is never mistaken for the resolved one.
    let runner = Shared::default();
    observation(&runner, 777, None);
    let herdr = Herdr::new(runner.clone(), None);
    herdr.set_resolved(server(90316));
    let error = session(&herdr)
        .snapshot(Some(&["term_65ca1161edc141".to_owned()]))
        .unwrap_err();
    assert!(error.to_string().contains("restarted"));
    // Without a resolution there is no snapshot at all.
    let herdr = Herdr::new(Shared::default(), None);
    assert!(session(&herdr).snapshot(Some(&[])).is_err());
}

#[test]
fn panes_disagreeing_about_their_server_are_not_evidence() {
    let runner = Shared::default();
    runner
        .json(list(vec![
            pane("w1:p1", "term_1", None),
            pane("w1:p2", "term_2", None),
        ]))
        .json(process("w1:p1", 501, "sh"))
        .json(process("w1:p2", 502, "sh"))
        .text("501 90316\n502 4242\n");
    let herdr = Herdr::new(runner.clone(), None);
    let error = herdr.observe(SOCKET, None, deadline()).err().unwrap();
    assert!(error.to_string().contains("disagree"));
    runner.done();
}

#[test]
fn malformed_or_repeated_panes_are_refused() {
    for panes in [
        vec![pane("%1", "term_1", None)],
        vec![pane("w1:p1", "%1", None)],
        vec![pane("w1:p1", "term_1", None), pane("w1:p2", "term_1", None)],
    ] {
        assert!(evidence::listed(&list(panes)["result"]).is_err());
    }
}

#[test]
fn a_probe_is_live_only_for_the_same_incarnation_and_dead_only_on_esrch() {
    let panes = ["term_65ca1161edc141".to_owned()];
    // Same server process and start: live, with the stored evidence.
    let runner = Shared::default();
    runner.json(status("0.9.1"));
    observation(&runner, 90316, None);
    let herdr = Herdr::new(runner.clone(), None);
    let EndpointProbe::Live(snapshot) = session(&herdr).probe(&server(90316), &panes).unwrap()
    else {
        panic!("same incarnation is live");
    };
    assert_eq!(snapshot.server, server(90316));
    runner.done();

    // Another incarnation at the socket: loss only if the old process is gone.
    for (recorded, expected) in [
        (GONE_PID, EndpointProbe::Dead),
        (u64::from(std::process::id()), EndpointProbe::Unknown),
    ] {
        let runner = Shared::default();
        runner.json(status("0.9.1"));
        observation(&runner, 777, None);
        let herdr = Herdr::new(runner.clone(), None);
        assert_eq!(
            session(&herdr).probe(&server(recorded), &panes).unwrap(),
            expected
        );
    }

    // No server at the socket: the same rule. A refused status is unknown.
    let runner = Shared::default();
    runner.refuse("server_not_running");
    let herdr = Herdr::new(runner.clone(), None);
    assert_eq!(
        session(&herdr).probe(&server(GONE_PID), &panes).unwrap(),
        EndpointProbe::Dead
    );
    let runner = Shared::default();
    runner.refuse("internal");
    let herdr = Herdr::new(runner.clone(), None);
    assert_eq!(
        session(&herdr).probe(&server(GONE_PID), &panes).unwrap(),
        EndpointProbe::Unknown
    );
    // Another host's server is never evidence here.
    let tmux = ServerEvidence {
        host: HostKind::Tmux,
        ..server(GONE_PID)
    };
    assert_eq!(
        session(&herdr).probe(&tmux, &panes).unwrap(),
        EndpointProbe::Unknown
    );
}

#[test]
fn a_marker_round_trips_and_long_names_span_continuation_tokens() {
    let mut marker = binding(90316).marker(&identity());
    assert_eq!(
        marker::decode(tokens(&marker).as_object()),
        Some(marker.clone())
    );
    marker.name = "N".repeat(200);
    marker.canonical_name = "n".repeat(200);
    let encoded = marker::encode(&marker).unwrap();
    assert!(
        encoded
            .iter()
            .all(|token| token.split_once('=').unwrap().1.chars().count() <= 80)
    );
    assert!(encoded.iter().any(|token| token.starts_with("tmt_name_3=")));
    assert_eq!(
        marker::decode(tokens(&marker).as_object()),
        Some(marker.clone())
    );
    marker.name = "N".repeat(321);
    assert_eq!(marker::encode(&marker), None);
    // Incomplete or malformed tokens are no marker.
    let mut partial = tokens(&binding(90316).marker(&identity()));
    partial.as_object_mut().unwrap().remove("tmt_server");
    assert_eq!(marker::decode(partial.as_object()), None);
    let mut bad_pid = tokens(&binding(90316).marker(&identity()));
    bad_pid["tmt_pid"] = json!("0");
    assert_eq!(marker::decode(bad_pid.as_object()), None);
}

#[test]
fn publishing_reports_tokens_under_tmt_and_verifies_them() {
    let binding = binding(90316);
    let marker = binding.marker(&identity());
    // `report-metadata` prints nothing on success.
    let runner = Shared::default();
    runner
        .json(list(vec![pane("w1:p1", &binding.pane_id, None)]))
        .text("")
        .json(list(vec![pane(
            "w1:p1",
            &binding.pane_id,
            Some(tokens(&marker)),
        )]));
    let herdr = Herdr::new(runner.clone(), None);
    session(&herdr).publish(&binding, &identity()).unwrap();
    let report = &runner.calls()[1];
    assert_eq!(
        report[3..8],
        ["pane", "report-metadata", "w1:p1", "--source", "tmt"]
    );
    assert!(report.contains(&format!("tmt_binding={}", binding.id)));
    runner.done();

    // A report Herdr dropped is an error, never a silent success.
    let runner = Shared::default();
    runner
        .json(list(vec![pane("w1:p1", &binding.pane_id, None)]))
        .text("")
        .json(list(vec![pane("w1:p1", &binding.pane_id, None)]));
    let herdr = Herdr::new(runner.clone(), None);
    assert!(session(&herdr).publish(&binding, &identity()).is_err());
}

#[test]
fn clearing_removes_only_this_bindings_marker() {
    let binding = binding(90316);
    let mut foreign = binding.clone();
    foreign.id = "00000000-0000-4000-8000-000000000001".into();
    let runner = Shared::default();
    runner.json(list(vec![pane(
        "w1:p1",
        &binding.pane_id,
        Some(tokens(&foreign.marker(&identity()))),
    )]));
    let herdr = Herdr::new(runner.clone(), None);
    assert!(!session(&herdr).clear(&binding).unwrap());
    assert_eq!(runner.calls().len(), 1);

    let runner = Shared::default();
    runner
        .json(list(vec![pane(
            "w1:p1",
            &binding.pane_id,
            Some(tokens(&binding.marker(&identity()))),
        )]))
        .text("");
    let herdr = Herdr::new(runner.clone(), None);
    assert!(session(&herdr).clear(&binding).unwrap());
    let report = &runner.calls()[1];
    assert!(report.contains(&"--clear-token".to_owned()));
    assert!(report.contains(&"tmt_binding".to_owned()));
    runner.done();
}

fn environment(pane: Option<&str>) -> CallerEnvironment {
    CallerEnvironment {
        tmux: None,
        pane: None,
        herdr_pane: pane.map(Into::into),
        herdr_socket: Some(SOCKET.into()),
        process_id: 4000,
    }
}

#[test]
fn a_caller_pane_counts_only_when_its_shell_is_an_ancestor() {
    for (chain, expected) in [
        (
            vec!["4000 90593\n", "90593 90316\n", "90316 1\n", "1 0\n"],
            Some(1),
        ),
        (vec!["4000 1\n", "1 0\n"], None),
    ] {
        let runner = Shared::default();
        runner
            .json(status("0.9.1"))
            .json(json!({"id": "cli:pane:get", "result": {"pane":
                pane("w1:p1", "term_65ca1161edc141", None), "type": "pane_info"}}))
            .json(process("w1:p1", 90593, "sh"));
        for line in chain {
            runner.text(line);
        }
        let herdr = Herdr::new(runner.clone(), None);
        let found = herdr
            .caller(&environment(Some("w1:p1")), deadline())
            .unwrap();
        assert_eq!(found.as_ref().map(|pane| pane.depth), expected);
        if let Some(found) = found {
            assert_eq!(found.terminal_id, "term_65ca1161edc141");
        }
        runner.done();
    }
    // No Herdr pane named, or one that is not Herdr syntax: nothing is run.
    let runner = Shared::default();
    let herdr = Herdr::new(runner.clone(), None);
    for pane in [None, Some("%1"), Some("term_1")] {
        assert_eq!(herdr.caller(&environment(pane), deadline()).unwrap(), None);
    }
    assert!(runner.calls().is_empty());
}

struct Ids(Vec<String>);

impl HostServerIds for Ids {
    type Error = Infallible;

    fn server_id(&mut self, server: &HostServerIncarnation<'_>) -> Result<String, Infallible> {
        self.0.push(format!(
            "{}|{}|{}|{}",
            server.host.as_str(),
            server.socket_path,
            server.server_pid,
            server.server_start_time
        ));
        Ok(SERVER_ID.into())
    }
}

#[test]
fn a_herdr_caller_resolves_its_server_once_before_binding() {
    let runner = Shared::default();
    // resolve_servers: status, then a witness pane for the server process.
    runner
        .json(status("0.9.1"))
        .json(list(vec![pane("w1:p1", "term_65ca1161edc141", None)]))
        .json(process("w1:p1", 90593, "sh"))
        .text("90593 90316\n")
        .text("Sun Sep 27 10:00:00 2026 S\n");
    let host = Host::for_caller_with(&environment(Some("w1:p1")), runner.clone());
    assert_eq!(host.kind(), HostKind::Herdr);
    let mut ids = Ids(Vec::new());
    host.resolve_servers(&mut ids).unwrap();
    assert_eq!(ids.0, [format!("herdr|{SOCKET}|90316|{START}")]);
    // A second resolution asks nothing.
    host.resolve_servers(&mut ids).unwrap();
    assert_eq!(ids.0.len(), 1);
    // The binding transaction's snapshot carries the resolved server.
    observation(&runner, 90316, None);
    let mut session = host.session();
    session.begin_coordination();
    let snapshot = session
        .current_snapshot(&["term_65ca1161edc141".to_owned()])
        .unwrap();
    assert_eq!(snapshot.server, server(90316));
    runner.done();
}

#[test]
fn a_tmux_session_probes_a_herdr_binding_on_herdr() {
    let entry = BindingEntry {
        identity: identity(),
        binding: Some(binding(90316)),
    };
    let marker = binding(90316).marker(&identity());
    let runner = Shared::default();
    runner.json(status("0.9.1"));
    observation(&runner, 90316, Some(tokens(&marker)));
    let host = Host::for_caller_with(&environment(None), runner.clone());
    assert_eq!(host.kind(), HostKind::Tmux);
    let mut session = host.session();
    session.begin_coordination();
    let binding = entry.binding.as_ref().unwrap();
    let probe = session
        .probe_binding(&binding.server, std::slice::from_ref(&binding.pane_id))
        .unwrap();
    assert!(matches!(
        evaluate_binding(&entry, &probe),
        BindingEvidence::Active(_)
    ));
    assert!(runner.calls().iter().all(|call| call[0] == "/usr/bin/env"));
    runner.done();
}

/// Herdr 0.9.1's observed `report-metadata` semantics: tokens merge key by
/// key into the source's set, then `--clear-token` keys are removed.
fn apply_report(tokens: &serde_json::Value, report: &[String]) -> serde_json::Value {
    let mut merged = tokens.as_object().cloned().unwrap_or_default();
    for pair in report.windows(2) {
        match pair[0].as_str() {
            "--token" => {
                let (key, value) = pair[1].split_once('=').unwrap();
                merged.insert(key.into(), json!(value));
            }
            "--clear-token" => {
                merged.remove(&pair[1]);
            }
            _ => {}
        }
    }
    serde_json::Value::Object(merged)
}

#[test]
fn a_rename_from_a_long_name_leaves_no_stale_name_parts() {
    use crate::host::PaneRefresh;
    let binding = binding(90316);
    let long = Identity {
        name: "L".repeat(200),
        canonical_name: "l".repeat(200),
        ..identity()
    };
    let before = tokens(&binding.marker(&long));
    assert!(before.get("tmt_name_3").is_some());
    let renamed = Identity {
        name: "Lead".into(),
        canonical_name: "lead".into(),
        ..identity()
    };
    let expected = tokens(&binding.marker(&renamed));
    // The report depends only on the binding and the new name; take it from
    // a first publish, then script what Herdr keeps when it is applied.
    let probe = Shared::default();
    probe
        .json(list(vec![pane("w1:p1", &binding.pane_id, None)]))
        .text("")
        .json(list(vec![pane("w1:p1", &binding.pane_id, None)]));
    let _ = session(&Herdr::new(probe.clone(), None)).publish(&binding, &renamed);
    let kept = apply_report(&before, &probe.calls()[1]);
    assert_eq!(kept, expected);

    let runner = Shared::default();
    runner
        .json(list(vec![pane(
            "w1:p1",
            &binding.pane_id,
            Some(before.clone()),
        )]))
        .json(list(vec![pane(
            "w1:p1",
            &binding.pane_id,
            Some(before.clone()),
        )]))
        .text("")
        .json(list(vec![pane("w1:p1", &binding.pane_id, Some(kept))]));
    let herdr = Herdr::new(runner.clone(), None);
    assert_eq!(
        session(&herdr).refresh_name(&binding, &renamed).unwrap(),
        PaneRefresh::Updated
    );
    assert_eq!(apply_report(&before, &runner.calls()[2]), expected);
    runner.done();
}

#[test]
fn a_rename_rewrites_only_this_bindings_stale_marker_name() {
    use crate::host::PaneRefresh;
    let binding = binding(90316);
    let stale = binding.marker(&identity());
    let renamed = Identity {
        name: "Lead".into(),
        canonical_name: "lead".into(),
        ..identity()
    };
    let current = binding.marker(&renamed);
    let runner = Shared::default();
    runner
        .json(list(vec![pane(
            "w1:p1",
            &binding.pane_id,
            Some(tokens(&stale)),
        )]))
        .json(list(vec![pane(
            "w1:p1",
            &binding.pane_id,
            Some(tokens(&stale)),
        )]))
        .text("")
        .json(list(vec![pane(
            "w1:p1",
            &binding.pane_id,
            Some(tokens(&current)),
        )]));
    let herdr = Herdr::new(runner.clone(), None);
    assert_eq!(
        session(&herdr).refresh_name(&binding, &renamed).unwrap(),
        PaneRefresh::Updated
    );
    assert!(runner.calls()[2].contains(&"tmt_name=Lead".to_owned()));
    runner.done();

    // Another binding's marker, or none, is not this binding's pane.
    let mut foreign = binding.clone();
    foreign.id = "00000000-0000-4000-8000-000000000001".into();
    let runner = Shared::default();
    runner.json(list(vec![pane(
        "w1:p1",
        &binding.pane_id,
        Some(tokens(&foreign.marker(&identity()))),
    )]));
    let herdr = Herdr::new(runner.clone(), None);
    assert_eq!(
        session(&herdr).refresh_name(&binding, &renamed).unwrap(),
        PaneRefresh::Absent
    );
    runner.done();
}
