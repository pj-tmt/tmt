//! The Allowance block of the Digest tab: each provider's weekly window, the
//! even burn that lasts until it refills and what each projection line leaves at
//! the reset. Numbers come from `limits::allowance`; this file only places them.
use crate::board::{
    app::App,
    limits::{Line as ProjectionLine, Need, Outcome, Provider, Row, Spent, Weekly},
    overview::cron_counts::GlyphSet,
    section_rule,
    usage_report::{View, model::Seat},
};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use std::collections::BTreeMap;
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

/// Rows the table below keeps when the block takes its share.
const BODY_MINIMUM: u16 = 10;
const SOURCE: usize = 8;
const LEFT: usize = 5;
const NEED: usize = 8;
const CELL: usize = 10;
const SINCE: usize = 11;
const RESETS: usize = 8;
const GAP: usize = 2;
/// All columns and their gaps, plus a one-cell margin each side.
const WIDE: u16 = (SOURCE + LEFT + NEED + CELL * 4 + SINCE + RESETS + GAP * 8 + 2) as u16;

/// Tokens each provider's members spent over the last 3 h and 24 h, from the usage report the
/// Digest tab already holds: one 24-cell hourly series per seat, a seat counted for the driver
/// its member is on now. Nothing is read for this; a missing or stale report, a seat with no
/// known driver or an incomplete cell leaves the provider out, never as zero.
fn spent(app: &App, now: u64) -> BTreeMap<Provider, Spent> {
    let View::Report(report) = app.usage_report.view_for(app.tok_range()) else {
        return BTreeMap::new();
    };
    let Some(view) = app.view.as_ref() else {
        return BTreeMap::new();
    };
    let drivers: BTreeMap<&str, &str> = view
        .home_rate
        .iter()
        .filter(|(_, rate)| rate.settings.enabled)
        .filter_map(|(name, _)| app.home_meter(name))
        .flat_map(|meter| {
            let rate = meter.history();
            report
                .seats
                .iter()
                .filter_map(|seat| seat.who.id.as_deref())
                .filter_map(|id| Some((id, rate.current_driver_of(id)?)))
                .collect::<Vec<_>>()
        })
        .collect();
    // A provider with any seat lacking a usable series has no honest total.
    let mut totals: BTreeMap<&str, Option<Spent>> = BTreeMap::new();
    for seat in &report.seats {
        let Some(driver) = seat.who.id.as_deref().and_then(|id| drivers.get(id)) else {
            continue;
        };
        let entry = totals
            .entry(driver)
            .or_insert(Some(Spent { last_3h: 0, day: 0 }));
        *entry = entry.and_then(|total| seat_spent(seat, now).map(|spent| total.plus(spent)));
    }
    totals
        .into_iter()
        .filter_map(|(driver, spent)| Some((Provider::from_driver(driver)?, spent?)))
        .collect()
}

/// One seat's tokens over the last 3 h and 24 h, only from a fresh series of complete cells.
fn seat_spent(seat: &Seat, now: u64) -> Option<Spent> {
    const HOUR: u64 = 3_600_000;
    let ends = seat.series.from_ms.checked_add(24 * HOUR)?;
    if seat.series.is_empty() || now.saturating_sub(ends) >= 2 * HOUR {
        return None;
    }
    let points: Vec<_> = seat.series.iter().collect();
    if points.len() != 24 || points.iter().any(|point| !point.complete) {
        return None;
    }
    let sum = |from: usize| points[from..].iter().map(|point| point.tokens).sum::<u64>();
    Some(Spent {
        last_3h: sum(21),
        day: sum(0),
    })
}

fn rows(app: &App, now: u64) -> Vec<Row> {
    let spent = spent(app, now);
    app.limits.as_deref().map_or_else(Vec::new, |standings| {
        crate::board::limits::rows(standings, now, &|provider| spent.get(provider).copied())
    })
}

fn wide(width: u16) -> bool {
    width >= WIDE
}

/// The rows the block takes: its rule, a column header when wide, and one row
/// per sampled provider; nothing when there is no reading to show or the table
/// below would be left too short.
pub(super) fn height(app: &App, width: u16, available: u16) -> u16 {
    let count = rows(app, crate::status::now_ms()).len() as u16;
    if count == 0 {
        return 0;
    }
    let needed = count + 1 + u16::from(wide(width));
    if available < needed.saturating_add(BODY_MINIMUM) {
        0
    } else {
        needed
    }
}

pub(super) fn paint(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let now = crate::status::now_ms();
    let look = app.look();
    let wide = wide(area.width);
    let title = if wide {
        "Allowance · weekly windows · left at reset if this rate holds"
    } else {
        "Allowance · left at reset, worst window"
    };
    let mut lines = vec![section_rule::line(
        title,
        Line::default(),
        usize::from(area.width),
        false,
        look,
    )];
    if wide {
        lines.push(header(look));
    }
    let zone = app.local_zone.clone().unwrap_or(jiff::tz::TimeZone::UTC);
    let ascii = matches!(app.cron_glyphs, GlyphSet::Ascii);
    for row in rows(app, now) {
        lines.push(if wide {
            wide_row(&row, now, &zone, ascii, look)
        } else {
            narrow_row(&row, now, &zone, ascii, look)
        });
    }
    for (offset, line) in lines.into_iter().enumerate() {
        let y = area.y.saturating_add(offset as u16);
        if y >= area.bottom() {
            break;
        }
        tmt_tui::components::strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, y, area.width, 1),
            line,
        );
    }
}

