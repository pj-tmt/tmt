//! HOME freezes its audience; this effect revalidates it before public dispatch.
#[cfg(test)]
use super::dispatch::acceptance;
use super::{LeadRecipient, refused};
use crate::{
    config::Config,
    core::{Core, SquadError},
    me,
    squad::Squad,
};
#[cfg(test)]
use serde_json::{Value, json};
#[cfg(test)]
use std::fs;

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
    super::dispatch::send(core, config, sender, recipients, "request", None, text)
}

#[cfg(test)]
mod tests;
