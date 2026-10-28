//! A narrow cross-machine projection, never the owner's full board document.
use crate::{error::RemoteError, limits};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Working,
    Waiting,
    WaitingCi,
    Idle,
    Resting,
    Blocked,
    Review,
    Testing,
    Hold,
    #[default]
    #[serde(other)]
    Unknown,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemberPresence {
    Active,
    Offline,
    #[default]
    #[serde(other)]
    Unknown,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub state: State,
    #[serde(default)]
    pub presence: MemberPresence,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Squad {
    pub squad: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead: Option<Member>,
    pub members: Vec<Member>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub version: u64,
    pub machine_name: String,
    pub observed_at: u64,
    #[serde(flatten)]
    pub observation: SquadObservation,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum SquadObservation {
    Available { squads: Vec<Squad> },
    Unavailable,
}
fn invalid() -> RemoteError {
    RemoteError::new("REMOTE_SUMMARY_INVALID", "Invalid machine squad summary.")
}
fn name(value: &Value) -> Result<String, RemoteError> {
    value
        .as_str()
        .filter(|s| {
            !s.trim().is_empty()
                && s.len() <= limits::SQUAD_SUMMARY_NAME_BYTES
                && !s.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        })
        .map(str::to_owned)
        .ok_or_else(invalid)
}
fn member(row: &Value) -> Result<Member, RemoteError> {
    Ok(Member {
        id: row["id"]
            .as_str()
            .filter(|id| crate::canonical::is_core_id(id))
            .ok_or_else(invalid)?
            .into(),
        name: name(&row["name"])?,
        // Only the public state field is eligible. Never consult activity or mood.
        state: match row["state"].as_str() {
            Some("working") => State::Working,
            Some("waiting") => State::Waiting,
            Some("waiting-ci") => State::WaitingCi,
            Some("idle") => State::Idle,
            Some("resting") => State::Resting,
            Some("blocked") => State::Blocked,
            Some("review") => State::Review,
            Some("testing") => State::Testing,
            Some("hold") => State::Hold,
            _ => State::Unknown,
        },
        presence: match row["presence"].as_str() {
            Some("active") => MemberPresence::Active,
            Some("offline") => MemberPresence::Offline,
            _ => MemberPresence::Unknown,
        },
    })
}
impl Summary {
    /// `observed_at` is captured after the public CLI read succeeds, not when a viewer reads it.
    pub fn from_ops(
        machine_name: &str,
        observed_at: u64,
        board: &Value,
    ) -> Result<Self, RemoteError> {
        let documents = board["squads"].as_array().ok_or_else(invalid)?;
        if documents.len() > limits::SQUAD_SUMMARY_SQUADS {
            return Err(invalid());
        }
        let mut squads = Vec::new();
        let mut count = 0;
        for document in documents {
            let head = &document["squad"];
            let lead_value = head.get("lead").ok_or_else(invalid)?;
            let lead = if lead_value.is_null() {
                None
            } else {
                Some(member(&head["lead"])?)
            };
            let mut members = Vec::new();
            count += usize::from(lead.is_some());
            for section in document["sections"].as_array().ok_or_else(invalid)? {
                for row in section["rows"].as_array().ok_or_else(invalid)? {
                    count += 1;
                    if count > limits::SQUAD_SUMMARY_MEMBERS {
                        return Err(invalid());
                    }
                    members.push(member(row)?);
                }
            }
            squads.push(Squad {
                squad: name(&head["name"])?,
                lead,
                members,
            });
        }
        let summary = Self {
            version: 1,
            machine_name: machine_name.into(),
            observed_at,
            observation: SquadObservation::Available { squads },
        };
        summary.validate()?;
        Ok(summary)
    }
    /// A failed public read is a publishable observation, never an empty available roster.
    pub fn unavailable(machine_name: &str, observed_at: u64) -> Result<Self, RemoteError> {
        let summary = Self {
            version: 1,
            machine_name: machine_name.into(),
            observed_at,
            observation: SquadObservation::Unavailable,
        };
        summary.validate()?;
        Ok(summary)
    }
    pub fn squads(&self) -> Option<&[Squad]> {
        match &self.observation {
            SquadObservation::Available { squads } => Some(squads),
            SquadObservation::Unavailable => None,
        }
    }
    /// Reading drops unlisted fields and normalizes unknown enum strings before any reserialization.
    pub fn read(bytes: &[u8]) -> Result<Self, RemoteError> {
        if bytes.len() > limits::SQUAD_SUMMARY_BYTES {
            return Err(invalid());
        }
        let mut value = crate::wire::strict_json(bytes).ok_or_else(invalid)?;
        // The receiver applies the same enum allowlists to every member, even non-string values.
        if value.get("status").and_then(Value::as_str) == Some("available") {
            let squads = value
                .get_mut("squads")
                .and_then(Value::as_array_mut)
                .ok_or_else(invalid)?;
            for squad in squads {
                let squad = squad.as_object_mut().ok_or_else(invalid)?;
                if let Some(lead) = squad.get_mut("lead").filter(|lead| !lead.is_null()) {
                    normalize(lead)?;
                }
                for row in squad
                    .get_mut("members")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(invalid)?
                {
                    normalize(row)?;
                }
            }
        }
        let summary: Self = serde_json::from_value(value).map_err(|_| invalid())?;
        summary.validate()?;
        Ok(summary)
    }
    /// Future timestamps cannot prove freshness; the consumer retains that uncertainty.
    pub fn age_ms(&self, now: u64) -> Option<u64> {
        now.checked_sub(self.observed_at)
    }
    pub fn bytes(&self) -> Result<Vec<u8>, RemoteError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| invalid())
    }
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.version != 1
            || self.observed_at > crate::cloud_hpke::INTEGER_MAX
            || self.squads().map_or(0, <[Squad]>::len) > limits::SQUAD_SUMMARY_SQUADS
        {
            return Err(invalid());
        }
        name(&Value::String(self.machine_name.clone()))?;
        let mut count = 0usize;
        for squad in self.squads().unwrap_or_default() {
            let mut ids = std::collections::HashSet::new();
            name(&Value::String(squad.squad.clone()))?;
            for row in squad.lead.iter().chain(&squad.members) {
                if !crate::canonical::is_core_id(&row.id) || !ids.insert(&row.id) {
                    return Err(invalid());
                }
                name(&Value::String(row.name.clone()))?;
                count = count.checked_add(1).ok_or_else(invalid)?;
                if count > limits::SQUAD_SUMMARY_MEMBERS {
                    return Err(invalid());
                }
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|_| invalid())?;
        if bytes.len() > limits::SQUAD_SUMMARY_BYTES {
            return Err(invalid());
        }
        Ok(())
    }
}

fn normalize(row: &mut Value) -> Result<(), RemoteError> {
    let parsed = member(row)?;
    *row = serde_json::to_value(parsed).map_err(|_| invalid())?;
    Ok(())
}
