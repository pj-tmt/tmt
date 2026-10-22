//! The Allowance block of the Digest tab: weekly windows, the even burn and what
//! each projection line leaves at the reset. Fixed clock, UTC, no I/O.
use super::*;
use crate::board::{
    app::App,
    limits::{Provider, Rates, Reading, Standing, Window},
    overview::cron_counts::GlyphSet,
    shell::OpsTab,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

const NOW: u64 = 1_791_600_000_000; // Sat 2026-10-10 02:40 UTC
const HOUR: u64 = 3_600_000;

fn standing(
    provider: &str,
    left: f64,
    resets_in_hours: u64,
    observed_ago_ms: u64,
    rates: Rates,
) -> Standing {
    Standing {
        provider: Provider::named(provider),
        latest: Some(Reading {
            observed_at_ms: NOW - observed_ago_ms,
            weekly: Window {
                left,
                resets_at_ms: NOW + resets_in_hours * HOUR,
            },
            short: None,
        }),
        burn: None,
        trend: None,
        rates,
    }
}

fn digest(limits: Vec<Standing>) -> App {
    let mut app = quota_board();
    app.shell.tab = OpsTab::Digest;
    app.local_zone = Some(jiff::tz::TimeZone::UTC);
    app.limits = Some(limits);
    app
}

fn claude() -> Standing {
    // 26 % left, 40 h to the reset (Sun 18:40): 1.0 pt/h empties at Sun 04:40,
    // 0.5 leaves 6 %, 0.2 leaves 18 %, and no burn since the reset leaves 26 %.
    standing(
        "claude",
        26.0,
        40,
        10 * 60_000,
        Rates::measured(Some(1.0), Some(0.5), Some(0.2), Some(0.0)),
    )
}

fn buffer(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    crate::status::with_now_ms(NOW, || {
        terminal
            .draw(|frame| {
                crate::board::view::prepare(app, frame.area());
                crate::board::view::render(frame, app);
            })
            .unwrap();
    });
    terminal.backend().buffer().clone()
}

fn rows(buffer: &Buffer) -> Vec<String> {
    let width = usize::from(buffer.area.width);
    buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn words(row: &str) -> Vec<&str> {
    row.split_whitespace().collect()
}

fn row_of<'a>(rows: &'a [String], source: &str) -> &'a str {
    rows.iter()
        .find(|row| words(row).first() == Some(&source))
        .unwrap_or_else(|| panic!("no {source} row in\n{}", rows.join("\n")))
}

#[test]
fn a_wide_row_shows_left_need_every_line_and_the_reset() {
    let mut app = digest(vec![claude()]);
    let rows = rows(&buffer(&mut app, 120, 40));
    assert!(
        rows.iter().any(
            |row| row.contains("Allowance · weekly windows · left at reset if this rate holds")
        ),
        "{}",
        rows.join("\n")
    );
    let header = rows.iter().find(|row| row.contains("NEED %/h")).unwrap();
    assert_eq!(
        words(header),
        [
            "SOURCE", "PLAN", "LEFT", "NEED", "%/h", "3h", "6h", "1d", "SINCE", "RESET", "RESETS"
        ]
    );
    assert_eq!(
        words(row_of(&rows, "Claude")),
        [
            "Claude", "week", "26%", "0.65", "✗", "Su", "04:40", "6%", "18%", "26%", "Su", "18:40"
        ]
    );
    assert!(!rows.iter().any(|row| row.contains("5h")), "no 5h rows");
}

#[test]
fn only_a_line_that_runs_out_takes_the_attention_colour() {
    let mut app = digest(vec![claude()]);
    let view = app.view.as_mut().unwrap();
    view.look = crate::look::Look::default();
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    let buffer = buffer(&mut app, 120, 40);
    let rows = rows(&buffer);
    let y = rows
        .iter()
        .position(|row| words(row).first() == Some(&"Claude"))
        .unwrap();
    let width = usize::from(buffer.area.width);
    let waiting = app
        .look()
        .role(tmt_cli_style::Role::Waiting)
        .fg
        .expect("the pinned colour assertion needs an attention foreground");
    let mut coloured = Vec::new();
    for x in 0..width {
        let cell = &buffer[(x as u16, y as u16)];
        if cell.fg == waiting && cell.symbol() != " " {
            coloured.push(cell.symbol().to_owned());
        }
    }
    assert_eq!(coloured.concat(), "✗Su04:40", "{rows:?}");
}

