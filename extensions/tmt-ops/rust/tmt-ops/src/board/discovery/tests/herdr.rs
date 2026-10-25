//! Herdr rows on the public discovery and guarded add JSON. Ops reads only
//! the CLI documents; the fake core script runs nothing on a host.
use super::*;

const HERDR_SOCKET: &str = "/tmp/private-herdr.sock";
const AGENT_NAME: &str = "$(never)\n--identity";

fn herdr_row(pane: &str) -> Value {
    json!({"host":"herdr","socket":HERDR_SOCKET,"server":{"pid":321,"start":"herdr-server-start","identity":null},"session":null,"window":null,"pane":pane,"target":"w1:p2","native":{"pid":654,"start":"herdr-pane-start"},
        "runtime":{"driver":"claude","state":"unknown","reason":"executable_hint_is_advisory","recordedState":null},"identity":null,"state":"unknown","reason":null,"duplicate":false,
        "display":{"title":"herdr title","command":"claude","cwd":"/tmp/private","truncated":false,"agentName":AGENT_NAME},"adoptable":true,"expect":DIGEST})
}
fn herdr_document() -> Value {
    json!({"version":1,"observedAtMs":1000,"truncated":false,
        "sources":[{"host":"herdr","socket":HERDR_SOCKET,"status":"ok","reason":null}],
        "candidates":[herdr_row("term_0a9z"),herdr_row("term_1")]})
}
fn herdr_controller() -> Controller {
    let mut c = Controller::new(1);
    let task = c.scan().unwrap();
    c.loaded(load::Completed {
        task,
        result: Ok(Outcome::Scan(
            ScanObservation::decode(&herdr_document()).unwrap(),
        )),
    });
    let key = c.observation.as_ref().unwrap().panes[0].key.clone();
    c.select(&key);
    c
}
fn not_adoptable(row: &mut Value) {
    row["server"] = Value::Null;
    row["native"] = Value::Null;
    row["expect"] = Value::Null;
    row["adoptable"] = false.into();
    row["reason"] = "native_evidence_missing".into();
}

#[test]
fn a_herdr_ok_source_gives_adoptable_rows_with_target_and_agent_name() {
    let scan = ScanObservation::decode(&herdr_document()).unwrap();
    assert_eq!(scan.panes.len(), 2);
    let p = &scan.panes[0];
    assert_eq!(p.key.host, "herdr");
    assert_eq!(p.key.pane, "term_0a9z");
    assert_eq!(p.key.socket.as_deref(), Some(HERDR_SOCKET));
    assert!(p.adoptable);
    assert_eq!(p.session, None);
    assert_eq!(p.target.as_deref(), Some("w1:p2"));
    assert_eq!(p.agent_name.as_deref(), Some(AGENT_NAME));
}

#[test]
fn a_herdr_row_without_a_fence_is_unavailable() {
    let mut v = herdr_document();
    not_adoptable(&mut v["candidates"][1]);
    let scan = ScanObservation::decode(&v).unwrap();
    assert!(!scan.panes[1].adoptable);
    let mut c = herdr_controller();
    c.observation = Some(scan);
    let key = c.observation.as_ref().unwrap().panes[1].key.clone();
    c.select(&key);
    assert!(!c.review(AgentSelector::NewName("worker".into()), false));
    assert!(c.rows().iter().any(|r| r.id == "bind" && r.disabled));
}

