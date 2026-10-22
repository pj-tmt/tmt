//! Opt-in in-memory event/draw measurements, never CI thresholds or physical
//! terminal latency claims. Uses the current workspace and Terminal buffer diff.
use super::*;
use crate::board::{app::workspace::tests::populated_board, workspace::OpsTarget};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

use crate::board::timing::probe::{measure, report};

/// Populate the measured surface through the real composer and typed completion
/// ports. No lane, subprocess or source acquisition runs inside the probe.
fn populated_conversation(rows: usize) -> (App, String, OpsTarget) {
    let mut app = populated_board(36, rows);
    let squad = &app.workspace_snapshot().unwrap().roster.squads[0];
    let member = squad.members[0].clone();
    let target = OpsTarget::Member {
        room_id: Some(squad.room_id.clone()),
        identity_id: member.clone(),
    };
    let index = app
        .workspace_rows()
        .iter()
        .position(|row| row.target == target)
        .unwrap();
    app.select(index);
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":member,"requestId":"req_77777777-7777-4777-8777-777777777777",
        "response":body("response", 2_000)
    })];
    for index in 0..3 {
        if app.input.is_none() {
            assert_eq!(
                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                app::Effect::None
            );
        }
        let text = format!("Populated outgoing turn {index}: retained exact message");
        assert_eq!(app.paste(&text), app::Effect::None);
        let app::Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("probe must submit a real composer turn");
        };
        let ticket = app.outgoing_ticket(&request).unwrap();
        assert_eq!(ticket.text, text);
        assert_eq!(ticket.key.1, member);
        app.finished_send(crate::board::conversation::outgoing::Completed {
            ticket: Some(ticket),
            outcome: Ok(crate::send::Accepted {
                request: format!("req_88888888-8888-4888-8888-{index:012}"),
                disposition: crate::send::Disposition::Queued,
                passthrough: false,
                message_state_label: None,
                can_send_now: false,
                fallback: None,
                warning: None,
            }),
            what: "Message".into(),
            to: "measured member".into(),
        });
    }
    assert_eq!(
        app.paste("Retained composer draft 漢字 e\u{301}"),
        app::Effect::None
    );
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        app::Effect::None
    );
    app.notice = None;
    (app, member, target)
}

