//! The allowance tile of the Digest summary: one row per provider with what is left, the
//! worst projection and the reset, scrolling inside the tile. Fixed clock, UTC, no I/O.
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
        current: None,
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

/// The tile's words on the row whose provider is `source`: what follows the tile's rule.
fn row_of(rows: &[String], source: &str) -> Vec<String> {
    rows.iter()
        .filter_map(|row| row.rsplit_once(['│', '|']).map(|(_, tile)| words(tile)))
        .find(|tile| tile.first() == Some(&source))
        .map(|tile| tile.into_iter().map(str::to_owned).collect())
        .unwrap_or_else(|| panic!("no {source} row in\n{}", rows.join("\n")))
}

#[test]
fn a_row_shows_left_the_worst_projection_and_the_reset() {
    let mut app = digest(vec![claude()]);
    let rows = rows(&buffer(&mut app, 120, 40));
    assert!(
        rows.iter()
            .any(|row| row.contains("ALLOWANCE LEFT  1 provider")),
        "{}",
        rows.join("\n")
    );
    // 1.0 pt/h over the last 3 h empties it first, at Su 04:40; it resets Su 18:40.
    assert_eq!(
        row_of(&rows, "Claude"),
        [
            "Claude", "26%", "✗", "runs", "out", "Su", "04:40", "resets", "Su", "18:40"
        ]
    );
    assert!(
        !rows.iter().any(|row| row.contains("NEED")),
        "no NEED column"
    );
}

#[test]
fn only_a_window_that_runs_out_takes_the_attention_colour() {
    let steady = standing(
        "openai",
        55.0,
        100,
        60_000,
        Rates::measured(Some(0.3), Some(0.3), None, Some(0.3)),
    );
    let mut app = digest(vec![claude(), steady]);
    let view = app.view.as_mut().unwrap();
    view.look = crate::look::Look::default();
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    let buffer = buffer(&mut app, 120, 40);
    let rows = rows(&buffer);
    let waiting = app
        .look()
        .role(tmt_cli_style::Role::Waiting)
        .fg
        .expect("the pinned colour assertion needs an attention foreground");
    let coloured = |source: &str| {
        let y = rows
            .iter()
            .position(|row| {
                row.rsplit_once('│')
                    .is_some_and(|(_, tile)| words(tile).first() == Some(&source))
            })
            .unwrap();
        (0..buffer.area.width)
            .map(|x| &buffer[(x, y as u16)])
            .filter(|cell| cell.fg == waiting && cell.symbol() != " ")
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    assert_eq!(coloured("Claude"), "✗runsoutSu04:40", "{rows:?}");
    assert_eq!(row_of(&rows, "Openai")[2..5], ["lasts", "past", "reset"]);
    assert_eq!(coloured("Openai"), "", "{rows:?}");
}

#[test]
fn a_stale_reading_is_not_quoted_and_an_unsampled_provider_has_no_row() {
    let stale = standing("muse", 81.0, 100, 3 * HOUR, Rates::default());
    let unsampled = Standing {
        current: None,
        provider: Provider::named("codex"),
        latest: None,
        burn: None,
        trend: None,
        rates: Rates::default(),
    };
    let fresh_without_rates = standing("openai", 55.0, 100, 60_000, Rates::default());
    let mut app = digest(vec![claude(), stale, unsampled, fresh_without_rates]);
    let rows = rows(&buffer(&mut app, 120, 40));
    assert_eq!(
        row_of(&rows, "Muse"),
        ["Muse", "—", "no", "recent", "reading"]
    );
    assert_eq!(
        row_of(&rows, "Openai"),
        ["Openai", "55%", "—", "resets", "We", "06:40"]
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("ALLOWANCE LEFT  3 providers")),
        "a provider never sampled has no row: {rows:?}"
    );
}

#[test]
fn more_providers_than_rows_scroll_inside_the_tile() {
    let providers = ["claude", "codex", "gemini", "muse", "openai"]
        .map(|name| standing(name, 50.0, 100, 60_000, Rates::default()));
    let mut app = digest(providers.to_vec());
    let shown = rows(&buffer(&mut app, 120, 40));
    // Four tile rows: the heading and three providers, with the track on the right edge.
    assert!(
        shown
            .iter()
            .any(|row| row.contains("ALLOWANCE LEFT  5 providers"))
    );
    assert!(
        shown.iter().any(|row| row.trim_end().ends_with('▲')),
        "{shown:?}"
    );
    assert!(
        shown.iter().any(|row| row.trim_end().ends_with('▼')),
        "{shown:?}"
    );
    row_of(&shown, "Claude");
    assert!(!shown.iter().any(|row| row.contains("Openai")));
    let y = shown
        .iter()
        .position(|row| row.contains("ALLOWANCE LEFT"))
        .unwrap();
    let x = shown[y].chars().position(|cell| cell == '│').unwrap();
    let inside = ratatui::layout::Position::new(x as u16 + 2, y as u16 + 1);
    for _ in 0..5 {
        assert!(app.digest_allowance.wheel(inside, true));
    }
    let scrolled = rows(&buffer(&mut app, 120, 40));
    row_of(&scrolled, "Openai");
    assert!(
        !scrolled.iter().any(|row| row.contains("Claude ")),
        "clamped to the last page: {scrolled:?}"
    );
    let outside = ratatui::layout::Position::new(0, 0);
    assert!(!app.digest_allowance.wheel(outside, false));
}

