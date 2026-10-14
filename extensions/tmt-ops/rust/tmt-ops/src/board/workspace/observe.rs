//! Projection of already acquired roster rows and global identities. No second
//! Core read or authoritative membership store belongs to this module.
use super::*;
use crate::{
    action::Bindings,
    config::{Config, OpsKeyScope},
    core::SquadError,
};
use crate::{squad::Squad, tab_view::Acquired};
use serde_json::Value;

#[derive(Debug, Default)]
pub(crate) struct OpsWorkspaceSnapshot {
    pub roster: OpsRoster,
    /// Exact admitted row projections used by the existing action/conversation
    /// adapters. Display models never reconstruct send data from visible text.
    pub rows: BTreeMap<OpsTarget, Value>,
    pub preferences: OpsPreferences,
    pub squad_looks: BTreeMap<String, crate::look::Look>,
    pub keys: BTreeMap<OpsKeyScope, Bindings>,
    pub hosts: BTreeMap<String, BTreeMap<OpsKeyScope, Bindings>>,
    pub sections: BTreeMap<OpsTarget, BTreeMap<OpsKeyScope, Bindings>>,
    pub notices: Vec<String>,
}

fn member(row: &Value) -> Option<OpsMember> {
    Some(OpsMember {
        id: row["id"].as_str()?.into(),
        name: row["name"].as_str()?.into(),
        state: row["state"].as_str().map(str::to_owned),
        waiting: crate::attention::waits_on_you(row),
        presence: match row["presence"].as_str() {
            Some("active") => Some(true),
            Some("offline") => Some(false),
            _ => None,
        },
        usage: None, // Source-provided formatted usage stays in the row projection.
    })
}

impl OpsWorkspaceSnapshot {
    pub(crate) fn configure(&mut self, config: &Config) -> Result<(), SquadError> {
        self.preferences = config.workspace_preferences()?;
        let scopes = [
            OpsKeyScope::Overview,
            OpsKeyScope::Squad,
            OpsKeyScope::Member,
            OpsKeyScope::Pool,
            OpsKeyScope::Reader,
        ];
        for scope in scopes {
            let keys = config.workspace_keys(scope, None, None)?;
            self.keys.insert(scope, keys.bindings);
            self.notices.extend(keys.notices);
        }
        for host in config.host_bindings()?.into_keys() {
            let mut tables = BTreeMap::new();
            for scope in scopes {
                let keys = config.workspace_keys(scope, Some(&host), None)?;
                tables.insert(scope, keys.bindings);
                self.notices.extend(keys.notices);
            }
            self.hosts.insert(host, tables);
        }
        for squad in &self.roster.squads {
            let (theme, notice) = config.theme(&squad.name)?;
            self.squad_looks
                .insert(squad.room_id.clone(), crate::look::Look::new(theme));
            if let Some(notice) = notice {
                self.notices.push(notice);
            }
            let sections = config.sections(&squad.name)?;
            for (target, row) in &self.rows {
                if !matches!(target, OpsTarget::Member {room_id: Some(id), ..} if id == &squad.room_id)
                {
                    continue;
                }
                if let Some(section) = sections.iter().find(|section| section.includes(row)) {
                    let host = row["pane"]["location"]["host"].as_str();
                    let mut tables = BTreeMap::new();
                    for scope in scopes {
                        let keys = config.workspace_keys(scope, host, Some(&section.bind))?;
                        tables.insert(scope, keys.bindings);
                        self.notices.extend(keys.notices);
                    }
                    self.sections.insert(target.clone(), tables);
                }
            }
        }
        self.notices.sort();
        self.notices.dedup();
        Ok(())
    }
    pub(crate) fn from_acquired(
        squads: &[&Squad],
        acquired: &Acquired,
        listed: &Result<Value, crate::core::SquadError>,
    ) -> Self {
        let mut result = Self::default();
        result.rows.insert(
            OpsTarget::Pool,
            serde_json::json!({"name":"Common pool","kind":"pool"}),
        );
        let identities = listed
            .as_ref()
            .ok()
            .and_then(|value| value["identities"].as_array());
        result.roster.observation = if identities.is_some_and(|rows| {
            rows.iter()
                .all(|row| row["retired"] == true || member(row).is_some())
        }) {
            OpsObservation::Complete
        } else {
            OpsObservation::Unavailable
        };
        result.roster.membership_complete =
            acquired.documents.len() == squads.len() && acquired.failures.is_empty();
        if let Some(identities) = identities {
            for row in identities {
                if row["retired"] == true {
                    continue;
                }
                if let Some(identity) = member(row) {
                    result.rows.insert(
                        OpsTarget::Member {
                            room_id: None,
                            identity_id: identity.id.clone(),
                        },
                        row.clone(),
                    );
                    result.roster.members.insert(identity.id.clone(), identity);
                }
            }
        }
        for squad in squads {
            let document = acquired.documents.get(&squad.name);
            let room = acquired.rooms.get(&squad.name);
            let room_admitted = room.is_some_and(|room| {
                room["id"] == squad.room_id
                    && room["name"] == crate::squad::room_name(&squad.name)
                    && room["retired"] == false
                    && room["revision"]
                        .as_u64()
                        .is_some_and(|revision| revision > 0)
                    && room["memberIds"].as_array().is_some_and(|ids| {
                        ids.iter()
                            .all(|id| id.as_str().is_some_and(crate::config::uuid_like))
                            && ids
                                .iter()
                                .enumerate()
                                .all(|(index, id)| !ids[..index].contains(id))
                    })
            });
            result.roster.membership_complete &= room_admitted;
            let observation = if document.is_some() && room_admitted {
                OpsObservation::Complete
            } else if document.is_some() {
                OpsObservation::Partial
            } else {
                OpsObservation::Unavailable
            };
            let lead = document
                .and_then(|document| document["squad"]["lead"]["id"].as_str())
                .map(str::to_owned);
            let mut projected = OpsSquad {
                room_id: squad.room_id.clone(),
                name: squad.name.clone(),
                members: if room_admitted {
                    room.unwrap()["memberIds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|id| id.as_str().map(str::to_owned))
                        .collect()
                } else {
                    Vec::new()
                },
                lead,
                member_states: BTreeMap::new(),
                observation,
            };
            result.rows.insert(OpsTarget::Squad(squad.room_id.clone()), serde_json::json!({"name":squad.name,"squad":squad.name,"roomId":squad.room_id,"revision":room.map(|room| &room["revision"]),"memberIds":room.map(|room| &room["memberIds"]),"kind":"squad","lead":document.map(|value| &value["squad"]["lead"])}));
            for (row, rank) in acquired.projected_rows(&squad.name) {
                if let Some(identity) = member(row) {
                    projected.members.push(identity.id.clone());
                    projected.member_states.insert(
                        identity.id.clone(),
                        OpsMembershipState {
                            state: identity.state.clone(),
                            waiting: identity.waiting,
                            rank: Some(rank),
                        },
                    );
                    result.rows.insert(
                        OpsTarget::Member {
                            room_id: Some(squad.room_id.clone()),
                            identity_id: identity.id.clone(),
                        },
                        row.clone(),
                    );
                    // Presence belongs to the global identity observation, not
                    // the roster's unknown placeholder for that same identity.
                    result
                        .roster
                        .members
                        .entry(identity.id.clone())
                        .or_insert(identity);
                }
            }
            projected.members.sort();
            projected.members.dedup();
            result.roster.squads.push(projected);
        }
        result
    }
}
