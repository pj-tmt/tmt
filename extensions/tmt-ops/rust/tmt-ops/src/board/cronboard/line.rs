//! The home tab's cron line and the text shared with job rows. Pure: it formats
//! the projection it is given and never reads core, the store or the clock.

use super::State;
#[cfg(test)]
use crate::look::Look;
use crate::{board::notes::sanitize, board::view::fit, cron_service::JobView};
#[cfg(test)]
use ratatui::text::{Line, Span};
use tmt_cli_style::Role;
use tmt_ops::cron::ClockStatus;
use unicode_width::UnicodeWidthStr;

/// One display line of untrusted text with terminal controls neutralized.
pub(super) fn first_line(text: &str) -> String {
    sanitize(text).lines().next().unwrap_or("").to_owned()
}

/// What the clock text needs beyond the status: whether the first read is still
/// settling, and where the holder's pane is, when a loaded member row says.
#[derive(Clone, Copy, Default)]
pub(in crate::board) struct ClockNote<'a> {
    pub checking: bool,
    pub place: Option<&'a str>,
}

/// The clock segment; `short` drops the hint and the lease age.
pub(in crate::board) fn clock(
    status: &ClockStatus,
    now_ms: i64,
    short: bool,
    note: ClockNote<'_>,
) -> (String, Role) {
    match status {
        // The board's own clock takes the lease just after the first read.
        ClockStatus::NoClock if note.checking => ("clock: checking…".into(), Role::Dim),
        ClockStatus::Running(holder) => {
            let place = note
                .place
                .map(str::to_owned)
                .or_else(|| holder.pane.clone())
                .unwrap_or_else(|| format!("pid {}", holder.pid));
            let text = if short {
                format!("clock: {place}")
            } else {
                let held = u64::try_from(now_ms.saturating_sub(holder.since_ms)).unwrap_or(0);
                format!(
                    "clock: {place} · {}",
                    tmt_cli_style::value::relative_time(held)
                )
            };
            (first_line(&text), Role::Dim)
        }
        ClockStatus::NoClock if short => ("no clock".into(), Role::Blocked),
        ClockStatus::NoClock => ("no clock · tmt ops sq cron run".into(), Role::Blocked),
        ClockStatus::Unknown => ("clock: unknown".into(), Role::Waiting),
    }
}

/// A future instant in the schedule's stored zone: the time today, else with its date.
pub(in crate::board) fn time(slot_ms: i64, now_ms: i64, zone: &str) -> Option<String> {
    let zone = jiff::tz::TimeZone::get(zone).ok()?;
    let slot = jiff::Timestamp::from_millisecond(slot_ms)
        .ok()?
        .to_zoned(zone.clone());
    let now = jiff::Timestamp::from_millisecond(now_ms)
        .ok()?
        .to_zoned(zone);
    Some(
        slot.strftime(if slot.date() == now.date() {
            "%H:%M"
        } else {
            "%a %m-%d %H:%M"
        })
        .to_string(),
    )
}

/// The member-row form: the time today, else weekday and time within the next
/// week, else month-day and time. It is short because the row-end label drops
/// before any column does.
pub(in crate::board) fn short_time(slot_ms: i64, now_ms: i64, zone: &str) -> Option<String> {
    let zone = jiff::tz::TimeZone::get(zone).ok()?;
    let slot = jiff::Timestamp::from_millisecond(slot_ms)
        .ok()?
        .to_zoned(zone.clone());
    let now = jiff::Timestamp::from_millisecond(now_ms)
        .ok()?
        .to_zoned(zone);
    let days = slot
        .date()
        .since(jiff::civil::DateDifference::new(now.date()).largest(jiff::Unit::Day))
        .ok()?
        .get_days();
    Some(
        slot.strftime(match days {
            ..1 => "%H:%M",
            1..7 => "%a %H:%M",
            _ => "%m-%d %H:%M",
        })
        .to_string(),
    )
}

pub(in crate::board) fn zone(view: &JobView) -> String {
    view.job.schedule.document()["zone"]
        .as_str()
        .unwrap_or("UTC")
        .to_owned()
}

pub(super) fn next_time(view: &JobView, now_ms: i64) -> Option<String> {
    time(*view.next_ms.first()?, now_ms, &zone(view))
}

/// A piece of display text and the role it is drawn in.
pub(in crate::board) type Piece = (String, Role);

/// Home keeps count, next time, owner, clock state and the list key; job text
/// and clock locations remain behind `c`. A stale/failure projection stays explicit.
/// The pieces add up to at most `width` cells; the painter owns their styles.
pub(in crate::board) fn pieces(state: &State, now_ms: i64, width: u16) -> Option<Vec<Piece>> {
    let width = usize::from(width);
    let Some(cron) = &state.cron else {
        let text = format!(
            "cron · ✗ jobs unavailable: {}",
            first_line(state.failure.as_deref()?)
        );
        return Some(vec![(fit(&text, width.min(text.width())), Role::Blocked)]);
    };
    let (clock, clock_role) = match &cron.clock {
        ClockStatus::Running(_) => ("clock on", Role::Dim),
        ClockStatus::NoClock if state.reads <= 1 => ("clock checking", Role::Dim),
        ClockStatus::NoClock => ("clock off", Role::Blocked),
        ClockStatus::Unknown => ("clock checking", Role::Waiting),
    };
    let count = format!(
        "cron · {} job{}",
        cron.jobs.len(),
        if cron.jobs.len() == 1 { "" } else { "s" }
    );
    let tail = format!(" · {clock} · c list");
    let width_of =
        |pieces: &[Piece]| -> usize { pieces.iter().map(|(text, _)| text.width()).sum() };
    let mut pieces = vec![(count.clone(), Role::Text)];
    if state.failure.is_some() {
        pieces.push((" · ! stale".into(), Role::Waiting));
    }
    if let Some((_, view)) = cron.next()
        && let Some(time) = next_time(view, now_ms)
    {
        let next = format!(" · next {time}");
        if width_of(&pieces) + next.width() + tail.width() <= width {
            pieces.push((next, Role::Text));
            let owner = first_line(view.owner_name.as_deref().unwrap_or("no owner"));
            let room = width.saturating_sub(width_of(&pieces) + tail.width() + 1);
            if room > 0 {
                pieces.push((
                    format!(" {}", fit(&owner, room.min(owner.width()))),
                    Role::Text,
                ));
            }
        }
    }
    pieces.extend([
        (" · ".into(), Role::Dim),
        (clock.into(), clock_role),
        (" · c list".into(), Role::Muted),
    ]);
    if width_of(&pieces) <= width {
        Some(pieces)
    } else {
        Some(vec![(fit(&format!("{count}{tail}"), width), clock_role)])
    }
}

/// The same pieces as a styled line, for the text the tests compare.
#[cfg(test)]
pub(in crate::board) fn line(
    state: &State,
    now_ms: i64,
    width: u16,
    look: Look,
) -> Option<Line<'static>> {
    Some(Line::from(
        pieces(state, now_ms, width)?
            .into_iter()
            .map(|(text, role)| Span::styled(text, look.role(role)))
            .collect::<Vec<_>>(),
    ))
}

#[cfg(test)]
pub(super) mod tests;
