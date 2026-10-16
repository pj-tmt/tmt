//! Same local P02 budgets with real interval data, explicit reporter population.
use super::*;
use crate::board::timing::probe::{measure, report};

/// Exercise real waiting targets; an empty S2 list would turn scroll into a no-op.
fn waiting_board(agents: usize) -> App {
    let mut app = crate::board::app::workspace::tests::populated_board(36, agents);
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    for member in snapshot.roster.members.values_mut() {
        member.waiting = true;
    }
    for squad in &mut snapshot.roster.squads {
        for member in squad.member_states.values_mut() {
            member.waiting = true;
        }
    }
    snapshot.shell_counts = snapshot.roster.shell_counts();
    app.overview_rows.borrow_mut().clear();
    fixtures::populate(&mut app);
    app.overview_view = crate::board::overview::monitor::View::Agents;
    app.overview_rows.borrow_mut().clear();
    assert_eq!(app.workspace_rows().len(), agents);
    app
}
#[test]
#[ignore = "explicit release M08 populated-chart P02 evidence, not a CI/user SLO"]
fn populated_chart_frame_cost() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    crate::status::with_now_ms(fixtures::NOW, || {
        for agents in [150, 1_000, 10_000] {
            let mut app = waiting_board(agents);
            let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
            for _ in 0..10 {
                measure(&mut app, &mut terminal, |_, _| {});
            }
            assert_eq!(
                app.usage_chart
                    .borrow()
                    .observed
                    .as_ref()
                    .unwrap()
                    .buckets
                    .len(),
                12
            );
            assert!(
                app.usage_chart
                    .borrow()
                    .observed
                    .as_ref()
                    .unwrap()
                    .buckets
                    .iter()
                    .any(|bucket| bucket.tokens.is_some_and(|value| value > 0))
            );
            for action in [
                "chart-steady",
                "chart-source-replace",
                "chart-scroll",
                "chart-resize",
                "chart-controls",
            ] {
                let samples = (0..100)
                    .map(|index| {
                        measure(&mut app, &mut terminal, |app, terminal| match action {
                            "chart-source-replace" => app.usage_chart.get_mut().invalidate_data(),
                            "chart-scroll" => {
                                press(
                                    app,
                                    if index % 2 == 0 {
                                        KeyCode::Down
                                    } else {
                                        KeyCode::Up
                                    },
                                );
                            }
                            "chart-resize" => {
                                app.invalidate_frame_geometry();
                                let width = if index % 2 == 0 { 100 } else { 160 };
                                terminal.backend_mut().resize(width, 36);
                                terminal.resize(Rect::new(0, 0, width, 36)).unwrap();
                            }
                            "chart-controls" => {
                                let mut state = app.usage_chart.borrow_mut();
                                state
                                    .apply(Target::Each(if index % 2 == 0 { 15 } else { 5 }))
                                    .unwrap();
                            }
                            _ => {}
                        })
                    })
                    .collect();
                let p95 = report(agents, action, samples);
                let cap = if agents == 150 { 10_000.0 } else { 50_000.0 };
                assert!(p95 <= cap, "{agents}/{action} p95 {p95} us exceeds {cap}");
            }
            eprintln!(
                "M08 population agents={agents} squads=36 reporting_identities=1 source_buckets=120 interval_bars=12; non-reporting identities remain partial"
            );
        }
    });
}

#[test]
#[ignore = "explicit release M08 ANSI/resource evidence, not physical terminal latency"]
fn populated_chart_ansi_phase_timing() {
    use crate::board::conversation::tests::performance::{AnsiSink, ansi_measure, ansi_report};
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    let output = std::env::var_os("TMT_OPS_M08_ANSI_SAMPLES")
        .expect("set a task-owned TMT_OPS_M08_ANSI_SAMPLES path");
    let mut reports = Vec::new();
    crate::status::with_now_ms(fixtures::NOW, || {
        for agents in [150, 1_000, 10_000] {
            let mut app = waiting_board(agents);
            let target = app.workspace.selected.clone();
            let sink = AnsiSink::default();
            let backend = ratatui::backend::CrosstermBackend::new(sink.clone());
            let mut terminal = Terminal::with_options(
                backend,
                ratatui::TerminalOptions {
                    viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, 160, 36)),
                },
            )
            .unwrap();
            let mut previous = None;
            for _ in 0..10 {
                ansi_measure(&mut app, &mut terminal, &sink, &mut previous, |_| {});
            }
            let source = app.usage_chart.borrow();
            assert_eq!(source.observed.as_ref().unwrap().buckets.len(), 12);
            assert!(
                source
                    .observed
                    .as_ref()
                    .unwrap()
                    .buckets
                    .iter()
                    .any(|bucket| {
                        bucket.tokens.is_some_and(|value| value > 0) && !bucket.complete()
                    })
            );
            drop(source);
            for action in [
                "chart-steady",
                "chart-source-replace",
                "chart-hover",
                "chart-controls",
            ] {
                let samples: Vec<_> = (0..100)
                    .map(|index| {
                        ansi_measure(&mut app, &mut terminal, &sink, &mut previous, |app| {
                            match action {
                                "chart-source-replace" => {
                                    app.usage_chart.get_mut().invalidate_data()
                                }
                                "chart-hover" => {
                                    let hit = app
                                        .usage_chart
                                        .borrow()
                                        .hits
                                        .iter()
                                        .find(|hit| hit.target == Target::Bar(index % 12))
                                        .copied()
                                        .unwrap();
                                    app.mouse(
                                        MouseEvent {
                                            kind: MouseEventKind::Moved,
                                            column: hit.area.x,
                                            row: hit.area.y,
                                            modifiers: KeyModifiers::NONE,
                                        },
                                        std::time::Instant::now(),
                                    );
                                }
                                "chart-controls" => {
                                    app.usage_chart
                                        .get_mut()
                                        .apply(Target::Each(if index % 2 == 0 { 15 } else { 5 }))
                                        .unwrap();
                                }
                                _ => {}
                            }
                        })
                    })
                    .collect();
                let value = ansi_report(agents, action, &samples);
                let cap = if agents == 150 { 10_000.0 } else { 50_000.0 };
                assert!(value["total_us"][1].as_f64().unwrap() <= cap);
                let changed = value["samples"].as_array().unwrap();
                if matches!(action, "chart-steady" | "chart-source-replace") {
                    assert!(changed.iter().all(|sample| sample["changed_cells"] == 0));
                } else {
                    assert!(
                        changed
                            .iter()
                            .any(|sample| sample["changed_cells"].as_u64().unwrap() > 0)
                    );
                }
                assert_eq!(app.workspace.selected, target);
                reports.push(value);
            }
        }
    });
    let value = serde_json::json!({"viewport":{"cols":160,"rows":36},
        "reportingIdentities":1,"sourceBuckets":120,"initialIntervalBars":12,
        "clockMs":fixtures::NOW,"output":"Crossterm ANSI encoded into a private memory sink",
        "scope":"Populated Overview chart event/paint/diff+encoding; process CPU measured externally; no PTY or terminal emulator latency",
        "reports":reports});
    std::fs::write(output, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