#[test]
fn an_ascii_terminal_gets_an_ascii_marker_and_rule() {
    let mut app = digest(vec![claude()]);
    app.cron_glyphs = GlyphSet::Ascii;
    let rows = rows(&buffer(&mut app, 120, 40));
    assert_eq!(
        row_of(&rows, "Claude")[2..7],
        ["x", "runs", "out", "Su", "04:40"]
    );
}

#[test]
fn no_reading_or_no_height_shows_no_allowance() {
    for (limits, height) in [(Vec::new(), 40), (vec![claude()], 12)] {
        let mut app = digest(limits);
        let rows = rows(&buffer(&mut app, 120, height));
        assert!(
            !rows.iter().any(|row| row.contains("ALLOWANCE")),
            "{height}: {}",
            rows.join("\n")
        );
    }
}

#[test]
fn a_narrow_frame_stacks_the_allowance_under_the_amounts() {
    let steady = standing(
        "openai",
        55.0,
        100,
        60_000,
        Rates::measured(Some(0.3), Some(0.3), None, Some(0.3)),
    );
    for (width, height) in [(80, 40), (60, 36)] {
        let mut app = digest(vec![claude(), steady.clone()]);
        let rows = rows(&buffer(&mut app, width, height));
        let heading = rows
            .iter()
            .position(|row| row.contains("ALLOWANCE LEFT  2 providers"))
            .unwrap_or_else(|| panic!("{width}: {}", rows.join("\n")));
        let amounts = rows
            .iter()
            .position(|row| row.contains("TOK USED"))
            .unwrap();
        assert_eq!(
            heading,
            amounts + 3,
            "{width}: right under the coverage lines"
        );
        assert!(!rows[heading].contains('│'), "no tile rule when stacked");
        assert_eq!(
            words(&rows[heading + 1]),
            [
                "Claude", "26%", "✗", "runs", "out", "Su", "04:40", "resets", "Su", "18:40"
            ],
            "{width}"
        );
        assert_eq!(
            words(&rows[heading + 2])[..5],
            ["Openai", "55%", "lasts", "past", "reset"]
        );
        assert!(
            rows[heading + 3].starts_with("○ Saved = context"),
            "{width}"
        );
    }
    // Rows the table cannot spare are clipped and scroll inside the stacked block.
    let mut app = digest(vec![claude(), steady]);
    let rows = rows(&buffer(&mut app, 80, 22));
    let heading = rows
        .iter()
        .position(|row| row.contains("ALLOWANCE LEFT"))
        .unwrap();
    assert!(rows[heading + 1].starts_with("  Claude"), "{rows:?}");
    assert!(rows[heading + 2].starts_with("○ Saved"), "{rows:?}");
}

/// A claude window at 20 % left, 40 h to the reset, that lost 0.5 points an hour over the last
/// day (12 points), with one member "a" on claude spending `cells` tokens an hour-cell.
fn claude_with_tokens(cells: [u64; 24], complete: [bool; 24], ends_ago_ms: u64) -> App {
    let mut app = digest(vec![standing(
        "claude",
        20.0,
        40,
        10 * 60_000,
        Rates::measured(Some(0.5), Some(0.5), Some(0.5), Some(0.5)),
    )]);
    crate::board::app::workspace::tests::install_hourly_tokens(
        &mut app,
        NOW,
        ends_ago_ms,
        complete,
        &[("a", "claude", cells)],
    );
    app
}

fn claude_row(app: &mut App) -> Vec<String> {
    row_of(&rows(&buffer(app, 120, 40)), "Claude")
}

/// The claude row above (20 % left, 40 h to the reset, 0.5 points an hour on every sampled line,
/// so each empties it at Su 18:40) when the worst projection runs out at `at`.
fn row_running_out(at: &str) -> Vec<String> {
    let (day, time) = at.split_once(' ').unwrap();
    [
        "Claude", "20%", "✗", "runs", "out", day, time, "resets", "Su", "18:40",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn the_tokens_line_projects_the_last_three_hours_at_the_cost_of_the_last_day() {
    // 12 points (0.5 an hour for 24 h) over 27 000 tokens, of which 6 000 in the last three
    // cells: 2 000 tokens an hour at 12 / 27 000 points a token is 0.89 points an hour, which
    // empties the 20 % in 22.5 h, at Su 01:10.
    let mut cells = [1_000u64; 24];
    cells[21..].copy_from_slice(&[1_000, 2_000, 3_000]);
    let mut app = claude_with_tokens(cells, [true; 24], 0);
    assert_eq!(claude_row(&mut app), row_running_out("Su 01:10"));
}

#[test]
fn the_tokens_line_is_unknown_without_a_usable_report_or_complete_cells() {
    // No report yet.
    let mut app = digest(vec![standing(
        "claude",
        20.0,
        40,
        10 * 60_000,
        Rates::measured(Some(0.5), Some(0.5), Some(0.5), Some(0.5)),
    )]);
    assert_eq!(claude_row(&mut app), row_running_out("Su 18:40"));
    let mut incomplete = [true; 24];
    incomplete[5] = false;
    for (label, complete, ends_ago) in [
        ("an incomplete cell", incomplete, 0),
        ("a stale series", [true; 24], 3 * HOUR),
    ] {
        let mut app = claude_with_tokens([1_000; 24], complete, ends_ago);
        assert_eq!(claude_row(&mut app), row_running_out("Su 18:40"), "{label}");
    }
}
