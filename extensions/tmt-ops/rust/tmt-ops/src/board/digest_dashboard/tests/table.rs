//! Agents by TOK: saved columns, their unit switch, the width order and the shared sum.
use super::super::table::{Field, narrow_hint, tracks};
use super::{MEMBER, paint, press, save, valued_board, valued_document};
use crate::{
    board::app::{App, Choice},
    config::{DigestWindow, SavingsUnit},
};
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::rc::Rc;

fn fields(width: u16, unit: SavingsUnit) -> Vec<Field> {
    tracks(width, unit)
        .into_iter()
        .map(|(field, _)| field)
        .collect()
}

#[test]
fn money_and_saved_columns_show_before_share_and_recipients_and_agent_names_yield_first() {
    use Field::*;
    let all = vec![
        Agent, State, Activity, Tokens, Share, Saved, Cost, SavedCost, Digest, Talks,
    ];
    let common = vec![
        Agent, State, Activity, Tokens, Saved, Cost, SavedCost, Digest,
    ];
    let narrow = vec![Agent, State, Activity, Tokens, Saved, Digest];
    for (width, expected) in [
        (160, all.clone()),
        (120, all.clone()),
        (112, all),
        (111, {
            let mut shared = common.clone();
            shared.insert(4, Share);
            shared
        }),
        (100, common.clone()),
        (96, common.clone()),
        (94, common),
        (93, narrow.clone()),
        (80, narrow.clone()),
        (76, narrow),
        (75, vec![Agent, State, Tokens, Saved, Digest]),
        (60, vec![Agent, State, Tokens, Saved, Digest]),
    ] {
        assert_eq!(
            fields(width, SavingsUnit::Tokens),
            expected,
            "width={width}"
        );
        let tracks = tracks(width, SavingsUnit::Tokens);
        let span = tracks.iter().map(|(_, w)| w + 1).sum::<u16>() - 1;
        assert!(span <= width, "width={width}: {tracks:?}");
    }
    assert_eq!(tracks(96, SavingsUnit::Tokens)[0], (Agent, 12));
    assert_eq!(tracks(94, SavingsUnit::Tokens)[0], (Agent, 10));
    assert_eq!(tracks(76, SavingsUnit::Tokens)[0], (Agent, 9));
    // A cost unit already shows money under SAVED; its width goes to share first.
    assert_eq!(
        fields(96, SavingsUnit::Cost),
        vec![Agent, State, Activity, Tokens, Share, Saved, Cost, Digest]
    );
    assert_eq!(
        fields(160, SavingsUnit::Cost),
        vec![
            Agent, State, Activity, Tokens, Share, Saved, Cost, Digest, Talks
        ]
    );
}

#[test]
fn the_narrow_hint_names_exactly_the_columns_left_out_for_width() {
    for (width, unit, hint) in [
        (160, SavingsUnit::Tokens, None),
        (112, SavingsUnit::Tokens, None),
        (
            100,
            SavingsUnit::Tokens,
            Some("Widen to 112 columns for share and recipients."),
        ),
        (
            80,
            SavingsUnit::Tokens,
            Some("Widen to 112 columns for share, price, saved price and recipients."),
        ),
        (
            60,
            SavingsUnit::Percent,
            Some("Widen to 112 columns for activity, share, price, saved price and recipients."),
        ),
        (102, SavingsUnit::Cost, None),
        (
            96,
            SavingsUnit::Cost,
            Some("Widen to 102 columns for recipients."),
        ),
        (
            80,
            SavingsUnit::Cost,
            Some("Widen to 102 columns for share, price and recipients."),
        ),
    ] {
        assert_eq!(
            narrow_hint(width, unit, usize::MAX).as_deref(),
            hint,
            "width={width} unit={unit:?}"
        );
    } // Names that do not fit the footer become a count; a single name always fits.
    assert_eq!(
        narrow_hint(60, SavingsUnit::Tokens, 50).as_deref(),
        Some("Widen to 112 columns for 5 more columns.")
    );
    assert_eq!(
        narrow_hint(96, SavingsUnit::Cost, 10).as_deref(),
        Some("Widen to 102 columns for recipients.")
    );
}

/// Each painted row as its cells, so a column reads from its heading's position.
fn grid(app: &mut App, width: u16, height: u16) -> Vec<Vec<String>> {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect()
        })
        .collect()
}

fn text(row: &[String], from: usize, width: usize) -> String {
    row[from..(from + width).min(row.len())]
        .concat()
        .trim()
        .to_owned()
}

