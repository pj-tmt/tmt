//! The allowance tile of the Digest summary: each provider's weekly window, what is left,
//! whether this rate runs it out before the reset, and when it resets. Numbers come from
//! `limits::allowance`; this file only places them.
use crate::board::{
    app::App,
    limits::{Outcome, Provider, Row, Spent},
    overview::cron_counts::GlyphSet,
    usage_report::{View, model::Seat},
};
use ratatui::{
    Frame,
    layout::{Position, Rect},
    text::{Line, Span},
};
use std::{cell::Cell, collections::BTreeMap};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

/// The summary's allowance tile: narrowest and widest, beside the amount tiles.
const TILE_MINIMUM: u16 = 44;
const TILE_MAXIMUM: u16 = 68;
const SOURCE: usize = 8;
const LEFT: usize = 5;

/// Tokens each provider's members spent over the last 3 h and 24 h, from the usage report the
/// Digest tab already holds: one 24-cell hourly series per seat, a seat counted for the driver
/// its member is on now. Nothing is read for this; a missing or stale report, a seat with no
/// known driver or an incomplete cell leaves the provider out, never as zero.
fn spent(app: &App, now: u64) -> BTreeMap<Provider, Spent> {
    let View::Report(report) = app.usage_report.view_for(app.digest_range()) else {
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
        // Tokens are counted per driver, never per account: with several accounts of one driver
        // there is no honest per-account spend, and one account reads as the plain driver.
        crate::board::limits::rows(standings, now, &|provider| {
            if provider.names_account() {
                None
            } else {
                spent.get(&provider.plain()).copied()
            }
        })
    })
}

/// Where the tile painted and how far it is scrolled. Paint records the rows and the area;
/// the wheel over that area moves the offset, clamped to the rows the tile holds.
#[derive(Default)]
pub(in crate::board) struct Scroll {
    area: Cell<Rect>,
    offset: Cell<usize>,
    hidden: Cell<usize>,
}

impl Scroll {
    /// Moves the rows when `point` is over the painted tile; false elsewhere.
    pub(in crate::board) fn wheel(&self, point: Position, down: bool) -> bool {
        if !self.area.get().contains(point) {
            return false;
        }
        let offset = self.offset.get();
        self.offset.set(if down {
            (offset + 1).min(self.hidden.get())
        } else {
            offset.saturating_sub(1)
        });
        true
    }
}

/// Providers with a sampled weekly window: the rows the allowance shows, without projecting.
fn sampled(app: &App) -> usize {
    app.limits.as_deref().map_or(0, |standings| {
        standings
            .iter()
            .filter(|standing| standing.latest.is_some())
            .count()
    })
}

/// A frame that paints no allowance leaves no wheel target behind.
pub(super) fn forget(app: &App) {
    app.digest_allowance.area.set(Rect::default());
}

/// The width the tile takes from `spare` columns: none without a sampled provider or room.
pub(super) fn tile_width(app: &App, spare: u16) -> u16 {
    if spare < TILE_MINIMUM || sampled(app) == 0 {
        forget(app);
        0
    } else {
        spare.min(TILE_MAXIMUM)
    }
}

/// The heading and provider rows wanted under the amounts when the tile has no room beside
/// them (`spare` columns); none when it fits beside or nothing is sampled.
pub(super) fn stacked_rows(app: &App, spare: u16) -> u16 {
    match sampled(app) {
        0 => 0,
        _ if spare >= TILE_MINIMUM => 0,
        rows => u16::try_from(rows + 1).unwrap_or(u16::MAX),
    }
}

/// The heading and one row per provider: what is left, when it runs out or that it lasts,
/// and when it resets. Beside the amounts a rule sets the tile off; stacked under them the
/// rows align with the amounts. More providers than rows scroll inside the area.
pub(super) fn paint_rows(frame: &mut Frame, app: &App, area: Rect, beside: bool) {
    let now = crate::status::now_ms();
    let look = app.look();
    let rows = rows(app, now);
    let zone = app.local_zone.clone().unwrap_or(jiff::tz::TimeZone::UTC);
    let ascii = matches!(app.cron_glyphs, GlyphSet::Ascii);
    let visible = usize::from(area.height.saturating_sub(1));
    let hidden = rows.len().saturating_sub(visible);
    let scroll = &app.digest_allowance;
    scroll.area.set(area);
    scroll.hidden.set(hidden);
    scroll.offset.set(scroll.offset.get().min(hidden));
    let rule = if ascii { "|" } else { "│" };
    let body = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(4),
        area.height,
    );
    for y in (area.y..area.bottom()).filter(|_| beside) {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, y, 1, 1),
            Line::styled(rule, look.role(Role::Dim)),
        );
    }
    strip::paint_left(
        frame.buffer_mut(),
        Rect { height: 1, ..body },
        Line::from(vec![
            Span::styled("ALLOWANCE LEFT  ", look.role(Role::Muted)),
            Span::styled(
                format!(
                    "{} provider{}",
                    rows.len(),
                    if rows.len() == 1 { "" } else { "s" }
                ),
                look.role(Role::Muted),
            ),
        ]),
    );
    for (index, row) in rows
        .iter()
        .skip(scroll.offset.get())
        .take(visible)
        .enumerate()
    {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(body.x, body.y + 1 + index as u16, body.width, 1),
            tile_row(row, &zone, ascii, look),
        );
    }
    if hidden > 0 {
        let track = Rect::new(area.right() - 1, area.y + 1, 1, area.height - 1);
        let (up, thumb, down) = if ascii {
            ("^", "#", "v")
        } else {
            ("▲", "█", "▼")
        };
        let span = usize::from(track.height).saturating_sub(2).max(1);
        let at = scroll.offset.get() * span.saturating_sub(1) / hidden.max(1);
        for (y, mark) in (track.y..track.bottom()).zip(0..) {
            let glyph = if mark == 0 {
                up
            } else if y + 1 == track.bottom() {
                down
            } else if mark - 1 == at {
                thumb
            } else {
                " "
            };
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(track.x, y, 1, 1),
                Line::styled(glyph, look.role(Role::Muted)),
            );
        }
    }
}

fn tile_row(
    row: &Row,
    zone: &jiff::tz::TimeZone,
    ascii: bool,
    look: crate::look::Look,
) -> Line<'static> {
    let label = pad(&row.provider.label(), SOURCE, false);
    let Some(weekly) = &row.weekly else {
        return Line::from(vec![
            Span::styled(label, look.role(Role::Text)),
            Span::styled(pad("—", LEFT, true), look.role(Role::Dim)),
            Span::styled("  no recent reading", look.role(Role::Muted)),
        ]);
    };
    let (outlook, runs_out) = match weekly.worst() {
        Outcome::RunsOut(at) => (
            format!(
                "{} runs out {}",
                if ascii { "x" } else { "✗" },
                clock(at, zone)
            ),
            true,
        ),
        Outcome::Lasts(_) => ("lasts past reset".into(), false),
        Outcome::Unknown => ("—".into(), false),
    };
    Line::from(vec![
        Span::styled(label, look.role(Role::Text)),
        Span::styled(
            pad(&format!("{:.0}%", weekly.left), LEFT, true),
            look.role(Role::Text)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{outlook:<20}"),
            look.role(if runs_out { Role::Waiting } else { Role::Text }),
        ),
        Span::styled(
            format!("resets {}", clock(weekly.resets_at_ms, zone)),
            look.role(Role::Muted),
        ),
    ])
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