fn pad(text: &str, width: usize, right: bool) -> String {
    let fitted = crate::board::view::fit(text, width);
    let fill = " ".repeat(width.saturating_sub(fitted.width()));
    if right {
        format!("{fill}{fitted}")
    } else {
        format!("{fitted}{fill}")
    }
}

fn header(look: crate::look::Look) -> Line<'static> {
    let mut text = String::from(" ");
    for (caption, width, right) in [
        ("SOURCE", SOURCE, false),
        ("LEFT", LEFT, true),
        ("NEED %/h", NEED, true),
        (ProjectionLine::Hours3.caption(), CELL, false),
        (ProjectionLine::Hours6.caption(), CELL, false),
        (ProjectionLine::Hours24.caption(), CELL, false),
        (ProjectionLine::Tokens.caption(), CELL, false),
        (ProjectionLine::SinceReset.caption(), SINCE, false),
        ("RESETS", RESETS, false),
    ] {
        text.push_str(&pad(caption, width, right));
        text.push_str(&" ".repeat(GAP));
    }
    Line::styled(text, look.role(Role::Muted))
}

fn clock(at_ms: u64, zone: &jiff::tz::TimeZone) -> String {
    i64::try_from(at_ms)
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map_or_else(
            || "—".into(),
            |time| {
                let zoned = time.to_zoned(zone.clone());
                // Two letters, as the weekday of a reset or an empty time reads best short.
                let day: String = zoned.strftime("%a").to_string().chars().take(2).collect();
                format!("{day} {}", zoned.strftime("%H:%M"))
            },
        )
}

/// The NEED %/h cell: capped at `>99`, an em dash in the last hour before the reset.
fn need(weekly: &Weekly, now: u64, lead: &str) -> String {
    let unit = if lead.is_empty() { "" } else { "%/h" };
    match weekly.need(now) {
        Need::Hidden => "—".into(),
        Need::Over => format!("{lead}>{}{unit}", crate::board::limits::NEED_CAP),
        Need::Rate(rate) => format!("{lead}{rate:.2}{unit}"),
    }
}

/// A projection's cell: percent left at the reset, or the time it runs out.
fn outcome(outcome: Outcome, zone: &jiff::tz::TimeZone, ascii: bool) -> (String, bool) {
    match outcome {
        Outcome::Unknown => ("—".into(), false),
        Outcome::Lasts(left) => (format!("{left:.0}%"), false),
        Outcome::RunsOut(at) => (
            format!("{} {}", if ascii { "x" } else { "✗" }, clock(at, zone)),
            true,
        ),
    }
}

fn spans(cells: Vec<(String, usize, bool, Role)>, look: crate::look::Look) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (text, width, right, role) in cells {
        spans.push(Span::styled(pad(&text, width, right), look.role(role)));
        spans.push(Span::raw(" ".repeat(GAP)));
    }
    Line::from(spans)
}

fn wide_row(
    row: &Row,
    now: u64,
    zone: &jiff::tz::TimeZone,
    ascii: bool,
    look: crate::look::Look,
) -> Line<'static> {
    let label = row.provider.label();
    let mut cells = vec![(label, SOURCE, false, Role::Text)];
    let Some(weekly) = &row.weekly else {
        cells.extend([
            ("—".into(), LEFT, true, Role::Dim),
            ("—".into(), NEED, true, Role::Dim),
        ]);
        return spans(cells, look);
    };
    cells.push((format!("{:.0}%", weekly.left), LEFT, true, Role::Text));
    cells.push((need(weekly, now, ""), NEED, true, Role::Text));
    for (index, line) in ProjectionLine::ALL.iter().enumerate() {
        let (text, runs_out) = outcome(weekly.lines[index], zone, ascii);
        let width = if *line == ProjectionLine::SinceReset {
            SINCE
        } else {
            CELL
        };
        cells.push((
            text,
            width,
            false,
            if runs_out { Role::Waiting } else { Role::Text },
        ));
    }
    cells.push((clock(weekly.resets_at_ms, zone), RESETS, false, Role::Muted));
    spans(cells, look)
}

fn narrow_row(
    row: &Row,
    now: u64,
    zone: &jiff::tz::TimeZone,
    ascii: bool,
    look: crate::look::Look,
) -> Line<'static> {
    let label = row.provider.label();
    let mut cells = vec![(label, SOURCE, false, Role::Text)];
    let Some(weekly) = &row.weekly else {
        cells.push(("—".into(), LEFT, true, Role::Dim));
        return spans(cells, look);
    };
    cells.push((format!("{:.0}%", weekly.left), LEFT, true, Role::Text));
    cells.push((
        need(weekly, now, "need "),
        "need 0.00%/h".len(),
        false,
        Role::Text,
    ));
    let (worst, runs_out) = worst(weekly, zone, ascii);
    cells.push((
        worst,
        CELL + 4,
        false,
        if runs_out { Role::Waiting } else { Role::Text },
    ));
    spans(cells, look)
}

/// The worst line of a row: the earliest empty time, else the least left.
fn worst(weekly: &Weekly, zone: &jiff::tz::TimeZone, ascii: bool) -> (String, bool) {
    match weekly.worst() {
        Outcome::Lasts(left) => (format!("lasts · {left:.0}%"), false),
        other => outcome(other, zone, ascii),
    }
}
