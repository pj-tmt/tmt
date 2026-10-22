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
            "SOURCE", "LEFT", "NEED", "%/h", "3h", "6h", "1d", "TOKENS", "SINCE", "RESET", "RESETS"
        ]
    );
    assert_eq!(
        words(row_of(&rows, "Claude")),
        [
            "Claude", "26%", "0.65", "✗", "Su", "04:40", "6%", "18%", "—", "26%", "Su", "18:40"
        ]
    );
    assert!(!rows.iter().any(|row| row.contains("5h")), "no 5h rows");
}

#[test]
fn only_a_line_that_runs_out_takes_the_attention_colour() {
    let mut app = digest(vec![claude()]);
    let buffer = buffer(&mut app, 120, 40);
    let rows = rows(&buffer);
    let y = rows
        .iter()
        .position(|row| words(row).first() == Some(&"Claude"))
        .unwrap();
    let width = usize::from(buffer.area.width);
    let waiting = app.look().role(tmt_cli_style::Role::Waiting).fg;
    let mut coloured = Vec::new();
    for x in 0..width {
        let cell = &buffer[(x as u16, y as u16)];
        if cell.fg == waiting.unwrap_or_default() && cell.symbol() != " " {
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
    assert_eq!(words(row_of(&rows, "Muse")), ["Muse", "—", "—"]);
    assert_eq!(
        words(row_of(&rows, "Openai")),
        [
            "Openai", "55%", "0.55", "—", "—", "—", "—", "—", "We", "06:40"
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
        ["Claude", "26%", "need", "0.65%/h", "✗", "Su", "04:40"]
    );
    assert_eq!(
        words(row_of(&rows, "Openai")),
        ["Openai", "55%", "need", "0.55%/h", "lasts", "·", "25%"]
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
    assert_eq!(words(row_of(&rows, "Claude"))[3..6], ["x", "Su", "04:40"]);
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

/// A claude window at 20 % left, 40 h to the reset, that lost 0.5 points an hour over the last
/// day (12 points), with one member "a" on claude spending `cells` tokens an hour-cell.
fn claude_with_tokens(cells: [u64; 24], complete: [bool; 24], ends_ago_ms: u64) -> App {
    use crate::board::usage_report::{
        Delivery, Report,
        model::{HourlySeries, Seat, Who},
    };
    let mut app = digest(vec![standing(
        "claude",
        20.0,
        40,
        10 * 60_000,
        Rates::measured(Some(0.5), Some(0.5), Some(0.5), Some(0.5)),
    )]);
    let input = crate::board::rate::tests::input(1_000);
    crate::board::app::tests::install_rate_observation(
        &mut app,
        "product".into(),
        crate::board::app::RateView {
            settings: crate::config::TokenRate {
                enabled: true,
                ..Default::default()
            },
            input,
            history: None,
        },
    );
    let range = app.tok_range();
    let from_ms = NOW - ends_ago_ms - 24 * HOUR;
    app.usage_report.apply(Delivery {
        since: range,
        result: Ok(Report {
            since: range,
            observed_at_ms: NOW,
            window_from_ms: Some(from_ms),
            seats: vec![Seat {
                who: Who {
                    id: Some("a".into()),
                    name: None,
                    retired: false,
                },
                model_inputs: None,
                input: 0,
                output: 0,
                cached: 0,
                covered_ms: 24 * HOUR,
                window_ms: 24 * HOUR,
                from_ms,
                to_ms: from_ms + 24 * HOUR,
                observed_at_ms: NOW,
                starts_at_ms: Some(from_ms),
                complete: true,
                series: HourlySeries::new(from_ms, cells, complete),
            }],
            unavailable: vec![],
            exchange: None,
            warnings: vec![],
        }),
    });
    app
}

fn claude_row(app: &mut App, width: u16) -> Vec<String> {
    let rows = rows(&buffer(app, width, 40));
    words(row_of(&rows, "Claude"))
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// The wide row of the claude window above (20 % left, 40 h to the reset, 0.5 points an hour on
/// every sampled line, so each empties it at Su 18:40) with `tokens` in the TOKENS cell.
fn wide_row_with(tokens: [&str; 3]) -> Vec<String> {
    let mut row = vec!["Claude", "20%", "0.50"];
    for _ in 0..3 {
        row.extend(["✗", "Su", "18:40"]);
    }
    row.extend(tokens);
    row.extend(["✗", "Su", "18:40", "Su", "18:40"]);
    row.into_iter().map(str::to_owned).collect()
}

/// The same row with a dash where the tokens cell would be.
fn dash_row() -> Vec<String> {
    let mut row = wide_row_with(["✗", "Su", "01:10"]);
    row.splice(12..15, ["—".to_owned()]);
    row
}

#[test]
fn the_tokens_cell_projects_the_last_three_hours_at_the_cost_of_the_last_day() {
    // 12 points (0.5 an hour for 24 h) over 27 000 tokens, of which 6 000 in the last three
    // cells: 2 000 tokens an hour at 12 / 27 000 points a token is 0.89 points an hour, which
    // empties the 20 % in 22.5 h, at Su 01:10.
    let mut cells = [1_000u64; 24];
    cells[21..].copy_from_slice(&[1_000, 2_000, 3_000]);
    let mut app = claude_with_tokens(cells, [true; 24], 0);
    assert_eq!(
        claude_row(&mut app, 120),
        wide_row_with(["✗", "Su", "01:10"])
    );
}

#[test]
fn the_tokens_cell_is_a_dash_without_a_usable_report_or_complete_cells() {
    // No report yet.
    let mut app = digest(vec![standing(
        "claude",
        20.0,
        40,
        10 * 60_000,
        Rates::measured(Some(0.5), Some(0.5), Some(0.5), Some(0.5)),
    )]);
    assert_eq!(claude_row(&mut app, 120), dash_row());
    let mut incomplete = [true; 24];
    incomplete[5] = false;
    for (label, complete, ends_ago) in [
        ("an incomplete cell", incomplete, 0),
        ("a stale series", [true; 24], 3 * HOUR),
    ] {
        let mut app = claude_with_tokens([1_000; 24], complete, ends_ago);
        assert_eq!(claude_row(&mut app, 120), dash_row(), "{label}");
    }
}
