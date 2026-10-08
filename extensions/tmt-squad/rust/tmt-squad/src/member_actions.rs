//! `tmt squad jump|back|open|copy`: the board's row
//! actions for one member, for scripts and terminals without the board.

use crate::{
    back,
    config::Config,
    core::{Core, SquadError},
    effects,
    membership::Outcome,
    squad::Squad,
    status,
    template::{DEFAULT_COPY, Template, field_value},
};
use serde_json::{Value, json};

fn refused(message: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_ACTION_REFUSED", message)
}

fn failed(message: String) -> SquadError {
    SquadError::new("SQUAD_ACTION_FAILED", message)
}

fn document(core: &Core, squad: &Squad, config: &Config) -> Result<Value, SquadError> {
    let layout = config.layout(&squad.name)?;
    let states = config.states(&squad.name, layout)?;
    let rows = config.rows(&squad.name)?;
    let providers = config.providers(&squad.name)?;
    let mut members = squad.members(core, rows.reads_metadata())?;
    crate::provider::apply(
        &providers,
        &mut members,
        &crate::provider::Cache::load(&squad.name),
    );
    Ok(status::document(
        squad,
        layout,
        &states,
        &[],
        &rows,
        members,
    ))
}

/// The member's status row (the lead included), exactly as the board sees it.
fn row(core: &Core, squad: &Squad, config: &Config, name: &str) -> Result<Value, SquadError> {
    find(document(core, squad, config)?, squad, name)
}

