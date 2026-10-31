//! Discovery vectors use the existing workspace ANSI measurement owner. No
//! command executes, and the fixed in-memory viewport never opens a host TTY.
use super::*;
use crate::board::{
    app::{App, Effect, workspace::tests::populated_board},
    conversation::tests::performance::{AnsiSink, ansi_measure, ansi_report},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, layout::Rect};

/// Half tmux and half Herdr rows: Herdr rows have no session or window and
/// carry a host agent name.
fn inventory() -> ScanObservation {
    let mut value = document();
    let prototype = value["candidates"][0].clone();
    value["sources"].as_array_mut().unwrap().push(
        json!({"host":"herdr","socket":"/tmp/private-herdr.sock","status":"ok","reason":null}),
    );
    value["candidates"] = Value::Array(
        (0..256)
            .map(|i| {
                let mut pane = prototype.clone();
                if i % 2 == 0 {
                    pane["pane"] = format!("%{i}").into();
                } else {
                    pane["host"] = "herdr".into();
                    pane["socket"] = "/tmp/private-herdr.sock".into();
                    pane["session"] = Value::Null;
                    pane["window"] = Value::Null;
                    pane["pane"] = format!("term_{i:x}").into();
                    pane["target"] = format!("w{i}:p1").into();
                    pane["display"]["agentName"] = "literal agent ".repeat(70).into();
                }
                pane["display"]["title"] = "literal title ".repeat(70).into();
                pane["display"]["cwd"] = "/private/observed/".repeat(55).into();
                pane
            })
            .collect(),
    );
    ScanObservation::decode(&value).unwrap()
}
fn observed(app: &mut App, scan: ScanObservation) {
    let controller = app.discovery.as_mut().unwrap();
    let task = controller.scan().unwrap();
    controller.loaded(load::Completed {
        task,
        result: Ok(Outcome::Scan(scan)),
    });
}

#[test]
#[ignore = "local release discovery phase/ANSI evidence; no source acquisition or CI threshold"]
fn discovery_ansi_phase_timing() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    let output = std::env::var_os("TMT_DISCOVERY_ANSI_SAMPLES")
        .expect("set a task-owned TMT_DISCOVERY_ANSI_SAMPLES path");
    let mut reports = Vec::new();
    crate::status::with_now_ms(1_791_600_000_000, || {
        for agents in [150, 1_000, 10_000] {
            let mut app = populated_board(36, agents);
            app.focus = 0;
            let sink = AnsiSink::default();
            let mut terminal = Terminal::with_options(
                ratatui::backend::CrosstermBackend::new(sink.clone()),
                ratatui::TerminalOptions {
                    viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, 160, 36)),
                },
            )
            .unwrap();
            let mut previous = None;
            for _ in 0..10 {
                ansi_measure(&mut app, &mut terminal, &sink, &mut previous, |_| {});
            }
            // Four vectors × 100 measured samples reuse the phase probe's
            // accounting. Each also has a setup draw outside the sample;
            // whole-process CPU includes that extra work and is reported
            // separately from the existing workspace CPU profile.
            for action in [
                "discovery-open",
                "scan-result-select",
                "detail-scroll",
                "name-paste-1KiB",
            ] {
                if action != "discovery-open" {
                    observed(&mut app, inventory());
                }
                let mut samples = Vec::new();
                for index in 0..100 {
                    let scan = if action == "scan-result-select" {
                        Some(inventory())
                    } else {
                        None
                    };
                    if action == "discovery-open" {
                        if let Some(controller) = &mut app.discovery {
                            controller.close();
                        }
                    } else if action == "detail-scroll" {
                        let controller = app.discovery.as_mut().unwrap();
                        let key = controller.observation.as_ref().unwrap().panes[0]
                            .key
                            .clone();
                        controller.select(&key);
                        controller.form_focus = true;
                    } else if action == "name-paste-1KiB" {
                        let controller = app.discovery.as_mut().unwrap();
                        let key = controller.observation.as_ref().unwrap().panes[0]
                            .key
                            .clone();
                        controller.select(&key);
                        controller.choose("bind");
                    }
                    ansi_measure(&mut app, &mut terminal, &sink, &mut previous, |_| {});
                    samples.push(ansi_measure(
                        &mut app,
                        &mut terminal,
                        &sink,
                        &mut previous,
                        |app| match action {
                            "discovery-open" => assert_eq!(app.open_discovery(), Effect::None),
                            "scan-result-select" => {
                                observed(app, scan.unwrap());
                                assert_eq!(
                                    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                                    Effect::None
                                );
                            }
                            "detail-scroll" => {
                                assert_eq!(
                                    app.key(KeyEvent::new(
                                        if index % 2 == 0 {
                                            KeyCode::PageDown
                                        } else {
                                            KeyCode::PageUp
                                        },
                                        KeyModifiers::NONE,
                                    )),
                                    Effect::None
                                );
                            }
                            "name-paste-1KiB" => {
                                assert_eq!(app.paste(&"x".repeat(1024)), Effect::None)
                            }
                            _ => unreachable!(),
                        },
                    ));
                    if action == "name-paste-1KiB" {
                        assert_eq!(app.discovery.as_ref().unwrap().editor.as_str().len(), 1024);
                    }
                    assert!(app.discovery.as_ref().unwrap().pending.is_none());
                }
                reports.push(ansi_report(agents, action, &samples));
            }
        }
    });
    std::fs::write(output, serde_json::to_vec_pretty(&json!({
        "version":1, "scope":"four discovery event/paint/diff+ANSI memory-sink vectors; no Core command or physical terminal latency", "reports":reports
    })).unwrap()).unwrap();
}
