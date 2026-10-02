//! Membership changes through public core commands. Each command is idempotent
//! and reports exactly what it applied; a re-run converges after a failure.

use crate::{
    config::{Config, Layout},
    core::{Core, SquadError},
    squad::{self, Squad, name_invalid, valid_name},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Mirrors core's public metadata entry limit; Squad reaches core only through commands.
const METADATA_ENTRY_LIMIT: usize = 64;

/// A command result plus whether every requested change was applied.
pub struct Outcome {
    pub document: Value,
    pub complete: bool,
}

impl From<Value> for Outcome {
    fn from(document: Value) -> Self {
        Self {
            document,
            complete: true,
        }
    }
}

struct Resolved {
    id: String,
    name: String,
}

fn identity(core: &Core, name: &str) -> Result<(Resolved, Value), SquadError> {
    let shown = core.json(&["identity", "show", name])?;
    let record = &shown["identity"];
    match (record["id"].as_str(), record["name"].as_str()) {
        (Some(id), Some(display)) => Ok((
            Resolved {
                id: id.into(),
                name: display.into(),
            },
            record.clone(),
        )),
        _ => Err(SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "tmt identity show returned no identity.",
        )),
    }
}

fn saved(core: &Core, name: &str) -> Result<Resolved, SquadError> {
    let (found, record) = identity(core, name)?;
    if record["lifetime"] != "saved" {
        return Err(SquadError::new(
            "SQUAD_IDENTITY_NOT_SAVED",
            format!(
                "'{}' is temporary; save it first with tmt identity create {} (or tmt name -s).",
                found.name, found.name
            ),
        ));
    }
    Ok(found)
}

fn fields(core: &Core, squad: &Squad, id: &str) -> Result<BTreeMap<String, String>, SquadError> {
    let listed = core.json(&["identity", "meta", "list", "--identity", id])?;
    Ok(listed["metadata"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            Some((
                key.strip_prefix(&squad.prefix())?.to_owned(),
                value.as_str()?.to_owned(),
            ))
        })
        .collect())
}

/// Creates the squad's room and nothing else; it never asks. `--me` (for
/// scripts) is checked before any effect, so a bad name changes nothing.
pub fn init(
    core: &Core,
    config: &mut Config,
    name: &str,
    me: Option<&str>,
) -> Result<Outcome, SquadError> {
    if !valid_name(name) {
        return Err(name_invalid(name));
    }
    let chosen = me.map(|requested| saved(core, requested)).transpose()?;
    let room = squad::room_name(name);
    let (shown, created) = match core.json(&["room", "show", &room]) {
        Ok(shown) => (shown, false),
        Err(error) if error.code == "ROOM_NOT_FOUND" => {
            (core.json(&["room", "create", &room])?, true)
        }
        Err(error) => return Err(error),
    };
    if let Some(chosen) = chosen {
        crate::me::record(
            config,
            &crate::me::Me {
                id: chosen.id,
                name: chosen.name,
            },
        )?;
    }
    Ok(json!({
        "squad": {"name": name, "roomId": shown["room"]["id"]},
        "created": created,
        "me": config.me()?,
    })
    .into())
}

fn record_lead(core: &Core, squad: &Squad, id: &str, lead: bool) -> Result<(), SquadError> {
    core.json(&[
        "identity",
        "meta",
        "set",
        &squad.lead_key(),
        if lead { "true" } else { "false" },
        "--identity",
        id,
    ])?;
    Ok(())
}

pub fn lead(core: &Core, squad: &Squad, name: &str) -> Result<Outcome, SquadError> {
    let leader = saved(core, name)?;
    let members = squad.roster(core)?;
    let targets = std::iter::once(leader.id.as_str()).chain(
        members
            .iter()
            .filter(|member| member.is_lead() && member.id != leader.id)
            .map(|member| member.id.as_str()),
    );
    // Check all required markers before any write. A concurrent metadata write
    // can still invalidate this preflight; core commands are not a transaction.
    for id in targets {
        let listed = core.json(&["identity", "meta", "list", "--identity", id])?;
        if let Some(metadata) = listed["metadata"].as_object()
            && !metadata.contains_key(&squad.lead_key())
            && metadata.len() >= METADATA_ENTRY_LIMIT
        {
            return Err(SquadError::new(
                "IDENTITY_METADATA_INVALID",
                format!("An identity may have at most {METADATA_ENTRY_LIMIT} metadata entries."),
            ));
        }
    }
    record_lead(core, squad, &leader.id, true)?;
    core.json(&["room", "join", &squad.room_id, "--identity", &leader.id])?;
    // Set the new lead before clearing others, so a failure never leaves none.
    let mut replaced = Vec::new();
    for member in members {
        if member.is_lead() && member.id != leader.id {
            record_lead(core, squad, &member.id, false)?;
            replaced.push(member.name);
        }
    }
    Ok(json!({"squad": squad.name, "lead": {"id": leader.id, "name": leader.name}, "replaced": replaced}).into())
}