#[test]
fn a_herdr_row_needs_its_own_ok_source_and_a_bounded_pane_literal() {
    // An adoptable row whose host and socket match no `ok` source.
    let mut v = herdr_document();
    v["sources"] = json!([{"host":"tmux","socket":HERDR_SOCKET,"status":"ok","reason":null}]);
    assert!(ScanObservation::decode(&v).is_err());
    let mut v = herdr_document();
    v["sources"][0]["status"] = "unknown".into();
    assert!(ScanObservation::decode(&v).is_err());
    for socket in [Value::Null, "".into()] {
        let mut v = herdr_document();
        v["candidates"][0]["socket"] = socket;
        assert!(ScanObservation::decode(&v).is_err());
    }
    // tmux keeps its own pane syntax.
    let mut v = document();
    v["candidates"][0]["pane"] = "term_1".into();
    assert!(ScanObservation::decode(&v).is_err());
    // Ops bounds another host's pane literal at 80 bytes.
    let long = format!("{}_{}", "a".repeat(15), "b".repeat(65));
    assert_eq!(long.len(), 81);
    for pane in [
        "",
        "term_\u{1b}[0m",
        "term 1",
        "term_1\n",
        "term_é",
        long.as_str(),
    ] {
        let mut v = herdr_document();
        v["candidates"][0]["pane"] = pane.into();
        assert!(ScanObservation::decode(&v).is_err(), "{pane:?}");
    }
    let mut v = herdr_document();
    v["candidates"][0]["pane"] = long[..80].into();
    assert!(ScanObservation::decode(&v).is_ok());
    let mut v = herdr_document();
    v["candidates"][0]["display"]["agentName"] = "a".repeat(1025).into();
    assert!(ScanObservation::decode(&v).is_err());
    let mut v = herdr_document();
    v["candidates"][0]["display"]["agentName"] = 7.into();
    assert!(ScanObservation::decode(&v).is_err());
}

#[test]
fn gap_offline_and_ambiguous_herdr_sources_are_data_beside_tmux() {
    let mut v = document();
    v["sources"]
        .as_array_mut()
        .unwrap()
        .push(json!({"host":"herdr","socket":null,"status":"unsupported","reason":"driver_approval_outdated"}));
    let scan = ScanObservation::decode(&v).unwrap();
    assert!(scan.panes[0].adoptable);
    assert_eq!(
        scan.sources[1].reason.as_deref(),
        Some("driver_approval_outdated")
    );
    let v = json!({"version":1,"observedAtMs":1000,"truncated":false,"sources":[{"host":"herdr","socket":null,"status":"offline","reason":null}],"candidates":[]});
    assert_eq!(
        ScanObservation::decode(&v).unwrap().sources[0].status,
        "offline"
    );
    let v = json!({"version":1,"observedAtMs":1000,"truncated":true,"sources":[
        {"host":"herdr","socket":null,"status":"unknown","reason":"ambiguous_caller_host"},
        {"host":"zellij","socket":null,"status":"unknown","reason":"ambiguous_caller_host"}],"candidates":[]});
    let scan = ScanObservation::decode(&v).unwrap();
    assert!(scan.truncated);
    assert_eq!(scan.sources.len(), 2);
    // A Herdr row that a refused source reported cannot be adopted.
    let mut v = herdr_document();
    v["sources"][0]["status"] = "unsupported".into();
    v["sources"][0]["reason"] = "driver_approval_outdated".into();
    v["candidates"] = json!([]);
    assert!(ScanObservation::decode(&v).unwrap().panes.is_empty());
}

#[test]
fn the_same_pane_text_on_two_hosts_keeps_two_keys() {
    let mut v = document();
    let mut herdr = herdr_row("%3");
    herdr["socket"] = SOCKET.into();
    v["sources"]
        .as_array_mut()
        .unwrap()
        .push(json!({"host":"herdr","socket":SOCKET,"status":"ok","reason":null}));
    v["candidates"].as_array_mut().unwrap().push(herdr);
    let scan = ScanObservation::decode(&v).unwrap();
    assert_eq!(scan.panes.len(), 2);
    assert_ne!(scan.panes[0].key, scan.panes[1].key);
}

#[test]
fn a_herdr_bind_passes_the_row_socket_and_pane() {
    let fixture = PublicCommandFixture::new();
    fixture.response(&herdr_document(), 0);
    let mut c = Controller::new(1);
    let task = c.scan().unwrap();
    c.loaded(load::complete(&fixture.core(), task));
    // No host or socket flag: Core selects the caller's host.
    assert_eq!(fixture.arguments(), ["discover", "--json"]);
    std::fs::remove_file(fixture.0.join("calls")).unwrap();
    let key = c.observation.as_ref().unwrap().panes[0].key.clone();
    c.select(&key);
    assert!(c.review(AgentSelector::NewName("worker".into()), false));
    c.readable.set(true);
    let task = c.confirm().unwrap();
    fixture.response(
        &json!({"version":1,"status":"bound","bound":true,"id":UUID,"name":"worker","pane":"term_0a9z","lifetime":"temporary",
            "agentName":{"status":"set","name":"worker"}}),
        0,
    );
    c.loaded(load::complete(&fixture.core(), task));
    assert_eq!(
        fixture.arguments(),
        [
            "add",
            "--expect",
            DIGEST,
            "--socket",
            HERDR_SOCKET,
            "--json",
            "--",
            "term_0a9z",
            "worker"
        ]
    );
    assert!(matches!(c.attempts[0].result, Ok(BindReport::Bound { .. })));
    let mut c = herdr_controller();
    assert!(c.review(AgentSelector::ExistingUuid(UUID.into()), true));
    c.readable.set(true);
    let (args, operands) = c.confirm().unwrap().job.arguments();
    assert_eq!(operands, ["term_0a9z"]);
    assert!(args.windows(2).any(|s| s == ["--socket", HERDR_SOCKET]));
    assert!(args.windows(2).any(|s| s == ["--identity", UUID]));
}

