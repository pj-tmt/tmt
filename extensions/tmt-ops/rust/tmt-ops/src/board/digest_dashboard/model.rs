use crate::labels::{Supplied, digest::SettingSource};

pub(in crate::board) fn summary(
    supplied: &Supplied,
    identities: Option<&std::collections::BTreeSet<&str>>,
) -> String {
    let Some((status, stale)) = supplied.digest() else {
        return "Digest status unavailable".into();
    };
    let mut held = 0u64;
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
                held = held.saturating_add(count);
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

pub(in crate::board) fn details(supplied: &Supplied, id: &str) -> Vec<String> {
    let mut fields: Vec<_> = labels(supplied, id)
        .into_iter()
        .map(|(text, _, _)| text)
        .collect();
    let Some((status, stale)) = supplied.digest() else {
        fields.push("Settings and statistics unavailable".into());
        return fields;
    };
    let Some(member) = status.members.get(id) else {
        fields.push("Member not reported by Digest".into());
        return fields;
    };
    if stale {
        fields.push("Stale observation · retained values; refresh before acting".into());
    }
    fields.push(format!(
        "Setting: {} ({})",
        member.setting_value,
        match member.setting_source {
            SettingSource::Default => "inherited default",
            SettingSource::Member => "member override",
        }
    ));
    fields.push(format!(
        "Held: {}",
        member
            .held_count
            .map_or("unavailable".into(), |n| n.to_string())
    ));
    fields.push(format!("Status observed: {} ms", status.observed_at_ms));
    fields.push(format!(
        "Stats observed: {}",
        member
            .stats_observed_at_ms
            .map_or("unavailable".into(), |n| format!("{n} ms"))
    ));
    if member.mode == crate::labels::digest::Mode::Auto {
        fields.push("Auto: when idle".into());
    } else {
        fields.push(format!(
            "Next send threshold: {}",
            member
                .next_send_at_ms
                .map_or("unavailable".into(), |n| format!("{n} ms"))
        ));
        fields.push(format!(
            "Threshold remaining: {}",
            member
                .time_to_send_ms
                .map_or("unavailable".into(), |n| format!("{n} ms"))
        ));
    }
    fields.extend(valuation_details(member));
    fields.extend(pricing_details(status));
    fields
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
            "Share of input: {}.{:02}% ({} avoided of {} input tokens) over {}–{} ms",
            percent.basis_points / 100,
            percent.basis_points % 100,
            percent.avoided,
            percent.avoided.saturating_add(percent.observed),
            percent.from_ms,
            percent.through_ms
        ));
    }
    fields
}

/// The supplier's offline list-price table, read-only. Source addresses are
/// plain text; the board never opens them.
fn pricing_details(status: &crate::labels::digest::Status) -> Vec<String> {
    let Some(pricing) = &status.pricing else {
        return Vec::new();
    };
    let mut fields = vec![format!(
        "Prices as of {} · table {} · {} (uncached input, list price)",
        pricing.as_of, pricing.version, pricing.basis
    )];
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

/// Formatted amount, source coverage and optional combination failure.
/// Captions, key/value rows and matrices share the same unit evidence.
pub(in crate::board) fn savings_value(
    summary: &crate::labels::digest::SavingsSummary,
    window: crate::config::DigestWindow,
    unit: crate::config::SavingsUnit,
) -> (Option<String>, String, Option<&'static str>) {
    use crate::config::SavingsUnit;
    let coverage = |included: usize| {
        if included < summary.total {
            format!(" ({included} of {})", summary.total)
        } else {
            String::new()
        }
    };
    // (value, coverage, note about values that could not be combined)
    match unit.shown(window) {
        SavingsUnit::Tokens => (
            summary
                .tokens
                .map(|tokens| crate::source::compact_tokens(u128::from(tokens))),
            coverage(summary.included),
            None,
        ),
        SavingsUnit::Percent => {
            let percent = &summary.percent;
            match (percent.value, percent.conflict) {
                (Some(points), _) => (
                    Some(format!("{} of input", percent_text(points))),
                    coverage(percent.included),
                    None,
                ),
                (None, true) => (None, String::new(), Some("intervals differ")),
                (None, false) => (None, coverage(percent.included), None),
            }
        }
        SavingsUnit::Cost => {
            let cost = &summary.cost;
            match (cost.value, cost.conflict) {
                (Some(micros), _) => (
                    Some(format!("{} at list price", usd_text(micros))),
                    coverage(cost.included),
                    None,
                ),
                (None, true) => (None, String::new(), Some("prices differ")),
                (None, false) => (None, coverage(cost.included), None),
            }
        }
    }
}

/// Shared secondary caption for member, squad, Overview and Digest usage
/// surfaces. One label for every unit; only the value changes. Estimate and
/// coverage stay visible even when the longer context label cannot fit; only
/// a combination note ("intervals differ") yields to width, before the segment.
pub(in crate::board) fn savings_caption(
    summary: &crate::labels::digest::SavingsSummary,
    window: crate::config::DigestWindow,
    unit: crate::config::SavingsUnit,
    width: usize,
) -> String {
    use unicode_width::UnicodeWidthStr;
    let (value, coverage, note) = savings_value(summary, window, unit);
    let value = value.unwrap_or_else(|| "—".into());
    let build = |label: &str, note: Option<&str>| {
        format!(
            "{label} {}: {value}{coverage}{}",
            window.label(),
            note.map_or_else(String::new, |note| format!(" ({note})"))
        )
    };
    let full = build("Est. ctx avoided", note);
    if full.width() <= width {
        return full;
    }
    let short = build("Est. avoided", note);
    if short.width() <= width {
        short
    } else {
        build("Est. avoided", None)
    }
}

/// Whole percent, half up; a nonzero share below one percent never reads as zero.
fn percent_text(basis_points: u16) -> String {
    match basis_points {
        0 => "0%".into(),
        1..=49 => "<1%".into(),
        points => format!("{}%", (u32::from(points) + 50) / 100),
    }
}

/// A list-price estimate to the cent, half up. Never worded as money saved.
fn usd_text(micros: u64) -> String {
    let cents = (u128::from(micros) + 5_000) / 10_000;
    if micros == 0 {
        return "$0.00".into();
    }
    if cents == 0 {
        return "< $0.01".into();
    }
    let dollars = (cents / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in dollars.chars().enumerate() {
        if index > 0 && (dollars.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("≈ ${grouped}.{:02}", cents % 100)
}

/// Dates describe the included members' own collection starts, never a common
/// reconstructed window. The caller supplies its local zone; tests pin one.
pub(in crate::board) fn savings_start(
    summary: &crate::labels::digest::SavingsSummary,
    window: crate::config::DigestWindow,
    zone: &jiff::tz::TimeZone,
    width: usize,
) -> Option<String> {
    if window != crate::config::DigestWindow::AllTime {
        return None;
    }
    let date = |ms| {
        jiff::Timestamp::from_millisecond(i64::try_from(ms).ok()?)
            .ok()
            .map(|time| time.to_zoned(zone.clone()).strftime("%m-%d").to_string())
    };
    let first = date(summary.earliest_from_ms?)?;
    let last = date(summary.latest_from_ms?)?;
    Some(if first == last {
        format!("since Digest began recording on {first}")
    } else {
        let prefix = "since Digest began recording ·";
        let range = format!("members started {first} to {last}");
        let line = format!("{prefix} {range}");
        // Keep the collection range together on narrow usage surfaces.
        if unicode_width::UnicodeWidthStr::width(line.as_str()) > width {
            format!("{prefix}\n{range}")
        } else {
            line
        }
    })
}
