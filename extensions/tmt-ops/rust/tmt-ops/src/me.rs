//! Who the user is, and who sends. `ops.toml` may record the user's saved
//! identity: the name a person reads (`me`) and the UUID that decides
//! (`me_id`), so `me` follows an identity rename in core. The rename
//! observation (`__tmt-hooks`) repairs the file at once; without hooks, the
//! next command that reads `me` repairs it. Nothing here ever asks: `me` is
//! optional, and what needs it derives it or says how to set it.

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

/// The user, without touching `ops.toml`: for the board's refresh, which
/// must neither write the user's file nor print over its screen.
pub fn current(core: &Core, config: &Config) -> Result<Option<Me>, SquadError> {
    Ok(decide(core, config)?.map(|(decided, _)| match decided {
        Decided::ById { me } | Decided::ByName { me } => me,
    }))
}

/// The user, with `ops.toml` brought up to date: a rename rewrites `me`, a
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
        &format!("ops.toml named '{written}' as you, but me_id is {kept}; still acting as {kept}."),
        Some(&format!("tmt ops squad me {written}")),
    );
}

/// The identity core attributes the invocation to (`tmt whoami`): `None`
/// outside a pane and on an unbound pane. A host that cannot say which agent
/// called is core's error, never a guess.
pub fn caller(core: &Core) -> Result<Option<Caller>, SquadError> {
    match core.json(&["whoami"]) {
        Ok(shown) if shown["bound"] == true => match (shown["id"].as_str(), shown["name"].as_str())
        {
            (Some(id), Some(name)) => Ok(Some(Caller {
                me: Me {
                    id: id.into(),
                    name: name.into(),
                },
                saved: shown["lifetime"] == "saved",
            })),
            _ => Err(SquadError::new(
                "SQUAD_CORE_UNAVAILABLE",
                "tmt whoami returned no identity.",
            )),
        },
        Ok(_) => Ok(None),
        Err(error) if error.code == "PANE_NOT_FOUND" => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub me: Me,
    pub saved: bool,
}

/// Who "waiting on you" is for: the recorded user, otherwise the saved
/// identity bound to the calling pane. Temporary agents are never "you".
pub fn you(recorded: Option<Me>, caller: Option<&Caller>) -> Option<Me> {
    recorded.or_else(|| {
        caller
            .filter(|caller| caller.saved)
            .map(|caller| caller.me.clone())
    })
}

/// Where "you" came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Recorded,
    Pane,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::Pane => "pane",
        }
    }
}

/// [`you`] for a command: `me` brought up to date, then the calling pane. A
/// caller that core cannot attribute only means no one is "you" here.
pub fn resolve_you(core: &Core, config: &mut Config) -> Result<Option<(Me, Source)>, SquadError> {
    if let Some(recorded) = resolve(core, config)? {
        return Ok(Some((recorded, Source::Recorded)));
    }
    Ok(you(None, caller(core).ok().flatten().as_ref()).map(|pane| (pane, Source::Pane)))
}

/// `tmt ops squad me <name>`: a saved identity becomes the user.
pub fn set(core: &Core, config: &mut Config, name: &str) -> Result<Me, SquadError> {
    let shown = core.json(&["identity", "show", name])?;
    let identity = &shown["identity"];
    let (Some(id), Some(display)) = (identity["id"].as_str(), identity["name"].as_str()) else {
        return Err(SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "tmt identity show returned no identity.",
        ));
    };
    if identity["lifetime"] != "saved" {
        return Err(SquadError::new(
            "SQUAD_IDENTITY_NOT_SAVED",
            format!(
                "'{display}' is temporary; save it first with tmt identity create {display} (or tmt name -s)."
            ),
        ));
    }
    let me = Me {
        id: id.into(),
        name: display.into(),
    };
    record(config, &me)?;
    Ok(me)
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

/// The saved identity, recorded with its UUID (`init --me`, `me <name>`).
pub fn record(config: &mut Config, me: &Me) -> Result<(), SquadError> {
    if config.me()? == Some(me.name.as_str()) && config.me_id()? == Some(me.id.as_str()) {
        return Ok(());
    }
    config.set_me(&me.name, &me.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(name: &str) -> Me {
        Me {
            id: format!("{name}-id"),
            name: name.into(),
        }
    }

    fn caller(name: &str, saved: bool) -> Caller {
        Caller {
            me: me(name),
            saved,
        }
    }

    #[test]
    fn you_is_the_recorded_user_then_a_saved_caller_never_a_temporary_one() {
        assert_eq!(
            you(Some(me("ben")), Some(&caller("sol", true))),
            Some(me("ben"))
        );
        assert_eq!(you(None, Some(&caller("sol", true))), Some(me("sol")));
        assert_eq!(you(None, Some(&caller("auth-fix", false))), None);
        assert_eq!(you(None, None), None);
    }

    /// A stand-in tmt that answers `whoami --json` with a fixed document on
    /// stdout, or a core error with its exit status.
    fn whoami(name: &str, stdout: &str, status: i32) -> Result<Option<Caller>, SquadError> {
        let dir =
            std::env::temp_dir().join(format!("tmt-ops-whoami-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("tmt");
        crate::test_support::write_ready_executable(
            &fake,
            &format!("#!/bin/sh\ncat <<'EOF'\n{stdout}\nEOF\nexit {status}\n"),
        );
        let found = super::caller(&Core::at(fake));
        let _ = std::fs::remove_dir_all(dir);
        found
    }

    #[test]
    fn whoami_maps_to_a_caller_no_caller_or_cores_error() {
        let bound = r#"{"bound":true,"id":"7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f","name":"sol","pane":"%1","lifetime":"saved"}"#;
        assert_eq!(
            whoami("bound", bound, 0).unwrap(),
            Some(Caller {
                me: Me {
                    id: "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f".into(),
                    name: "sol".into()
                },
                saved: true
            })
        );
        let temporary =
            r#"{"bound":true,"id":"x","name":"auth-fix","pane":"%3","lifetime":"temporary"}"#;
        assert!(!whoami("temporary", temporary, 0).unwrap().unwrap().saved);
        assert_eq!(
            whoami("unbound", r#"{"bound":false,"pane":"%0"}"#, 0).unwrap(),
            None
        );
        let outside = r#"{"error":{"code":"PANE_NOT_FOUND","message":"Not running inside a resolvable tmux pane."}}"#;
        assert_eq!(whoami("outside", outside, 3).unwrap(), None);
        let shared = r#"{"error":{"code":"CALLER_IDENTITY_AMBIGUOUS","message":"The runtime host does not establish which agent invoked this command."}}"#;
        assert_eq!(
            whoami("shared", shared, 1).unwrap_err().code,
            "CALLER_IDENTITY_AMBIGUOUS",
            "a host that cannot attribute the caller is never a fallback to the user"
        );
    }
}
