//! HOME freezes its audience; this effect revalidates it before public dispatch.
use super::{LeadRecipient, new_operation, refused};
use crate::{
    config::Config,
    core::{Core, SquadError},
    me,
    squad::Squad,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, io::Write, os::unix::fs::OpenOptionsExt};

pub fn leads(
    core: &Core,
    sender: &str,
    recipients: &[LeadRecipient],
    all: bool,
    text: &str,
) -> Result<String, SquadError> {
    let config = Config::load(core)?;
    run(core, &config, sender, recipients, all, text)
}

fn run(
    core: &Core,
    config: &Config,
    sender: &str,
    recipients: &[LeadRecipient],
    all: bool,
    text: &str,
) -> Result<String, SquadError> {
    if text.trim().is_empty() || recipients.is_empty() {
        return Err(refused("Nothing to send."));
    }
    let actor = match me::current(core, config)? {
        Some(actor) => Some(actor),
        None => me::you(None, me::caller(core)?.as_ref()),
    };
    if actor.as_ref().map(|actor| actor.id.as_str()) != Some(sender) {
        return Err(refused("The sender changed; nothing sent."));
    }
    let squads = Squad::list(core)?;
    let mut current = Vec::new();
    for squad in squads.iter().filter(|squad| {
        all || recipients
            .iter()
            .any(|recipient| recipient.squad == squad.name)
    }) {
        if let Some(lead) = squad
            .roster(core)?
            .into_iter()
            .find(|member| member.is_lead())
        {
            current.push(LeadRecipient {
                squad: squad.name.clone(),
                id: lead.id,
                name: lead.name,
            });
        }
    }
    let unchanged = recipients
        .iter()
        .all(|recipient| current.contains(recipient))
        && (!all || current.len() == recipients.len());
    if !unchanged {
        return Err(refused(
            "The lead audience changed; reopen the composer. Nothing sent.",
        ));
    }
    let ids: BTreeSet<&str> = recipients
        .iter()
        .map(|recipient| recipient.id.as_str())
        .collect();
    let operation = new_operation()?;
    let input = json!({"operationId":operation,"recipientIds":ids,"message":text,"kind":"request"});
    // Own extension state, never core storage. Preserve uncertain intent across exit;
    // successful acceptance removes the journal. No replay or automatic resend.
    let directory = config
        .path()
        .parent()
        .ok_or_else(|| refused("No Squad state directory."))?
        .join("board-dispatches");
    fs::create_dir_all(&directory).map_err(io_error)?;
    let path = directory.join(format!("{operation}.json"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(io_error)?;
    file.write_all(&serde_json::to_vec(&json!({"version":1,"senderId":sender,"recipients":recipients.iter().map(|recipient| json!({"squad":recipient.squad,"id":recipient.id,"name":recipient.name})).collect::<Vec<_>>(),"input":input})).expect("serializable dispatch intent")).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    fs::File::open(&directory)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)?;
    let receipt = match core.api_write("dispatch.create", input, Some(sender)) {
        Ok(receipt) => receipt,
        Err(error) if matches!(error.code.as_str(), "SQUAD_CORE_UNAVAILABLE" | "STORAGE_UNAVAILABLE") => {
            core.api("dispatch.show", json!({"operationId":operation})).map_err(|_| SquadError::new(&error.code,
                format!("Acceptance uncertain for operation {operation}; inspect dispatch.show before sending again. Saved intent: {}", path.display())))?
        }
        Err(error) => {
            fs::remove_file(&path).map_err(io_error)?;
            return Err(error);
        }
    };
    let outcome = acceptance(&operation, recipients, &receipt).map_err(|error| {
        SquadError::new(
            &error.code,
            format!("{} Saved intent: {}", error.message, path.display()),
        )
    })?;
    // Acceptance is already committed; cleanup failure must never suggest resending.
    if let Err(error) = fs::remove_file(&path) {
        return Ok(format!(
            "{outcome} Accepted operation {operation}; could not remove saved intent: {error}"
        ));
    }
    Ok(outcome)
}

fn io_error(error: std::io::Error) -> SquadError {
    SquadError::new("SQUAD_DISPATCH_IO", error.to_string())
}

fn acceptance(
    operation: &str,
    recipients: &[LeadRecipient],
    receipt: &Value,
) -> Result<String, SquadError> {
    let ids: BTreeSet<&str> = recipients
        .iter()
        .map(|recipient| recipient.id.as_str())
        .collect();
    let items = receipt["items"].as_array().filter(|items| receipt["operationId"] == operation && items.len() == ids.len())
        .ok_or_else(|| refused(format!("Acceptance unavailable for operation {operation}; inspect dispatch.show before sending again.")))?;
    let mut seen = BTreeSet::new();
    let mut outcomes = Vec::new();
    for item in items {
        let id = item["recipientId"].as_str().filter(|id| ids.contains(id) && seen.insert(*id))
            .ok_or_else(|| refused(format!("Acceptance does not match operation {operation}; inspect dispatch.show before sending again.")))?;
        let accepted = match item["acceptance"].as_str() {
            Some("queued") if item["requestId"].as_str().is_some() => "queued",
            Some("recipientUnavailable") => "unavailable",
            _ => {
                return Err(refused(format!(
                    "Acceptance unavailable for operation {operation}; inspect dispatch.show before sending again."
                )));
            }
        };
        let names: BTreeSet<_> = recipients
            .iter()
            .filter(|recipient| recipient.id == id)
            .map(|recipient| tmt_cli_style::table::escape(&recipient.name))
            .collect();
        outcomes.push(format!(
            "{}: {accepted}",
            names.into_iter().collect::<Vec<_>>().join("/")
        ));
    }
    Ok(outcomes.join("; "))
}

#[cfg(test)]
mod tests;
