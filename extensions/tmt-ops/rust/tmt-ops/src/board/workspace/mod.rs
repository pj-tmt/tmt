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

#[derive(Debug, Default)]
pub struct OpsWorkspace {
    pub selected: Option<OpsTarget>,
    pub expanded: BTreeSet<String>,
    pub pool_expanded: bool,
    pub query: String,
    pub filter: AgentsFilter,
}

/// Session-local tree scope; it never changes membership or saved preferences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentsFilter {
    #[default]
    All,
    NeedsYou,
    Pinned,
}

impl AgentsFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::NeedsYou,
            Self::NeedsYou => Self::Pinned,
            Self::Pinned => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::NeedsYou => "Needs you",
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
        let mut waiting = BTreeSet::new();
        let mut blocked = BTreeSet::new();
        for member in self.members.values() {
            if member.waiting {
                waiting.insert(&member.id);
            }
            if member.state.as_deref() == Some("blocked") {
                blocked.insert(&member.id);
            }
        }
        for squad in &self.squads {
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
                .filter(|member| member.presence == Some(true))
                .count(),
            waiting: waiting.len(),
            blocked: blocked.len(),
            complete: self.complete()
                && self
                    .members
                    .values()
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
        let query = self.query.to_lowercase();
        let matches = |name: &str| name.to_lowercase().contains(&query);
        let pool: Vec<_> = roster.pool().collect();
        // All keeps the virtual pool first, including unknown/empty observations.
        // Attention filtering retains incomplete evidence instead of proving no match.
        let show_pool = match self.filter {
            AgentsFilter::All => true,
            AgentsFilter::NeedsYou => {
                !roster.complete()
                    || pool
                        .iter()
                        .any(|member| member.waiting || member.state.as_deref() == Some("blocked"))
            }
            AgentsFilter::Pinned => false,
        };
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
        if show_pool && (self.pool_expanded || !query.is_empty()) {
            let mut pool = pool;
            pool.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
            for member in pool.into_iter().filter(|member| matches(&member.name)) {
                rows.push(member_row(member, None, false, roster.observation, None));
            }
        }
        let mut squads: Vec<_> = roster.squads.iter().collect();
        squads.sort_by(|a, b| {
            let pinned = preferences
                .pinned
                .contains(&b.room_id)
                .cmp(&preferences.pinned.contains(&a.room_id));
            if pinned != Ordering::Equal {
                return pinned;
            }
            let order = match preferences.sort {
                OpsSort::Name => a.name.cmp(&b.name),
                OpsSort::Attention => roster.attention(b).cmp(&roster.attention(a)),
                OpsSort::Members => a.members.len().cmp(&b.members.len()),
            };
            let order = if preferences.reversed {
                order.reverse()
            } else {
                order
            };
            order.then_with(|| a.room_id.cmp(&b.room_id))
        });
        for squad in squads {
            if match self.filter {
                AgentsFilter::All => false,
                AgentsFilter::NeedsYou => {
                    squad.observation == OpsObservation::Complete && roster.attention(squad) == 0
                }
                AgentsFilter::Pinned => !preferences.pinned.contains(&squad.room_id),
            } {
                continue;
            }
            let squad_matches = matches(&squad.name)
                || squad
                    .lead
                    .as_ref()
                    .and_then(|id| roster.members.get(id))
                    .is_some_and(|lead| matches(&lead.name));
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
            if self.expanded.contains(&squad.room_id) || !query.is_empty() {
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
