use super::*;
use crate::board::{
    app::workspace::tests::MEMBER,
    digest_dashboard::tests::{savings_board, valued_board},
    shell::OpsTab,
};
use crate::look::Look;
use ratatui::{Terminal, backend::TestBackend};
fn summary(app: &App) -> super::super::summary::Summary {
    super::super::monitor::scene(app, app.workspace_snapshot().unwrap())
        .summary
        .clone()
}
#[test]
fn metrics_reuse_observed_values_and_never_write_the_unit_preference() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = valued_board();
    app.shell.tab = OpsTab::Overview;
    let preferences = app.workspace_snapshot().unwrap().preferences.clone();
    let rows = rows(&app, &summary(&app));
    assert_eq!(rows[0].amount.as_deref(), Some("0"));
    assert_eq!(rows[1].amount.as_deref(), Some("1 · 0 · 0"));
    assert_eq!(rows[2].key, "ctx avoided 24h");
    assert_eq!(rows[2].amount.as_deref(), Some("18.2k"));
    assert_eq!(rows[3].amount.as_deref(), Some("≈ $12.40 at list price"));
    assert_eq!(rows[4].amount.as_deref(), Some("2026-10-14"));
    assert_eq!(app.workspace_snapshot().unwrap().preferences, preferences);
}
#[test]
fn missing_stale_and_partial_counts_do_not_turn_into_measured_zero() {
    let mut app = savings_board();
    app.shell.tab = OpsTab::Overview;
    let mut partial = summary(&app);
    partial.complete = false;
    let values = rows(&app, &partial);
    assert!(values[0].amount.is_none());
    assert_eq!(values[1].amount.as_deref(), Some("~1 · — · —"));
    assert_eq!(values[2].amount.as_deref(), Some("~18.2k"));
    app.labels = crate::labels::Supplied::from_document(
        "digest",
        &crate::board::digest_dashboard::tests::savings_document(),
        true,
    );
    app.overview_rows.borrow_mut().clear();
    assert!(
        rows(&app, &summary(&app))
            .iter()
            .all(|row| row.amount.is_none()),
        "stale supplier evidence is not current zero"
    );
    app.labels = crate::labels::Supplied::default();
    app.overview_rows.borrow_mut().clear();
    let values = rows(&app, &summary(&app));
    assert!(values.iter().all(|row| row.amount.is_none()));
    // The source keeps its own identity and coverage; no row document creates it.
    assert!(
        app.workspace_snapshot()
            .unwrap()
            .roster
            .members
            .contains_key(MEMBER)
    );
}

#[test]
fn narrow_cost_rows_keep_the_full_window_before_the_price_basis_note() {
    use tmt_cli_style::{Depth, Theme, theme::Base};
    for base in [Base::Tmt, Base::TmtLight] {
        for depth in [Depth::TrueColor, Depth::None] {
            let look = Look {
                theme: Theme::new(base),
                depth,
            };
            let mut app = valued_board();
            let view = app.view.as_mut().unwrap();
            view.look = look;
            // A squad's saved look would win over the one under test.
            view.home.as_mut().unwrap().workspace.squad_looks.clear();
            for (amount, coverage, expected) in [
                (Some("≈ $37.20 at list price"), "", "≈ $37.20"),
                (Some("~≈ $37.20 at list price"), " · 1/2", "~≈ $37.20"),
                (Some("≈ $12,400.00 at list price"), "", "≈ $12,400.00"),
                (None, "", "—"),
            ] {
                let metric = Metric {
                    key: "cost avoided 24h".into(),
                    amount: amount.map(str::to_owned),
                    coverage: coverage.into(),
                    note: None,
                };
                for width in [32, 38, 60] {
                    let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
                    terminal
                        .draw(|frame| {
                            let area = frame.area();
                            metric.paint(frame, &app, area);
                        })
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let line = buffer
                        .content
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>();
                    assert!(line.contains("cost avoided 24h"), "{line}");
                    assert!(line.contains(expected), "{line}");
                    assert!(!line.contains("cost avoided 2…"));
                    assert_eq!(
                        buffer[(1, 0)].fg,
                        look.role(tmt_cli_style::Role::Muted)
                            .fg
                            .unwrap_or(ratatui::style::Color::Reset)
                    );
                }
            }
        }
    }
}
