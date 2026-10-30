//! Existing-data Digest view. Acquisition, effects, scrolls and terminal diffing
//! remain with their existing owners; this module projects already-held values.
mod allowance;
mod details;
mod metrics;
mod model;
mod render;
mod summary;
pub(in crate::board) use allowance::Scroll as AllowanceScroll;
pub(in crate::board) use details::{Basis as EstimateBasis, Tab as DetailsTab};
pub(in crate::board) use render::open_basis;
mod table;

use crate::board::{
    app::App,
    usage_report::Report,
    workspace::{OpsTarget, OpsTreeRow, OpsVisibility, observe::OpsWorkspaceSnapshot},
};
use crate::labels::digest::SavingsSummary;
use std::{
    collections::HashMap,
    rc::{self, Rc},
    sync::{self, Arc},
};

#[derive(Default)]
pub(in crate::board) struct Cache {
    held: Option<(Key, Projection)>,
    savings: Option<(SavingsKey, Rc<SavingsSummary>)>,
    held_text: Option<(SavingsKey, Rc<str>)>,
}

/// The admitted rows' saved sum is one read of one status; the projection key leaves labels out
/// so supplier text stays live, so the sum keys on the held status itself. Weak pointers name
/// the status and rows without keeping a replaced generation alive, and its address taken.
struct SavingsKey {
    status: Option<(sync::Weak<crate::labels::digest::Status>, bool)>,
    rows: rc::Weak<[OpsTreeRow]>,
    window: crate::config::DigestWindow,
}

impl SavingsKey {
    fn of(app: &App, rows: &Rc<[OpsTreeRow]>) -> Self {
        Self {
            status: app
                .labels
                .digest_held()
                .map(|(status, stale)| (Arc::downgrade(status), stale)),
            rows: Rc::downgrade(rows),
            window: app.digest_window(),
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.window == other.window
            && rc::Weak::ptr_eq(&self.rows, &other.rows)
            && match (&self.status, &other.status) {
                (Some((a, stale)), Some((b, now))) => sync::Weak::ptr_eq(a, b) && stale == now,
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Clone)]
struct Projection {
    rows: Rc<[OpsTreeRow]>,
    exchanges: Rc<[crate::board::exchange_matrix::ExchangeParticipant]>,
    talks: Rc<Talks>,
}

/// Whom each sender wrote to most, found once per report so a frame never searches the traffic.
#[derive(Default)]
pub(super) struct Talks(HashMap<Arc<str>, (Option<Arc<str>>, u64)>);

impl Talks {
    fn of(report: Option<&Report>) -> Self {
        let Some(traffic) = report.and_then(|report| report.exchange.as_ref()) else {
            return Self::default();
        };
        // Sized once: a map grown by doubling while it fills leaves its peak in the process.
        let mut talks = HashMap::with_capacity(traffic.busiest_flows().count());
        talks.extend(
            traffic.busiest_flows().filter_map(|flow| {
                Some((flow.sender.clone()?, (flow.recipient.clone(), flow.count)))
            }),
        );
        Self(talks)
    }

    /// The recipient (`None` when the report could not identify it) and its message count.
    pub(super) fn busiest(&self, id: &str) -> Option<(Option<&str>, u64)> {
        let (recipient, count) = self.0.get(id)?;
        Some((recipient.as_deref(), *count))
    }
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
        self.savings = None;
    }

    /// Holds the projection `build` makes for `key`. The stale one is released first: it pins
    /// the previous usage report through its key, and rows plus ranking are built while the
    /// new report is held, so keeping it would put both generations on the heap at once.
    fn rebuild(&mut self, key: Key, build: impl FnOnce() -> Projection) -> Projection {
        self.held = None;
        let projection = build();
        self.held.insert((key, projection.clone())).1.clone()
    }
}

/// Cursor changes reuse admitted facts. Snapshot/preferences reconciliation
/// invalidates them; an immutable usage receipt also changes the key.
fn projection(app: &App, snapshot: &OpsWorkspaceSnapshot) -> Projection {
    let key = Key {
        query: app.workspace.query.clone(),
        visibility: app.workspace.visibility.clone(),
        range: app.digest_range(),
        report: app.usage_report.retained_report(app.digest_range()),
    };
    let mut cache = app.digest_rows.borrow_mut();
    if let Some((old, held)) = &cache.held
        && old.matches(&key)
    {
        return held.clone();
    }
    cache.rebuild(key, || {
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
        let talks = Rc::new(Talks::of(metrics::report(app)));
        Projection {
            rows,
            exchanges,
            talks,
        }
    })
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

/// The saved sum over the admitted rows, shared by the summary tiles, Details for `all` and the
/// table's `all` row.
pub(super) fn savings(app: &App) -> Rc<SavingsSummary> {
    let Some(snapshot) = app.workspace_snapshot() else {
        return Rc::new(metrics::savings(app, &[]));
    };
    let rows = projection(app, snapshot).rows;
    let key = SavingsKey::of(app, &rows);
    let mut cache = app.digest_rows.borrow_mut();
    if let Some((held, summary)) = &cache.savings
        && held.matches(&key)
    {
        return summary.clone();
    }
    let ids: Vec<_> = rows.iter().filter_map(metrics::id).collect();
    let summary = Rc::new(metrics::savings(app, &ids));
    cache.savings.insert((key, summary)).1.clone()
}

pub(super) fn talks(app: &App) -> Rc<Talks> {
    app.workspace_snapshot()
        .map_or_else(Default::default, |snapshot| projection(app, snapshot).talks)
}

fn exchange_participants(app: &App) -> Rc<[crate::board::exchange_matrix::ExchangeParticipant]> {
    app.workspace_snapshot().map_or_else(
        || Rc::from([]),
        |snapshot| projection(app, snapshot).exchanges,
    )
}

/// Session scope uses exactly the admitted Digest identities, including its
/// current search, while saved hiding remains an Agents/Overview preference.
/// The status row and Details for `all` share one count per held status and row set.
pub(super) fn held_text(app: &crate::board::app::App) -> Rc<str> {
    let rows = app.workspace_rows();
    let key = SavingsKey::of(app, &rows);
    let mut cache = app.digest_rows.borrow_mut();
    if let Some((held, text)) = &cache.held_text
        && held.matches(&key)
    {
        return text.clone();
    }
    let identities = rows
        .iter()
        .filter_map(|row| match &row.target {
            crate::board::workspace::OpsTarget::Member { identity_id, .. } => {
                Some(identity_id.as_str())
            }
            _ => None,
        })
        .collect();
    let text = Rc::from(model::held_text(&app.labels, Some(&identities)));
    cache.held_text.insert((key, text)).1.clone()
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
    let report = match app.usage_report.view_for(app.digest_range()) {
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
