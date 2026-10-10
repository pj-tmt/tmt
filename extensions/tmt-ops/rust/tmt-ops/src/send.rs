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

/// What Core did with an accepted detached request. Acceptance is neither
/// a reply nor proof that the recipient's session saw anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Written to the recipient's pane.
    Sent,
    /// Handed to a channel that gives no receipt.
    Uncertain,
    /// Waiting in the recipient's inbox.
    Queued,
    /// The recipient has no live session; it waits for their inbox read.
    Offline,
    /// The recipient's digest checklist holds the notice.
    Held,
}

/// An accepted request and its disposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub request: String,
    pub disposition: Disposition,
}

impl Accepted {
    /// Reads Core's talk document. Anything not plainly sent counts as queued,
    /// so the board never claims a delivery Core did not report.
    fn from_document(document: &serde_json::Value) -> Result<Self, SquadError> {
        let request = document["requestId"].as_str().ok_or_else(|| {
            SquadError::new("SQUAD_CORE_UNAVAILABLE", "tmt talk returned no request ID.")
        })?;
        let disposition = if document["deliveryState"] == "uncertain" {
            Disposition::Uncertain
        } else if document["notification"] == "held" {
            Disposition::Held
        } else if document["offline"] == true {
            Disposition::Offline
        } else if matches!(document["status"].as_str(), Some("sent" | "completed")) {
            Disposition::Sent
        } else {
            Disposition::Queued
        };
        Ok(Self {
            request: request.to_owned(),
            disposition,
        })
    }

    /// The word under the row, such as `✓ queued`.
    pub fn mark(&self) -> &'static str {
        match self.disposition {
            Disposition::Sent => "sent",
            Disposition::Uncertain => "handed over",
            Disposition::Queued => "queued",
            Disposition::Offline => "queued · offline",
            Disposition::Held => "held",
        }
    }

    /// The board notice; `what` names the message, such as `Note on alpha`.
    pub fn describe(&self, what: &str, to: &str) -> String {
        let request = &self.request;
        match self.disposition {
            Disposition::Sent => format!("{what} sent to {to} ({request})."),
            Disposition::Uncertain => format!(
                "{what} handed to {to} ({request}); delivery is unconfirmed, do not resend."
            ),
            Disposition::Queued => format!("{what} queued in {to}'s inbox ({request})."),
            Disposition::Offline => {
                format!("{what} queued for {to} ({request}): offline, it waits for their inbox.")
            }
            Disposition::Held => {
                format!("{what} held for {to} ({request}) until their digest checklist ends.")
            }
        }
    }
}

/// A failure after Core accepted the request names that request, so nobody
/// resends a message that may still arrive.
fn retained(mut error: SquadError) -> SquadError {
    if let Some(request) = &error.request
        && !error.message.contains(request.as_str())
    {
        error.message = format!(
            "{} Request {request} is retained; do not resend.",
            error.message
        );
    }
    error
}

/// A detached request from `me` to `to` in the squad's room. Text that is
/// empty or only whitespace sends nothing.
pub fn talk(
    core: &Core,
    squad: &str,
    me: &str,
    to: &str,
    text: &str,
) -> Result<Accepted, SquadError> {
    if text.trim().is_empty() {
        return Err(refused("Nothing to send."));
    }
    let room = room_name(squad);
    let sent = core
        .json_with_operands(
            &["talk", "--identity", me, "--room", &room, "--detach"],
            &[to, text],
        )
        .map_err(retained)?;
    Accepted::from_document(&sent)
}

/// A talk tagged `[<squad> · <row>]`, so the row can show it until answered.
pub fn annotate(
    core: &Core,
    squad: &str,
    me: &str,
    to: &str,
    row: &str,
    text: &str,
) -> Result<Accepted, SquadError> {
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
    crate::id::new_v4().map_err(|error| SquadError::new("SQUAD_DISPATCH_IO", error.to_string()))
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

#[cfg(test)]
mod tests;