fn find(mut document: Value, squad: &Squad, name: &str) -> Result<Value, SquadError> {
    let lead = document["squad"]["lead"].take();
    document["sections"][0]["rows"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .map(Value::take)
        .chain([lead])
        .find(|row| row["name"] == name)
        .ok_or_else(|| {
            SquadError::new(
                "SQUAD_NOT_MEMBER",
                format!("'{name}' is not in squad {}.", squad.name),
            )
        })
}

pub fn jump(
    core: &Core,
    squad: &Squad,
    config: &Config,
    name: &str,
) -> Result<Outcome, SquadError> {
    row(core, squad, config, name)?;
    let (focus, warning) = back::jump(core, name)?;
    let mut document = json!({
        "member": name,
        "focused": {"pane": focus.pane},
        "from": focus.from.map(|pane| json!({"pane": pane})),
        "client": focus.client,
    });
    if let Some(warning) = warning {
        document["warning"] = json!(format!("back will not return here: {warning}"));
    }
    Ok(document.into())
}

/// `tmt squad jump --lead`: the squad's lead, through the same jump as any
/// member, so `back` returns from it. No lead changes nothing.
pub fn jump_lead(core: &Core, squad: &Squad, config: &Config) -> Result<Outcome, SquadError> {
    let lead = squad
        .roster(core)?
        .into_iter()
        .find(crate::squad::Member::is_lead)
        .ok_or_else(|| {
            refused(format!(
                "Squad {} has no lead; set one with tmt squad lead <name>.",
                squad.name
            ))
        })?;
    jump(core, squad, config, &lead.name)
}

/// The squad `jump --lead` means without `--squad`: the one the calling
/// pane's identity is in, else the only squad. A caller in several squads
/// must choose; one in none falls back to the only squad.
pub fn caller_squad(core: &Core, explicit: Option<&str>) -> Result<Squad, SquadError> {
    if explicit.is_some() {
        return Squad::resolve(core, explicit);
    }
    // Outside a pane core cannot say who calls; that is not this command's
    // failure, only no caller.
    let Some(caller) = crate::me::caller(core).ok().flatten() else {
        return Squad::resolve(core, None);
    };
    let mut mine = Vec::new();
    for squad in Squad::list(core)? {
        if squad
            .roster(core)?
            .iter()
            .any(|member| member.id == caller.me.id)
        {
            mine.push(squad);
        }
    }
    match mine.len() {
        0 => Squad::resolve(core, None),
        1 => Ok(mine.remove(0)),
        _ => Err(SquadError::new(
            "SQUAD_AMBIGUOUS",
            format!(
                "{} is in several squads ({}); choose one with --squad.",
                caller.me.name,
                mine.iter()
                    .map(|squad| squad.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// `tmt squad back`: needs no squad; the stack belongs to the tmux client.
pub fn back(core: &Core) -> Result<Outcome, SquadError> {
    Ok(match back::back(core)? {
        Some(focus) => json!({"back": {"focused": {"pane": focus.pane}, "client": focus.client}}),
        None => json!({"back": null, "message": "Nothing to go back to."}),
    }
    .into())
}

/// `field` names the link field; otherwise the row's default link.
pub fn open(
    core: &Core,
    squad: &Squad,
    config: &Config,
    name: &str,
    field: Option<&str>,
) -> Result<Outcome, SquadError> {
    let row = row(core, squad, config, name)?;
    let link = match field {
        Some(field) => field_value(&row, field)
            .filter(|link| !link.is_empty())
            .ok_or_else(|| refused(format!("{name} has no {field}.")))?,
        None => effects::default_link(&row)
            .ok_or_else(|| refused(format!("{name} has no link field.")))?,
    };
    effects::web_link(link).map_err(refused)?;
    effects::open(link, config.program("opener")?.as_deref()).map_err(failed)?;
    Ok(json!({"member": name, "opened": link}).into())
}

pub fn copy(
    core: &Core,
    squad: &Squad,
    config: &Config,
    name: &str,
    format: Option<&str>,
) -> Result<Outcome, SquadError> {
    let row = row(core, squad, config, name)?;
    let template = Template::parse(format.unwrap_or(DEFAULT_COPY))
        .map_err(|error| refused(format!("--format: {error}.")))?;
    let text = template
        .fill(&row)
        .map_err(|error| refused(format!("{error}.")))?;
    let copied = effects::copy(
        &text,
        config.program("clipboard")?.as_deref(),
        effects::tmux_socket().as_deref(),
    )
    .map_err(failed)?;
    Ok(
        json!({"member": name, "copied": text, "to": copied.name(), "message": copied.describe()})
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    /// A fake `tmt` with squads `a` and `b`: `rooms.roster` answers per room,
    /// `whoami` from a file the test writes, and `focus` is logged and never
    /// reports a client, so no `back` stack is ever written.
    struct Fixture {
        root: PathBuf,
        core: Core,
    }

    impl Fixture {
        fn new(name: &str, a: Value, b: Value, caller: Option<&str>) -> Self {
            let root =
                std::env::temp_dir().join(format!("squad-jump-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir(&root).unwrap();
            fs::write(root.join("roster-a"), json!({"members": a}).to_string()).unwrap();
            fs::write(root.join("roster-b"), json!({"members": b}).to_string()).unwrap();
            let whoami = match caller {
                Some(id) => {
                    json!({"bound": true, "id": id, "name": id.to_lowercase(), "lifetime": "saved"})
                }
                None => json!({"bound": false}),
            };
            fs::write(root.join("whoami"), whoami.to_string()).unwrap();
            let executable = root.join("tmt");
            crate::test_support::write_ready_executable(
                &executable,
                r#"#!/bin/sh
root=${0%/*}
case "$1" in
  api)
    input=$(cat)
    case "$input" in
      *'"room-a"'*) cat "$root/roster-a" ;;
      *'"room-b"'*) cat "$root/roster-b" ;;
      *) exit 2 ;;
    esac ;;
  room)
    case "$2" in
      list) printf '%s\n' '{"rooms":[{"id":"room-a","name":"squad-a"},{"id":"room-b","name":"squad-b"}]}' ;;
      show) printf '{"room":{"id":"room-%s","name":"squad-%s"}}\n' "${3#squad-}" "${3#squad-}" ;;
      *) exit 2 ;;
    esac ;;
  whoami) cat "$root/whoami" ;;
  ls) printf '%s\n' '{"identities":[]}' ;;
  focus) printf '%s\n' "focus $2" >> "$root/calls"; printf '%s\n' '{"focused":{"pane":"%9"}}' ;;
  *) exit 2 ;;
esac
"#,
            );
            Self {
                core: Core::at(executable),
                root,
            }
        }

        fn focused(&self) -> String {
            fs::read_to_string(self.root.join("calls")).unwrap_or_default()
        }

        fn config(&self) -> Config {
            let path = self.root.join("ops.toml");
            fs::write(&path, "").unwrap();
            Config::read(path).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn member(id: &str, lead: bool) -> Value {
        let mut metadata = json!({});
        if lead {
            metadata["squad.a.lead.marker"] = json!("true");
            metadata["squad.b.lead.marker"] = json!("true");
        }
        json!({"id": id, "name": id.to_lowercase(), "lifetime": "saved", "metadata": metadata})
    }

    #[test]
    fn jump_lead_focuses_the_callers_squad_lead_like_any_member() {
        let fixture = Fixture::new(
            "mine",
            json!([member("SOL", true), member("RIN", false)]),
            json!([member("ADA", true)]),
            Some("RIN"),
        );
        let squad = caller_squad(&fixture.core, None).unwrap();
        assert_eq!(squad.name, "a", "the calling pane's own squad");
        let outcome = jump_lead(&fixture.core, &squad, &fixture.config()).unwrap();
        assert_eq!(fixture.focused(), "focus sol\n");
        assert_eq!(outcome.document["member"], "sol");
        assert_eq!(outcome.document["focused"]["pane"], "%9");
        // As for any jump: no client reported means `back` cannot return.
        assert!(outcome.document["warning"].is_string());
        // --squad wins over the caller's squad.
        let named = caller_squad(&fixture.core, Some("b")).unwrap();
        assert_eq!(named.name, "b");
    }

    #[test]
    fn without_a_lead_nothing_is_focused() {
        let fixture = Fixture::new(
            "leaderless",
            json!([member("RIN", false)]),
            json!([]),
            Some("RIN"),
        );
        let squad = caller_squad(&fixture.core, None).unwrap();
        let Err(error) = jump_lead(&fixture.core, &squad, &fixture.config()) else {
            panic!("a squad without a lead cannot be jumped to");
        };
        assert_eq!(error.code, "SQUAD_ACTION_REFUSED");
        assert_eq!(
            error.message,
            "Squad a has no lead; set one with tmt squad lead <name>."
        );
        assert_eq!(fixture.focused(), "", "nothing changed");
    }

    #[test]
    fn a_caller_in_several_squads_or_none_is_resolved_or_asked() {
        // In both squads: the caller must choose.
        let both = Fixture::new(
            "both",
            json!([member("RIN", false)]),
            json!([member("RIN", false)]),
            Some("RIN"),
        );
        let error = caller_squad(&both.core, None).unwrap_err();
        assert_eq!(error.code, "SQUAD_AMBIGUOUS");
        assert_eq!(
            error.message,
            "rin is in several squads (a, b); choose one with --squad."
        );
        // In none, or no pane at all: the only-squad rule, which with two
        // squads asks for --squad too.
        for caller in [Some("ZED"), None] {
            let none = Fixture::new("none", json!([]), json!([]), caller);
            assert_eq!(
                caller_squad(&none.core, None).unwrap_err().code,
                "SQUAD_AMBIGUOUS"
            );
        }
    }
}