pub fn add(
    core: &Core,
    squad: &Squad,
    layout: Layout,
    names: &[String],
) -> Result<Outcome, SquadError> {
    let state = squad.key("state")?;
    let members = squad.roster(core)?;
    let mut complete = true;
    let results: Vec<Value> = names
        .iter()
        .map(|name| {
            let added = (|| {
                let (member, _) = identity(core, name)?;
                let current = fields(core, squad, &member.id)?;
                let already_joined = members.iter().any(|existing| existing.id == member.id);
                let marker = current
                    .get(squad::LEAD_MARKER)
                    .map(|marker| marker == "true");
                let derives_lead = squad::lead_from_fields(&current, marker);
                // Mask a new member's pre-existing lead data before joining;
                // at the metadata cap, failure leaves membership untouched.
                if !already_joined && derives_lead {
                    record_lead(core, squad, &member.id, false)?;
                }
                core.json(&["room", "join", &squad.room_id, "--identity", &member.id])?;
                let initial = layout
                    .states()
                    .first()
                    .copied()
                    .filter(|_| !current.contains_key("state"));
                if let Some(initial) = initial {
                    core.json(&[
                        "identity",
                        "meta",
                        "set",
                        &state,
                        initial,
                        "--identity",
                        &member.id,
                    ])?;
                }
                Ok::<_, SquadError>(
                    json!({"name": member.name, "id": member.id, "stateSet": initial}),
                )
            })();
            added.unwrap_or_else(|error| {
                complete = false;
                json!({"name": name, "error": {"code": error.code, "message": error.message}})
            })
        })
        .collect();
    Ok(Outcome {
        document: json!({"squad": squad.name, "results": results}),
        complete,
    })
}

pub fn remove(core: &Core, squad: &Squad, name: &str) -> Result<Outcome, SquadError> {
    let (member, _) = identity(core, name)?;
    // Membership is the authority; leave first, then clear this squad's fields.
    core.json(&["room", "leave", &squad.room_id, "--identity", &member.id])?;
    let cleared: Vec<_> = fields(core, squad, &member.id)?.into_keys().collect();
    for field in &cleared {
        core.json(&[
            "identity",
            "meta",
            "rm",
            &format!("{}{field}", squad.prefix()),
            "--identity",
            &member.id,
        ])?;
    }
    Ok(json!({"squad": squad.name, "removed": {"id": member.id, "name": member.name}, "cleared": cleared}).into())
}

enum Change {
    Set(String, String),
    Clear(String),
}

fn parse_change(squad: &Squad, pair: &str) -> Result<Change, SquadError> {
    let (field, value) = pair.split_once('=').ok_or_else(|| {
        SquadError::new(
            "SQUAD_FIELD_INVALID",
            format!("Expected field=value, got '{pair}'."),
        )
    })?;
    let key = squad.key(field)?;
    if value.is_empty() {
        return Ok(Change::Clear(key));
    }
    if field == "note" {
        return Err(SquadError::hinted(
            "SQUAD_NOTE_RETIRED",
            "The per-member note is retired",
            "; use ",
            "tmt notes path --identity <member> for the member's notebook, task= for what the member is doing, and pending= for what it waits on you for (note= still clears an old value).",
        ));
    }
    if value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(SquadError::new(
            "SQUAD_FIELD_INVALID",
            format!("Field '{field}' must be one line of at most 1024 bytes."),
        ));
    }
    Ok(Change::Set(key, value.into()))
}

/// Every pair is validated before the first write. Writes are sequential core
/// commands, not one transaction: the result names what was applied.
pub fn set(
    core: &Core,
    squad: &Squad,
    name: &str,
    pairs: &[String],
) -> Result<Outcome, SquadError> {
    let changes = pairs
        .iter()
        .map(|pair| parse_change(squad, pair))
        .collect::<Result<Vec<_>, _>>()?;
    let (member, _) = identity(core, name)?;
    let members = squad.members(core, false)?;
    let Some(existing) = members.iter().find(|candidate| candidate.id == member.id) else {
        return Err(SquadError::new(
            "SQUAD_NOT_MEMBER",
            format!(
                "'{}' is not in squad {}; add it first.",
                member.name, squad.name
            ),
        ));
    };
    let role = squad.key("role")?;
    let changes_lead = changes.iter().any(|change| match change {
        Change::Set(key, value) => key == &role && (value == "lead") != existing.is_lead(),
        Change::Clear(key) => key == &role && existing.is_lead(),
    });
    // Persist only when a role write would change legacy-derived leadership,
    // before applying any user pair, so a full identity changes nothing.
    if existing.lead_marker.is_none() && changes_lead {
        record_lead(core, squad, &member.id, existing.is_lead())?;
    }
    let mut applied = Vec::new();
    for (index, change) in changes.iter().enumerate() {
        let result = match change {
            Change::Set(key, value) => core.json(&[
                "identity",
                "meta",
                "set",
                key,
                value,
                "--identity",
                &member.id,
            ]),
            Change::Clear(key) => {
                core.json(&["identity", "meta", "rm", key, "--identity", &member.id])
            }
        };
        let key = match change {
            Change::Set(key, _) | Change::Clear(key) => key.as_str(),
        };
        if let Err(error) = result {
            return Ok(Outcome {
                document: json!({
                    "squad": squad.name, "member": member.name, "applied": applied,
                    "failed": {"key": key, "error": {"code": error.code, "message": error.message}},
                    "notAttempted": pairs[index + 1..].to_vec(),
                }),
                complete: false,
            });
        }
        applied.push(key.to_owned());
    }
    Ok(json!({"squad": squad.name, "member": member.name, "applied": applied}).into())
}

#[cfg(test)]
mod tests;
