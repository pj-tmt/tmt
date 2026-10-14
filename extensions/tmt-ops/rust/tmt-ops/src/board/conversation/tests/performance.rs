//! Opt-in in-memory event/draw measurements, never CI thresholds or physical
//! terminal latency claims. Uses the current workspace and Terminal buffer diff.
use super::*;
use crate::board::{app::workspace::tests::populated_board, workspace::OpsTarget};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

struct Sample {
    event_draw: Duration,
    changed_cells: usize,
}

fn measure(
    app: &mut App,
    terminal: &mut Terminal<TestBackend>,
    event: impl FnOnce(&mut App, &mut Terminal<TestBackend>),
) -> Sample {
    let previous = terminal.backend().buffer().clone();
    let start = Instant::now();
    event(app, terminal);
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    let event_draw = start.elapsed();
    let current = terminal.backend().buffer();
    // Counting is outside the timed event/draw. Resize is a full clear/redraw;
    // this is changed buffer cells, not ANSI bytes or physical terminal output.
    let changed_cells = if current.area == previous.area {
        previous.diff(current).len()
    } else {
        current.content.len()
    };
    Sample {
        event_draw,
        changed_cells,
    }
}

fn report(rows: usize, action: &str, samples: Vec<Sample>) {
    let mut times: Vec<_> = samples.iter().map(|sample| sample.event_draw).collect();
    let mut cells: Vec<_> = samples.iter().map(|sample| sample.changed_cells).collect();
    times.sort();
    cells.sort_unstable();
    let at = |quantile: f64| ((times.len() - 1) as f64 * quantile).round() as usize;
    eprintln!(
        "workspace agents={rows} squads=36 action={action} samples={} event_draw_us p50={:.1} p95={:.1} changed_cells p50={} p95={}",
        times.len(),
        times[at(0.5)].as_secs_f64() * 1_000_000.0,
        times[at(0.95)].as_secs_f64() * 1_000_000.0,
        cells[at(0.5)],
        cells[at(0.95)]
    );
}

#[test]
#[ignore = "local release-mode timing evidence; no CI threshold"]
fn conversation_frame_timing() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    crate::status::with_now_ms(1_791_600_000_000, || {
        for rows in [150, 1_000, 10_000] {
            let mut app = populated_board(36, rows);
            let room = app.workspace_snapshot().unwrap().roster.squads[0]
                .room_id
                .clone();
            let member = app.workspace_snapshot().unwrap().roster.squads[0].members[0].clone();
            let target = OpsTarget::Member {
                room_id: Some(room.clone()),
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
                                app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
                                app::Effect::None
                            );
                        })
                    })
                    .collect(),
            );
            assert_eq!(app.workspace.selected.as_ref(), Some(&target));
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
            app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
            assert!(app.input.is_some());
            for kib in [1, 10, 100] {
                let payload = "x".repeat(kib * 1024);
                let mut samples = Vec::new();
                for _ in 0..100 {
                    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
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
            assert_eq!(app.workspace.selected, Some(target));
        }
    });
}