#[test]
fn missing_data_is_a_dash_and_a_stale_reading_is_not_quoted() {
    let stale = standing("muse", 81.0, 100, 3 * HOUR, Rates::default());
    let unsampled = Standing {
        provider: Provider::named("codex"),
        latest: None,
        burn: None,
        trend: None,
        rates: Rates::default(),
    };
    let fresh_without_rates = standing("openai", 55.0, 100, 60_000, Rates::default());
    let mut app = digest(vec![claude(), stale, unsampled, fresh_without_rates]);
    let rows = rows(&buffer(&mut app, 120, 40));
    assert_eq!(words(row_of(&rows, "Muse")), ["Muse", "week", "—", "—"]);
    assert_eq!(
        words(row_of(&rows, "Openai")),
        [
            "Openai", "week", "55%", "0.55", "—", "—", "—", "—", "We", "06:40"
        ]
    );
    assert!(
        !rows.iter().any(|row| words(row).first() == Some(&"Codex")),
        "a provider never sampled has no row"
    );
}

#[test]
fn a_narrow_row_is_one_line_with_the_worst_projection() {
    let steady = standing(
        "openai",
        55.0,
        100,
        60_000,
        Rates::measured(Some(0.3), Some(0.3), None, Some(0.3)),
    );
    let mut app = digest(vec![claude(), steady]);
    let rows = rows(&buffer(&mut app, 80, 40));
    assert!(
        rows.iter()
            .any(|row| row.contains("Allowance · left at reset, worst window")),
        "{}",
        rows.join("\n")
    );
    assert_eq!(
        words(row_of(&rows, "Claude")),
        [
            "Claude", "week", "26%", "need", "0.65%/h", "✗", "Su", "04:40"
        ]
    );
    assert_eq!(
        words(row_of(&rows, "Openai")),
        [
            "Openai", "week", "55%", "need", "0.55%/h", "lasts", "·", "25%"
        ]
    );
    assert!(
        !rows.iter().any(|row| row.contains("NEED %/h")),
        "no column header"
    );
}

#[test]
fn an_ascii_terminal_gets_an_ascii_marker() {
    let mut app = digest(vec![claude()]);
    app.cron_glyphs = GlyphSet::Ascii;
    let rows = rows(&buffer(&mut app, 120, 40));
    assert_eq!(words(row_of(&rows, "Claude"))[4..7], ["x", "Su", "04:40"]);
}

#[test]
fn no_reading_or_no_room_shows_no_block_and_leaves_the_table_whole() {
    let mut none = digest(Vec::new());
    assert!(
        !rows(&buffer(&mut none, 120, 40))
            .iter()
            .any(|row| row.contains("Allowance"))
    );
    let mut short = digest(vec![claude()]);
    let rows = rows(&buffer(&mut short, 120, 12));
    assert!(
        !rows.iter().any(|row| row.contains("Allowance")),
        "{}",
        rows.join("\n")
    );
}

fn resetting_in(ms: u64, left: f64) -> Standing {
    let mut standing = standing("claude", left, 0, 10 * 60_000, Rates::default());
    standing.latest.as_mut().unwrap().weekly.resets_at_ms = NOW + ms;
    standing
}

#[test]
fn need_is_hidden_a_minute_before_the_reset_and_capped_just_outside_the_last_hour() {
    for (width, shown_hidden, shown_capped) in [
        (
            120,
            vec!["Claude", "week", "30%", "—"],
            vec!["Claude", "week", "100%", ">99"],
        ),
        (
            80,
            vec!["Claude", "week", "30%", "—"],
            vec!["Claude", "week", "100%", "need", ">99%/h"],
        ),
    ] {
        let mut app = digest(vec![resetting_in(60_000, 30.0)]);
        let hidden = rows(&buffer(&mut app, width, 40));
        let row = words(row_of(&hidden, "Claude"));
        assert!(
            shown_hidden.iter().all(|word| row.contains(word)),
            "{width}: {row:?}"
        );
        assert!(
            !row.iter()
                .any(|word| word.contains('%') && word.contains('/')),
            "{width}: no figure in the last hour: {row:?}"
        );
        let mut app = digest(vec![resetting_in(HOUR, 100.0)]);
        let capped = rows(&buffer(&mut app, width, 40));
        let row = words(row_of(&capped, "Claude"));
        assert!(
            shown_capped.iter().all(|word| row.contains(word)),
            "{width}: {row:?}"
        );
    }
}
