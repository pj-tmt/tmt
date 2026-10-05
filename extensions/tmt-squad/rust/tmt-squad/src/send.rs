//! Messages from the user to squad members through public core commands:
//! detached talk, tagged annotations and answers. Nothing here
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

/// Answers one open request from `from` to `me` through `tmt answer`: core
/// checks that it is addressed to `me` and derives the proof, so no receipt
/// passes through Squad. The incoming item stays unacknowledged.
pub fn answer(
    core: &Core,
    me: &str,
    request: &str,
    from: &str,
    text: &str,
) -> Result<(), SquadError> {
    if text.trim().is_empty() {
        return Err(refused("Nothing to send."));
    }
    core.json_with_operands(
        &["answer", "--identity", me, "--request", request],
        &[from, text],
    )?;
    Ok(())
}

/// New explicit sends retain this UUID through acceptance recovery.
pub(crate) fn new_operation() -> Result<String, SquadError> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| SquadError::new("SQUAD_DISPATCH_IO", error.to_string()))?;
    bytes[6] = (bytes[6] & 15) | 0x40;
    bytes[8] = (bytes[8] & 63) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeadRecipient {
    pub squad: String,
    pub id: String,
    pub name: String,
}

mod leads;
pub use leads::leads;

mod dispatch;

pub(crate) use dispatch::Intent;