#[test]
#[ignore = "local release-mode timing evidence; no CI threshold"]
fn conversation_frame_timing() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    crate::status::with_now_ms(1_791_600_000_000, || {
        for rows in [150, 1_000, 10_000] {
            let (mut app, member, target) = populated_conversation(rows);
            let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
            for _ in 0..10 {
                measure(&mut app, &mut terminal, |_, _| {});
            }
            report(
                rows,
                "steady",
                (0..100)
                    .map(|_| measure(&mut app, &mut terminal, |_, _| {}))
                    .collect(),
            );
            report(
                rows,
                "scoped-state-projection",
                (0..100)
                    .map(|index| {
                        measure(&mut app, &mut terminal, |app, _| {
                            let state = if index % 2 == 0 { "working" } else { "blocked" };
                            let workspace =
                                &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                            workspace.roster.members.get_mut(&member).unwrap().state =
                                Some(state.into());
                            workspace.roster.squads[0]
                                .member_states
                                .get_mut(&member)
                                .unwrap()
                                .state = Some(state.into());
                            workspace.rows.get_mut(&target).unwrap()["state"] = json!(state);
                        })
                    })
                    .collect(),
            );
            report(
                rows,
                "append-response-projection",
                (0..100)
                    .map(|_| {
                        measure(&mut app, &mut terminal, |app, _| {
                            let response = &mut app.view.as_mut().unwrap().replies[0]["response"];
                            let mut text = response.as_str().unwrap().to_owned();
                            text.push_str("appended response line\n");
                            *response = json!(text);
                        })
                    })
                    .collect(),
            );
            app.scroll_pane(Pane::Replies, Step::Top);
            app.focus = 1;
            measure(&mut app, &mut terminal, |_, _| {});
            report(
                rows,
                "reader-key-scroll",
                (0..100)
                    .map(|_| {
                        measure(&mut app, &mut terminal, |app, _| {
                            assert_eq!(
                                app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                                app::Effect::None
                            );
                        })
                    })
                    .collect(),
            );
            assert_eq!(app.workspace.selected.as_ref(), Some(&target));
            assert!(
                app.input.is_none(),
                "Reader scrolling never opens an editor"
            );
            assert!(app.scrolls.offset(Pane::Replies) > 0);
            report(
                rows,
                "resize",
                (0..100)
                    .map(|index| {
                        measure(&mut app, &mut terminal, |app, terminal| {
                            app.invalidate_frame_geometry();
                            let width = if index % 2 == 0 { 100 } else { 160 };
                            terminal.backend_mut().resize(width, 36);
                            terminal.resize(Rect::new(0, 0, width, 36)).unwrap();
                            assert_eq!(terminal.size().unwrap().width, width);
                        })
                    })
                    .collect(),
            );
            app.focus = 0;
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert!(app.input.is_some());
            for kib in [1, 10, 100] {
                let payload = "x".repeat(kib * 1024);
                let mut samples = Vec::new();
                for _ in 0..100 {
                    app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
                    app.key(KeyEvent::new(
                        KeyCode::End,
                        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                    ));
                    app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
                    measure(&mut app, &mut terminal, |_, _| {});
                    samples.push(measure(&mut app, &mut terminal, |app, _| {
                        app.paste(&payload);
                    }));
                    assert_eq!(app.input.as_ref().unwrap().text, payload.as_str());
                }
                report(rows, &format!("paste-{kib}KiB"), samples);
            }
            report(
                rows,
                "type-after-100KiB-paste",
                (0..100)
                    .map(|_| {
                        measure(&mut app, &mut terminal, |app, _| {
                            app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
                        })
                    })
                    .collect(),
            );
            assert_eq!(
                app.input.as_ref().unwrap().draft_key.as_ref().unwrap().1,
                member
            );
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(app.input.is_none());
            let OpsTarget::Member {
                room_id: Some(room),
                ..
            } = &target
            else {
                unreachable!("measured member has a room");
            };
            report(
                rows,
                "launch-scope-projection",
                (0..100)
                    .map(|index| {
                        measure(&mut app, &mut terminal, |app, _| {
                            if index % 2 == 0 {
                                app.workspace.visibility.scope = vec![room.clone()];
                                app.workspace_reconcile();
                                app.invalidate_frame_geometry();
                            } else {
                                app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                                assert!(app.workspace.visibility.scope.is_empty());
                            }
                        })
                    })
                    .collect(),
            );
            let hidden: std::collections::BTreeSet<_> = app
                .workspace_snapshot()
                .unwrap()
                .roster
                .squads
                .iter()
                .filter(|squad| &squad.room_id != room)
                .map(|squad| squad.room_id.clone())
                .collect();
            report(
                rows,
                "saved-hidden-projection",
                (0..100)
                    .map(|index| {
                        measure(&mut app, &mut terminal, |app, _| {
                            let mut preferences =
                                app.workspace_snapshot().unwrap().preferences.clone();
                            preferences.hidden = if index % 2 == 0 {
                                hidden.clone()
                            } else {
                                Default::default()
                            };
                            // The confirmed CAS result is the input port; the probe does
                            // no Config/Core I/O and includes its real reconciliation.
                            app.finished_workspace_preferences(preferences, "Saved".into());
                        })
                    })
                    .collect(),
            );
            assert_eq!(app.workspace.selected, Some(target));
        }
    });
}

