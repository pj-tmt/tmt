//! Existing-data Digest view. Acquisition, effects, scrolls and terminal diffing
//! remain with their existing owners; this module projects already-held values.
mod metrics;
mod model;
mod render;
mod table;

/// Session scope uses exactly the admitted Digest identities, including its
/// current search, while saved hiding remains an Agents/Overview preference.
pub(super) fn summary(app: &crate::board::app::App) -> String {
    let rows = app.workspace_rows();
    let identities = rows
        .iter()
        .filter_map(|row| match &row.target {
            crate::board::workspace::OpsTarget::Member { identity_id, .. } => {
                Some(identity_id.as_str())
            }
            _ => None,
        })
        .collect();
    model::summary(&app.labels, Some(&identities))
}
pub(super) use model::{
    compact_savings_caption, percent_text, savings_caption, savings_start, savings_value, usd_text,
};
pub(super) use render::{details_clock_mark, render};

#[cfg(test)]
pub(in crate::board) mod tests;

/// Table ordering and its aggregate target share the same UUID projection used
/// by input. A paint-only sort cannot silently change the action recipient.
pub(in crate::board) fn rows(
    app: &crate::board::app::App,
    snapshot: &crate::board::workspace::observe::OpsWorkspaceSnapshot,
) -> Vec<crate::board::workspace::OpsTreeRow> {
    use crate::board::{
        usage_report::View,
        workspace::{self, OpsTarget, OpsTreeRow},
    };
    let mut rows = workspace::admitted_identity_rows(
        snapshot,
        &app.workspace.query,
        app.workspace.selected.as_ref(),
        &workspace::OpsVisibility {
            scope: app.workspace.visibility.scope.clone(),
            reveal_hidden: true,
        },
        |_| true,
    );
    let report = match app.usage_report.view_for(app.tok_range()) {
        View::Report(report) => Some(report),
        _ => None,
    };
    for row in &mut rows {
        if let OpsTarget::Member { identity_id, .. } = &row.target {
            row.usage = report
                .and_then(|report| report.seat(identity_id))
                .and_then(|seat| seat.observed_total());
        }
    }
    rows.sort_by(|a, b| {
        b.usage
            .cmp(&a.usage)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.target.cmp(&b.target))
    });
    let count = rows.len();
    rows.insert(
        0,
        OpsTreeRow {
            target: OpsTarget::DigestAll,
            title: "all".into(),
            depth: 0,
            lead: false,
            state: None,
            presence: None,
            usage: None,
            activity: None,
            count: Some(count),
            observation: snapshot.roster.observation,
        },
    );
    rows
}
