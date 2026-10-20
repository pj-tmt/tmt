use super::*;
use crate::board::{
    app::workspace::tests::{MEMBER, populated_board},
    digest_dashboard::tests::{valued_board, valued_document},
    overview::capture,
    shell::OpsTab,
    workspace::OpsVisibility,
};
fn many_agents(count: usize) -> App {
    let mut app = populated_board(1, count);
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    // The matrix is the Measure view of the right column.
    app.right_tab = crate::board::shell::RightTab::Measure;
    let first = app
        .workspace_snapshot()
        .unwrap()
        .rows
        .keys()
        .find(|key| matches!(key, crate::board::workspace::OpsTarget::Member { .. }))
        .unwrap()
        .clone();
    app.workspace.selected = Some(first);
    capture::populate_lower_band(&mut app, "digest-rows");
    capture::populate_final(&mut app, "digest-rows");
    app
}
fn paint(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}
#[test]
fn top_identity_capacity_follows_height_and_keeps_the_same_all_totals() {
    crate::status::with_now_ms(1_791_600_000_000, || {
        let app = many_agents(24);
        let short = Matrix::project(&app, 5);
        let tall = Matrix::project(&app, 12);
        assert_eq!(short.rows.len(), 4);
        assert_eq!(tall.rows.len(), 11);
        assert_eq!(short.rows[0].values, tall.rows[0].values);
        assert_eq!(
            short.rows[1..].iter().map(|r| &r.name).collect::<Vec<_>>(),
            tall.rows[1..4].iter().map(|r| &r.name).collect::<Vec<_>>()
        );
        assert!(short.note.starts_with("all + top 3 by TOK"));
        assert!(tall.note.starts_with("all + top 10 by TOK"));
        assert_eq!(Matrix::project(&app, 2).rows.len(), 1);
    });
}
#[test]
fn native_wide_table_draws_one_three_and_many_identities_below_the_all_row() {
    let _commands = crate::runner::forbid_commands();
    crate::status::with_now_ms(1_791_600_000_000, || {
        for width in [148, 160] {
            for count in [1, 3, 24] {
                let mut app = many_agents(count);
                let buffer = paint(&mut app, width, 51);
                let lines: Vec<String> = buffer
                    .content
                    .chunks(usize::from(width))
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                    .collect();
                let header = lines
                    .iter()
                    .position(|line| line.contains("CTX 24h"))
                    .expect("wide table header");
                let left = lines[header].chars().position(|c| c == '│').unwrap() + 1;
                // The full-width footer can extend past the pane divider; it is
                // outside the matrix even when its cells below this table are nonblank.
                let body_bottom = lines.len().saturating_sub(1);
                let rows: Vec<_> = lines[header + 1..body_bottom]
                    .iter()
                    .map(|line| line.chars().skip(left).collect::<String>())
                    .take_while(|line| !line.trim().is_empty())
                    .collect();
                assert!(rows[0].trim_start().starts_with("all "));
                // Two footer rows follow the matrix.
                let available = usize::from(buffer.area.height).saturating_sub(header + 3);
                let shown = count.min(available.saturating_sub(1));
                assert_eq!(rows.len(), shown + 1, "{width}/{count}: {rows:?}");
                assert!(
                    lines[header - 1].contains(&format!("all + top {shown} by TOK")),
                    "{}",
                    lines[header - 1]
                );
                assert!(
                    buffer[((left + 1) as u16, (header + 1) as u16)]
                        .modifier
                        .contains(ratatui::style::Modifier::BOLD)
                );
                assert!(rows[1].contains("agent-00000"));
            }
        }
    });
}
#[test]
#[ignore = "exports task-owned transposed Digest evidence without recording fixtures"]
fn capture_digest_identity_rows() {
    use tmt_cli_style::{Depth, Theme, theme::Base};
    let output = std::env::var("TMT_DIGEST_ROWS_PACKET").expect("task-owned output path");
    let mut frames = Vec::new();
    crate::status::with_now_ms(1_791_600_000_000, || {
        for width in [103, 148, 160] {
            for count in [1, 3, 24] {
                for (name, base, depth) in [
                    ("tmt", Base::Tmt, Depth::TrueColor),
                    ("tmt-light", Base::TmtLight, Depth::TrueColor),
                    ("NO_COLOR", Base::Tmt, Depth::None),
                ] {
                    let mut app = many_agents(count);
                    app.view.as_mut().unwrap().look = crate::look::Look {
                        theme: Theme::new(base),
                        depth,
                    };
                    let buffer = paint(&mut app, width, 51);
                    let lines: Vec<String> = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                        .collect();
                    let cells:Vec<_>=buffer.content.iter().map(|cell|serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect();
                    frames.push(serde_json::json!({"width":width,"height":51,"look":name,"agents":count,"lines":lines,"cells":cells}));
                }
            }
        }
    });
    assert_eq!(frames.len(), 27);
    std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
}
#[test]
fn matrix_reuses_values_in_fixed_units_without_writing_preferences() {
    let mut app = valued_board();
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    capture::populate_final(&mut app, "squads");
    let saved = app.workspace_snapshot().unwrap().preferences.clone();
    let matrix = Matrix::project(&app, 5);
    assert_eq!(matrix.rows.len(), 2);
    assert_eq!(matrix.rows[0].name, "all");
    assert_eq!(matrix.rows[1].name, "worker");
    assert_eq!(matrix.rows[0].values[0].as_deref(), Some("Auto"));
    assert_eq!(matrix.rows[0].values[1].as_deref(), Some("0"));
    assert_eq!(matrix.rows[0].values[3].as_deref(), Some("18.2k"));
    assert_eq!(matrix.rows[0].values[4].as_deref(), Some("18%"));
    assert_eq!(matrix.rows[0].values[5].as_deref(), Some("$12.40"));
    assert_eq!(matrix.rows[0].values[3..], matrix.rows[1].values[3..]);
    assert_eq!(app.workspace_snapshot().unwrap().preferences, saved);
    app.labels = crate::labels::Supplied::from_document("digest", &valued_document(), true);
    app.overview_rows.borrow_mut().clear();
    let stale = Matrix::project(&app, 5);
    assert!(stale.rows[0].values[0].is_none());
    assert!(
        stale
            .rows
            .iter()
            .all(|column| column.values[3..].iter().all(Option::is_none))
    );
    assert!(
        stale.rows[1].values[2].is_some(),
        "independent HOUR observations are not supplier zeroes"
    );
}
#[test]
fn rank_and_all_totals_use_unique_scoped_hour_identities_and_grow_by_rows() {
    let mut app = populated_board(2, 6);
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    capture::populate_lower_band(&mut app, "squads");
    capture::populate_final(&mut app, "squads");
    let wide = Matrix::project(&app, 5);
    assert_eq!(wide.rows.len(), 4);
    assert_eq!(wide.rows[0].name, "all");
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
        wide.rows[1..]
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        names
    );
    assert_eq!(Matrix::project(&app, 4).rows.len(), 3);
    let room = app.workspace_snapshot().unwrap().roster.squads[1]
        .room_id
        .clone();
    app.workspace.visibility = OpsVisibility {
        scope: vec![room],
        reveal_hidden: false,
    };
    app.overview_rows.borrow_mut().clear();
    let other = Matrix::project(&app, 5);
    assert_eq!(other.rows.len(), 1);
    assert!(other.rows[0].values[2].is_none());
    assert_eq!(other.rows[0].values[0].as_deref(), Some("Auto"));
}
#[test]
fn a_different_supplier_window_never_changes_the_fixed_twenty_four_hour_captions() {
    let mut app = valued_board();
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    let mut document = valued_document();
    document["members"][0]["savings"]["window"] = serde_json::json!("7d");
    app.labels = crate::labels::Supplied::from_document("digest", &document, false);
    let matrix = Matrix::project(&app, 5);
    assert_eq!(digest_matrix::HEADERS[4], "CTX 24h");
    assert!(matrix.rows[0].values[3..].iter().all(Option::is_none));
    assert!(
        app.workspace_snapshot()
            .unwrap()
            .roster
            .members
            .contains_key(MEMBER)
    );
}

