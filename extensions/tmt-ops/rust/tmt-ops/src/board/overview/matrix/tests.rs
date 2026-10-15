use super::*;
use crate::board::{
    app::workspace::tests::{MEMBER, populated_board},
    digest_dashboard::tests::{valued_board, valued_document},
    overview::capture,
    shell::OpsTab,
    workspace::OpsVisibility,
};
#[test]
fn matrix_reuses_values_in_fixed_units_without_writing_preferences() {
    let mut app = valued_board();
    app.shell.tab = OpsTab::Overview;
    capture::populate_final(&mut app, "squads");
    let saved = app.workspace_snapshot().unwrap().preferences.clone();
    let matrix = Matrix::project(&app, 59);
    assert_eq!(matrix.columns.len(), 2);
    assert_eq!(matrix.columns[0].name, "all");
    assert_eq!(matrix.columns[1].name, "worker");
    assert_eq!(matrix.columns[0].values[0].as_deref(), Some("1 Auto"));
    assert_eq!(matrix.columns[0].values[1].as_deref(), Some("0"));
    assert_eq!(matrix.columns[0].values[3].as_deref(), Some("18.2k"));
    assert_eq!(matrix.columns[0].values[4].as_deref(), Some("18%"));
    assert_eq!(matrix.columns[0].values[5].as_deref(), Some("$12.40"));
    assert_eq!(matrix.columns[0].values[3..], matrix.columns[1].values[3..]);
    assert_eq!(app.workspace_snapshot().unwrap().preferences, saved);
    app.labels = crate::labels::Supplied::from_document("digest", &valued_document(), true);
    app.overview_rows.borrow_mut().clear();
    let stale = Matrix::project(&app, 59);
    assert!(stale.columns[0].values[0].is_none());
    assert!(
        stale
            .columns
            .iter()
            .all(|column| column.values[3..].iter().all(Option::is_none))
    );
    assert!(
        stale.columns[1].values[2].is_some(),
        "independent HOUR observations are not supplier zeroes"
    );
}
#[test]
fn rank_and_all_totals_use_unique_scoped_hour_identities_and_grow_by_ten_cells() {
    let mut app = populated_board(2, 6);
    app.shell.tab = OpsTab::Overview;
    capture::populate_lower_band(&mut app, "squads");
    capture::populate_final(&mut app, "squads");
    let wide = Matrix::project(&app, 59);
    assert_eq!(wide.columns.len(), 4);
    assert_eq!(wide.columns[0].name, "all");
    let names: Vec<_> = app.workspace_snapshot().unwrap().roster.squads[0]
        .members
        .iter()
        .take(3)
        .map(|id| {
            app.workspace_snapshot().unwrap().roster.members[id]
                .name
                .as_str()
        })
        .collect();
    assert_eq!(
        wide.columns[1..]
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        names
    );
    assert_eq!(Matrix::project(&app, 49).columns.len(), 3);
    let room = app.workspace_snapshot().unwrap().roster.squads[1]
        .room_id
        .clone();
    app.workspace.visibility = OpsVisibility {
        scope: vec![room],
        reveal_hidden: false,
    };
    app.overview_rows.borrow_mut().clear();
    let other = Matrix::project(&app, 59);
    assert_eq!(other.columns.len(), 1);
    assert!(other.columns[0].values[2].is_none());
    assert_eq!(other.columns[0].values[0].as_deref(), Some("3 Auto"));
}
#[test]
fn a_different_supplier_window_never_changes_the_fixed_twenty_four_hour_captions() {
    let mut app = valued_board();
    app.shell.tab = OpsTab::Overview;
    let mut document = valued_document();
    document["members"][0]["savings"]["window"] = serde_json::json!("7d");
    app.labels = crate::labels::Supplied::from_document("digest", &document, false);
    let matrix = Matrix::project(&app, 59);
    assert_eq!(matrix.labels[3], "ctx avoided 24h");
    assert!(matrix.columns[0].values[3..].iter().all(Option::is_none));
    assert!(
        app.workspace_snapshot()
            .unwrap()
            .roster
            .members
            .contains_key(MEMBER)
    );
}
