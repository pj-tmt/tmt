//! Which saved identity is the user. `squad.toml` keeps the name a person
//! reads (`me`) and the UUID that decides (`me_id`), so `me` follows an
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

/// Who the user is. The UUID decides, as core's binding markers do: while
/// `me_id` names an active identity, that identity is the user under its
/// current name. Only when `me_id` is missing or no longer active does the
/// name decide, and its UUID is what gets recorded.
#[derive(Debug, PartialEq, Eq)]
enum Decided {
    ById { me: Me },
    ByName { me: Me },
}

fn decide(core: &Core, config: &Config) -> Result<Option<(Decided, String)>, SquadError> {
    let Some(name) = config.me()?.map(str::to_owned) else {
        return Ok(None);
    };
    if let Some(id) = config.me_id()?
        && let Some(me) = lookup(core, id)?
    {
        return Ok(Some((Decided::ById { me }, name)));
    }
    let me = lookup(core, &name)?.ok_or_else(|| {
        SquadError::new(
            "NAME_NOT_FOUND",
            format!("Identity '{name}' was not found."),
        )
    })?;
    Ok(Some((Decided::ByName { me }, name)))
}

/// The user, without touching `squad.toml`: for the board's refresh, which
/// must neither write the user's file nor print over its screen.
pub fn current(core: &Core, config: &Config) -> Result<Option<Me>, SquadError> {
    Ok(decide(core, config)?.map(|(decided, _)| match decided {
        Decided::ById { me } | Decided::ByName { me } => me,
    }))
}

/// The user, with `squad.toml` brought up to date: a rename rewrites `me`, a
/// missing `me_id` is recorded. When `me` was edited to name a different
/// identity, `me_id` still decides and one warning says how to change who
/// the user is. A failed write never fails the command; the next retries.
pub fn resolve(core: &Core, config: &mut Config) -> Result<Option<Me>, SquadError> {
    let Some((decided, written)) = decide(core, config)? else {
        return Ok(None);
    };
    let me = match decided {
        Decided::ById { me } => {
            if me.name != written && lookup(core, &written)?.is_some_and(|other| other.id != me.id)
            {
                warn_edited(&written, &me.name);
            }
            me
        }
        Decided::ByName { me } => me,
    };
    if me.name != written || config.me_id()? != Some(me.id.as_str()) {
        let _ = config.set_me(&me.name, &me.id);
    }
    Ok(Some(me))
}

/// `me` named someone else while `me_id` still names the user.
fn warn_edited(written: &str, kept: &str) {
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    let _ = tmt_cli_style::message::warning(
        &mut stderr,
        terminal,
        &format!(
            "squad.toml named '{written}' as you, but me_id is {kept}; still acting as {kept}."
        ),
        Some(&format!("tmt squad init <squad> --me {written}")),
    );
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
