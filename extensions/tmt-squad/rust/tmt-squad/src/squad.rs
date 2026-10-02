//! A squad is the core room `squad-<name>`; member fields are the identity
//! metadata keys `squad.<name>.<field>`. These remain authoritative; optional
//! observed ages live in the extension-owned disposable staleness cache.

use crate::{
    core::{Core, SquadError},
    filter::Row,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// Core metadata keys are at most 64 bytes.
const KEY_LIMIT: usize = 64;

pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=24).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn valid_field(field: &str) -> bool {
    let bytes = field.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes[1..].iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

#[derive(Debug, Clone)]
pub struct Squad {
    pub name: String,
    pub room_id: String,
}

/// A missing squad, named or not, with the command that creates one.
fn not_found(name: Option<&str>) -> SquadError {
    match name {
        Some(name) => SquadError::hinted(
            "SQUAD_NOT_FOUND",
            &format!("Squad '{name}' does not exist"),
            "; run: ",
            &format!("tmt squad init {name}"),
        ),
        None => SquadError::hinted(
            "SQUAD_NOT_FOUND",
            "No squad exists yet",
            "; run: ",
            "tmt squad init <name>",
        ),
    }
}

/// The core room that is a squad.
pub fn room_name(name: &str) -> String {
    format!("squad-{name}")
}

/// Reserved metadata suffix; neither user fields nor column/provider names allow dots.
pub const LEAD_MARKER: &str = "lead.marker";

impl Squad {
    pub fn lead_key(&self) -> String {
        format!("{}{LEAD_MARKER}", self.prefix())
    }

    pub fn prefix(&self) -> String {
        format!("squad.{}.", self.name)
    }

    /// The full metadata key for one member field, validated before any write.
    pub fn key(&self, field: &str) -> Result<String, SquadError> {
        let key = format!("{}{field}", self.prefix());
        if !valid_field(field) || key.len() > KEY_LIMIT {
            return Err(SquadError::new(
                "SQUAD_FIELD_INVALID",
                format!(
                    "Field '{field}' must match [a-z][a-z0-9_-]* and fit a {KEY_LIMIT}-byte key."
                ),
            ));
        }
        Ok(key)
    }

    /// An explicit name, or the only active squad room when omitted.
    pub fn resolve(core: &Core, explicit: Option<&str>) -> Result<Self, SquadError> {
        if let Some(name) = explicit {
            if !valid_name(name) {
                return Err(name_invalid(name));
            }
            return match core.json(&["room", "show", &room_name(name)]) {
                Ok(shown) => Ok(Self {
                    name: name.into(),
                    room_id: room_id(&shown["room"])?,
                }),
                Err(error) if error.code == "ROOM_NOT_FOUND" => Err(not_found(Some(name))),
                Err(error) if error.code == "ROOM_AMBIGUOUS" => Err(SquadError::new(
                    "SQUAD_AMBIGUOUS",
                    format!(
                        "More than one room is named {}; rename one with tmt room.",
                        room_name(name)
                    ),
                )),
                Err(error) => Err(error),
            };
        }
        let mut squads = Self::list(core)?;
        match squads.len() {
            1 => Ok(squads.remove(0)),
            0 => Err(not_found(None)),
            _ => Err(SquadError::new(
                "SQUAD_AMBIGUOUS",
                format!(
                    "Several squads exist ({}); choose one with --squad.",
                    squads
                        .iter()
                        .map(|squad| squad.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
        }
    }

    /// Every active squad room, in core's room order (by name).
    pub fn list(core: &Core) -> Result<Vec<Self>, SquadError> {
        let listed = core.json(&["room", "list"])?;
        listed["rooms"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|room| {
                let name = room["name"].as_str()?.strip_prefix("squad-")?;
                valid_name(name).then(|| {
                    Ok(Self {
                        name: name.into(),
                        room_id: room_id(room)?,
                    })
                })
            })
            .collect()
    }

    /// One roster snapshot joined with presence from `ls`, which owns host
    /// observation. A member missing from `ls` (joined in between) is unknown.
    /// `metadata` also reads identity metadata beyond this squad's fields,
    /// for columns bound to `meta.<key>`.
    pub fn members(&self, core: &Core, metadata: bool) -> Result<Vec<Member>, SquadError> {
        let mut members = self.roster_with(core, metadata)?;
        join_presence(&mut members, &core.json(&["ls", "--room", &self.room_id])?);
        Ok(members)
    }

    /// The roster alone: names, fields and activity, with presence unknown and
    /// no pane. Enough for a tab's attention without asking `ls` (#507).
    pub fn roster(&self, core: &Core) -> Result<Vec<Member>, SquadError> {
        self.roster_with(core, false)
    }

    fn roster_with(&self, core: &Core, metadata: bool) -> Result<Vec<Member>, SquadError> {
        let prefix = self.prefix();
        let input = if metadata {
            json!({"room": self.room_id})
        } else {
            json!({"room": self.room_id, "metadataPrefix": prefix})
        };
        let roster = core.api("rooms.roster", input)?;
        roster["members"]
            .as_array()
            .ok_or_else(|| {
                SquadError::new(
                    "SQUAD_CORE_UNAVAILABLE",
                    "rooms.roster returned no members.",
                )
            })?
            .iter()
            .map(|member| {
                let metadata = member["metadata"].as_object().into_iter().flatten();
                let (fields, meta): (Vec<_>, Vec<_>) = metadata
                    .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                    .partition(|(key, _)| key.starts_with(&prefix));
                let mut fields: BTreeMap<_, _> = fields
                    .into_iter()
                    .map(|(key, value)| (key[prefix.len()..].to_owned(), value))
                    .collect();
                let lead_marker = fields.remove(LEAD_MARKER).map(|marker| marker == "true");
                // Retired row notes remain in core metadata until explicitly cleared.
                // Exclude them before columns, filters and templates see the projection.
                fields.remove("note");
                Ok(Member {
                    lead_marker,
                    id: text(member, "id")?,
                    name: text(member, "name")?,
                    lifetime: text(member, "lifetime")?,
                    presence: "unknown".into(),
                    pane: Value::Null,
                    activity: member["status"].clone(),
                    fields,
                    meta: meta.into_iter().collect(),
                    seen: Value::Null,
                    numbers: BTreeMap::new(),
                    colors: BTreeMap::new(),
                    failed: Default::default(),
                })
            })
            .collect()
    }
}

/// Presence and pane from an `ls --json` document, for the members it lists.
pub fn join_presence(members: &mut [Member], listed: &Value) {
    let presence: HashMap<&str, &Value> = listed["identities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row["id"].as_str()?, row)))
        .collect();
    for member in members {
        let Some(seen) = presence.get(member.id.as_str()) else {
            continue;
        };
        member.seen = (*seen).clone();
        if let Some(state) = seen["presence"].as_str() {
            member.presence = state.into();
        }
        if seen["pane"].is_string() {
            member.pane =
                json!({"id": seen["pane"], "target": seen.get("target"), "cwd": seen.get("cwd")});
        }
    }
}

#[derive(Debug, Clone)]
pub struct Member {
    /// Reserved ownership metadata, outside public fields; None is legacy state.
    pub lead_marker: Option<bool>,
    pub id: String,
    pub name: String,
    pub lifetime: String,
    pub presence: String,
    pub pane: Value,
    pub activity: Value,
    pub fields: BTreeMap<String, String>,
    /// Identity metadata outside this squad's fields; read only when a
    /// column is bound to `meta.<key>`.
    pub meta: BTreeMap<String, String>,
    /// The member's `ls --json` row, when `ls` listed it; Null otherwise.
    pub seen: Value,
    /// Bound columns' numeric values, so sorts order `1.2M` after `487k`.
    pub numbers: BTreeMap<String, f64>,
    /// Each cell's color token, by field: a provider's suggestion, then the
    /// column's threshold over it. Resolved by `status::document`.
    pub colors: BTreeMap<String, String>,
    /// Provided fields whose last run failed; they show as `?`.
    pub failed: std::collections::BTreeSet<String>,
}

/// Filterable values: identity basics, self-reported activity and every
/// `squad.<name>.*` field.
impl Row for Member {
    fn value(&self, field: &str) -> Option<&str> {
        match field {
            "name" => Some(&self.name),
            "presence" => Some(&self.presence),
            "lifetime" => Some(&self.lifetime),
            "activity" => self.activity["activity"].as_str(),
            _ => self.fields.get(field).map(String::as_str),
        }
    }
}

/// Reserved raw metadata decides leadership; ordinary fields cannot override it.
pub fn lead_from_fields(fields: &BTreeMap<String, String>, marker: Option<bool>) -> bool {
    marker.unwrap_or_else(|| fields.get("role").is_some_and(|role| role == "lead"))
}

impl Member {
    pub fn is_lead(&self) -> bool {
        lead_from_fields(&self.fields, self.lead_marker)
    }
}

pub fn name_invalid(name: &str) -> SquadError {
    SquadError::new(
        "SQUAD_NAME_INVALID",
        format!("Squad name '{name}' must match [a-z][a-z0-9-]{{0,23}}."),
    )
}

fn room_id(room: &Value) -> Result<String, SquadError> {
    text(room, "id")
}

fn text(value: &Value, key: &str) -> Result<String, SquadError> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| SquadError::new("SQUAD_CORE_UNAVAILABLE", format!("tmt returned no {key}.")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_squad_keeps_its_json_message_and_splits_the_command_for_people() {
        let named = not_found(Some("product"));
        assert_eq!(
            named.to_json().to_string(),
            r#"{"error":{"code":"SQUAD_NOT_FOUND","message":"Squad 'product' does not exist; run: tmt squad init product"}}"#
        );
        assert_eq!(
            named.human(),
            (
                "Squad 'product' does not exist",
                Some("tmt squad init product")
            )
        );
        assert_eq!(
            not_found(None).to_json().to_string(),
            r#"{"error":{"code":"SQUAD_NOT_FOUND","message":"No squad exists yet; run: tmt squad init <name>"}}"#
        );
    }

    /// A stand-in tmt: `api` logs its request and answers one roster member
    /// with a squad field and another key; `ls` lists that member.
    fn roster_core(name: &str) -> (Core, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("squad-roster-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("tmt");
        crate::test_support::write_executable(
            &fake,
            &format!(
                "#!/bin/sh
case \"$1\" in\n\
                 api) cat > '{requests}'; echo '{{\"members\":[{{\"id\":\"R\",\"name\":\"rin\",\"lifetime\":\"saved\",\"status\":null,\"metadata\":{{\"squad.p.state\":\"working\",\"team.role\":\"reviewer\"}}}}]}}' ;;\n\
                 ls) echo '{{\"identities\":[{{\"id\":\"R\",\"presence\":\"active\",\"pane\":\"%1\",\"cwd\":\"/src\",\"resume\":{{\"model\":\"opus\"}}}}]}}' ;;\n\
                 esac\n",
                requests = dir.join("request").display()
            ),
        );
        (Core::at(fake), dir)
    }

    #[test]
    fn members_carry_the_ls_row_and_read_other_metadata_only_when_asked() {
        let squad = Squad {
            name: "p".into(),
            room_id: "room".into(),
        };
        for metadata in [false, true] {
            let (core, dir) = roster_core(&format!("{metadata}"));
            let members = squad.members(&core, metadata).unwrap();
            let request: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("request")).unwrap())
                    .unwrap();
            let rin = &members[0];
            assert_eq!(
                rin.fields,
                BTreeMap::from([("state".into(), "working".into())])
            );
            assert_eq!(rin.presence, "active");
            assert_eq!(rin.seen["resume"]["model"], "opus");
            assert_eq!(rin.seen["cwd"], "/src");
            if metadata {
                assert_eq!(request["input"], json!({"room": "room"}), "every key");
                assert_eq!(
                    rin.meta,
                    BTreeMap::from([("team.role".into(), "reviewer".into())])
                );
            } else {
                assert_eq!(request["input"]["metadataPrefix"], "squad.p.");
            }
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn names_and_fields_fit_core_metadata_keys() {
        for name in ["a", "product", "pr-queue-2", &"x".repeat(24)] {
            assert!(valid_name(name), "{name}");
        }
        for name in ["", "Product", "1a", "a_b", "a.b", "-a", &"x".repeat(25)] {
            assert!(!valid_name(name), "{name}");
        }
        let squad = Squad {
            name: "x".repeat(24),
            room_id: String::new(),
        };
        assert_eq!(
            squad.key("state").unwrap(),
            format!("squad.{}.state", "x".repeat(24))
        );
        // "squad." + 24 + "." leaves 33 bytes for a field.
        assert!(squad.key(&"f".repeat(33)).is_ok());
        for field in [&"f".repeat(34), "", "State", "a.b", "a b", "9a"] {
            assert_eq!(
                squad.key(field).unwrap_err().code,
                "SQUAD_FIELD_INVALID",
                "{field}"
            );
        }
    }
}
