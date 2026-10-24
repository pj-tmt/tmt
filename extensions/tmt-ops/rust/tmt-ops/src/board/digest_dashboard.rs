//! Existing-data Digest view. Acquisition, effects, scrolls and terminal diffing
//! remain with their existing owners; this module projects already-held values.
mod allowance;
mod metrics;
mod model;
mod render;
mod table;

use crate::board::{
    app::App,
    usage_report::Report,
    workspace::{OpsTarget, OpsTreeRow, OpsVisibility, observe::OpsWorkspaceSnapshot},
};
use std::{rc::Rc, sync::Arc};

#[derive(Default)]
pub(in crate::board) struct Cache {
    held: Option<(Key, Projection)>,
}

#[derive(Clone)]
struct Projection {
    rows: Rc<[OpsTreeRow]>,
    exchanges: Rc<[crate::board::exchange_matrix::ExchangeParticipant]>,
}

struct Key {
    query: String,
    visibility: OpsVisibility,
    range: crate::config::TokRange,
    report: Option<Arc<Report>>,
}

impl Key {
    fn matches(&self, other: &Self) -> bool {
        self.query == other.query
            && self.visibility == other.visibility
            && self.range == other.range
            && match (&self.report, &other.report) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

impl Cache {
    pub fn clear(&mut self) {
        self.held = None;
    }
}

/// Cursor changes reuse admitted facts. Snapshot/preferences reconciliation
/// invalidates them; an immutable usage receipt also changes the key.
fn projection(app: &App, snapshot: &OpsWorkspaceSnapshot) -> Projection {
    let key = Key {
        query: app.workspace.query.clone(),
        visibility: app.workspace.visibility.clone(),
        range: app.tok_range(),
        report: app.usage_report.retained_report(app.tok_range()),
    };
    let mut cache = app.digest_rows.borrow_mut();
    if !cache
        .held
        .as_ref()
        .is_some_and(|(old, _)| old.matches(&key))
    {
        let rows: Rc<[OpsTreeRow]> = projected_rows(app, snapshot, None).into();
        let exchanges = crate::board::exchange_matrix::by_traffic(
            rows.iter()
                .filter_map(|row| {
                    Some(crate::board::exchange_matrix::ExchangeParticipant {
                        id: metrics::id(row)?.into(),
                        name: row.title.clone(),
                    })
                })
                .collect(),
            metrics::report(app).and_then(|report| report.exchange.as_ref()),
        )
        .into();
        cache.held = Some((key, Projection { rows, exchanges }));
    }
    cache.held.as_ref().unwrap().1.clone()
}

pub(in crate::board) fn rows(app: &App, snapshot: &OpsWorkspaceSnapshot) -> Rc<[OpsTreeRow]> {
    let canonical = projection(app, snapshot).rows;
    // An explicit noncanonical membership keeps the pre-existing occurrence's
    // scoped state, action target and existing ordering tie-breaks. The cached
    // canonical projection stays unchanged when that explicit selection ends.
    if let Some(selected @ OpsTarget::Member { identity_id, .. }) = app.workspace.selected.as_ref()
        && canonical.iter().any(|row| matches!(&row.target,
            OpsTarget::Member { identity_id: id, .. } if id == identity_id && &row.target != selected))
    {
        return projected_rows(app, snapshot, Some(selected)).into();
    }
    canonical
}

fn exchange_participants(app: &App) -> Rc<[crate::board::exchange_matrix::ExchangeParticipant]> {
    app.workspace_snapshot().map_or_else(
        || Rc::from([]),
        |snapshot| projection(app, snapshot).exchanges,
    )
}

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
pub(super) use render::{details_clock_mark, render};

#[cfg(test)]
pub(in crate::board) mod tests;

/// Table ordering and its aggregate target share the same UUID projection used
/// by input. A paint-only sort cannot silently change the action recipient.
fn projected_rows(
    app: &crate::board::app::App,
    snapshot: &crate::board::workspace::observe::OpsWorkspaceSnapshot,
    preferred: Option<&OpsTarget>,
) -> Vec<crate::board::workspace::OpsTreeRow> {
    use crate::board::{
        usage_report::View,
        workspace::{self, OpsTarget, OpsTreeRow},
    };
    let mut rows = workspace::admitted_identity_rows(
        snapshot,
        &app.workspace.query,
        preferred,
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
