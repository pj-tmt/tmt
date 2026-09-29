//! Membership changes through public core commands. Each command is idempotent
//! and reports exactly what it applied; a re-run converges after a failure.

use crate::{
    config::{Config, Layout},
    core::{Core, SquadError},
    squad::{self, Squad, name_invalid, valid_name},
};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

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

fn fields(core: &Core, squad: &Squad, id: &str) -> Result<Vec<String>, SquadError> {
    let listed = core.json(&["identity", "meta", "list", "--identity", id])?;
    Ok(listed["metadata"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, _)| key.strip_prefix(&squad.prefix()).map(str::to_owned))
        .collect())
}

/// Asks which saved identity is the user; only on an interactive terminal.
fn prompt_me(core: &Core) -> Result<String, SquadError> {
    let listed = core.json(&["identity", "list"])?;
    let saved: Vec<&str> = listed["identities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|identity| identity["lifetime"] == "saved")
        .filter_map(|identity| identity["name"].as_str())
        .collect();
    let mut stderr = tmt_cli_style::stream::stderr();
    let _ = writeln!(
        stderr,
        "Which saved identity is you? It receives what members need from you."
    );
    let _ = writeln!(
        stderr,
        "Saved identities: {}",
        if saved.is_empty() {
            "(none yet)".into()
        } else {
            saved.join(", ")
        }
    );
    let _ = write!(stderr, "me = ");
    let _ = stderr.flush();
    drop(stderr);
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|_| SquadError::new("SQUAD_ME_REQUIRED", "No answer was read."))?;
    let answer = line.trim();
    if answer.is_empty() {
        return Err(SquadError::new(
            "SQUAD_ME_REQUIRED",
            "No identity was chosen.",
        ));
    }
    Ok(answer.into())
}

pub fn init(
    core: &Core,
    config: &mut Config,
    name: &str,
    me: Option<&str>,
    interactive: bool,
) -> Result<Outcome, SquadError> {
    if !valid_name(name) {
        return Err(name_invalid(name));
    }
    // Settle `me` before any effect: an unanswerable question changes nothing.
    let chosen = match (me, config.me()?) {
        (Some(requested), _) => Some(saved(core, requested)?.name),
        (None, Some(_)) => None,
        (None, None) if interactive => Some(saved(core, &prompt_me(core)?)?.name),
        (None, None) => {
            return Err(SquadError::new(
                "SQUAD_ME_REQUIRED",
                format!(
                    "Set the user's saved identity: tmt squad init {name} --me <name> (recorded in {}).",
                    config.path().display()
                ),
            ));
        }
    };
    let room = squad::room_name(name);
    let (shown, created) = match core.json(&["room", "show", &room]) {
        Ok(shown) => (shown, false),
        Err(error) if error.code == "ROOM_NOT_FOUND" => {
            (core.json(&["room", "create", &room])?, true)
        }
        Err(error) => return Err(error),
    };
    if let Some(chosen) = chosen
        .as_deref()
        .filter(|chosen| config.me().ok().flatten() != Some(chosen))
    {
        config.set_me(chosen)?;
    }
    Ok(json!({
        "squad": {"name": name, "roomId": shown["room"]["id"]},
        "created": created,
        "me": config.me()?,
    })
    .into())
}

pub fn lead(core: &Core, squad: &Squad, name: &str) -> Result<Outcome, SquadError> {
    let leader = saved(core, name)?;
    core.json(&["room", "join", &squad.room_id, "--identity", &leader.id])?;
    let role = squad.key("role")?;
    core.json(&[
        "identity",
        "meta",
        "set",
        &role,
        "lead",
        "--identity",
        &leader.id,
    ])?;
    // Set the new lead before clearing others, so a failure never leaves none.
    let mut replaced = Vec::new();
    for member in squad.members(core)? {
        if member.is_lead() && member.id != leader.id {
            core.json(&["identity", "meta", "rm", &role, "--identity", &member.id])?;
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
    let mut complete = true;
    let results: Vec<Value> = names
        .iter()
        .map(|name| {
            let added = (|| {
                let (member, _) = identity(core, name)?;
                core.json(&["room", "join", &squad.room_id, "--identity", &member.id])?;
                let initial = match layout.states().first() {
                    Some(first)
                        if !fields(core, squad, &member.id)?
                            .iter()
                            .any(|f| f == "state") =>
                    {
                        Some(*first)
                    }
                    _ => None,
                };
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
    let cleared = fields(core, squad, &member.id)?;
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
    if !squad
        .members(core)?
        .iter()
        .any(|candidate| candidate.id == member.id)
    {
        return Err(SquadError::new(
            "SQUAD_NOT_MEMBER",
            format!(
                "'{}' is not in squad {}; add it first.",
                member.name, squad.name
            ),
        ));
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
