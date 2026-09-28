//! `tmt squad jump|open|copy <member>`: the board's row actions for one
//! member, for scripts and terminals without the board.

use crate::{
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

/// The member's status row (the lead included), exactly as the board sees it.
fn row(core: &Core, squad: &Squad, config: &Config, name: &str) -> Result<Value, SquadError> {
    let layout = config.layout(&squad.name)?;
    let states = config.states(&squad.name, layout)?;
    let mut document = status::document(squad, layout, &states, &[], squad.members(core)?);
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
    let focus = effects::focus(core, name)?;
    Ok(json!({
        "member": name,
        "focused": {"pane": focus.pane},
        "from": focus.from.map(|pane| json!({"pane": pane})),
    })
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
