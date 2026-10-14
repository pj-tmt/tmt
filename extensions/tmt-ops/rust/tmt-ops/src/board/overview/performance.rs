//! Opt-in timing of the actual Overview frame; no CI threshold or source I/O.
use crate::board::{
    app::workspace::tests::populated_board,
    shell::OpsTab,
    timing::probe::{measure, report},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
};

#[test]
#[ignore = "local release-mode Overview timing evidence; no CI threshold"]
fn overview_monitor_frame_timing() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    crate::status::with_now_ms(1_791_600_000_000, || {
        for rows in [150, 1_000, 10_000] {
            let mut app = populated_board(36, rows);
            app.shell.tab = OpsTab::Overview;
            app.workspace_reconcile();
            let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
            for _ in 0..10 {
                measure(&mut app, &mut terminal, |_, _| {});
            }
            report(
                rows,
                "overview-steady",
                (0..100)
                    .map(|_| measure(&mut app, &mut terminal, |_, _| {}))
                    .collect(),
            );
            report(
                rows,
                "overview-key-scroll",
                (0..100)
                    .map(|_| {
                        measure(&mut app, &mut terminal, |app, _| {
                            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                        })
                    })
                    .collect(),
            );
            report(
                rows,
                "overview-resize",
                (0..100)
                    .map(|index| {
                        measure(&mut app, &mut terminal, |app, terminal| {
                            app.invalidate_frame_geometry();
                            let width = if index % 2 == 0 { 100 } else { 160 };
                            terminal.backend_mut().resize(width, 36);
                            terminal.resize(Rect::new(0, 0, width, 36)).unwrap();
                        })
                    })
                    .collect(),
            );
        }
    });
}
