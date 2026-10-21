//! The Measure pane's `Exchanges · <range>` section: who talks to whom, ranked by traffic.
use super::*;
use crate::{board::shell::RightTab, config::TokRange};

fn id(i: usize) -> String {
    format!("40000000-0000-4000-8000-{i:012}")
}

fn measured(extra: usize, flows: &[(usize, usize, u64)]) -> App {
    let mut app = board();
    crate::board::app::workspace::tests::measure_exchanges(&mut app, extra, flows);
    app
}

/// The Measure pane's rows from the `Exchanges ·` rule on, or none when the section is absent.
fn section(screen: &str, width: u16) -> Option<Vec<String>> {
    let rows: Vec<Vec<char>> = screen
        .chars()
        .collect::<Vec<_>>()
        .chunks(width as usize)
        .map(<[char]>::to_vec)
        .collect();
    let text = |row: &[char]| row.iter().collect::<String>();
    let (start, column) = rows.iter().enumerate().find_map(|(y, row)| {
        let x = text(row).find("Exchanges ·")?;
        Some((y, text(row)[..x].chars().count().saturating_sub(3)))
    })?;
    Some(
        rows[start..]
            .iter()
            .map(|row| text(&row[column.min(row.len())..]))
            .collect(),
    )
}

#[test]
fn the_section_names_the_shared_range_and_the_sum_column() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = measured(2, &[(1, 2, 7), (2, 0, 3)]);
    let screen = paint(&mut app, 160, 48);
    assert!(
        screen.contains(&format!("Exchanges · {}", app.tok_range().argument())),
        "{screen}"
    );
    assert!(screen.contains("sum"), "{screen}");
}

#[test]
fn measure_wants_the_usage_report_so_the_matrix_has_traffic_at_the_default_range() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = board();
    app.right_tab = RightTab::Talk;
    assert_eq!(app.tok_range(), TokRange::default());
    assert!(
        app.usage_report_want().is_none(),
        "Talk keeps the report unwanted"
    );
    app.right_tab = RightTab::Measure;
    assert!(app.usage_report_want().is_some());
}

#[test]
fn a_cell_click_selects_the_sender_and_opens_talk_with_a_notice() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = measured(2, &[(1, 2, 7)]);
    paint(&mut app, 160, 48);
    let (area, choice) = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find(|(_, choice)| {
            matches!(choice, crate::board::app::Choice::ExchangeCell { sender, recipient }
                if *sender == id(1) && *recipient == id(2))
        })
        .cloned()
        .expect("a cell for agent-1 → agent-2");
    let _ = choice;
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert_eq!(app.right_tab, RightTab::Talk);
    assert!(matches!(
        app.workspace.selected,
        Some(crate::board::workspace::OpsTarget::Member { ref identity_id, .. })
            if *identity_id == id(1)
    ));
    let status = app
        .notice
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(
        status.contains("agent-1") && status.contains("agent-2"),
        "{status}"
    );
}

#[test]
fn the_busiest_agents_come_first_and_the_section_fits_whole_or_not_at_all() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = measured(8, &[(8, 7, 90), (7, 8, 80), (1, 2, 1)]);
    let screen = paint(&mut app, 160, 60);
    let rows = section(&screen, 160).expect("the section paints at 160x60");
    let position = |name: &str| rows.iter().skip(2).position(|row| row.contains(name));
    let busy = position("agent-8").expect("agent-8 listed");
    let quiet = position("agent-1").expect("agent-1 listed");
    assert!(busy < quiet, "{rows:#?}");
    assert!(position("agent-6").is_none_or(|p| p > quiet), "{rows:#?}");
    for height in 8..40 {
        let mut app = measured(2, &[(1, 2, 7)]);
        let screen = paint(&mut app, 160, height);
        if let Some(rows) = section(&screen, 160) {
            assert!(
                rows.iter().skip(1).any(|row| row.contains("from\\to")),
                "{height}: a titled section keeps its header: {rows:#?}"
            );
        }
    }
}
