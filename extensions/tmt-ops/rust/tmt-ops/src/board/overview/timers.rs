//! A bounded preview over the held global cron observation; never a service read.
use crate::board::{app::App, cronboard, m9_timer_rows};
use std::collections::BTreeSet;
use tmt_ops::cron::ClockStatus;

pub(super) struct Model {
    pub rows: Vec<m9_timer_rows::Row>,
    pub total: usize,
    pub title: &'static str,
    pub note: String,
    pub empty: &'static str,
}
impl Model {
    pub fn height(&self) -> usize {
        1 + self.rows.len().max(1) + 1
    }
}

pub(super) fn project(app: &App) -> Model {
    let title = if app.workspace.visibility.scope.is_empty()
        && app.workspace.query.is_empty()
        && app.overview_filter == super::monitor::Filter::All
        && (app.workspace.visibility.reveal_hidden
            || app
                .workspace_snapshot()
                .is_none_or(|snapshot| snapshot.hidden_count == 0))
    {
        "Timers"
    } else {
        "Timers · global"
    };
    let Some(cron) = &app.cron.cron else {
        return Model {
            rows: vec![],
            total: 0,
            title,
            note: "unavailable".into(),
            empty: "Timers unavailable",
        };
    };
    let stale = app.cron.failure.is_some();
    let mut seen = BTreeSet::new();
    let mut jobs: Vec<_> = cron
        .jobs_in(None)
        .filter(|view| seen.insert((view.job.room_id.as_str(), view.job.id())))
        .collect();
    let paused = jobs
        .iter()
        .filter(|view| view.owner_missing || view.job.state() == "paused")
        .count();
    let next = |view: &&crate::cron_service::JobView| {
        (!view.owner_missing && view.job.state() == "on")
            .then(|| view.next_ms.first().copied())
            .flatten()
    };
    // The acquired order is already stable by squad and numeric job ID.
    jobs.sort_by(|a, b| match (next(a), next(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let total = jobs.len();
    let mut note = if stale {
        "stale".into()
    } else if let Some(count) = app
        .cron
        .counts
        .count(super::cron_counts::CronCountScope::All)
    {
        let mut label = format!("{} on · {paused} paused", count.active);
        let unassigned = usize::try_from(count.total)
            .unwrap_or(usize::MAX)
            .saturating_sub(count.active as usize + paused);
        if unassigned > 0 {
            label.push_str(&format!(" · {unassigned} no owner"));
        }
        label
    } else {
        "unavailable".into()
    };
    if !stale {
        match cron.clock {
            ClockStatus::Running(_) => {}
            ClockStatus::NoClock => note.push_str(if app.cron.clock_note(None).checking {
                " · checking"
            } else {
                " · no clock"
            }),
            ClockStatus::Unknown => note.push_str(" · clock unknown"),
        }
    }
    let rows = jobs
        .iter()
        .take(m9_timer_rows::MAX_ROWS)
        .map(|view| {
            let state = view.state_label();
            let next = if stale {
                None
            } else if view.owner_missing || view.job.state() != "on" {
                Some(
                    if view.owner_missing {
                        "no owner"
                    } else {
                        state
                    }
                    .into(),
                )
            } else if !matches!(cron.clock, ClockStatus::Running(_)) {
                None
            } else {
                next(view).map(|at| cronboard::countdown(at, cron.read_ms))
            };
            m9_timer_rows::Row {
                id: view.job.id(),
                squad: view.job.squad.clone(),
                schedule: crate::cron_service::schedule_text(&view.job.schedule),
                next,
                purpose: view.job.message.lines().next().unwrap_or("").into(),
                dim: stale || view.owner_missing || view.job.state() != "on",
            }
        })
        .collect();
    Model {
        rows,
        total,
        title,
        note,
        empty: if stale {
            "Timers unavailable"
        } else {
            "No timers"
        },
    }
}

#[cfg(test)]
mod tests;