/// The value under `heading` on the row whose AGENT cell starts with `agent`.
fn cell(grid: &[Vec<String>], heading: &str, agent: &str) -> Option<String> {
    let width = heading.chars().count();
    let (header, column) = grid.iter().enumerate().find_map(|(y, row)| {
        (0..row.len().saturating_sub(width))
            .find(|&x| row[x..x + width].concat() == heading)
            .map(|x| (y, x))
    })?;
    let agent_column = (0..grid[header].len())
        .find(|&x| grid[header][x..].concat().starts_with("AGENT"))
        .unwrap();
    grid[header + 1..]
        .iter()
        .find(|row| text(row, agent_column, 12).starts_with(agent))
        .map(|row| text(row, column, width.max(7)))
}

fn digest(app: &mut App) -> Vec<Vec<String>> {
    press(app, '2');
    grid(app, 160, 36)
}

#[test]
fn saved_cells_show_the_supplied_estimate_in_the_shared_unit_and_at_list_price() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = valued_board();
    let shown = digest(&mut app);
    for agent in ["worker", "all"] {
        assert_eq!(
            cell(&shown, "SAVED ▾", agent).as_deref(),
            Some("18.2k"),
            "{agent}"
        );
        assert_eq!(
            cell(&shown, "≈ $ SAVED", agent).as_deref(),
            Some("$12.4"),
            "{agent}"
        );
        assert!(cell(&shown, "≈ $ USED", agent).is_some(), "{agent}");
    }
    assert!(cell(&shown, "≈ $ in", "worker").is_none());
}

/// Seven agents of which only `MEMBER` has an estimate.
fn crowd(document: serde_json::Value) -> App {
    let mut app = super::layout::fixture();
    app.labels = crate::labels::Supplied::from_document("digest", &document, false);
    app
}

#[test]
fn a_sum_that_leaves_agents_out_is_a_lower_bound_and_unknown_never_reads_zero() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = crowd(valued_document());
    let shown = grid(&mut app, 160, 40);
    assert_eq!(cell(&shown, "SAVED ▾", "all").as_deref(), Some("≥18.2k"));
    assert_eq!(cell(&shown, "≈ $ SAVED", "all").as_deref(), Some("≥$12.4"));
    assert_eq!(cell(&shown, "SAVED ▾", "agent-1").as_deref(), Some("—"));
    assert_eq!(cell(&shown, "≈ $ SAVED", "agent-1").as_deref(), Some("—"));
    assert!(!shown.concat().concat().contains("SAVED % covers"));
    // A partial percent carries no bound mark; the table names its coverage instead.
    let effect = app.perform(&crate::action::Action::parse("savings-unit").unwrap());
    save(&mut app, effect);
    let shown = grid(&mut app, 160, 40);
    assert_eq!(cell(&shown, "SAVED ▾", "all").as_deref(), Some("18%"));
    assert!(
        shown
            .concat()
            .concat()
            .contains("SAVED % covers 1 of 7 agents"),
        "the partial percent names its coverage"
    );

    // A known zero is zero.
    let mut zero = valued_document();
    zero["members"][0]["savings"]["estimatedInputTokensAvoided"] = serde_json::json!(0);
    zero["members"][0]["savings"]["cost"]["usdMicros"] = serde_json::json!(0);
    let mut app = valued_board();
    app.labels = crate::labels::Supplied::from_document("digest", &zero, false);
    let shown = digest(&mut app);
    assert_eq!(cell(&shown, "SAVED ▾", "worker").as_deref(), Some("0"));
    assert_eq!(cell(&shown, "≈ $ SAVED", "worker").as_deref(), Some("$0"));

    // Stale or absent evidence is unavailable, never a measured value.
    for labels in [
        crate::labels::Supplied::from_document("digest", &valued_document(), true),
        crate::labels::Supplied::default(),
    ] {
        let mut app = valued_board();
        app.labels = labels;
        let shown = digest(&mut app);
        for agent in ["worker", "all"] {
            assert_eq!(
                cell(&shown, "SAVED ▾", agent).as_deref(),
                Some("—"),
                "{agent}"
            );
            assert_eq!(
                cell(&shown, "≈ $ SAVED", agent).as_deref(),
                Some("—"),
                "{agent}"
            );
        }
    }
}

