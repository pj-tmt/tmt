//! One identity row over the existing acquired workspace and label/meter owners.
use super::{columns::Column, cron_counts, sort};
use crate::board::{
    app::App,
    workspace::{OpsTarget, OpsTreeRow, observe::OpsWorkspaceSnapshot},
};
use std::rc::Rc;

pub(in crate::board) struct Values {
    shared: crate::board::workspace::columns::WorkspaceColumnValues,
    pub cron: Option<cron_counts::Count>,
}
impl std::ops::Deref for Values {
    type Target = crate::board::workspace::columns::WorkspaceColumnValues;
    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}
impl Values {
    fn order<'a>(&'a self, row: &'a OpsTreeRow) -> sort::Row<'a> {
        let OpsTarget::Member { identity_id, .. } = &row.target else {
            unreachable!("monitor identity")
        };
        sort::Row {
            id: identity_id,
            agent: &row.title,
            squad: self.squad.as_deref(),
            state: row.state.as_deref(),
            tokens_per_hour: self.tokens,
            digest: self.digest.as_deref(),
            cron: self.cron.map(|c| (c.active, c.total)),
            runtime: self.runtime.as_deref(),
            model: self.model.as_deref(),
        }
    }
    pub fn text(&self, column: Column, row: &OpsTreeRow) -> String {
        if column == Column::Cron {
            cron_counts::label(self.cron, false)
        } else {
            self.shared.text(column, row)
        }
    }
}

pub(in crate::board) fn values(
    app: &App,
    snapshot: &OpsWorkspaceSnapshot,
    row: &OpsTreeRow,
) -> Values {
    let OpsTarget::Member { identity_id, .. } = &row.target else {
        unreachable!("monitor identity")
    };
    let cron = app.cron.counts.count(cron_counts::CronCountScope::Owner {
        id: identity_id,
        room: None,
    });
    let shared = crate::board::workspace::columns::project(
        crate::board::workspace::columns::WorkspaceColumnInput {
            row,
            snapshot,
            labels: &app.labels,
            usage: app.overview_member_usage(identity_id),
            agent: app.references.agents.get(identity_id),
            timers: cron.map(|count| (count.active, count.total)),
        },
    )
    .expect("monitor member projection");
    Values { shared, cron }
}

#[derive(Default)]
pub(in crate::board) struct Cache {
    held: Option<(Key, Rc<Scene>)>,
}
pub(in crate::board) struct Scene {
    pub rows: Rc<[OpsTreeRow]>,
    pub matching_agents: usize,
    pub summary: super::summary::Summary,
}
/// What a squad is doing now: its lead's (else any member's) open request,
/// otherwise their task line, and whether that is a request waiting on you.
pub(in crate::board) fn squad_now(
    snapshot: &OpsWorkspaceSnapshot,
    squad: &crate::board::workspace::OpsSquad,
) -> (String, bool) {
    let rows: Vec<_> = squad
        .lead
        .iter()
        .chain(
            squad
                .members
                .iter()
                .filter(|id| Some(*id) != squad.lead.as_ref()),
        )
        .filter_map(|id| {
            snapshot.rows.get(&OpsTarget::Member {
                room_id: Some(squad.room_id.clone()),
                identity_id: id.clone(),
            })
        })
        .collect();
    let pending = rows.iter().find_map(|row| pending(row));
    let task = rows.iter().find_map(|row| {
        row["fields"]["task"]
            .as_str()
            .filter(|text| !text.trim().is_empty())
    });
    (
        pending
            .or(task)
            .and_then(|text| text.lines().next())
            .unwrap_or("—")
            .to_owned(),
        pending.is_some(),
    )
}

