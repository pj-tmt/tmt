//! Opt-in local measurements, never a CI latency threshold. Uses the real App
//! event path and Terminal buffer diff; no provider, clock or external I/O.
use super::*;
use crate::config::{Board, BoardMode, Direction};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

fn measure(app: &mut App, terminal: &mut Terminal<TestBackend>) -> Duration {
    let start = Instant::now();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    start.elapsed()
}

fn report(rows: usize, action: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let us = |quantile: f64| {
        samples[((samples.len() - 1) as f64 * quantile).round() as usize].as_secs_f64()
            * 1_000_000.0
    };
    eprintln!(
        "conversation rows={rows} action={action} samples={} frame_us p50={:.1} p95={:.1}",
        samples.len(),
        us(0.5),
        us(0.95)
    );
}

#[test]
#[ignore = "local release-mode timing evidence; no CI threshold"]
fn conversation_frame_timing() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    for rows in [150, 1_000, 10_000] {
        let mut app = fixture();
        let view = app.view.as_mut().unwrap();
        view.document["sections"][0]["rows"] = json!(
            (0..rows)
                .map(|index| json!({
                    "id": if index == 0 {"a".into()} else {format!("agent-{index}")},
                    "name":format!("agent-{index:05}"),
                    "state":"working", "fields":{"task":"bounded renderer fixture"}
                }))
                .collect::<Vec<_>>()
        );
        view.board = Board::simple(
            BoardMode::Split,
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Replies],
            &[40, 60],
        );
        view.replies[0]["response"] = json!(body("response", 2_000));
        let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
        for _ in 0..10 {
            measure(&mut app, &mut terminal);
        }
        report(
            rows,
            "idle",
            (0..100).map(|_| measure(&mut app, &mut terminal)).collect(),
        );
        app.scroll_pane(Pane::Replies, Step::Top);
        let samples = (0..100)
            .map(|_| {
                app.scroll_pane(Pane::Replies, Step::Lines(1));
                measure(&mut app, &mut terminal)
            })
            .collect();
        report(rows, "response-scroll", samples);
        app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(app.input.is_some());
        app.paste(&"exact folded source 漢字\n".repeat(4_000));
        let samples = (0..100)
            .map(|_| {
                app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
                measure(&mut app, &mut terminal)
            })
            .collect();
        report(rows, "type-after-large-paste", samples);
        assert_eq!(
            app.input.as_ref().unwrap().draft_key.as_ref().unwrap().1,
            "a"
        );
        assert_eq!(app.scrolls.offset(Pane::Replies), 100);
    }
}
