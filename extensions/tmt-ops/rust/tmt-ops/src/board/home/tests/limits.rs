//! The provider-limits row inside the whole HOME frame: it follows the usage
//! switch and the width, holds its row before the first sample, and moves
//! nothing else when the sample arrives.
use super::interaction::board;
use super::*;
use crate::{
    board::{
        app::{App, RateView},
        limits::{Provider, Reading, Standing, Window},
        rate::Input,
    },
    config::TokenRate,
};
use ratatui::{Terminal, backend::TestBackend};

const NOW: u64 = 1_791_600_000_000;
const HOUR: u64 = 3_600_000;

fn app(usage_enabled: bool) -> App {
    let mut app = board(&[(
        "a",
        document("a", row("a", "product-lead", "working"), vec![]),
    )]);
    let view = app.view.as_mut().unwrap();
    view.home_rate.insert(
        "a".into(),
        RateView {
            input: Input {
                room: "room".into(),
                resumes: Default::default(),
                names: Default::default(),
            },
            settings: TokenRate {
                enabled: usage_enabled,
                ..Default::default()
            },
            history: None,
        },
    );
    let mut snapshot = crate::board::app::tests::snapshot(ALL, json!([]));
    snapshot.tabs = app.tabs.clone();
    snapshot.view = Ok(app.view.take().unwrap());
    app.apply(snapshot);
    app
}

fn standings() -> Vec<Standing> {
    vec![Standing {
        provider: Provider::named("codex"),
        latest: Some(Reading {
            observed_at_ms: NOW - 60_000,
            weekly: Window {
                left: 78.0,
                resets_at_ms: NOW + 58 * HOUR,
            },
            short: None,
        }),
        burn: Some(0.4),
    }]
}

fn rows(app: &App, width: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
    crate::status::with_now_ms(NOW, || {
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
    });
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
        .map(|row| row.trim_end().to_owned())
        .collect()
}

fn index_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?} in {rows:#?}"))
}

#[test]
fn the_row_holds_its_place_before_the_first_sample_and_moves_nothing_when_it_fills() {
    for width in [100, 160] {
        let mut app = app(true);
        let before = rows(&app, width);
        let pending = index_of(&before, "Updating limits…");
        let body_before = index_of(&before, "product-lead");
        assert!(app.set_limits(standings()));
        let after = rows(&app, width);
        assert_eq!(index_of(&after, "Codex  7d 78%"), pending, "{width}");
        assert_eq!(index_of(&after, "product-lead"), body_before, "{width}");
    }
}

#[test]
fn the_row_sits_directly_under_the_header_and_the_body_follows() {
    let mut app = app(true);
    app.set_limits(standings());
    let rows = rows(&app, 100);
    assert_eq!(
        rows[index_of(&rows, "Codex  7d 78%")],
        "Codex  7d 78% · 2d10h · ~0.4%/h"
    );
    assert!(index_of(&rows, "product-lead") > index_of(&rows, "Codex  7d 78%"));
}

#[test]
fn no_row_below_md_or_with_the_usage_meter_off() {
    let mut narrow = app(true);
    narrow.set_limits(standings());
    let shown = rows(&narrow, 99);
    assert!(
        shown
            .iter()
            .all(|row| !row.contains("Codex") && !row.contains("limits"))
    );
    let mut off = app(false);
    off.set_limits(standings());
    let shown = rows(&off, 160);
    assert!(
        shown
            .iter()
            .all(|row| !row.contains("Codex") && !row.contains("limits"))
    );
}

#[test]
fn a_board_with_no_provider_gives_the_row_back_once_the_first_sample_says_so() {
    let mut app = app(true);
    let waiting = rows(&app, 100);
    assert!(index_of(&waiting, "Updating limits…") > 0);
    app.set_limits(Vec::new());
    let settled = rows(&app, 100);
    assert!(settled.iter().all(|row| !row.contains("limits")));
}

#[test]
fn an_unchanged_sample_paints_nothing_and_a_changed_one_does() {
    let mut app = app(true);
    assert!(app.set_limits(standings()));
    assert!(!app.set_limits(standings()));
    let mut moved = standings();
    moved[0].burn = Some(0.5);
    assert!(app.set_limits(moved));
}

#[test]
fn help_explains_the_row_only_where_it_can_show() {
    let section = |app: &App| {
        crate::board::help::model(app)
            .sections
            .into_iter()
            .find(|section| section.title == "provider limits")
    };
    let help = section(&app(true)).expect("HOME with the usage meter on explains the row");
    let text = help
        .entries
        .iter()
        .map(|entry| format!("{} {}", entry.keys, entry.description))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("share of each account window left"), "{text}");
    assert!(text.contains("older than 2 hours"), "{text}");
    assert!(section(&app(false)).is_none());
}