/// Capture real Crossterm encoding into a private memory sink; no host TTY is opened.
#[derive(Clone, Default)]
pub(in crate::board) struct AnsiSink(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
impl std::io::Write for AnsiSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(in crate::board) struct AnsiSample {
    event_us: f64,
    paint_us: f64,
    diff_encode_us: f64,
    total_us: f64,
    bytes: usize,
    changed_cells: usize,
}

pub(in crate::board) fn ansi_measure(
    app: &mut App,
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<AnsiSink>>,
    sink: &AnsiSink,
    previous: &mut Option<ratatui::buffer::Buffer>,
    event: impl FnOnce(&mut App),
) -> AnsiSample {
    sink.0.borrow_mut().clear();
    let total_start = Instant::now();
    event(app);
    let event = total_start.elapsed();
    let mut paint = Duration::ZERO;
    let draw_start = Instant::now();
    let completed = terminal
        .draw(|frame| {
            let start = Instant::now();
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
            paint = start.elapsed();
        })
        .unwrap();
    let draw = draw_start.elapsed();
    let total = total_start.elapsed();
    // Buffer cloning, diff accounting and JSON publication are outside all timings.
    let current = completed.buffer.clone();
    let changed_cells = previous.as_ref().map_or(current.content.len(), |old| {
        if old.area == current.area {
            old.diff(&current).len()
        } else {
            current.content.len()
        }
    });
    *previous = Some(current);
    AnsiSample {
        event_us: event.as_secs_f64() * 1_000_000.0,
        paint_us: paint.as_secs_f64() * 1_000_000.0,
        diff_encode_us: draw.saturating_sub(paint).as_secs_f64() * 1_000_000.0,
        total_us: total.as_secs_f64() * 1_000_000.0,
        bytes: sink.0.borrow().len(),
        changed_cells,
    }
}

pub(in crate::board) fn ansi_report(
    rows: usize,
    action: &str,
    samples: &[AnsiSample],
) -> serde_json::Value {
    let quantiles = |values: Vec<f64>| {
        let mut values = values;
        values.sort_by(f64::total_cmp);
        [values[49], values[94]]
    };
    let event = quantiles(samples.iter().map(|s| s.event_us).collect());
    let paint = quantiles(samples.iter().map(|s| s.paint_us).collect());
    let diff = quantiles(samples.iter().map(|s| s.diff_encode_us).collect());
    let total = quantiles(samples.iter().map(|s| s.total_us).collect());
    let bytes = quantiles(samples.iter().map(|s| s.bytes as f64).collect());
    let cells = quantiles(samples.iter().map(|s| s.changed_cells as f64).collect());
    eprintln!(
        "workspace_ansi agents={rows} squads=36 action={action} samples={} event_us={event:?} paint_us={paint:?} diff_encode_memory_sink_us={diff:?} total_us={total:?} ansi_bytes={bytes:?} changed_cells={cells:?}",
        samples.len()
    );
    json!({"agents":rows,"squads":36,"action":action,"quantiles":"p50/p95",
        "event_us":event,"paint_us":paint,"diff_encode_memory_sink_us":diff,
        "total_us":total,"ansi_bytes":bytes,"changed_cells":cells,
        "samples":samples.iter().map(|s|json!({"event_us":s.event_us,
            "paint_us":s.paint_us,"diff_encode_memory_sink_us":s.diff_encode_us,
            "total_us":s.total_us,"ansi_bytes":s.bytes,"changed_cells":s.changed_cells})).collect::<Vec<_>>()})
}

#[test]
#[ignore = "local release-mode phase/ANSI accounting; not physical terminal latency or a CI threshold"]
fn workspace_ansi_phase_timing() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    let output = std::env::var_os("TMT_WORKSPACE_ANSI_SAMPLES")
        .expect("set a task-owned TMT_WORKSPACE_ANSI_SAMPLES path");
    let mut reports = Vec::new();
    crate::status::with_now_ms(1_791_600_000_000, || {
        for rows in [150, 1_000, 10_000] {
            let (mut app, member, target) = populated_conversation(rows);
            let sink = AnsiSink(Default::default());
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
            for action in [
                "steady",
                "scoped-state-projection",
                "reader-key-scroll",
                "paste-100KiB",
            ] {
                if action == "reader-key-scroll" {
                    app.scroll_pane(Pane::Replies, Step::Top);
                    app.focus = 1;
                    ansi_measure(&mut app, &mut terminal, &sink, &mut previous, |_| {});
                }
                if action == "paste-100KiB" {
                    app.focus = 0;
                    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    assert!(app.input.is_some());
                }
                let payload = "x".repeat(100 * 1024);
                let mut samples = Vec::new();
                for index in 0..100 {
                    if action == "paste-100KiB" {
                        app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
                        app.key(KeyEvent::new(
                            KeyCode::End,
                            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                        ));
                        app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
                        ansi_measure(&mut app, &mut terminal, &sink, &mut previous, |_| {});
                    }
                    let sample =
                        ansi_measure(&mut app, &mut terminal, &sink, &mut previous, |app| {
                            match action {
                                "scoped-state-projection" => {
                                    let state = if index % 2 == 0 { "working" } else { "blocked" };
                                    let workspace = &mut app
                                        .view
                                        .as_mut()
                                        .unwrap()
                                        .home
                                        .as_mut()
                                        .unwrap()
                                        .workspace;
                                    workspace.roster.members.get_mut(&member).unwrap().state =
                                        Some(state.into());
                                    workspace.roster.squads[0]
                                        .member_states
                                        .get_mut(&member)
                                        .unwrap()
                                        .state = Some(state.into());
                                    workspace.rows.get_mut(&target).unwrap()["state"] =
                                        json!(state);
                                }
                                "reader-key-scroll" => {
                                    assert_eq!(
                                        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                                        app::Effect::None
                                    );
                                }
                                "paste-100KiB" => {
                                    assert_eq!(app.paste(&payload), app::Effect::None);
                                }
                                "steady" => {}
                                _ => unreachable!(),
                            }
                        });
                    if action == "paste-100KiB" {
                        assert_eq!(app.input.as_ref().unwrap().text, payload.as_str());
                    }
                    samples.push(sample);
                }
                if action == "steady" {
                    assert!(samples.iter().all(|s| s.changed_cells == 0));
                } else {
                    assert!(
                        samples.iter().any(|s| s.changed_cells > 0),
                        "{action} must paint causal changes"
                    );
                }
                assert_eq!(app.workspace.selected.as_ref(), Some(&target));
                if action == "reader-key-scroll" {
                    assert!(
                        app.input.is_none(),
                        "Reader scrolling never opens an editor"
                    );
                    assert!(app.scrolls.offset(Pane::Replies) > 0);
                }
                reports.push(ansi_report(rows, action, &samples));
            }
        }
    });
    let report = json!({"viewport":{"cols":160,"rows":36},"responseLines":2000,
        "outgoingTurns":3,"retainedComposerDraft":true,"clockMs":1791600000000u64,"output":"Crossterm ANSI encoded into a private memory sink",
        "scope":"event/paint/diff+encoding walltime; process CPU requires external resource measurement; no PTY or terminal emulator latency", "reports":reports});
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}

