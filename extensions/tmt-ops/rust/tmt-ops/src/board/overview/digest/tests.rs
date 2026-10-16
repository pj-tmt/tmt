use super::*;
use crate::board::{
    app::workspace::tests::MEMBER,
    digest_dashboard::tests::{savings_board, valued_board},
    shell::OpsTab,
};
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