#[test]
fn narrow_overview_keeps_the_cost_window_in_actual_frames() {
    use tmt_cli_style::{Depth, Theme, theme::Base};
    crate::status::with_now_ms(1_791_600_000_000, || {
        for (base, depth) in [
            (Base::Tmt, Depth::TrueColor),
            (Base::TmtLight, Depth::TrueColor),
            (Base::Tmt, Depth::None),
        ] {
            for width in [60, 80, 103] {
                for count in [1, 3, 24] {
                    let mut app = many_agents(count);
                    app.view.as_mut().unwrap().look = crate::look::Look {
                        theme: Theme::new(base),
                        depth,
                    };
                    // One pane at a time below 100 cells: Measure is the right pane.
                    if width < 100 {
                        app.focus = 1;
                    }
                    let buffer = paint(&mut app, width, 51);
                    let lines: Vec<String> = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                        .collect();
                    let cost = lines
                        .iter()
                        .find(|line| line.contains("cost avoided"))
                        .unwrap_or_else(|| panic!("{width}/{count}: the narrow Digest cost row is visible\n{}", lines.join("\n")));
                    assert!(cost.contains("cost avoided 24h"), "{width}/{count}: {cost}");
                    assert!(!cost.contains("cost avoided 2…"));
                    assert!(cost.contains("≈ $"), "cost remains an estimate: {cost}");
                }
            }
        }
    });
}
