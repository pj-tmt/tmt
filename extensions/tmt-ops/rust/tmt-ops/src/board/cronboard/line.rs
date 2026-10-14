//! Text shared by current job rows and the modal clock. Pure: formats the
//! acquired projection without reading Core, the store or the clock.

use crate::{board::notes::sanitize, cron_service::JobView};
use tmt_cli_style::Role;
use tmt_ops::cron::ClockStatus;

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

pub(in crate::board) fn zone(view: &JobView) -> String {
    view.job.schedule.document()["zone"]
        .as_str()
        .unwrap_or("UTC")
        .to_owned()
}

pub(super) fn next_time(view: &JobView, now_ms: i64) -> Option<String> {
    time(*view.next_ms.first()?, now_ms, &zone(view))
}

#[cfg(test)]
pub(super) mod tests;
