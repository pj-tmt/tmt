//! The home tab's ⑤ cron line and the text shared with job rows. Pure: it formats
//! the projection it is given and never reads core, the store or the clock.

use super::{Cron, State};
use crate::{board::notes::sanitize, board::view::fit, cron_service::JobView, look::Look};
use ratatui::text::{Line, Span};
use tmt_cli_style::Role;
use tmt_squad::cron::ClockStatus;
use unicode_width::UnicodeWidthStr;

/// One display line of untrusted text with terminal controls neutralized.
pub(super) fn first_line(text: &str) -> String {
    sanitize(text).lines().next().unwrap_or("").to_owned()
}

/// The clock segment; `short` drops the hint and the lease age.
pub(super) fn clock(status: &ClockStatus, now_ms: i64, short: bool) -> (String, Role) {
    match status {
        ClockStatus::Running(holder) => {
            let place = holder
                .pane
                .clone()
                .unwrap_or_else(|| format!("pid {}", holder.pid));
            let text = if short {
                format!("clock: {place}")
            } else {
                let held = u64::try_from(now_ms.saturating_sub(holder.since_ms)).unwrap_or(0);
                format!("clock: {place} · {}", crate::requests::age(held, 0))
            };
            (first_line(&text), Role::Dim)
        }
        ClockStatus::NoClock if short => ("no clock".into(), Role::Blocked),
        ClockStatus::NoClock => ("no clock · tmt sq cron run".into(), Role::Blocked),
        ClockStatus::Unknown => ("clock: unknown".into(), Role::Waiting),
    }
}

/// A future instant in the schedule's stored zone: the time today, else with its date.
pub(super) fn time(slot_ms: i64, now_ms: i64, zone: &str) -> Option<String> {
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

pub(super) fn zone(view: &JobView) -> String {
    view.job.schedule.document()["zone"]
        .as_str()
        .unwrap_or("UTC")
        .to_owned()
}

pub(super) fn next_time(view: &JobView, now_ms: i64) -> Option<String> {
    time(*view.next_ms.first()?, now_ms, &zone(view))
}

/// `⑤ ⏱ N cron jobs · next <time> <owner> <what> · <clock> · c list`. The preview
/// steps aside first, then the owner; count, time, clock and the key stay.
pub(in crate::board) fn line(
    state: &State,
    now_ms: i64,
    width: u16,
    look: Look,
) -> Option<Line<'static>> {
    let width = usize::from(width);
    let Some(cron) = &state.cron else {
        let reason = first_line(state.failure.as_deref()?);
        let text = format!("⑤ ⏱ ✗ cron jobs unavailable: {reason}");
        return Some(Line::styled(
            fit(&text, width.min(text.width())),
            look.role(Role::Blocked),
        ));
    };
    Some(summary(cron, state.failure.is_some(), now_ms, width, look))
}

fn summary(cron: &Cron, stale: bool, now_ms: i64, width: usize, look: Look) -> Line<'static> {
    let count = format!(
        "{} {}",
        cron.jobs.len(),
        if width >= 100 { "cron jobs" } else { "jobs" }
    );
    let mut parts = vec![
        ("⑤ ".to_owned(), Role::Dim),
        ("⏱ ".to_owned(), Role::Accent),
        (count, Role::Text),
    ];
    if stale {
        parts.push((" ! stale".into(), Role::Waiting));
    }
    let next = cron.next();
    if let Some((_, view)) = next
        && let Some(time) = next_time(view, now_ms)
    {
        parts.push((" · next ".into(), Role::Dim));
        parts.push((time, Role::Text));
    }
    let tail = " · c list";
    let used = |parts: &[(String, Role)]| parts.iter().map(|(text, _)| text.width()).sum::<usize>();
    let (mut clock, clock_role) = clock(&cron.clock, now_ms, false);
    let sep = " · ";
    let room = width.saturating_sub(used(&parts) + sep.width() + tail.width());
    if clock.width() > room {
        clock = clock_text_for(&cron.clock, now_ms, room);
    }
    if let Some((_, view)) = next
        && let Some(_) = next_time(view, now_ms)
    {
        let owner = first_line(view.owner_name.as_deref().unwrap_or("no owner"));
        let room = width.saturating_sub(used(&parts) + sep.width() + clock.width() + tail.width());
        if owner.width() < room {
            parts.push((format!(" {owner}"), Role::Text));
            let what = first_line(&view.job.message);
            let room = room.saturating_sub(owner.width() + 1);
            if !what.is_empty() && room >= 5 {
                parts.push((
                    format!(" {}", fit(&what, (room - 1).min(what.width()))),
                    Role::Muted,
                ));
            }
        }
    }
    parts.push((sep.into(), Role::Dim));
    parts.push((clock, clock_role));
    parts.push((" · ".into(), Role::Dim));
    parts.push(("c list".into(), Role::Muted));
    if used(&parts) > width {
        // Too narrow for the structure: count and clock, fitted as one run.
        let compact = format!(
            "⑤ ⏱ {} · {}",
            cron.jobs.len(),
            clock_text_for(&cron.clock, now_ms, width)
        );
        return Line::styled(
            fit(&compact, width.min(compact.width())),
            look.role(clock_role),
        );
    }
    Line::from(
        parts
            .into_iter()
            .map(|(text, role)| Span::styled(text, look.role(role)))
            .collect::<Vec<_>>(),
    )
}

/// The longest clock wording that fits `room`, ending in an ellipsis when cut.
fn clock_text_for(status: &ClockStatus, now_ms: i64, room: usize) -> String {
    let (full, _) = clock(status, now_ms, false);
    if full.width() <= room {
        return full;
    }
    let (short, _) = clock(status, now_ms, true);
    fit(&short, room.min(short.width()))
}

#[cfg(test)]
pub(super) mod tests;
