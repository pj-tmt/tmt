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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum BindingContext {
    Global,
    Host(String),
    Section(OpsTarget),
}
type BindingSources = BTreeMap<String, String>;

#[derive(Debug, Default)]
pub(crate) struct OpsWorkspaceSnapshot {
    pub roster: OpsRoster,
    pub shell_counts: ShellCounts,
    pub hidden_count: usize,
    pub hidden_attention: ShellCounts,
    /// Exact admitted row projections used by the existing action/conversation
    /// adapters. Display models never reconstruct send data from visible text.
    pub rows: BTreeMap<OpsTarget, Value>,
    pub preferences: OpsPreferences,
    pub read_markers: crate::config::ReadMarkers,
    pub squad_looks: BTreeMap<String, crate::look::Look>,
    pub keys: BTreeMap<OpsKeyScope, Bindings>,
    pub hosts: BTreeMap<String, BTreeMap<OpsKeyScope, Bindings>>,
    pub sections: BTreeMap<OpsTarget, BTreeMap<OpsKeyScope, Bindings>>,
    pub config_path: String,
    pub binding_sources: BTreeMap<(OpsKeyScope, BindingContext), BindingSources>,
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

fn pool_row(row: &Value) -> Value {
    let mut projected = row.clone();
    if let Some(pane) = crate::squad::observed_pane(row) {
        projected["pane"] = pane;
    }
    projected
}

impl OpsWorkspaceSnapshot {
    pub(crate) fn set_preferences(&mut self, preferences: OpsPreferences) {
        self.hidden_count = self
            .roster
            .squads
            .iter()
            .filter(|squad| preferences.hidden.contains(&squad.room_id))
            .count();
        self.hidden_attention = self.roster.hidden_counts(&preferences.hidden);
        self.preferences = preferences;
    }

    pub(crate) fn configure(&mut self, config: &Config) -> Result<(), SquadError> {
        self.shell_counts = self.roster.shell_counts();
        self.set_preferences(config.workspace_preferences()?);
        self.read_markers = config.read_markers()?;
        self.config_path = config.path().display().to_string();
        let scopes = [
            OpsKeyScope::Overview,
            OpsKeyScope::Squad,
            OpsKeyScope::Member,
            OpsKeyScope::Pool,
            OpsKeyScope::Reader,
            OpsKeyScope::OverviewReader,
        ];
        for scope in scopes {
            let keys = config.workspace_keys(scope, None, None)?;
            self.binding_sources
                .insert((scope, BindingContext::Global), keys.sources);
            self.keys.insert(scope, keys.bindings);
            self.notices.extend(keys.notices);
        }
        for host in config.host_bindings()?.into_keys() {
            let mut tables = BTreeMap::new();
            for scope in scopes {
                let keys = config.workspace_keys(scope, Some(&host), None)?;
                self.binding_sources
                    .insert((scope, BindingContext::Host(host.clone())), keys.sources);
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
                if let Some((index, section)) = sections
                    .iter()
                    .enumerate()
                    .find(|(_, section)| section.includes(row))
                {
                    let host = row["pane"]["location"]["host"].as_str();
                    let mut tables = BTreeMap::new();
                    for scope in scopes {
                        let mut keys = config.workspace_keys(scope, host, Some(&section.bind))?;
                        for (key, source) in &mut keys.sources {
                            if source == "selected squad section.bind" {
                                *source =
                                    format!("squad.{}.section[{index}].bind.{key}", squad.name);
                            }
                        }
                        self.binding_sources.insert(
                            (scope, BindingContext::Section(target.clone())),
                            keys.sources,
                        );
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
                        pool_row(row),
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
                        observed_member_row(
                            row,
                            result.rows.get(&OpsTarget::Member {
                                room_id: None,
                                identity_id: identity.id.clone(),
                            }),
                        ),
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

fn observed_member_row(row: &Value, observed: Option<&Value>) -> Value {
    let mut projected = row.clone();
    // HOME's roster-only acquisition does not observe panes. Reuse the same
    // global observation as the workspace presence, retaining squad metadata.
    projected["presence"] = observed
        .and_then(|row| row.get("presence"))
        .cloned()
        .unwrap_or_else(|| Value::String("unknown".into()));
    projected["pane"] = observed
        .and_then(|row| row.get("pane"))
        .cloned()
        .unwrap_or(Value::Null);
    projected
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pool_uses_the_same_observed_pane_as_squad_rows_for_host_actions() {
        let mut listed = json!({"id":"uuid", "name":"worker", "presence":"active",
            "pane":"%7", "target":"work:2.1", "cwd":"/work",
            "location":{"host":"tmux", "session":"work", "window":"2"}});
        let row = pool_row(&listed);
        assert_eq!(row["pane"]["id"], "%7");
        assert_eq!(row["pane"]["location"], listed["location"]);
        assert_eq!(
            crate::board::overview::actions::host(&row, true),
            Ok("uuid")
        );
        listed["location"]["host"] = json!("herdr");
        assert!(crate::board::overview::actions::host(&pool_row(&listed), true).is_err());
        listed.as_object_mut().unwrap().remove("location");
        assert!(pool_row(&listed)["pane"].get("location").is_none());
        assert!(crate::board::overview::actions::host(&pool_row(&listed), true).is_err());
        listed["pane"] = Value::Null;
        assert!(pool_row(&listed)["pane"].is_null());
    }

    #[test]
    fn squad_actions_reuse_global_presence_without_replacing_squad_state() {
        let roster = json!({"id":"uuid", "state":"blocked", "presence":"unknown",
            "pane":null, "fields":{"task":"keep squad task"}});
        let observed = pool_row(&json!({"id":"uuid", "presence":"active", "pane":"%7",
            "location":{"host":"tmux"}, "state":"working"}));
        let row = observed_member_row(&roster, Some(&observed));
        assert_eq!(
            crate::board::overview::actions::host(&row, true),
            Ok("uuid")
        );
        assert_eq!(row["state"], "blocked");
        assert_eq!(row["fields"], roster["fields"]);
        let absent = observed_member_row(&row, None);
        assert_eq!(absent["presence"], "unknown");
        assert!(absent["pane"].is_null());
        assert!(crate::board::overview::actions::host(&absent, true).is_err());
    }
}
