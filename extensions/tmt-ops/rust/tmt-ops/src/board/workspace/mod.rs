//! Pure workspace projection over one acquired roster. This is not a membership
//! store: IDs and observations come from the worker, and no action performs I/O.

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
        // Common pool is always first, including unknown/empty observations.
        let mut rows = vec![OpsTreeRow {
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
        }];
        if self.pool_expanded {
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
            let squad_matches = matches(&squad.name);
            let mut members: Vec<_> = roster.members(squad).collect();
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
            if self.expanded.contains(&squad.room_id) {
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
