//! Pure workspace projection over one acquired roster. This is not a membership
//! store: IDs and observations come from the worker, and no action performs I/O.

pub(crate) mod columns;
pub(crate) mod manage;
pub(crate) mod observe;
pub(crate) mod rename_member;
pub(crate) mod render;

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OpsObservation {
    Complete,
    Stale,
    Partial,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpsMember {
    pub id: String,
    pub name: String,
    pub state: Option<String>,
    pub waiting: bool,
    pub presence: Option<bool>,
    pub usage: Option<u64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OpsMembershipState {
    pub state: Option<String>,
    pub waiting: bool,
    pub rank: Option<crate::config::Rank>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpsSquad {
    pub room_id: String,
    pub name: String,
    pub members: Vec<String>,
    pub lead: Option<String>,
    /// Squad metadata is scoped even when an identity belongs to several rooms.
    pub member_states: BTreeMap<String, OpsMembershipState>,
    pub observation: OpsObservation,
}

#[derive(Debug, Default)]
pub struct OpsRoster {
    pub members: BTreeMap<String, OpsMember>,
    pub observation: OpsObservation,
    pub squads: Vec<OpsSquad>,
    /// False when any source needed to prove zero memberships was not read.
    pub membership_complete: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShellCounts {
    pub online: usize,
    pub waiting: usize,
    pub blocked: usize,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpsTarget {
    Pool,
    Squad(String),
    Member {
        room_id: Option<String>,
        identity_id: String,
    },
}

pub use crate::config::{OpsPreferences, OpsSort};

/// Display scope and explicit hidden-room reveal belong to this UI session,
/// never the acquired roster or retained view preferences.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OpsVisibility {
    pub scope: Vec<String>,
    pub reveal_hidden: bool,
}

impl OpsVisibility {
    /// Admission for view totals uses the same room visibility as the tree.
    /// Unassigned identities remain visible only outside an explicit scope;
    /// a hidden membership never turns its agent into a Common pool member.
    pub(super) fn member_ids<'a>(
        &self,
        roster: &'a OpsRoster,
        preferences: &OpsPreferences,
    ) -> Option<BTreeSet<&'a String>> {
        if self.scope.is_empty()
            && (self.reveal_hidden
                || !roster
                    .squads
                    .iter()
                    .any(|squad| preferences.hidden.contains(&squad.room_id)))
        {
            return None;
        }
        let mut visible: BTreeSet<_> = roster
            .squads
            .iter()
            .filter(|squad| self.shows_squad(&squad.room_id, preferences))
            .flat_map(|squad| squad.members.iter())
            .filter(|id| roster.members.contains_key(*id))
            .collect();
        if self.scope.is_empty() {
            visible.extend(roster.pool().map(|member| &member.id));
        }
        Some(visible)
    }

    pub fn shows_squad(&self, id: &str, preferences: &OpsPreferences) -> bool {
        if self.scope.is_empty() {
            self.reveal_hidden || !preferences.hidden.contains(id)
        } else {
            self.scope.iter().any(|scoped| scoped == id)
        }
    }
}

#[derive(Debug, Default)]
pub struct OpsWorkspace {
    pub selected: Option<OpsTarget>,
    pub expanded: BTreeSet<String>,
    pub pool_expanded: bool,
    pub query: String,
    pub(in crate::board) order: Option<crate::board::view_state::Order>,
    pub filter: AgentsFilter,
    pub visibility: OpsVisibility,
}

/// Session-local tree scope; it never changes membership or saved preferences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentsFilter {
    #[default]
    All,
    Pinned,
}

impl AgentsFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Pinned,
            Self::Pinned => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Pinned => "Pinned",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionState {
    None,
    Visible,
    Hidden,
    Missing,
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
pub struct OpsTreeRow {
    pub target: OpsTarget,
    pub title: String,
    pub depth: u8,
    pub lead: bool,
    pub state: Option<String>,
    pub presence: Option<bool>,
    pub usage: Option<u64>,
    /// A total is known only after a complete observation, never from a
    /// failed/partial/stale read. Observed children can still be displayed.
    pub count: Option<usize>,
    pub observation: OpsObservation,
}

impl OpsRoster {
    /// Independent UUID sets; presence and request attention are different facts.
    pub fn shell_counts(&self) -> ShellCounts {
        self.counts(None)
    }

    pub(super) fn hidden_counts(&self, rooms: &BTreeSet<String>) -> ShellCounts {
        self.counts(Some(rooms))
    }

    fn counts(&self, rooms: Option<&BTreeSet<String>>) -> ShellCounts {
        let identities: Option<BTreeSet<_>> = rooms.map(|rooms| {
            self.squads
                .iter()
                .filter(|squad| rooms.contains(&squad.room_id))
                .flat_map(|squad| squad.members.iter())
                .collect()
        });
        let admitted = |id: &String| identities.as_ref().is_none_or(|ids| ids.contains(id));
        let mut waiting = BTreeSet::new();
        let mut blocked = BTreeSet::new();
        for member in self.members.values().filter(|member| admitted(&member.id)) {
            if member.waiting {
                waiting.insert(&member.id);
            }
            if member.state.as_deref() == Some("blocked") {
                blocked.insert(&member.id);
            }
        }
        for squad in self
            .squads
            .iter()
            .filter(|squad| rooms.is_none_or(|rooms| rooms.contains(&squad.room_id)))
        {
            let members: BTreeSet<_> = squad.members.iter().collect();
            for (id, state) in &squad.member_states {
                if !members.contains(id) || !self.members.contains_key(id) {
                    continue;
                }
                if state.waiting {
                    waiting.insert(id);
                }
                if state.state.as_deref() == Some("blocked") {
                    blocked.insert(id);
                }
            }
        }
        ShellCounts {
            online: self
                .members
                .values()
                .filter(|member| admitted(&member.id) && member.presence == Some(true))
                .count(),
            waiting: waiting.len(),
            blocked: blocked.len(),
            complete: self.complete()
                && self
                    .members
                    .values()
                    .filter(|member| admitted(&member.id))
                    .all(|member| member.presence.is_some()),
        }
    }

    fn members<'a>(&'a self, squad: &'a OpsSquad) -> impl Iterator<Item = &'a OpsMember> {
        squad.members.iter().filter_map(|id| self.members.get(id))
    }

    fn pool(&self) -> impl Iterator<Item = &OpsMember> {
        let complete = self.membership_complete
            && matches!(
                self.observation,
                OpsObservation::Complete | OpsObservation::Stale
            )
            && self.squads.iter().all(|squad| {
                matches!(
                    squad.observation,
                    OpsObservation::Complete | OpsObservation::Stale
                )
            });
        let assigned: BTreeSet<_> = self
            .squads
            .iter()
            .flat_map(|squad| squad.members.iter())
            .collect();
        self.members
            .values()
            .filter(move |member| complete && !assigned.contains(&member.id))
    }

    fn attention(&self, squad: &OpsSquad) -> u8 {
        let states = || {
            squad
                .members
                .iter()
                .filter_map(|id| squad.member_states.get(id))
        };
        if states().any(|member| member.waiting) {
            2
        } else if states().any(|member| member.state.as_deref() == Some("blocked")) {
            1
        } else {
            0
        }
    }

    pub(in crate::board) fn contains(&self, target: &OpsTarget) -> bool {
        match target {
            OpsTarget::Pool => true,
            OpsTarget::Squad(id) => self.squads.iter().any(|squad| squad.room_id == *id),
            OpsTarget::Member {
                room_id: Some(room),
                identity_id,
            } => {
                self.members.contains_key(identity_id)
                    && self
                        .squads
                        .iter()
                        .any(|squad| squad.room_id == *room && squad.members.contains(identity_id))
            }
            OpsTarget::Member {
                room_id: None,
                identity_id,
            } => self.pool().any(|member| member.id == *identity_id),
        }
    }

    pub(in crate::board) fn complete(&self) -> bool {
        self.membership_complete
            && self.observation == OpsObservation::Complete
            && self
                .squads
                .iter()
                .all(|squad| squad.observation == OpsObservation::Complete)
    }
}

/// One row per identity, independent of squad expansion and duplicate membership.
/// Retain a real squad occurrence for the existing management admission lane.
pub(in crate::board) fn identity_rows(
    snapshot: &observe::OpsWorkspaceSnapshot,
    query: &str,
    preferred: Option<&OpsTarget>,
    visibility: &OpsVisibility,
) -> Vec<OpsTreeRow> {
    identity_rows_matching(snapshot, query, preferred, visibility, |_| true)
}

/// Apply a destination's evidence predicate before choosing one UUID occurrence.
/// An ineligible preferred membership cannot hide another eligible occurrence.
pub(in crate::board) fn identity_rows_matching(
    snapshot: &observe::OpsWorkspaceSnapshot,
    query: &str,
    preferred: Option<&OpsTarget>,
    visibility: &OpsVisibility,
    admit: impl Fn(&OpsTreeRow) -> bool,
) -> Vec<OpsTreeRow> {
    let query = query.to_lowercase();
    // Canonical roster projection owns membership and unknown-pool admission.
    // Deduplicate its occurrences once; stale row documents grant no authority.
    let workspace = OpsWorkspace {
        expanded: snapshot
            .roster
            .squads
            .iter()
            .map(|squad| squad.room_id.clone())
            .collect(),
        pool_expanded: true,
        visibility: visibility.clone(),
        ..Default::default()
    };
    let mut identities: std::collections::BTreeMap<String, OpsTreeRow> = Default::default();
    for mut row in workspace.rows(&snapshot.roster, &snapshot.preferences) {
        let OpsTarget::Member { identity_id, .. } = &row.target else {
            continue;
        };
        if !snapshot.rows.contains_key(&row.target)
            || !row.title.to_lowercase().contains(&query)
            || !admit(&row)
        {
            continue;
        }
        let id = identity_id.clone();
        let replace = identities.get(&id).is_none_or(|current| {
            Some(&row.target) == preferred
                || (Some(&current.target) != preferred && row.target < current.target)
        });
        if replace {
            row.depth = 0;
            identities.insert(id, row);
        }
    }
    let mut rows: Vec<_> = identities.into_values().collect();
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

impl OpsWorkspace {
    /// Pin is a Config preference; virtual pool and member rows cannot be pinned.
    pub fn toggle_pin(&self, preferences: &mut OpsPreferences) -> bool {
        let Some(OpsTarget::Squad(id)) = &self.selected else {
            return false;
        };
        if !preferences.pinned.remove(id) {
            preferences.pinned.insert(id.clone());
        }
        true
    }

    pub fn toggle_expanded(&mut self, target: &OpsTarget) -> bool {
        match target {
            OpsTarget::Pool => self.pool_expanded = !self.pool_expanded,
            OpsTarget::Squad(id) => {
                if !self.expanded.remove(id) {
                    self.expanded.insert(id.clone());
                }
            }
            OpsTarget::Member { .. } => return false,
        }
        true
    }

    pub fn selection_state(&self, roster: &OpsRoster, rows: &[OpsTreeRow]) -> SelectionState {
        let Some(selected) = &self.selected else {
            return SelectionState::None;
        };
        if rows.iter().any(|row| row.target == *selected) {
            SelectionState::Visible
        } else if roster.contains(selected) {
            SelectionState::Hidden
        } else if roster.complete() {
            SelectionState::Missing
        } else {
            SelectionState::Unknown
        }
    }

    pub fn rows(&self, roster: &OpsRoster, preferences: &OpsPreferences) -> Vec<OpsTreeRow> {
        self.project_rows(roster, preferences, &self.query, false)
    }

    /// Count identities, not their repeated memberships or collapsed rows.
    pub fn search_counts(
        &self,
        roster: &OpsRoster,
        preferences: &OpsPreferences,
    ) -> (usize, usize) {
        let count = |query: &str| {
            self.project_rows(roster, preferences, query, true)
                .into_iter()
                .filter_map(|row| match row.target {
                    OpsTarget::Member { identity_id, .. } => Some(identity_id),
                    _ => None,
                })
                .collect::<BTreeSet<_>>()
                .len()
        };
        (count(&self.query), count(""))
    }

    fn project_rows(
        &self,
        roster: &OpsRoster,
        preferences: &OpsPreferences,
        query: &str,
        expand: bool,
    ) -> Vec<OpsTreeRow> {
        let query = query.to_lowercase();
        let matches = |name: &str| name.to_lowercase().contains(&query);
        let pool: Vec<_> = roster.pool().collect();
        // Keep an unfiltered unknown/empty pool, but search only shows matching
        // identities and their containing group.
        let show_pool = self.visibility.scope.is_empty()
            && match self.filter {
                AgentsFilter::All => true,
                AgentsFilter::Pinned => false,
            }
            && (query.is_empty()
                || matches("Common pool")
                || pool.iter().any(|member| matches(&member.name)));
        let mut rows = Vec::new();
        if show_pool {
            rows.push(OpsTreeRow {
                target: OpsTarget::Pool,
                title: "Common pool".into(),
                depth: 0,
                lead: false,
                state: None,
                presence: None,
                usage: None,
                count: roster.complete().then_some(pool.len()),
                observation: if roster.complete() {
                    OpsObservation::Complete
                } else if roster.observation == OpsObservation::Complete {
                    OpsObservation::Partial
                } else {
                    roster.observation
                },
            });
        }
        if show_pool && (expand || self.pool_expanded || !query.is_empty()) {
            let mut pool = pool;
            pool.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
            for member in pool
                .into_iter()
                .filter(|member| matches("Common pool") || matches(&member.name))
            {
                rows.push(member_row(member, None, false, roster.observation, None));
            }
        }
        let mut squads: Vec<_> = roster
            .squads
            .iter()
            .filter(|squad| self.visibility.shows_squad(&squad.room_id, preferences))
            .collect();
        squads.sort_by(|a, b| {
            let pinned = preferences
                .pinned
                .contains(&b.room_id)
                .cmp(&preferences.pinned.contains(&a.room_id));
            if pinned != Ordering::Equal {
                return pinned;
            }
            let order = match self
                .order
                .unwrap_or_else(|| crate::board::view_state::Order::from(preferences))
                .agents
            {
                OpsSort::Name => a.name.cmp(&b.name),
                OpsSort::Attention => roster.attention(b).cmp(&roster.attention(a)),
                OpsSort::Members => a.members.len().cmp(&b.members.len()),
            };
            let order = if self
                .order
                .unwrap_or_else(|| crate::board::view_state::Order::from(preferences))
                .reversed
            {
                order.reverse()
            } else {
                order
            };
            order.then_with(|| a.room_id.cmp(&b.room_id))
        });
        for squad in squads {
            if match self.filter {
                AgentsFilter::All => false,
                AgentsFilter::Pinned => !preferences.pinned.contains(&squad.room_id),
            } {
                continue;
            }
            let squad_matches = matches(&squad.name);
            let mut members: Vec<_> = roster.members(squad).collect();
            let mut seen = BTreeSet::new();
            members.retain(|member| seen.insert(&member.id));
            if !squad_matches && !members.iter().any(|member| matches(&member.name)) {
                continue;
            }
            let attention = match roster.attention(squad) {
                2 => Some("waiting"),
                1 => Some("blocked"),
                _ => squad
                    .member_states
                    .values()
                    .filter(|member| member.state.is_some())
                    .min_by_key(|member| member.rank)
                    .and_then(|member| member.state.as_deref()),
            };
            rows.push(OpsTreeRow {
                target: OpsTarget::Squad(squad.room_id.clone()),
                title: squad.name.clone(),
                depth: 0,
                lead: false,
                state: attention.map(str::to_owned),
                presence: None,
                usage: None,
                count: (squad.observation == OpsObservation::Complete)
                    .then(|| squad.members.iter().collect::<BTreeSet<_>>().len()),
                observation: squad.observation,
            });
            if expand || self.expanded.contains(&squad.room_id) || !query.is_empty() {
                let unknown = OpsMembershipState::default();
                members.sort_by(|a, b| {
                    (squad.lead.as_deref() != Some(&a.id))
                        .cmp(&(squad.lead.as_deref() != Some(&b.id)))
                        .then_with(|| a.name.cmp(&b.name))
                        .then_with(|| a.id.cmp(&b.id))
                });
                for member in members
                    .into_iter()
                    .filter(|member| squad_matches || matches(&member.name))
                {
                    rows.push(member_row(
                        member,
                        Some(&squad.room_id),
                        squad.lead.as_deref() == Some(&member.id),
                        squad.observation,
                        Some(squad.member_states.get(&member.id).unwrap_or(&unknown)),
                    ));
                }
            }
        }
        rows
    }
}

fn member_row(
    member: &OpsMember,
    room: Option<&str>,
    lead: bool,
    observation: OpsObservation,
    scope: Option<&OpsMembershipState>,
) -> OpsTreeRow {
    OpsTreeRow {
        target: OpsTarget::Member {
            room_id: room.map(str::to_owned),
            identity_id: member.id.clone(),
        },
        title: member.name.clone(),
        depth: 1,
        lead,
        state: if scope.map_or(member.waiting, |membership| membership.waiting) {
            Some("waiting".into())
        } else {
            scope.map_or_else(
                || member.state.clone(),
                |membership| membership.state.clone(),
            )
        },
        presence: member.presence,
        usage: member.usage,
        count: None,
        observation,
    }
}

#[cfg(test)]
mod tests;