#[test]
fn quotes_under_different_price_tables_never_combine() {
    let _no_commands = crate::runner::forbid_commands();
    let mut document = valued_document();
    let mut other = document["members"][0].clone();
    let id = "40000000-0000-4000-8000-000000000001";
    other["identityId"] = serde_json::json!(id);
    other["savings"]["cost"]["pricingVersion"] = serde_json::json!("2026-10-01");
    other["savings"]["percent"]["fromMs"] = serde_json::json!(0);
    document["members"].as_array_mut().unwrap().push(other);
    let mut app = crowd(document);
    let shown = grid(&mut app, 160, 40);
    assert_eq!(cell(&shown, "≈ $ SAVED", "all").as_deref(), Some("—"));
    assert_eq!(
        cell(&shown, "≈ $ SAVED", "agent-1").as_deref(),
        Some("$12.4")
    );
    // Tokens still add up; only the money and the ratio cannot combine.
    assert_eq!(cell(&shown, "SAVED ▾", "all").as_deref(), Some("≥36.4k"));
}

fn click_heading(app: &mut App, shown: &[Vec<String>]) -> crate::board::app::Effect {
    let hit = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find(|(area, choice)| {
            *choice == Choice::SavingsUnit
                && shown[usize::from(area.y)][usize::from(area.x)..]
                    .concat()
                    .starts_with("SAVED ▾")
        })
        .map(|(area, _)| *area)
        .expect("the SAVED heading answers a click");
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    )
}

#[test]
fn the_saved_heading_cycles_the_shared_unit_and_cost_hides_the_duplicate_money_column() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = valued_board();
    let shown = digest(&mut app);
    let effect = click_heading(&mut app, &shown);
    // Nothing changes until Config accepts the write.
    let pending = grid(&mut app, 160, 36);
    assert_eq!(
        cell(&pending, "SAVED ▾", "worker").as_deref(),
        Some("18.2k")
    );
    save(&mut app, effect);
    assert_eq!(app.savings_unit(), SavingsUnit::Percent);
    let shown = grid(&mut app, 160, 36);
    assert_eq!(cell(&shown, "SAVED ▾", "worker").as_deref(), Some("18%"));
    assert_eq!(
        cell(&shown, "≈ $ SAVED", "worker").as_deref(),
        Some("$12.4")
    );

    let effect = click_heading(&mut app, &shown);
    save(&mut app, effect);
    assert_eq!(app.savings_unit(), SavingsUnit::Cost);
    let shown = grid(&mut app, 160, 36);
    assert_eq!(cell(&shown, "SAVED ▾", "worker").as_deref(), Some("≈$12.4"));
    assert!(cell(&shown, "≈ $ SAVED", "worker").is_none());
    assert!(cell(&shown, "SHARE", "worker").is_some());

    // The Actions route is the same switch.
    let effect = app.perform(&crate::action::Action::parse("savings-unit").unwrap());
    save(&mut app, effect);
    assert_eq!(app.savings_unit(), SavingsUnit::Tokens);
    let shown = grid(&mut app, 160, 36);
    assert_eq!(cell(&shown, "SAVED ▾", "worker").as_deref(), Some("18.2k"));
}

fn sum(app: &App) -> Rc<crate::labels::digest::SavingsSummary> {
    super::super::savings(app)
}

#[test]
fn the_summary_and_the_all_row_share_one_sum_until_its_status_or_window_changes() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = crowd(valued_document());
    paint(&mut app, 160, 40);
    let first = sum(&app);
    assert_eq!(first.tokens, Some(18_200));
    paint(&mut app, 160, 40);
    assert!(
        Rc::ptr_eq(&first, &sum(&app)),
        "a frame reuses the held sum"
    );

    // A member's Details read its own estimate and leave the shared sum in place.
    let index = app
        .workspace_rows()
        .iter()
        .position(|row| super::super::metrics::id(row) == Some(MEMBER))
        .unwrap();
    app.workspace.selected = Some(app.workspace_rows()[index].target.clone());
    paint(&mut app, 160, 40);
    assert!(Rc::ptr_eq(&first, &sum(&app)));

    app.digest_window = DigestWindow::Week;
    paint(&mut app, 160, 40);
    let week = sum(&app);
    assert!(!Rc::ptr_eq(&first, &week), "a window change sums again");
    assert_eq!(week.tokens, None, "the supplier read the day window");

    app.digest_window = DigestWindow::Day;
    app.labels = crate::labels::Supplied::from_document("digest", &valued_document(), false);
    paint(&mut app, 160, 40);
    let read = sum(&app);
    assert!(!Rc::ptr_eq(&first, &read), "a new read sums again");
    assert_eq!(*read, *first);
}
