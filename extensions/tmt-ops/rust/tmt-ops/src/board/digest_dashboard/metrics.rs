//! Values projected from held evidence. No display total grants action authority.
use crate::board::{
    app::App,
    usage_report::{Report, View, model::Point},
    workspace::{OpsTarget, OpsTreeRow},
};
use crate::{
    config::{DigestWindow, TokRange},
    labels::digest::SavingsSummary,
};

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
pub(super) fn tokens(report: Option<&Report>, ids: &[&str]) -> Option<(u128, bool)> {
    let report = report?;
    let mut total = 0;
    let mut known = 0;
    let mut partial = false;
    let mut window = None;
    for id in ids {
        if let Some((seat, tokens)) = report
            .seat(id)
            .and_then(|seat| Some((seat, seat.observed_total()?)))
        {
            let source_window = (seat.from_ms, seat.to_ms);
            if window.is_some_and(|old| old != source_window) {
                return None;
            }
            window = Some(source_window);
            total += u128::from(tokens);
            known += 1;
            partial |= !seat.complete;
        } else {
            partial = true;
        }
    }
    (known > 0).then_some((total, partial))
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
        if seat.series.len() != 24 {
            unknown.fill(true);
            continue;
        }
        let clock: Vec<_> = seat
            .series
            .iter()
            .map(|point| (point.from_ms, point.to_ms))
            .collect();
        if boundaries.as_ref().is_some_and(|old| old != &clock) {
            return vec![None; 24];
        }
        boundaries = Some(clock);
        for ((sum, missing), point) in values.iter_mut().zip(&mut unknown).zip(&seat.series) {
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
pub(super) fn busiest<'a>(report: &'a Report, id: &str) -> Option<&'a Point> {
    report
        .seat(id)?
        .series
        .iter()
        .filter(|point| point.complete || point.total() > 0)
        .max_by_key(|point| point.total())
}
pub(super) fn input_price(app: &App, report: Option<&Report>, ids: &[&str]) -> Option<u64> {
    let report = report?;
    // A price total must cover the same admitted source window as TOK. Partial
    // or absent usage cannot acquire a precise-looking price through attribution.
    let (_, partial) = tokens(Some(report), ids)?;
    if partial {
        return None;
    }
    let (status, false) = app.labels.digest()? else {
        return None;
    };
    let prices = &status.pricing.as_ref()?.models;
    let mut millionths = 0u128;
    if ids.is_empty() {
        return None;
    }
    for id in ids {
        for (model, input) in report.seat(id)?.model_inputs.as_ref()? {
            let rate = prices
                .iter()
                .find(|price| &price.model_id == model)?
                .usd_micros_per_million_input_tokens;
            millionths = millionths.checked_add(u128::from(*input) * u128::from(rate))?;
        }
    }
    u64::try_from((millionths + 500_000) / 1_000_000).ok()
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
