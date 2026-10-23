//! User messages and tagged annotations through Core's public ports.
//! Core owns delivery, question settlement and captured turn output.

use crate::{
    core::{Core, SquadError},
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
    /// Verbatim Core delivery wording with its state mark and optional countdown.
    pub message_state_label: Option<String>,
    /// Core's `canSendNow` for that state, read in the same document as the label.
    pub can_send_now: bool,
    /// Core's `messageState.state == "working"`, read in the same document as the label.
    pub working: bool,
    /// Core accepted this as typed pane input, not a request notice.
    pub passthrough: bool,
    /// A capability-based request fallback, rendered beside its composer.
    pub fallback: Option<String>,
    /// An accepted operation whose private journal could not be removed.
    pub warning: Option<String>,
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
            message_state_label: crate::requests::message_state_label(document),
            can_send_now: crate::requests::can_send_now(document),
            working: crate::requests::is_working(document),
            passthrough: false,
            fallback: None,
            warning: None,
        })
    }

    /// The board notice; `what` names the message, such as `Note on alpha`.
    pub fn describe(&self, what: &str, to: &str) -> String {
        if let Some(text) = &self.message_state_label {
            return format!("{to}: {text}");
        }
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

/// Takes one of the person's own messages out of the recipient's Digest hold and types it now
/// (`tmt send-now`). Core decides who may ask and what the message's state allows, and a refusal
/// is its one sentence. This runs the command once and never retries: a hand-over that may have
/// happened is not repeated. The notice is the state Core reports afterwards, in its words.
pub fn send_now(core: &Core, me: &str, request: &str, to: &str) -> Result<String, SquadError> {
    core.json(&["send-now", request, "--identity", me])?;
    Ok(crate::requests::show_request(core, request)
        .ok()
        .and_then(|detail| crate::requests::message_state_label(&detail))
        .map_or_else(
            || format!("Sent now to {to}."),
            |label| format!("{to}: {label}"),
        ))
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

mod chat;
mod dispatch;
pub(crate) use chat::chat;

pub(crate) use dispatch::Intent;

#[cfg(test)]
mod tests;
