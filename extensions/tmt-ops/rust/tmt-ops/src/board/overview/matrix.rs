//! Wide Digest identity rows adapt held scoped observations; no new aggregation policy.
use crate::{
    board::{
        app::App,
        digest_dashboard,
        digest_matrix::{self, Row},
    },
    config::DigestWindow,
    labels::digest::{Mode, SavingsSummary},
};
use std::collections::BTreeSet;

pub(super) struct Matrix {
    pub rows: Vec<Row>,
    pub note: String,
}
fn estimate(savings: &SavingsSummary, complete: bool) -> [Option<String>; 3] {
    let amount = |value: Option<String>, included: usize| {
        value.map(|value| {
            if !complete || included < savings.total {
                format!("~{value}")
            } else {
                value
            }
        })
    };
    [
        amount(
            savings
                .tokens
                .map(|n| crate::source::compact_tokens(u128::from(n))),
            savings.included,
        ),
        amount(
            savings.percent.value.map(digest_dashboard::percent_text),
            savings.percent.included,
        ),
        amount(
            savings.cost.value.map(|n| {
                digest_dashboard::usd_text(n)
                    .trim_start_matches("≈ ")
                    .to_owned()
            }),
            savings.cost.included,
        ),
    ]
}
fn tokens(app: &App, identities: &[&str], complete: bool) -> Option<String> {
    let mut total = 0u128;
    let mut known = 0;
    let mut partial = !complete;
    for id in identities {
        if let Some(reading) = app.overview_member_usage(id) {
            total = total.saturating_add(reading.tokens);
            known += 1;
            partial |= reading.partial;
        } else {
            partial = true;
        }
    }
    (known > 0).then(|| {
        format!(
            "{}{}",
            if partial { "~" } else { "" },
            crate::source::compact_tokens(total)
        )
    })
}
impl Matrix {
    pub fn project(app: &App, height: u16) -> Self {
        let snapshot = app.workspace_snapshot().expect("admitted workspace");
        let summary = super::monitor::scene(app, snapshot).summary.clone();
        let visible = app
            .workspace
            .visibility
            .member_ids(&snapshot.roster, &snapshot.preferences);
        let ids: Vec<_> = snapshot
            .roster
            .members
            .keys()
            .filter(|id| visible.as_ref().is_none_or(|ids| ids.contains(*id)))
            .map(String::as_str)
            .collect();
        let digest = app
            .labels
            .digest()
            .filter(|(_, stale)| !stale)
            .map(|(status, _)| status);
        let all_mode = if !summary.digest_available || summary.unknown_mode > 0 {
            None
        } else {
            let present: Vec<_> = [
                (summary.auto, "Auto"),
                (summary.timed, "timed"),
                (summary.off, "Off"),
            ]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .collect();
            Some(if present.len() == 1 {
                format!(
                    "{}{}",
                    if summary.complete { "" } else { "~" },
                    present[0].1
                )
            } else if present.is_empty() && summary.complete {
                "0".into()
            } else {
                format!("{}mixed", if summary.complete { "" } else { "~" })
            })
        };
        let held = digest.and_then(|_| {
            if summary.complete && summary.unknown_held == 0 {
                Some(summary.held.to_string())
            } else {
                (summary.held > 0).then(|| format!("~{}", summary.held))
            }
        });
        // The matrix's fixed caption is 24h. A different acquired supplier window does
        // not silently relabel its values or change the saved preference.
        let savings = crate::labels::digest::summarize_savings(
            app.labels.digest(),
            ids.iter().copied(),
            DigestWindow::Day,
        );
        let [ctx, percent, cost] = estimate(&savings, summary.complete);
        let mut rows = vec![Row {
            name: "all".into(),
            values: [
                all_mode,
                held,
                tokens(app, &ids, summary.complete),
                ctx,
                percent,
                cost,
            ],
        }];
        let mut seen = BTreeSet::new();
        let mut ranked: Vec<_> = ids
            .iter()
            .filter_map(|id| {
                if !seen.insert(*id) {
                    return None;
                }
                app.overview_member_usage(id).map(|reading| (*id, reading))
            })
            .collect();
        ranked.sort_by(|(a, ra), (b, rb)| {
            rb.tokens
                .cmp(&ra.tokens)
                .then_with(|| {
                    snapshot.roster.members[*a]
                        .name
                        .cmp(&snapshot.roster.members[*b].name)
                })
                .then_with(|| a.cmp(b))
        });
        for (id, _) in ranked.into_iter().take(digest_matrix::agents(height)) {
            let member = digest.and_then(|status| status.members.get(id));
            let mode = member.map(|member| match member.mode {
                Mode::Auto => "Auto".into(),
                Mode::Off => "Off".into(),
                Mode::Interval => member.setting_value.clone(),
            });
            let held = member
                .and_then(|member| member.held_count)
                .map(|count| count.to_string());
            let savings = crate::labels::digest::summarize_savings(
                app.labels.digest(),
                [id],
                DigestWindow::Day,
            );
            let [ctx, percent, cost] = estimate(&savings, true);
            rows.push(Row {
                name: snapshot.roster.members[id].name.clone(),
                values: [mode, held, tokens(app, &[id], true), ctx, percent, cost],
            });
        }
        let binding = app
            .workspace_bindings()
            .into_iter()
            .filter(|_| app.input.is_none())
            .find(|(_, action)| {
                action.verb == crate::action::Verb::Ui(crate::action::OpsUiAction::DigestDashboard)
            });
        let destination = binding.map_or_else(
            || "Digest for all".into(),
            |(key, _)| {
                format!(
                    "{} Digest for all",
                    crate::board::view::footer::key_label(&key)
                )
            },
        );
        Self {
            note: format!(
                "all + top {} by TOK · {destination}",
                rows.len().saturating_sub(1)
            ),
            rows,
        }
    }
}

#[cfg(test)]
mod tests;
