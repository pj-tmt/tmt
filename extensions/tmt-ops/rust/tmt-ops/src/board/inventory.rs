//! Global identity observations acquired once by the refresh worker. Message
//! completion and conversation metadata consume the same snapshot.
use std::collections::BTreeMap;
use tmt_tui::components::Candidate;

/// Worker-acquired global references, including agents without a squad. IDs
/// come from Core identities and squad rooms, never display-name aliases.
#[derive(Clone, Debug, Default)]
pub(super) struct Inventory {
    pub candidates: Vec<Candidate>,
    pub agents: BTreeMap<String, Agent>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Agent {
    pub runtime: Option<String>,
    pub model: Option<String>,
}

impl Inventory {
    pub(super) fn from_listed(
        squads: &[crate::squad::Squad],
        listed: &Result<serde_json::Value, crate::core::SquadError>,
    ) -> Self {
        let mut candidates = BTreeMap::new();
        let mut agents = BTreeMap::new();
        for squad in squads {
            let id = format!("squad:{}", squad.room_id);
            candidates.insert(
                id.clone(),
                Candidate {
                    id,
                    kind: "squad".into(),
                    label: squad.name.clone(),
                    insertion: format!("@{}", squad.name),
                },
            );
        }
        if let Ok(listed) = listed {
            for row in listed["identities"].as_array().into_iter().flatten() {
                let (Some(id), Some(name)) = (row["id"].as_str(), row["name"].as_str()) else {
                    continue;
                };
                let text = |value: &serde_json::Value| {
                    value
                        .as_str()
                        .filter(|text| !text.is_empty())
                        .map(str::to_owned)
                };
                agents.insert(
                    id.to_owned(),
                    Agent {
                        runtime: text(&row["runningDriver"]),
                        model: super::rate::resume_model(&row["resume"]).map(str::to_owned),
                    },
                );
                let id = format!("agent:{id}");
                candidates.insert(
                    id.clone(),
                    Candidate {
                        id,
                        kind: "agent".into(),
                        label: name.into(),
                        insertion: format!("@{name}"),
                    },
                );
            }
        }
        let mut candidates: Vec<_> = candidates.into_values().collect();
        candidates.sort_by(|a, b| (&a.label, &a.kind, &a.id).cmp(&(&b.label, &b.kind, &b.id)));
        Self {
            candidates,
            agents,
            error: listed.as_ref().err().map(ToString::to_string),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Inventory;
    use serde_json::json;

    #[test]
    fn agent_model_is_the_last_completed_turn_then_the_starting_hook_then_unknown() {
        let listed = Ok(json!({"identities": [
            {"id": "a", "name": "switched", "runningDriver": "claude",
             "resume": {"model": "start", "consumption": {"modelId": "later"}}},
            {"id": "b", "name": "no-turn-yet", "resume": {"model": "start"}},
            {"id": "c", "name": "unusable", "resume": {"consumption": {"modelId": ""}}},
            {"id": "d", "name": "no-resume"},
        ]}));
        let inventory = Inventory::from_listed(&[], &listed);
        let model = |id: &str| inventory.agents[id].model.as_deref();
        assert_eq!(model("a"), Some("later"));
        assert_eq!(model("b"), Some("start"));
        assert_eq!(model("c"), None);
        assert_eq!(model("d"), None);
        assert_eq!(inventory.agents["a"].runtime.as_deref(), Some("claude"));
    }
}
