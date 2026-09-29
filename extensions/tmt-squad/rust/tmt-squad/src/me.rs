//! Which saved identity is the user. `squad.toml` keeps the name a person
//! reads and edits (`me`) and the UUID it named (`me_id`), so `me` follows an
//! identity rename in core. The rename observation (`__tmt-hooks`) repairs the
//! file at once; without hooks, the next command that needs `me` repairs it.

use crate::{
    config::Config,
    core::{Core, SquadError},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    pub id: String,
    pub name: String,
}

/// `tmt identity show` by name or UUID; `None` when core has no such identity.
fn lookup(core: &Core, selector: &str) -> Result<Option<Me>, SquadError> {
    match core.json(&["identity", "show", selector]) {
        Ok(shown) => match (
            shown["identity"]["id"].as_str(),
            shown["identity"]["name"].as_str(),
        ) {
            (Some(id), Some(name)) => Ok(Some(Me {
                id: id.into(),
                name: name.into(),
            })),
            _ => Err(SquadError::new(
                "SQUAD_CORE_UNAVAILABLE",
                "tmt identity show returned no identity.",
            )),
        },
        Err(error) if error.code == "NAME_NOT_FOUND" => Ok(None),
        Err(error) => Err(error),
    }
}

/// The user, resolved and written back when the file lags core.
///
/// The name decides while it resolves, so a hand edit of `me` wins and its
/// UUID is recorded. When the name no longer resolves but `me_id` does, the
/// identity was renamed: `me` takes the new name. A failed write never fails
/// the command; the next one retries.
pub fn resolve(core: &Core, config: &mut Config) -> Result<Option<Me>, SquadError> {
    let Some(name) = config.me()?.map(str::to_owned) else {
        return Ok(None);
    };
    let recorded = config.me_id()?.map(str::to_owned);
    let found = match lookup(core, &name)? {
        Some(found) => found,
        None => match recorded.as_deref().map(|id| lookup(core, id)).transpose()? {
            Some(Some(renamed)) => renamed,
            _ => {
                return Err(SquadError::new(
                    "NAME_NOT_FOUND",
                    format!("Identity '{name}' was not found."),
                ));
            }
        },
    };
    if found.name != name || recorded.as_deref() != Some(found.id.as_str()) {
        let _ = config.set_me(&found.name, &found.id);
    }
    Ok(Some(found))
}

/// The user, for commands that act as them.
pub fn required(core: &Core, config: &mut Config) -> Result<Me, SquadError> {
    resolve(core, config)?.ok_or_else(|| {
        SquadError::new(
            "SQUAD_ME_REQUIRED",
            "Record which saved identity is you first: tmt squad init <squad> --me <name>.",
        )
    })
}

/// An `identity.renamed` observation: when it is the user, `me` takes the
/// new name now. Anything else is not squad's concern.
pub fn renamed(core: &Core, config: &mut Config, identity_id: &str) -> Result<bool, SquadError> {
    if config.me_id()? != Some(identity_id) {
        return Ok(false);
    }
    let Some(current) = lookup(core, identity_id)? else {
        return Ok(false);
    };
    if config.me()? == Some(current.name.as_str()) {
        return Ok(false);
    }
    config.set_me(&current.name, &current.id)?;
    Ok(true)
}

/// `init --me <name>`: the saved identity, recorded with its UUID.
pub fn record(config: &mut Config, me: &Me) -> Result<(), SquadError> {
    if config.me()? == Some(me.name.as_str()) && config.me_id()? == Some(me.id.as_str()) {
        return Ok(());
    }
    config.set_me(&me.name, &me.id)
}
