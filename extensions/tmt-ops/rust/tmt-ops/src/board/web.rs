//! `g  Web`: the top bar opens the web UI by running Remote's public
//! `tmt remote open`, which opens the user's own browser on the running local
//! Remote. Ops reads only that command's published result and error codes, and
//! never installs, serves or pairs anything itself; the one-time read-only offer
//! for the opened browser is Remote's own behaviour of that command.

use crate::core::{Core, SquadError};
use serde_json::Value;

/// Runs `tmt remote open --json` and says what happened in one status line.
pub(super) fn open(core: &Core) -> Result<String, String> {
    outcome(core.json(&["remote", "open"]))
}

/// The status line for one `tmt remote open --json` result.
pub(super) fn outcome(result: Result<Value, SquadError>) -> Result<String, String> {
    match result {
        Ok(document) => {
            let link = document["link"].as_str().unwrap_or_default();
            if link.is_empty() {
                return Err("tmt remote open returned no link.".into());
            }
            Ok(if document["opened"] == true {
                format!("Opened {link} in the browser.")
            } else if document["state"] == "failed" {
                format!("Could not open the browser; open {link} yourself.")
            } else {
                format!("Open {link} in your browser.")
            })
        }
        Err(error) => Err(match error.code.as_str() {
            "EXTENSION_NOT_INSTALLED" => {
                "The web UI needs Remote: run tmt extension install remote.".into()
            }
            // Remote names the command that starts it; keep its words when it does.
            "REMOTE_NOT_RUNNING" if error.message.contains("tmt remote serve") => error.message,
            "REMOTE_NOT_RUNNING" => {
                "Remote is not running: run tmt remote serve --background.".into()
            }
            _ => error.message,
        }),
    }
}

#[cfg(test)]
mod tests;
