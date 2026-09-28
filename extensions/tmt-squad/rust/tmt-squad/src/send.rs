//! Messages from the user to squad members through public core commands:
//! detached talk, tagged annotations and receipt-based replies. Nothing here
//! waits for an answer or acknowledges anything.

use crate::{
    core::{Core, SquadError},
    requests::tag,
    squad::room_name,
};

fn refused(message: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_ACTION_REFUSED", message)
}

fn request_id(document: &serde_json::Value) -> Result<String, SquadError> {
    document["requestId"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            SquadError::new("SQUAD_CORE_UNAVAILABLE", "tmt talk returned no request ID.")
        })
}

/// A detached request from `me` to `to` in the squad's room. Text that is
/// empty or only whitespace sends nothing.
pub fn talk(
    core: &Core,
    squad: &str,
    me: &str,
    to: &str,
    text: &str,
) -> Result<String, SquadError> {
    if text.trim().is_empty() {
        return Err(refused("Nothing to send."));
    }
    let room = room_name(squad);
    let sent = core.json_with_operands(
        &["talk", "--identity", me, "--room", &room, "--detach"],
        &[to, text],
    )?;
    request_id(&sent)
}

/// A talk tagged `[<squad> · <row>]`, so the row can show it until answered.
pub fn annotate(
    core: &Core,
    squad: &str,
    me: &str,
    to: &str,
    row: &str,
    text: &str,
) -> Result<String, SquadError> {
    if text.trim().is_empty() {
        return Err(refused("Nothing to send."));
    }
    talk(core, squad, me, to, &format!("{}{text}", tag(squad, row)))
}

/// Answers one open request to `me`, with the receipt core shows only to its
/// recipient. The incoming item stays unacknowledged.
pub fn answer(core: &Core, me: &str, request: &str, text: &str) -> Result<(), SquadError> {
    if text.trim().is_empty() {
        return Err(refused("Nothing to send."));
    }
    let shown = core.json(&["x", "show", request, "--incoming", "--identity", me])?;
    let receipt = shown["exchange"]["reply"]["receipt"]
        .as_str()
        .ok_or_else(|| refused(format!("{request} no longer takes a reply.")))?;
    core.json(&[
        "reply",
        request,
        "--receipt",
        receipt,
        &format!("--message={text}"),
    ])?;
    Ok(())
}
