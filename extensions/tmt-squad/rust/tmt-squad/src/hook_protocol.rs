//! Squad's side of the consented extension hook protocol:
//! `tmt-squad __tmt-hooks 1 capabilities|observe`.
//!
//! Core runs it only after the user enabled squad's hooks, after a command
//! committed, within a shared deadline. Squad observes one event: an identity
//! rename, so `me` in `squad.toml` follows the user's renamed identity at
//! once. Hooks are optional: without them the next command that needs `me`
//! repairs it (`me::resolve`). Output is empty, and core ignores the status.

use crate::{config::Config, core::Core, me};
use serde_json::Value;
use std::{
    ffi::OsString,
    io::{self, Read, Write},
    process::ExitCode,
};

pub const PREFIX: &str = "__tmt-hooks";
const PROTOCOL_VERSION: &str = "1";
const LIFECYCLE_CAPABILITY: &str = "lifecycle_observations_v1";
const INPUT_LIMIT: u64 = 64 * 1024;

pub fn run(arguments: &[OsString]) -> ExitCode {
    let operation = match arguments {
        [prefix, version, operation] if prefix == PREFIX && version == PROTOCOL_VERSION => {
            operation.to_string_lossy().into_owned()
        }
        _ => return ExitCode::from(2),
    };
    match operation.as_str() {
        "capabilities" => print_capabilities(),
        "observe" => {
            let mut input = Vec::new();
            let read = io::stdin()
                .lock()
                .take(INPUT_LIMIT + 1)
                .read_to_end(&mut input);
            if read.is_err() || input.len() as u64 > INPUT_LIMIT {
                return ExitCode::from(2);
            }
            let Some(renamed) = renamed_identities(&input) else {
                return ExitCode::from(2);
            };
            if renamed.is_empty() {
                return ExitCode::SUCCESS;
            }
            let followed = Core::discover().and_then(|core| {
                let mut config = Config::load(&core)?;
                renamed
                    .iter()
                    .try_for_each(|id| me::renamed(&core, &mut config, id).map(drop))
            });
            if followed.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        _ => ExitCode::from(2),
    }
}

/// The protocol handshake, byte for byte.
fn print_capabilities() -> ExitCode {
    let mut stdout = tmt_cli_style::stream::stdout(true);
    let reply = format!("TMT-HOOKS/{PROTOCOL_VERSION}\n{LIFECYCLE_CAPABILITY}\n");
    match stdout
        .write_all(reply.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// The identity IDs an observation renamed, in order; `None` when the input
/// is not `{"version":1,"events":[...]}`. Other events are ignored.
fn renamed_identities(input: &[u8]) -> Option<Vec<String>> {
    let value: Value = serde_json::from_slice(input).ok()?;
    if value["version"] != 1 {
        return None;
    }
    Some(
        value["events"]
            .as_array()?
            .iter()
            .filter(|event| event["kind"] == "identity.renamed")
            .filter_map(|event| event["identityId"].as_str())
            .filter(|id| crate::config::uuid_like(id))
            .map(str::to_owned)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f";

    #[test]
    fn only_rename_events_with_a_uuid_are_followed() {
        let input = serde_json::json!({"version": 1, "events": [
            {"kind": "identity.created", "identityId": ID, "lifetime": "saved", "retired": false},
            {"kind": "identity.renamed", "identityId": ID, "lifetime": "saved", "retired": false},
            {"kind": "identity.renamed", "identityId": "not-a-uuid"},
            {"kind": "room.updated", "roomId": ID, "revision": 2, "retired": false},
        ]});
        assert_eq!(
            renamed_identities(input.to_string().as_bytes()),
            Some(vec![ID.to_owned()])
        );
        assert_eq!(renamed_identities(br#"{"version":2,"events":[]}"#), None);
        assert_eq!(renamed_identities(b"not json"), None);
    }
}
