//! Core's side of the version-1 remote message boundary: hand one prepared request to Remote's
//! `message --json` command and read its single answer (`contracts/remote-message-v1.md`).
//! Remote owns the wire to the peer; this module owns what Core sends, what it accepts back, and
//! how every way the exchange can end maps to what is known about delivery.

use crate::process::{CommandFailure, CommandRequest, CommandRunner};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    time::Instant,
};

/// The message protocol this module speaks; an installed Remote must declare it.
pub const PROTOCOL: u8 = 1;

const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// One prepared request. `reply` is the request's normal receipt: Remote keeps it locally to
/// submit the peer's final with `tmt reply`, and never sends it to the peer or logs it.
pub struct RemoteMessage<'a> {
    pub remote: &'a str,
    /// The bare UUID of the request, without its `req_` prefix.
    pub request_uuid: &'a str,
    pub created_at_ms: u64,
    pub from_id: &'a str,
    pub from_name: &'a str,
    /// Set when the target was named by UUID; `target_name` is then the expected name.
    pub target_id: Option<&'a str>,
    pub target_name: &'a str,
    pub message: &'a str,
    pub reply_request_id: &'a str,
    pub reply_receipt: &'a str,
}

/// What is known about delivery once the command has ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteMessageOutcome {
    /// Stored in the peer's inbox; not yet delivered to its agent.
    Sent,
    /// Held for the peer owner's approval.
    Held,
    /// Refused before any effect.
    Refused { reason: Option<String> },
    /// The peer may have processed it. Never resend.
    Unconfirmed { reason: Option<String> },
    /// The route is unavailable; nothing was delivered.
    Unavailable { reason: Option<String> },
    /// Remote reported an error before any effect, or could not start: nothing was sent.
    NotSent { code: String },
    /// Remote may have sent it: it timed out, died, or gave no valid answer.
    Unknown { detail: &'static str },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Wire<'a> {
    version: u8,
    remote: &'a str,
    request_id: &'a str,
    created_at_ms: u64,
    from: Party<'a>,
    target: Target<'a>,
    message: &'a str,
    reply: Reply<'a>,
}

#[derive(Serialize)]
struct Party<'a> {
    id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct Target<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Reply<'a> {
    request_id: &'a str,
    receipt: &'a str,
}

#[derive(Deserialize)]
struct Answer {
    version: u64,
    state: String,
    remote: String,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
}

/// Runs `program message --json` with the request on stdin and classifies how it ended. A nonzero
/// exit with Remote's error envelope is the only proof nothing was sent besides a refused spawn.
pub fn send(
    runner: &dyn CommandRunner,
    program: &OsStr,
    message: &RemoteMessage<'_>,
    deadline: Instant,
) -> RemoteMessageOutcome {
    let wire = Wire {
        version: PROTOCOL,
        remote: message.remote,
        request_id: message.request_uuid,
        created_at_ms: message.created_at_ms,
        from: Party {
            id: message.from_id,
            name: message.from_name,
        },
        target: Target {
            id: message.target_id,
            name: message.target_name,
        },
        message: message.message,
        reply: Reply {
            request_id: message.reply_request_id,
            receipt: message.reply_receipt,
        },
    };
    let Ok(input) = serde_json::to_vec(&wire) else {
        return RemoteMessageOutcome::NotSent {
            code: "REMOTE_MESSAGE_INVALID".into(),
        };
    };
    let args = [OsString::from("message"), OsString::from("--json")];
    let result = runner.execute(CommandRequest {
        program,
        args: &args,
        input: &input,
        deadline,
        max_output_bytes: MAX_OUTPUT_BYTES,
    });
    match result {
        Ok(output) => classify(message, &output.stdout),
        Err(error) => match error.kind {
            CommandFailure::Spawn => RemoteMessageOutcome::NotSent {
                code: "REMOTE_UNAVAILABLE".into(),
            },
            CommandFailure::Exit { .. } => error
                .output
                .as_ref()
                .and_then(|output| serde_json::from_slice::<ErrorEnvelope>(&output.stdout).ok())
                .filter(|failure| fixed(&failure.error.code))
                .map_or(
                    RemoteMessageOutcome::Unknown {
                        detail: "Remote exited without a valid answer",
                    },
                    |failure| RemoteMessageOutcome::NotSent {
                        code: failure.error.code,
                    },
                ),
            CommandFailure::Timeout => RemoteMessageOutcome::Unknown {
                detail: "Remote did not answer in time",
            },
            CommandFailure::OutputLimit | CommandFailure::Io => RemoteMessageOutcome::Unknown {
                detail: "Remote's answer could not be read",
            },
        },
    }
}

fn classify(message: &RemoteMessage<'_>, stdout: &[u8]) -> RemoteMessageOutcome {
    let unknown = RemoteMessageOutcome::Unknown {
        detail: "Remote gave no valid answer",
    };
    let Ok(answer) = serde_json::from_slice::<Answer>(stdout) else {
        return unknown;
    };
    if answer.version != u64::from(PROTOCOL) || answer.remote != message.remote {
        return unknown;
    }
    let reason = answer.reason.filter(|reason| fixed(reason));
    match answer.state.as_str() {
        "sent" => RemoteMessageOutcome::Sent,
        "held" => RemoteMessageOutcome::Held,
        "refused" => RemoteMessageOutcome::Refused { reason },
        "unconfirmed" => RemoteMessageOutcome::Unconfirmed { reason },
        "unavailable" => RemoteMessageOutcome::Unavailable { reason },
        _ => unknown,
    }
}

/// A fixed Remote code, not text: only these are ever shown or recorded.
fn fixed(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
mod tests;
