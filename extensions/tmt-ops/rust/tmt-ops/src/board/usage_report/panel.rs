//! The panel's rows: a pure projection of the held report, wording and layout
//! as tmt-ux-lead specified them. Painting only places these rows.

use super::model::{Line, Report, Seat, Since, Unavailable, Who};
use crate::board::spark_row::{glyph, scale};
use tmt_cli_style::Role;

const NAME_MIN: usize = 12;
const NAME_MAX: usize = 20;
const NUMBER: usize = 8;
const KEY: usize = 18;
const GUTTER: usize = 1;
const TOP_SEATS: usize = 8;
const TOP_EXCHANGE: usize = 5;
const WARNINGS: usize = 3;
const HOUR_MS: u64 = 3_600_000;
const DAY_MS: u64 = 24 * HOUR_MS;

/// One piece of a row with its role.
pub type Piece = (String, Role);

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// The section rule with its window, time and note.
    Rule(String),
    /// The window chips; painting adds the hits.
    Chips(Since),
    Line(Vec<Piece>),
}

impl Row {
    fn muted(text: impl Into<String>) -> Self {
        Self::Line(vec![(text.into(), Role::Muted)])
    }
}

/// What the board holds for the selected window.
pub enum View<'a> {
    Loading,
    Failed(&'a str),
    Report(&'a Report),
}

/// `text` cut to `width` cells, never padded.
fn clip(text: &str, width: usize) -> String {
    crate::board::view::fit(text, width).trim_end().to_owned()
}

fn cells(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

fn pad(text: &str, width: usize) -> String {
    let cut = clip(text, width);
    let missing = width.saturating_sub(cells(&cut));
    format!("{cut}{}", " ".repeat(missing))
}

fn right(text: &str, width: usize) -> String {
    let cut = clip(text, width);
    let missing = width.saturating_sub(cells(&cut));
    format!("{}{cut}", " ".repeat(missing))
}

fn tokens(value: u64) -> String {
    crate::source::tokens(u128::from(value))
}

/// Hours up to two days, then days and hours.
pub fn duration(ms: u64) -> String {
    if ms <= 2 * DAY_MS {
        return match ms / HOUR_MS {
            0 => format!("{}m", ms / 60_000),
            hours => format!("{hours}h"),
        };
    }
    match (ms / DAY_MS, (ms % DAY_MS) / HOUR_MS) {
        (days, 0) => format!("{days}d"),
        (days, hours) => format!("{days}d {hours}h"),
    }
}

fn window_label(minutes: u64) -> String {
    if minutes.is_multiple_of(1440) {
        format!("{}d", minutes / 1440)
    } else if minutes.is_multiple_of(60) {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

fn zoned(ms: u64, zone: &jiff::tz::TimeZone) -> Option<jiff::Zoned> {
    i64::try_from(ms)
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map(|time| time.to_zoned(zone.clone()))
}

fn clock(ms: u64, zone: &jiff::tz::TimeZone) -> String {
    zoned(ms, zone).map_or_else(
        || "unknown".into(),
        |time| time.strftime("%H:%M").to_string(),
    )
}

/// A time of day, with its date unless it is on the day of `now_ms`.
fn moment(ms: u64, now_ms: u64, zone: &jiff::tz::TimeZone) -> String {
    match (zoned(ms, zone), zoned(now_ms, zone)) {
        (Some(at), Some(now)) if at.date() == now.date() => at.strftime("%H:%M").to_string(),
        (Some(at), _) => at.strftime("%m-%d %H:%M").to_string(),
        _ => "unknown".into(),
    }
}

/// The name of a participant: its name, else the start of its UUID (muted).
fn who(who: &Who) -> (String, Role) {
    match (&who.name, &who.id) {
        (Some(name), _) => (name.clone(), Role::Text),
        (None, Some(id)) => (id.chars().take(8).collect(), Role::Muted),
        (None, None) => ("unknown".into(), Role::Muted),
    }
}

fn spark(values: Option<&[u64]>, buckets: usize) -> String {
    let Some(values) = values.filter(|values| values.len() == buckets) else {
        return glyph(None, 1).repeat(buckets);
    };
    let scale = scale(values.iter().map(|value| Some(*value))).unwrap_or(0);
    values
        .iter()
        .map(|value| glyph(Some(*value), scale))
        .collect()
}

/// One bucket-aligned series per line: the sum of the series of the seats whose
/// tokens belong to that line alone. A seat spanning lines is not split here.
fn line_series(report: &Report, line: &Line, buckets: usize) -> Option<Vec<u64>> {
    let mut sum = vec![0u64; buckets];
    let mut any = false;
    for seat in &report.seats {
        if seat.lines.len() != 1 || seat.lines[0] != line.name || seat.series.len() != buckets {
            continue;
        }
        any = true;
        for (slot, value) in sum.iter_mut().zip(&seat.series) {
            *slot = slot.saturating_add(*value);
        }
    }
    any.then_some(sum)
}

fn buckets(report: &Report) -> usize {
    report
        .seats
        .iter()
        .map(|seat| seat.series.len())
        .max()
        .unwrap_or(0)
}

/// Whether the spark column and the closing text fit beside the numbers.
struct Columns {
    name: usize,
    spark: usize,
    text: bool,
}

/// The text beside the numbers is evidence (partial coverage), the spark is
/// shape: when both do not fit, the spark goes first.
fn columns(width: usize, name: usize, buckets: usize, closing: usize) -> Columns {
    let base = name + 4 * (NUMBER + 1);
    let spark = buckets > 0;
    let both = width >= base + 1 + buckets + 2 + closing;
    let text = both || width >= base + 2 + closing;
    Columns {
        name,
        spark: if both || (spark && !text && width >= base + 1 + buckets) {
            buckets
        } else {
            0
        },
        text,
    }
}

fn header(columns: &Columns, last: &str) -> Row {
    let spark = if columns.spark > 0 {
        " ".repeat(1 + columns.spark)
    } else {
        String::new()
    };
    let mut text = format!(
        "{}{spark}{}{}{}{}",
        " ".repeat(columns.name),
        right("IN", NUMBER + 1),
        right("OUT", NUMBER + 1),
        right("CACHED", NUMBER + 1),
        right("TOTAL", NUMBER + 1),
    );
    if columns.text {
        text.push_str(&format!("  {last}"));
    }
    Row::muted(text)
}

fn numbers(input: u64, output: u64, cached: u64) -> String {
    let total = input.saturating_add(output);
    [input, output, cached, total]
        .into_iter()
        .map(|value| format!(" {}", right(&tokens(value), NUMBER)))
        .collect()
}

fn line_row(report: &Report, line: &Line, columns: &Columns, buckets: usize) -> Row {
    let mut pieces: Vec<Piece> = vec![(pad(&line.name, columns.name), Role::Text)];
    if columns.spark > 0 {
        let series = line_series(report, line, buckets);
        pieces.push((
            format!(" {}", spark(series.as_deref(), buckets)),
            Role::Muted,
        ));
    }
    pieces.push((numbers(line.input, line.output, line.cached), Role::Text));
    if columns.text {
        let mut seats = format!("  {}", line.seats);
        if line.partial_seats > 0 {
            seats.push_str(&format!(" · {} partial", line.partial_seats));
        }
        pieces.push((seats, Role::Muted));
    }
    Row::Line(pieces)
}

/// Partial coverage in words: where the evidence began when that is inside the
/// window, else how much of the window it covers.
fn partial(report: &Report, seat: &Seat, zone: &jiff::tz::TimeZone) -> String {
    match (seat.starts_at_ms, report.window_from_ms) {
        (Some(start), Some(from)) if start > from => {
            format!(
                "partial · since {}",
                moment(start, report.observed_at_ms, zone)
            )
        }
        _ => format!(
            "partial · {} of {}",
            duration(seat.covered_ms),
            duration(seat.window_ms)
        ),
    }
}

fn seat_row(seat: &Seat, columns: &Columns, buckets: usize, partial: &str) -> Row {
    let (name, role) = who(&seat.who);
    let retired = if seat.who.retired { " retired" } else { "" };
    let name = clip(&name, columns.name.saturating_sub(retired.len()));
    let used = cells(&name) + retired.len();
    let mut pieces: Vec<Piece> = vec![(name, role)];
    if !retired.is_empty() {
        pieces.push((retired.into(), Role::Muted));
    }
    pieces.push((" ".repeat(columns.name.saturating_sub(used)), Role::Text));
    if columns.spark > 0 {
        pieces.push((
            format!(" {}", spark(Some(&seat.series), buckets)),
            Role::Muted,
        ));
    }
    pieces.push((numbers(seat.input, seat.output, seat.cached), Role::Text));
    if columns.text && !seat.complete {
        pieces.push((format!("  {partial}"), Role::Muted));
    }
    Row::Line(pieces)
}

fn limit_rows(report: &Report) -> Vec<Row> {
    let line_of = |seats: &[String]| -> String {
        let mut lines: Vec<&str> = seats
            .iter()
            .filter_map(|id| {
                report
                    .seats
                    .iter()
                    .find(|seat| seat.who.id.as_deref() == Some(id))
            })
            .filter(|seat| seat.lines.len() == 1 && seat.lines[0] != "unattributed")
            .map(|seat| seat.lines[0].as_str())
            .collect();
        lines.sort_unstable();
        lines.dedup();
        match lines.as_slice() {
            [line] => (*line).into(),
            _ => "account".into(),
        }
    };
    let mut limits: Vec<_> = report
        .limits
        .iter()
        .map(|limit| {
            let line = line_of(&limit.seats);
            let key = format!("{line} · {}", window_label(limit.window_minutes));
            let used = format!("{:.0}% used", limit.used_percent);
            let value = match (limit.stale, limit.elapsed_percent, limit.pace_delta_points) {
                (true, ..) => format!("{used} · window reset, not observed since"),
                (false, Some(elapsed), Some(pace)) => {
                    let pace = if pace < -0.5 {
                        format!("{:.0} pts under pace", pace.abs())
                    } else if pace > 0.5 {
                        format!("{pace:.0} pts over pace")
                    } else {
                        "on pace".into()
                    };
                    format!("{used} · {elapsed:.0}% of window gone · {pace}")
                }
                _ => used,
            };
            (
                (line, limit.window_minutes, limit.stale),
                Row::Line(vec![(pad(&key, KEY), Role::Muted), (value, Role::Text)]),
            )
        })
        .collect();
    // A live window comes before the stale one of the same line and length.
    limits.sort_by(|a, b| a.0.cmp(&b.0));
    let mut rows: Vec<Row> = limits.into_iter().map(|(_, row)| row).collect();
    rows.extend(report.limits_unavailable.iter().map(|row| {
        Row::muted(match row.reason.as_str() {
            "no_source" => format!("{} · no limit source yet", row.line),
            "not_observed" => format!("{} · no limit observed", row.line),
            reason => format!("{} · limits unavailable ({reason})", row.line),
        })
    }));
    rows
}

fn unavailable_row(row: &Unavailable) -> Row {
    let (name, _) = who(&row.who);
    match row.reason.as_str() {
        "no_evidence" => Row::muted(format!("{name} · no usage recorded in this window")),
        "not_found" => Row::muted(format!("{name} · not found")),
        "read_failed" => Row::Line(vec![
            ("✗ ".into(), Role::Blocked),
            (format!("{name} · could not be read"), Role::Muted),
        ]),
        reason => Row::muted(format!("{name} · unavailable ({reason})")),
    }
}

fn top(label: &str, tallies: &[super::model::Tally]) -> Row {
    let items: Vec<_> = tallies
        .iter()
        .take(TOP_EXCHANGE)
        .map(|tally| format!("{} {}", who(&tally.who).0, tally.count))
        .collect();
    Row::Line(vec![
        (pad(label, KEY), Role::Muted),
        (
            if items.is_empty() {
                "none".into()
            } else {
                items.join(" · ")
            },
            Role::Text,
        ),
    ])
}

fn rule(report: Option<&Report>, since: Since, width: usize, zone: &jiff::tz::TimeZone) -> Row {
    let note = report.map_or_else(
        || since.text().to_owned(),
        |report| {
            format!(
                "{} · as of {} · newest 5m not included",
                since.text(),
                clock(report.observed_at_ms, zone)
            )
        },
    );
    // "── Usage report " + fill + " " + note + " ──", the fill at least two cells.
    let head = "── Usage report ";
    let tail = " ──";
    let fixed = cells(head) + 1 + cells(tail);
    let note = clip(&note, width.saturating_sub(fixed + 2));
    let fill = width.saturating_sub(fixed + cells(&note)).max(2);
    Row::Rule(format!("{head}{} {note}{tail}", "─".repeat(fill)))
}

/// The width of the name column: the longest displayed name, within bounds.
fn name_width(report: &Report, width: usize) -> usize {
    let longest = report
        .lines
        .iter()
        .map(|line| cells(&line.name))
        .chain(report.seats.iter().map(|seat| {
            cells(&who(&seat.who).0)
                + if seat.who.retired {
                    " retired".len()
                } else {
                    0
                }
        }))
        .max()
        .unwrap_or(0);
    longest
        .clamp(NAME_MIN, NAME_MAX)
        .min((width / 3).max(NAME_MIN))
}

/// Every row the panel could show at this width, top to bottom, none wider.
pub fn rows(view: &View<'_>, since: Since, width: u16, zone: &jiff::tz::TimeZone) -> Vec<Row> {
    let mut rows = layout(view, since, width, zone);
    for row in &mut rows {
        match row {
            Row::Rule(text) if cells(text) > usize::from(width) => {
                *text = clip(text, usize::from(width));
            }
            Row::Rule(_) => {}
            Row::Chips(_) => {}
            Row::Line(pieces) => {
                pieces.insert(0, (" ".repeat(GUTTER), Role::Text));
                let mut room = usize::from(width);
                pieces.retain_mut(|(text, _)| {
                    let cut = if cells(text) <= room {
                        text.clone()
                    } else {
                        clip(text, room)
                    };
                    room -= cells(&cut).min(room);
                    let keep = !cut.is_empty();
                    *text = cut;
                    keep
                });
            }
        }
    }
    rows
}

fn layout(view: &View<'_>, since: Since, width: u16, zone: &jiff::tz::TimeZone) -> Vec<Row> {
    let full = usize::from(width);
    // Every row but the rule starts one cell in, as the rest of the board does.
    let width = full.saturating_sub(GUTTER);
    let report = match view {
        View::Report(report) => Some(*report),
        _ => None,
    };
    let mut rows = vec![rule(report, since, full, zone), Row::Chips(since)];
    let report = match view {
        View::Loading => {
            rows.push(Row::muted("Loading usage report…"));
            return rows;
        }
        View::Failed(message) => {
            rows.push(Row::muted(format!("Usage report unavailable · {message}")));
            return rows;
        }
        View::Report(report) => report,
    };
    if report.lines.is_empty() && report.seats.is_empty() && report.unavailable.is_empty() {
        rows.push(Row::muted("No usage recorded in this window."));
    }
    let buckets = buckets(report);
    let name = name_width(report, width);
    if !report.lines.is_empty() {
        let layout = columns(width, name, buckets, "6 · 1 partial".len());
        rows.push(header(&layout, "SEATS"));
        rows.extend(
            report
                .lines
                .iter()
                .map(|line| line_row(report, line, &layout, buckets)),
        );
    }
    rows.extend(limit_rows(report));
    if !report.seats.is_empty() {
        let layout = columns(width, name, buckets, "partial · since 10-16 21:20".len());
        rows.push(header(&layout, "COVERAGE"));
        let mut seats: Vec<&Seat> = report.seats.iter().collect();
        seats.sort_by_key(|seat| std::cmp::Reverse(seat.total()));
        rows.extend(
            seats
                .iter()
                .take(TOP_SEATS)
                .map(|seat| seat_row(seat, &layout, buckets, &partial(report, seat, zone))),
        );
        if seats.len() > TOP_SEATS {
            rows.push(Row::muted(format!(
                "+{} more · tmt digest usage report --csv usage",
                seats.len() - TOP_SEATS
            )));
        }
    }
    rows.extend(report.unavailable.iter().map(unavailable_row));
    match &report.exchange {
        Some(exchange) => {
            rows.push(top("Top senders", &exchange.senders));
            rows.push(top("Top recipients", &exchange.recipients));
        }
        None => rows.push(Row::muted("Exchange flow unavailable for this window.")),
    }
    for warning in report.warnings.iter().take(WARNINGS) {
        rows.push(Row::muted(format!(
            "warning · {} · {}",
            warning.section, warning.message
        )));
    }
    if report.warnings.len() > WARNINGS {
        rows.push(Row::muted(format!(
            "+{} more warnings",
            report.warnings.len() - WARNINGS
        )));
    }
    rows
}