/// 100 wheel steps up over the conversation pane: event and draw time, and the
/// cells that changed outside the scrolled region or in the input and footer
/// rows (Investigation of #3986).
fn scroll_samples(label: &str, app: &mut App) {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
    for _ in 0..5 {
        measure(app, &mut terminal, |_, _| {});
    }
    let area = app
        .conversation
        .borrow()
        .response_area
        .expect("a painted response area");
    let input = app.input_band.get();
    app.scroll_pane(Pane::Replies, Step::Bottom);
    measure(app, &mut terminal, |_, _| {});
    let mut steady: Vec<f64> = (0..100)
        .map(|_| {
            let start = Instant::now();
            terminal
                .draw(|frame| {
                    crate::board::view::prepare(app, frame.area());
                    crate::board::view::render(frame, app);
                })
                .unwrap();
            start.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    steady.sort_by(f64::total_cmp);
    eprintln!("steady {label} draw_us p50={:.0}", steady[50]);
    let (mut event_us, mut draw_us, mut outside, mut chrome) = (vec![], vec![], vec![], vec![]);
    for _ in 0..100 {
        let previous = terminal.backend().buffer().clone();
        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: area.x + area.width / 2,
            row: area.y + area.height / 2,
            modifiers: KeyModifiers::NONE,
        };
        let start = Instant::now();
        app.mouse(wheel, Instant::now());
        event_us.push(start.elapsed().as_secs_f64() * 1e6);
        let start = Instant::now();
        terminal
            .draw(|frame| {
                crate::board::view::prepare(app, frame.area());
                crate::board::view::render(frame, app);
            })
            .unwrap();
        draw_us.push(start.elapsed().as_secs_f64() * 1e6);
        let diff = previous.diff(terminal.backend().buffer());
        outside.push(
            diff.iter()
                .filter(|(x, y, _)| !area.contains((*x, *y).into()))
                .count(),
        );
        chrome.push(
            diff.iter()
                .filter(|(x, y, _)| {
                    input.is_some_and(|band| band.contains((*x, *y).into())) || *y == 49
                })
                .count(),
        );
    }
    let pf = |mut v: Vec<f64>, q: f64| {
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * q).round() as usize]
    };
    let pu = |mut v: Vec<usize>, q: f64| {
        v.sort_unstable();
        v[((v.len() - 1) as f64 * q).round() as usize]
    };
    eprintln!(
        "scroll {label} area={}x{} event_us p50={:.0} p95={:.0} draw_us p50={:.0} p95={:.0} cells_outside_area max={} input_or_footer_cells max={} offset={}",
        area.width,
        area.height,
        pf(event_us.clone(), 0.5),
        pf(event_us, 0.95),
        pf(draw_us.clone(), 0.5),
        pf(draw_us, 0.95),
        pu(outside, 1.0),
        pu(chrome, 1.0),
        app.scrolls.offset(Pane::Replies)
    );
}

