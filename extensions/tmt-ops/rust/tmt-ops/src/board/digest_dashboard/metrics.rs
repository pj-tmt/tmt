//! Values projected from held evidence. No display total grants action authority.
use crate::board::{
    app::App,
    usage_report::{Report, View},
    workspace::{OpsTarget, OpsTreeRow},
};
use crate::{
    config::{DigestWindow, TokRange},
    labels::digest::SavingsSummary,
};

pub(super) use crate::board::usage_report::valuation::{input_price, tokens};

pub(super) fn report(app: &App) -> Option<&Report> {
    match app.usage_report.view_for(app.tok_range()) {
        View::Report(report) => Some(report),
        _ => None,
    }
}
pub(super) fn id(row: &OpsTreeRow) -> Option<&str> {
    match &row.target {
        OpsTarget::Member { identity_id, .. } => Some(identity_id),
        _ => None,
    }
}
pub(super) fn identities<'a>(row: &'a OpsTreeRow, rows: &'a [OpsTreeRow]) -> Vec<&'a str> {
    id(row).map_or_else(|| rows.iter().filter_map(id).collect(), |id| vec![id])
}
pub(super) fn token_text(reading: Option<(u128, bool)>) -> String {
    reading.map_or("—".into(), |(tokens, partial)| {
        format!(
            "{}{}",
            if partial { "≥" } else { "" },
            crate::source::tokens(tokens)
        )
    })
}
pub(super) fn share(part: Option<(u128, bool)>, total: Option<(u128, bool)>) -> String {
    match (part, total) {
        (Some((part, false)), Some((total, false))) if total > 0 => {
            format!("{}%", part.saturating_mul(100) / total)
        }
        _ => "—".into(),
    }
}
pub(super) fn activity(report: Option<&Report>, ids: &[&str]) -> Vec<Option<u64>> {
    let Some(report) = report else {
        return vec![None; 24];
    };
    let mut values = vec![None; 24];
    let mut boundaries = None;
    let mut unknown = [false; 24];
    for id in ids {
        let Some(seat) = report.seat(id) else {
            unknown.fill(true);
            continue;
        };
        if seat.series.is_empty() {
            unknown.fill(true);
            continue;
        }
        if boundaries.is_some_and(|old| old != seat.series.from_ms) {
            return vec![None; 24];
        }
        boundaries = Some(seat.series.from_ms);
        for ((sum, missing), point) in values.iter_mut().zip(&mut unknown).zip(seat.series.iter()) {
            *missing |= !point.complete;
            if point.complete || point.total() > 0 {
                let Some(total) = sum.unwrap_or(0u64).checked_add(point.total()) else {
                    return vec![None; 24];
                };
                *sum = Some(total);
            }
        }
    }
    for (value, missing) in values.iter_mut().zip(unknown) {
        if missing && *value == Some(0) {
            *value = None;
        }
    }
    values
}
pub(super) struct BusiestHour {
    pub from_ms: u64,
    pub to_ms: u64,
    pub tokens: u64,
    pub complete: bool,
}
pub(super) fn busiest(report: &Report, id: &str) -> Option<BusiestHour> {
    let series = &report.seat(id)?.series;
    let (index, point) = series
        .iter()
        .enumerate()
        .filter(|(_, point)| point.complete || point.total() > 0)
        .max_by_key(|(_, point)| point.total())?;
    let from_ms = series.from_ms + index as u64 * 3_600_000;
    Some(BusiestHour {
        from_ms,
        to_ms: from_ms + 3_600_000,
        tokens: point.tokens,
        complete: point.complete,
    })
}
pub(super) fn savings(app: &App, ids: &[&str]) -> Option<SavingsSummary> {
    let window = match app.tok_range() {
        TokRange::Day => DigestWindow::Day,
        TokRange::Week => DigestWindow::Week,
        _ => return None,
    };
    Some(crate::labels::digest::summarize_savings(
        app.labels.digest(),
        ids.iter().copied(),
        window,
    ))
}
pub(super) fn held(app: &App, ids: &[&str]) -> Option<u64> {
    let (status, false) = app.labels.digest()? else {
        return None;
    };
    ids.iter().try_fold(0u64, |total, id| {
        total.checked_add(status.members.get(*id)?.held_count?)
    })
}
pub(super) fn mode(app: &App, id: Option<&str>) -> String {
    let Some(id) = id else { return "—".into() };
    let Some((status, false)) = app.labels.digest() else {
        return "—".into();
    };
    status
        .members
        .get(id)
        .map_or("—".into(), |member| match member.mode {
            crate::labels::digest::Mode::Auto => "Auto".into(),
            crate::labels::digest::Mode::Off => "Off".into(),
            crate::labels::digest::Mode::Interval => format!("timed {}", member.setting_value),
        })
}
pub(super) fn recipient<'a>(report: &'a Report, id: &str) -> Option<(Option<&'a str>, u64)> {
    report
        .exchange
        .as_ref()?
        .messages
        .get(&Some(id.into()))?
        .iter()
        .max_by(|(a, ca), (b, cb)| ca.cmp(cb).then_with(|| b.cmp(a)))
        .map(|(id, count)| (id.as_deref(), *count))
}
pub(super) fn name(app: &App, id: Option<&str>) -> String {
    id.map_or("unknown".into(), |id| {
        app.workspace_snapshot()
            .and_then(|snapshot| snapshot.roster.members.get(id))
            .map_or_else(
                || id.chars().take(8).collect(),
                |member| member.name.clone(),
            )
    })
}
