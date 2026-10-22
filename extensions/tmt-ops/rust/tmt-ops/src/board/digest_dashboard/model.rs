use crate::{
    board::savings::{percent_text, usd_text},
    labels::{Supplied, digest::SettingSource},
};
use tmt_cli_style::mark::Mark;

pub(in crate::board) fn summary(
    supplied: &Supplied,
    identities: Option<&std::collections::BTreeSet<&str>>,
) -> String {
    let Some((status, stale)) = supplied.digest() else {
        return "Digest status unavailable".into();
    };
    let mut held = 0u128;
    let mut unknown = identities.map_or(0, |ids| {
        ids.iter()
            .filter(|id| !status.members.contains_key(**id))
            .count()
    });
    let mut known = 0;
    for (id, member) in &status.members {
        if identities.is_some_and(|ids| !ids.contains(id.as_str())) {
            continue;
        }
        match member.held_count {
            Some(count) => {
                held += u128::from(count);
                known += 1;
            }
            None => unknown += 1,
        }
    }
    let suffix = if unknown > 0 {
        format!(" · {unknown} unknown")
    } else {
        String::new()
    };
    let count = if known == 0 && unknown > 0 {
        "held unavailable".into()
    } else if unknown > 0 {
        format!("≥{held} observed held")
    } else {
        format!("{held} held")
    };
    format!(
        "Digest · {count}{suffix}{}",
        if stale { " · stale observation" } else { "" }
    )
}

/// The clock ticks every minute; a Digest whose last admitted tick is older than this holds and
/// delivers nothing, and no other screen says so.
const TICK_STALE_MS: u64 = 3 * 60_000;

/// What the Digest tab says about a tick that stopped: only while Digest is on, since a stopped
/// tick has no effect otherwise, and only for a supplier that reports its liveness record. Both
/// times come from the one status document, so the age never depends on this machine's clock.
/// The shared warning mark leads, so the state never rests on colour alone.
pub(super) fn tick_notice(supplied: &Supplied) -> Option<String> {
    let (status, _) = supplied.digest()?;
    if status.global.as_ref().is_some_and(|global| !global.enabled) {
        return None;
    }
    match status.tick?.last_at_ms {
        None => Some(format!("{} No tick yet", Mark::Warning.symbol())),
        Some(last) => {
            let age = status.observed_at_ms.saturating_sub(last);
            (age > TICK_STALE_MS).then(|| {
                format!(
                    "{} No tick for {}",
                    Mark::Warning.symbol(),
                    human_duration(age)
                )
            })
        }
    }
}

/// Supplier labels own wording and actions; truncation never invents a status.
pub(in crate::board) fn labels(
    supplied: &Supplied,
    id: &str,
) -> Vec<(String, tmt_cli_style::Role, bool)> {
    supplied
        .of(id)
        .filter(|(source, _)| *source == "digest")
        .map(|(_, label)| {
            let mut text = crate::board::notes::sanitize(&label.text);
            if label.action.is_some() {
                text.push_str(" ▾");
            }
            (text, label.role, label.action.is_some())
        })
        .collect()
}

pub(in crate::board) fn details(
    supplied: &Supplied,
    id: &str,
    now_ms: u64,
    zone: &jiff::tz::TimeZone,
) -> Vec<String> {
    let mut fields = Vec::new();
    let Some((status, stale)) = supplied.digest() else {
        // Labels have independent admission and may still carry a valid chooser
        // when the typed statistics document is unavailable.
        fields.extend(labels(supplied, id).into_iter().map(|(text, _, _)| text));
        fields.push("Settings and statistics unavailable".into());
        return fields;
    };
    let Some(member) = status.members.get(id) else {
        fields.push("Member not reported by Digest".into());
        return fields;
    };
    let choice = if labels(supplied, id).iter().any(|(_, _, choice)| *choice) {
        " ▾"
    } else {
        ""
    };
    let mode = match member.mode {
        crate::labels::digest::Mode::Auto => format!("Auto{choice} · when idle"),
        crate::labels::digest::Mode::Off => format!("Off{choice}"),
        crate::labels::digest::Mode::Interval => format!(
            "Every {}{choice}",
            member
                .interval_ms
                .map_or("unavailable".into(), human_duration)
        ),
    };
    fields.push(format!(
        "Digest: {mode} ({})",
        match member.setting_source {
            SettingSource::Default => "default",
            SettingSource::Member => "override",
        }
    ));
    if stale {
        fields.push("Stale observation · retained values; refresh before acting".into());
    }
    fields.push(format!(
        "Held: {}",
        member
            .held_count
            .map_or("unavailable".into(), |n| crate::source::tokens(u128::from(
                n
            )))
    ));
    fields.push(format!(
        "Status observed: {}",
        observation_time(Some(status.observed_at_ms), now_ms, zone)
    ));
    fields.push(format!(
        "Stats observed: {}",
        observation_time(member.stats_observed_at_ms, now_ms, zone)
    ));
    if member.mode == crate::labels::digest::Mode::Interval {
        fields.push(format!(
            "Next send in: {}",
            member
                .time_to_send_ms
                .map_or("unavailable".into(), human_duration)
        ));
    }
    if let Some(savings) = &member.savings {
        fields.push(format!(
            "Supplier context avoided {}: {} · {}{}",
            savings.window.label(),
            savings
                .tokens
                .map_or("—".into(), |tokens| crate::source::tokens(u128::from(
                    tokens
                ))),
            human_window(savings.from_ms, savings.through_ms),
            if savings.complete { "" } else { " · partial" }
        ));
    }
    fields.extend(valuation_details(member));
    fields
}