#[test]
#[ignore = "local release-mode timing evidence; no CI threshold"]
fn conversation_scroll_cost() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    crate::status::with_now_ms(1_791_600_000_000, || {
        for lines in [200usize, 2_000, 20_000] {
            let (mut app, ..) = populated_conversation(150);
            app.view.as_mut().unwrap().replies[0]["response"] = json!(body("response", lines));
            scroll_samples(&format!("response-lines={lines}"), &mut app);
        }
        for turns in [200usize, 2_000] {
            let (mut app, member, _) = populated_conversation(150);
            for index in 3..turns {
                let text = format!("Outgoing turn {index}: retained exact message");
                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                app.paste(&text);
                let app::Effect::Act(request) =
                    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                else {
                    panic!("probe must submit a real composer turn");
                };
                let ticket = app.outgoing_ticket(&request).unwrap();
                app.finished_send(crate::board::conversation::outgoing::Completed {
                    ticket: Some(ticket),
                    outcome: Ok(crate::send::Accepted {
                        request: format!("req_88888888-8888-4888-8888-{index:012}"),
                        disposition: crate::send::Disposition::Queued,
                        passthrough: false,
                        message_state_label: None,
                        can_send_now: false,
                        fallback: None,
                        warning: None,
                    }),
                    what: "Message".into(),
                    to: member.clone(),
                });
                app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            }
            app.notice = None;
            scroll_samples(&format!("outgoing-turns={turns}"), &mut app);
        }
    });
}

/// Scrolling the history changes only the history: the input band and the footer
/// stay cell for cell, and the caret never moves (it is hidden while the changed
/// rows are written, then shown again in the same place).
#[test]
fn scrolling_leaves_the_input_the_footer_and_the_caret_alone() {
    use ratatui::backend::Backend as _;
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    crate::status::with_now_ms(1_791_600_000_000, || {
        let (mut app, ..) = populated_conversation(150);
        app.view.as_mut().unwrap().replies[0]["response"] = json!(body("response", 2_000));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.input.is_some(), "the composer is open");
        let mut cells = Terminal::new(TestBackend::new(160, 50)).unwrap();
        let sink = AnsiSink::default();
        let mut bytes = Terminal::with_options(
            crate::board::terminal::Backend::new(sink.clone()),
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, 160, 50)),
            },
        )
        .unwrap();
        fn draw<B: ratatui::backend::Backend>(app: &mut App, terminal: &mut Terminal<B>)
        where
            B::Error: std::fmt::Debug,
        {
            terminal
                .draw(|frame| {
                    crate::board::view::prepare(app, frame.area());
                    crate::board::view::render(frame, app);
                })
                .unwrap();
        }
        for _ in 0..3 {
            draw(&mut app, &mut cells);
            draw(&mut app, &mut bytes);
        }
        let area = app
            .conversation
            .borrow()
            .response_area
            .expect("history area");
        let band = app.input_band.get().expect("the input band is painted");
        let caret = cells.backend_mut().get_cursor_position().unwrap();
        assert!(
            band.contains(caret),
            "the caret sits in the input: {caret:?}"
        );
        let mut scrolled = 0;
        for step in 0..30 {
            let previous = cells.backend().buffer().clone();
            sink.0.borrow_mut().clear();
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::ScrollUp,
                    column: area.x + area.width / 2,
                    row: area.y + area.height / 2,
                    modifiers: KeyModifiers::NONE,
                },
                Instant::now(),
            );
            draw(&mut app, &mut cells);
            draw(&mut app, &mut bytes);
            let diff = previous.diff(cells.backend().buffer());
            assert!(!diff.is_empty(), "step {step} moved the history");
            scrolled += 1;
            assert!(
                diff.iter().all(|(x, y, _)| area.contains((*x, *y).into())),
                "step {step}: only the scrolled history changes"
            );
            assert_eq!(cells.backend_mut().get_cursor_position().unwrap(), caret);
            let text = String::from_utf8(sink.0.borrow().clone()).unwrap();
            let hide = text
                .find("\x1b[?25l")
                .expect("hidden while cells are written");
            assert!(
                hide == 0,
                "step {step}: hidden before anything is written: {text:?}"
            );
            let placed = format!("\x1b[{};{}H\x1b[?25h", caret.y + 1, caret.x + 1);
            assert!(
                text.ends_with(&placed),
                "step {step}: the caret is placed in the input and shown last: {:?}",
                &text[text.len().saturating_sub(40)..]
            );
            assert_eq!(text.matches("\x1b[?25h").count(), 1);
        }
        assert_eq!(scrolled, 30);
    });
}
