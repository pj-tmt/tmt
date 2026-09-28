//! Membership changes through public core commands. Each command is idempotent
//! and reports exactly what it applied; a re-run converges after a failure.

use crate::{
    config::Config,
    core::{Core, SquadError},
    squad::{self, name_invalid, valid_name},
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

/// The display name of a saved identity; temporary identities are refused.
fn saved(core: &Core, name: &str) -> Result<String, SquadError> {
    let shown = core.json(&["identity", "show", name])?;
    let record = &shown["identity"];
    let display = record["name"].as_str().ok_or_else(|| {
        SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "tmt identity show returned no identity.",
        )
    })?;
    if record["lifetime"] != "saved" {
        return Err(SquadError::new(
            "SQUAD_IDENTITY_NOT_SAVED",
            format!(
                "'{display}' is temporary; save it first with tmt identity create {display} (or tmt name -s)."
            ),
        ));
    }
    Ok(display.into())
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
    let mut stderr = std::io::stderr().lock();
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
        (Some(requested), _) => Some(saved(core, requested)?),
        (None, Some(_)) => None,
        (None, None) if interactive => Some(saved(core, &prompt_me(core)?)?),
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
