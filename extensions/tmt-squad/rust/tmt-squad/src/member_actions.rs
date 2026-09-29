//! `tmt squad jump|back|open|copy|annotate`: the board's row
//! actions for one member, for scripts and terminals without the board.

use crate::{
    back,
    config::Config,
    core::{Core, SquadError},
    effects,
    membership::Outcome,
    send,
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
    Ok(status::document(
        squad,
        layout,
        &states,
        &[],
        squad.members(core)?,
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

/// `tmt squad annotate <member> <text> [--to lead|member]`: a note about a
/// row, sent to the lead (default) or the member; never a notebook write.
pub fn annotate(
    core: &Core,
    squad: &Squad,
    config: &mut Config,
    identity: Option<&str>,
    name: &str,
    to_lead: bool,
    text: &str,
) -> Result<Outcome, SquadError> {
    let me = crate::me::resolve_sender(core, config, identity)?;
    let document = document(core, squad, config)?;
    let lead = document["squad"]["lead"]["name"]
        .as_str()
        .map(str::to_owned);
    find(document, squad, name)?;
    let to = if to_lead {
        lead.ok_or_else(|| refused(format!("Squad {} has no lead to annotate for.", squad.name)))?
    } else {
        name.to_owned()
    };
    let request = send::annotate(core, &squad.name, &me.name, &to, name, text)?;
    Ok(json!({"requestId": request, "to": to, "as": me.name, "row": name, "room": crate::squad::room_name(&squad.name)}).into())
}
