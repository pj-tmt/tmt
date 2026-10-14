use crate::{
    board::workspace::{OpsTarget, OpsTreeRow, observe::OpsWorkspaceSnapshot},
    labels::{Supplied, digest::SettingSource},
};

/// One row per identity, independent of squad expansion and duplicate membership.
/// Retain a real squad occurrence for the existing management admission lane.
pub(in crate::board) fn rows(snapshot: &OpsWorkspaceSnapshot, query: &str) -> Vec<OpsTreeRow> {
    let query = query.to_lowercase();
    // One pass over occurrences; do not rescan the entire roster for each agent.
    let mut targets = std::collections::BTreeMap::new();
    for target in snapshot.rows.keys() {
        if let OpsTarget::Member {
            identity_id,
            room_id,
        } = target
        {
            let selected = targets.entry(identity_id.as_str()).or_insert(target);
            if room_id.is_some() && matches!(selected, OpsTarget::Member { room_id: None, .. }) {
                *selected = target;
            }
        }
    }
    let mut rows: Vec<_> = snapshot
        .roster
        .members
        .values()
        .filter_map(|member| {
            if !member.name.to_lowercase().contains(&query) {
                return None;
            }
            let target = targets.get(member.id.as_str())?;
            Some(OpsTreeRow {
                target: (*target).clone(),
                title: member.name.clone(),
                depth: 0,
                lead: false,
                state: member.state.clone(),
                presence: member.presence,
                usage: member.usage,
                count: None,
                observation: snapshot.roster.observation,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then(a.target.cmp(&b.target))
    });
    if snapshot.preferences.reversed {
        rows.reverse();
    }
    rows
}

pub(in crate::board) fn summary(supplied: &Supplied) -> String {
    let Some((status, stale)) = supplied.digest() else {
        return "Digest status unavailable".into();
    };
    let mut held = 0u64;
    let mut unknown = 0;
    let mut known = 0;
    for member in status.members.values() {
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