fn pending(row: &serde_json::Value) -> Option<&str> {
    row["pending"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            row["waitingOnYou"].as_array()?.iter().find_map(|request| {
                request["preview"]
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
            })
        })
}
pub(super) fn waiting_since(
    snapshot: &OpsWorkspaceSnapshot,
    row: &OpsTreeRow,
    now: u64,
) -> Option<u64> {
    snapshot.rows.get(&row.target)?["waitingOnYou"]
        .as_array()?
        .iter()
        .filter_map(|request| request["preparedAtMs"].as_u64())
        .filter(|at| *at > 0 && *at <= now)
        .min()
}
pub(super) fn waits(snapshot: &OpsWorkspaceSnapshot, row: &OpsTreeRow) -> bool {
    let OpsTarget::Member {
        room_id,
        identity_id,
    } = &row.target
    else {
        return false;
    };
    if let Some(room) = room_id {
        snapshot
            .roster
            .squads
            .iter()
            .find(|squad| &squad.room_id == room)
            .and_then(|squad| squad.member_states.get(identity_id))
            .is_some_and(|member| member.waiting)
    } else {
        snapshot
            .roster
            .members
            .get(identity_id)
            .is_some_and(|member| member.waiting)
    }
}
#[derive(PartialEq, Eq)]
struct Key {
    order: Option<crate::config::OverviewSort>,
    query: String,
    pins: std::collections::BTreeSet<String>,
    hidden: std::collections::BTreeSet<String>,
    digest_window: crate::config::DigestWindow,
    savings_unit: crate::config::SavingsUnit,
    visibility: crate::board::workspace::OpsVisibility,
    /// Crossing the stale rule changes the activity counts.
    activity_stale: bool,
}
impl Cache {
    pub fn clear(&mut self) {
        self.held = None;
    }
}

pub(in crate::board) fn rows(app: &App, snapshot: &OpsWorkspaceSnapshot) -> Rc<[OpsTreeRow]> {
    Rc::clone(&scene(app, snapshot).rows)
}

pub(in crate::board) fn scene(app: &App, snapshot: &OpsWorkspaceSnapshot) -> Rc<Scene> {
    let key = Key {
        order: app.workspace_order().overview,
        query: app.workspace.query.clone(),
        pins: snapshot.preferences.agent_pins.clone(),
        hidden: snapshot.preferences.hidden.clone(),
        digest_window: snapshot.preferences.digest_window,
        savings_unit: snapshot.preferences.savings_unit,
        visibility: app.workspace.visibility.clone(),
        activity_stale: app.activity_stale(std::time::Instant::now()),
    };
    let mut cache = app.overview_rows.borrow_mut();
    if let Some((old, scene)) = &cache.held
        && *old == key
    {
        return Rc::clone(scene);
    }
    // Choose a deterministic admitted membership occurrence for each UUID.
    // Cursor movement must not change row facts, ordering or rebuild this scene.
    // The existing projection owns pool/partial-roster admission.
    let waiting_rows = crate::board::workspace::identity_rows_matching(
        snapshot,
        &app.workspace.query,
        None,
        &key.visibility,
        |row| waits(snapshot, row),
    );
    let all_rows = crate::board::workspace::identity_rows(
        snapshot,
        &app.workspace.query,
        None,
        &key.visibility,
    );
    let mut seen = std::collections::BTreeSet::new();
    let rows = waiting_rows.into_iter().chain(all_rows).filter(|row| {
        let OpsTarget::Member { identity_id, .. } = &row.target else {
            return false;
        };
        seen.insert(identity_id.clone())
    });
    let now = crate::status::now_ms();
    let mut projected: Vec<_> = rows
        .map(|row| {
            let v = values(app, snapshot, &row);
            (row, v)
        })
        .collect();
    let pinned = |row: &OpsTreeRow| {
        matches!(&row.target,
        OpsTarget::Member { identity_id, .. } if key.pins.contains(identity_id))
    };
    let unread = app.unread_identities();
    let has_unread = |row: &OpsTreeRow| {
        matches!(&row.target,
        OpsTarget::Member { identity_id, .. } if unread.contains(identity_id.as_str()))
    };
    projected.sort_by(|(a, av), (b, bv)| {
        has_unread(b)
            .cmp(&has_unread(a))
            .then_with(|| pinned(b).cmp(&pinned(a)))
            .then_with(|| waits(snapshot, b).cmp(&waits(snapshot, a)))
            .then_with(|| match key.order {
                Some(order) => sort::compare(&av.order(a), &bv.order(b), order),
                None => match (
                    waiting_since(snapshot, a, now),
                    waiting_since(snapshot, b, now),
                ) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| av.order(a).id.cmp(bv.order(b).id)),
            })
    });
    let agents: Rc<[OpsTreeRow]> = projected
        .into_iter()
        .map(|(row, _)| row)
        .collect::<Vec<_>>()
        .into();
    let matching_agents = agents.len();
    let scene = Rc::new(Scene {
        matching_agents,
        rows: agents,
        summary: super::summary::project(
            snapshot,
            &app.labels,
            &key.visibility,
            key.activity_stale,
        ),
    });
    cache.held = Some((key, Rc::clone(&scene)));
    scene
}

#[cfg(test)]
mod tests;