#[test]
fn the_board_scans_with_no_host_or_socket_flag_in_a_pane_and_in_a_popup() {
    // Core's caller selection keeps the server that `TMUX` names, so a popup
    // needs no explicit socket, and a gap host beside tmux stays visible.
    for popup in [false, true] {
        let fixture = PublicCommandFixture::new();
        fixture.response(&herdr_document(), 0);
        let mut app = crate::board::app::workspace::tests::board();
        app.popup = popup;
        assert_eq!(app.open_discovery(), crate::board::app::Effect::None);
        let c = app.discovery.as_mut().unwrap();
        let task = c.choose("scan").unwrap();
        c.loaded(load::complete(&fixture.core(), task));
        assert_eq!(fixture.arguments(), ["discover", "--json"], "popup {popup}");
        assert!(c.observation.is_some());
    }
}

#[test]
fn the_host_agent_name_is_inert_and_never_fills_the_name_field() {
    let mut c = herdr_controller();
    let details = c.notes();
    assert!(
        details.contains(
            &r#"Host agent name: "$(never)\n--identity" (from the host, not checked)"#.to_owned()
        ),
        "{details:?}"
    );
    let rendered = paint(&c, 160, 36);
    assert!(rendered.contains(r#""$(never)\n--identity""#));
    assert!(!rendered.contains('\u{1b}'));
    c.choose("bind");
    assert_eq!(c.screen, Screen::Form);
    assert!(!c.existing);
    assert_eq!(c.editor.as_str(), "");
    assert!(c.pending.is_none());
}

#[test]
fn a_herdr_candidate_names_its_host_and_target_without_session_or_window() {
    let mut c = herdr_controller();
    c.choose("back");
    let row = &c.rows()[0];
    assert!(
        row.label.starts_with("term_0a9z · herdr w1:p2 · "),
        "{}",
        row.label
    );
    assert!(!row.label.contains("session"));
    c.choose("candidate:0");
    let details = c.notes();
    assert!(details.iter().any(|l| l == "Target w1:p2"), "{details:?}");
    assert!(
        !details
            .iter()
            .any(|l| l.contains("Session") || l.contains("window"))
    );
    // tmux rows keep their session and window.
    let c = controller();
    assert!(
        c.notes()
            .iter()
            .any(|l| l == "Session work · window 0 · target work:0.0")
    );
}

#[test]
fn a_herdr_review_names_the_host_and_its_socket() {
    let mut c = herdr_controller();
    assert!(c.review(AgentSelector::NewName("worker".into()), false));
    assert!(
        c.notes()
            .contains(&format!("Target: term_0a9z on herdr {HERDR_SOCKET}"))
    );
}

#[test]
fn a_gap_source_shows_its_reason_and_the_install_text_only() {
    for (reason, fix) in [
        (
            "driver_not_approved",
            "◆ Action needed: approve the herdr driver to scan it: tmt driver install herdr",
        ),
        (
            "driver_approval_outdated",
            "◆ Action needed: approve the herdr driver again: tmt driver install herdr",
        ),
        (
            "driver_not_shipped",
            "◆ Action needed: this tmt release does not ship the herdr driver. Upgrade tmt with: tmt upgrade, then approve it: tmt driver install herdr",
        ),
    ] {
        let mut v = document();
        v["sources"]
            .as_array_mut()
            .unwrap()
            .push(json!({"host":"herdr","socket":null,"status":"unsupported","reason":reason}));
        let mut c = controller();
        c.observation = Some(ScanObservation::decode(&v).unwrap());
        c.choose("back");
        let notes = c.notes();
        assert!(notes.contains(&format!(
            "herdr: no server selected · unsupported · {reason}"
        )));
        assert!(notes.contains(&fix.to_owned()), "{notes:?}");
        // One tmux row, Scan again and Close: the gap source has no row and no action.
        let ids: Vec<_> = c.rows().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["candidate:0", "scan", "close"]);
    }
}

#[test]
fn herdr_partial_and_refused_reports_survive_close_and_rescan_without_replay() {
    let mut c = herdr_controller();
    let task = bind(&mut c);
    c.close();
    finished(
        &mut c,
        task,
        Ok(Outcome::Bind(BindReport::Partial {
            code: "CLEANUP_ERROR".into(),
            message: "Naming child not cleaned up".into(),
        })),
    );
    c.open();
    assert!(c.reload_blocked());
    let task = c.scan().unwrap();
    finished(
        &mut c,
        task,
        Ok(Outcome::Scan(
            ScanObservation::decode(&herdr_document()).unwrap(),
        )),
    );
    let key = c.observation.as_ref().unwrap().panes[1].key.clone();
    c.select(&key);
    let task = bind(&mut c);
    c.close();
    finished(
        &mut c,
        task,
        Ok(Outcome::Bind(BindReport::Refused {
            code: "USAGE_ERROR".into(),
            message: "Pane syntax is not admitted".into(),
        })),
    );
    c.open();
    assert_eq!(c.attempts.len(), 2);
    assert_eq!(c.attempts[0].selection.key.pane, "term_0a9z");
    assert_eq!(c.attempts[1].selection.key.pane, "term_1");
    assert!(c.pending.is_none());
    assert!(c.reload_blocked());
}

#[test]
fn a_herdr_bind_returns_the_bound_uuid_once_and_shows_the_host_agent_name() {
    let mut c = herdr_controller();
    let Job::Bind(selection) = bind(&mut c).job else {
        panic!("bind")
    };
    let mut c = herdr_controller();
    let task = bind(&mut c);
    let bound = |agent: Value| {
        let mut v = json!({"version":1,"status":"bound","bound":true,"id":UUID,"name":"worker","pane":"term_0a9z","lifetime":"temporary"});
        if let Some(object) = agent.as_object() {
            v.as_object_mut().unwrap().extend(object.clone());
        }
        BindReport::decode(&v, &selection, true).unwrap()
    };
    let completed = finished(
        &mut c,
        task.clone(),
        Ok(Outcome::Bind(bound(
            json!({"agentName":{"status":"set","name":"worker"}}),
        ))),
    );
    assert_eq!(completed.bound_uuid.as_deref(), Some(UUID));
    assert!(
        finished(&mut c, task, Err(model::invalid()))
            .bound_uuid
            .is_none()
    );
    assert_eq!(c.attempts.len(), 1);
    let line = |c: &Controller| {
        c.notes()
            .into_iter()
            .filter(|l| l.starts_with("Host agent name"))
            .collect::<Vec<_>>()
    };
    assert_eq!(line(&c), ["Host agent name: set to \"worker\""]);
    for (agent, expected) in [
        (
            json!({"agentName":{"status":"not_linked"}}),
            "Host agent name: the host has no agent names",
        ),
        (
            json!({"agentName":{"status":"refused","name":"worker","message":"$(never)\nname taken"}}),
            r#"Host agent name: not set: $(never)\nname taken"#,
        ),
        (
            json!({"warnings":[{"code":"AGENT_NAME_UNCONFIRMED","message":"TMT could not confirm the agent name"}]}),
            "Host agent name: TMT could not confirm the agent name",
        ),
    ] {
        c.attempts[0].result = Ok(bound(agent));
        assert_eq!(line(&c), [expected]);
    }
    // A tmux bind has no host agent name, and other warnings are not one.
    c.attempts[0].result = Ok(bound(json!({"warnings":[{"code":"OTHER","message":"x"}]})));
    assert!(line(&c).is_empty());
    c.attempts[0].result = Ok(bound(json!({"agentName":{"status":"later"}})));
    assert!(line(&c).is_empty());
}
