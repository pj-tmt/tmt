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
    fields
}

/// Shared secondary caption for member, squad and Digest usage surfaces.
/// Keep estimate and coverage visible even when the longer context label cannot fit.
pub(in crate::board) fn savings_caption(
    summary: &crate::labels::digest::SavingsSummary,
    window: crate::config::DigestWindow,
    width: usize,
) -> String {
    use unicode_width::UnicodeWidthStr;
    let value = summary.tokens.map_or_else(
        || "—".into(),
        |tokens| crate::source::compact_tokens(u128::from(tokens)),
    );
    let coverage = if summary.included < summary.total {
        format!(" ({} of {})", summary.included, summary.total)
    } else {
        String::new()
    };
    let full = format!("Est. ctx avoided {}: {value}{coverage}", window.label());
    if full.width() <= width {
        full
    } else {
        format!("Est. avoided {}: {value}{coverage}", window.label())
    }
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