/// Read-only observation wording using the shared Ops age formatter and held local zone.
pub(super) fn observation_time(
    at_ms: Option<u64>,
    now_ms: u64,
    zone: &jiff::tz::TimeZone,
) -> String {
    let Some(at_ms) = at_ms.filter(|at| *at > 0 && *at <= now_ms) else {
        return "unknown".into();
    };
    let Some(at) = i64::try_from(at_ms)
        .ok()
        .and_then(|at| jiff::Timestamp::from_millisecond(at).ok())
    else {
        return "unknown".into();
    };
    format!(
        "{} ({})",
        tmt_cli_style::value::relative_time(now_ms - at_ms),
        at.to_zoned(zone.clone()).strftime("%H:%M")
    )
}

/// What stands behind the cost and share figures of one member, so each can be
/// traced to its own quote and interval. Both are estimates, never bills.
fn valuation_details(member: &crate::labels::digest::Member) -> Vec<String> {
    let mut fields = Vec::new();
    let Some(savings) = &member.savings else {
        return fields;
    };
    if let Some(cost) = &savings.cost {
        fields.push(format!(
            "Cost quote: {} at list price · {} at {} per 1M input tokens · price table {}",
            usd_text(cost.usd_micros).trim_start_matches("≈ "),
            cost.model_id,
            rate_text(cost.usd_micros_per_million_input_tokens),
            cost.pricing_version
        ));
    }
    if let Some(percent) = &savings.percent {
        fields.push(format!(
            "Share of input: {} ({} of {} input tokens) over {}",
            detail_percent(percent.basis_points),
            crate::source::tokens(u128::from(percent.avoided)),
            crate::source::tokens(u128::from(percent.avoided) + u128::from(percent.observed)),
            human_window(percent.from_ms, percent.through_ms)
        ));
    }
    fields
}

fn detail_percent(points: u16) -> String {
    match points {
        0 => "0%".into(),
        1..=4 => "<0.1%".into(),
        5..=94 => format!("0.{}%", (points + 5) / 10),
        _ => percent_text(points),
    }
}
fn duration_parts(ms: u64) -> (String, &'static str) {
    let (unit, label) = if ms >= 86_400_000 {
        (86_400_000, "d")
    } else if ms >= 3_600_000 {
        (3_600_000, "h")
    } else if ms >= 60_000 {
        (60_000, "min")
    } else {
        (1_000, "s")
    };
    (crate::source::tokens(u128::from(ms / unit)), label)
}
fn human_duration(ms: u64) -> String {
    if ms < 1_000 {
        return "under 1 s".into();
    }
    let (value, unit) = duration_parts(ms);
    format!("{value} {unit}")
}
fn human_window(from: u64, through: u64) -> String {
    if through <= 1_000 {
        return "under 1 s".into();
    }
    let (start, start_unit) = duration_parts(from);
    let (end, end_unit) = duration_parts(through);
    if start_unit == end_unit {
        format!("{start}–{end} {end_unit}")
    } else {
        format!("{}–{}", human_duration(from), human_duration(through))
    }
}

/// The supplier's list-price table and where it came from, read-only. Source
/// addresses are plain text; the board never opens them.
pub(super) fn pricing_details(status: &crate::labels::digest::Status) -> Vec<String> {
    let Some(pricing) = &status.pricing else {
        return Vec::new();
    };
    let mut line = format!(
        "Prices as of {} · table {} · {} (uncached input, list price)",
        pricing.as_of, pricing.version, pricing.basis
    );
    if let Some(origin) = pricing.origin {
        line.push_str(&format!(" · source: {origin}"));
    }
    let mut fields = vec![line];
    fields.extend(pricing.models.iter().map(|model| {
        format!(
            "{}: {} per 1M input tokens · {}",
            model.model_id,
            rate_text(model.usd_micros_per_million_input_tokens),
            model.source
        )
    }));
    fields
}

/// An exact USD rate: at least cents, up to micro-dollars, no rounding.
fn rate_text(micros: u64) -> String {
    let whole = micros / 1_000_000;
    let mut fraction = format!("{:06}", micros % 1_000_000);
    while fraction.len() > 2 && fraction.ends_with('0') {
        fraction.pop();
    }
    format!("${whole}.{fraction}")
}

#[cfg(test)]
mod detail_copy_tests {
    use super::*;
    #[test]
    fn readable_detail_units_preserve_unknown_and_small_nonzero_values() {
        for (points, expected) in [
            (0, "0%"),
            (1, "<0.1%"),
            (5, "0.1%"),
            (49, "0.5%"),
            (99, "1%"),
            (1820, "18%"),
        ] {
            assert_eq!(detail_percent(points), expected);
        }
        assert_eq!(human_window(0, 1_000), "under 1 s");
        assert_eq!(human_window(60_000, 120_000), "1–2 min");
        assert_eq!(human_duration(600_000), "10 min");
    }
}
