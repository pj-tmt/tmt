//! Versioned local process protocol. Resource owners retain admission and encoding.

use crate::{
    config::{ConfigFiles, ConfigPaths},
    dispatch, notes, request_history,
    request_runtime::wall_time_ms,
    room,
    storage::{RosterError, Storage, StorageError},
};
use serde::Deserialize;
use serde_json::{json, value::RawValue};
use tmt_core::{
    dispatch::DispatchInput,
    identity_metadata::MetadataKey,
    request::{Originator, RequestService, history::HistoryQuery},
    room::{RoomRepository, RoomWrite},
};

// Preserve the canonical message limit even when every byte is JSON-escaped.
pub const INPUT_LIMIT: usize = dispatch::INPUT_LIMIT + 4096;
pub const OUTPUT_LIMIT: usize = 12 * 1_048_576 + 65_536;
const OPS: &[&str] = &[
    "capabilities",
    "requests.list",
    "requests.show",
    "dispatch.show",
    "dispatch.create",
    "rooms.write",
    "rooms.roster",
    "notes.read",
];

#[derive(Debug)]
pub struct Fault {
    code: &'static str,
    message: &'static str,
}
impl Fault {
    pub fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
    pub fn unavailable() -> Self {
        Self::new(
            "API_UNAVAILABLE",
            "Operation could not be confirmed. For a write, inspect its operation receipt or current revision before retrying.",
        )
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut error = json!({"code":self.code,"message":self.message});
        if self.code == "API_VERSION_UNSUPPORTED" {
            error["supported"] = json!({"min":1,"max":1});
        }
        serde_json::to_vec(&json!({"error":error})).expect("bounded error")
    }
}
fn invalid() -> Fault {
    Fault::new(
        "API_INPUT_INVALID",
        "Invalid operation, fields or bounded input.",
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u64,
    operation: String,
    #[serde(default)]
    identity: Option<String>,
    input: Box<RawValue>,
}
pub enum Request {
    Capabilities,
    History(HistoryQuery),
    Detail(String),
    Receipt(String),
    Dispatch {
        identity: String,
        input: DispatchInput,
    },
    Room {
        identity: String,
        id: String,
        input: RoomWrite,
    },
    Notes(String),
    Roster {
        room: String,
        prefix: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomInput {
    room_id: String,
    room: Box<RawValue>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RosterInput {
    room: String,
    #[serde(default)]
    metadata_prefix: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdentityInput {
    identity_id: String,
}

pub fn decode(body: &str) -> Result<Request, Fault> {
    if body.len() > INPUT_LIMIT {
        return Err(invalid());
    }
    let wire: Envelope = serde_json::from_str(body).map_err(|_| invalid())?;
    if wire.version != 1 {
        return Err(Fault::new(
            "API_VERSION_UNSUPPORTED",
            "Unsupported API major version.",
        ));
    }
    let input = wire.input.get().as_bytes();
    let writing = matches!(wire.operation.as_str(), "dispatch.create" | "rooms.write");
    if writing != wire.identity.is_some()
        || wire
            .identity
            .as_ref()
            .is_some_and(|id| id.trim().is_empty() || id.len() > 256)
    {
        return Err(invalid());
    }
    Ok(match wire.operation.as_str() {
        "capabilities"
            if serde_json::from_slice::<serde_json::Value>(input).is_ok_and(|v| v == json!({})) =>
        {
            Request::Capabilities
        }
        "requests.list" => {
            Request::History(request_history::decode_history_query(input).ok_or_else(invalid)?)
        }
        "requests.show" => {
            Request::Detail(request_history::decode_history_request(input).ok_or_else(invalid)?)
        }
        "dispatch.show" => {
            Request::Receipt(dispatch::decode_dispatch_lookup(input).ok_or_else(invalid)?)
        }
        "dispatch.create" => Request::Dispatch {
            identity: wire.identity.expect("write identity"),
            input: dispatch::decode_input(input).ok_or_else(invalid)?,
        },
        "rooms.write" => {
            let value: RoomInput = serde_json::from_slice(input).map_err(|_| invalid())?;
            if !tmt_core::dispatch::canonical_id(&value.room_id) {
                return Err(invalid());
            }
            Request::Room {
                identity: wire.identity.expect("write identity"),
                id: value.room_id,
                input: room::decode_write(value.room.get().as_bytes()).ok_or_else(invalid)?,
            }
        }
        "rooms.roster" => {
            let value: RosterInput = serde_json::from_slice(input).map_err(|_| invalid())?;
            // The key grammar is prefix-closed: every non-empty prefix of a valid
            // key is itself a valid key, so key admission also admits prefixes.
            if value.room.is_empty()
                || value.room.len() > 256
                || value
                    .metadata_prefix
                    .as_deref()
                    .is_some_and(|prefix| MetadataKey::parse(prefix).is_err())
            {
                return Err(invalid());
            }
            Request::Roster {
                room: value.room,
                prefix: value.metadata_prefix,
            }
        }
        "notes.read" => {
            let value: IdentityInput = serde_json::from_slice(input).map_err(|_| invalid())?;
            if !tmt_core::dispatch::canonical_id(&value.identity_id) {
                return Err(invalid());
            }
            Request::Notes(value.identity_id)
        }
        _ => return Err(invalid()),
    })
}

pub fn capabilities() -> Vec<u8> {
    serde_json::to_vec(&json!({"version":1,"supported":{"min":1,"max":1},"operations":OPS,"limits":{"inputBytes":INPUT_LIMIT,"outputBytes":OUTPUT_LIMIT},"commands":["identity","list","room","x","reply","result","notes"]})).expect("constant capabilities")
}

fn identity(storage: &mut Storage, selector: &str) -> Result<String, Fault> {
    let found = storage
        .resolve_identity(selector)
        .map_err(|_| Fault::unavailable())?;
    found.map(|id| id.id).ok_or_else(|| {
        Fault::new(
            "NAME_NOT_FOUND",
            "Explicit originator identity was not found.",
        )
    })
}

pub fn execute(paths: &ConfigPaths, request: Request) -> Result<Vec<u8>, Fault> {
    if matches!(request, Request::Capabilities) {
        return Ok(capabilities());
    }
    if let Request::Notes(id) = request {
        return notes::read(paths, &id)
            .map(|note| notes::encode(&note))
            .map_err(|error| Fault::new(error.code(), "Saved-identity notes could not be read."));
    }
    // Only dispatch uses settings; read operations do not depend on unrelated config.
    let settings = if matches!(request, Request::Dispatch { .. }) {
        Some(
            ConfigFiles {
                paths: paths.clone(),
            }
            .load()
            .map_err(|_| Fault::new("CONFIG_ERROR", "Could not load dispatch settings."))?
            .settings,
        )
    } else {
        None
    };
    let mut storage = Storage::open(&paths.database).map_err(|_| Fault::unavailable())?;
    let pending = (|| match request {
        Request::Capabilities | Request::Notes(_) => unreachable!("handled before storage"),
        Request::Roster { room, prefix } => storage
            .room_roster(&room, prefix.as_deref())
            .map(|roster| room::encode_roster(&roster, wall_time_ms()))
            .map_err(|error| match error {
                RosterError::NotFound => Fault::new("ROOM_NOT_FOUND", "Active room was not found."),
                RosterError::Ambiguous => Fault::new(
                    "ROOM_AMBIGUOUS",
                    "Room name is not unique; select the room by UUID.",
                ),
                RosterError::Storage(_) => Fault::unavailable(),
            }),
        Request::History(query) => RequestService::new(&mut storage, wall_time_ms)
            .request_history(query)
            .map(|value| request_history::encode_history_page(&value))
            .map_err(request_error),
        Request::Detail(id) => RequestService::new(&mut storage, wall_time_ms)
            .request_detail(&id)
            .map(|value| request_history::encode_history_detail(&value))
            .map_err(request_error),
        Request::Receipt(id) => storage
            .dispatch_receipt(&id)
            .map_err(|_| Fault::unavailable())?
            .map(|value| dispatch::encode_receipt(&value))
            .ok_or_else(|| Fault::new("DISPATCH_NOT_FOUND", "Operation receipt was not found.")),
        Request::Room {
            identity: selector,
            id,
            input,
        } => {
            identity(&mut storage, &selector)?;
            storage
                .save_meeting_room(&id, input)
                .map(|value| room::encode_room(&value))
                .map_err(|error| {
                    Fault::new(
                        error.code(),
                        "Room write failed; refresh its revision and membership before retrying.",
                    )
                })
        }
        Request::Dispatch {
            identity: selector,
            mut input,
        } => {
            input.originator = Originator::Explicit(identity(&mut storage, &selector)?);
            let settings = settings.expect("dispatch settings");
            let direct = input.kind == tmt_core::request::RequestKind::Request
                && input.recipient_ids.len() == 1
                && !matches!(
                    input.room.as_ref(),
                    Some(tmt_core::dispatch::DispatchRoom::Roster { .. })
                );
            let (receipt, created) = storage.dispatch_request_with_creation(input, settings.retention_days, wall_time_ms).map_err(|error| Fault::new(error.code(), "Dispatch could not be confirmed; retain the operation ID and recover its receipt."))?;
            let wake = if direct
                && created
                && receipt
                    .items
                    .first()
                    .is_some_and(|item| item.acceptance == tmt_core::dispatch::Acceptance::Queued)
            {
                let item = &receipt.items[0];
                let message = format!(
                    "[tmt] request {} is queued: tmt x show {} --incoming --identity {} --json",
                    item.request_id, item.request_id, item.recipient_id
                );
                Some(crate::delivery::wake_request(
                    &mut storage,
                    &item.request_id,
                    &item.recipient_id,
                    &message,
                    std::time::Duration::from_secs_f64(
                        settings.paste_enter_delay_ms.min(500.0) / 1000.0,
                    ),
                ))
            } else {
                None
            };
            Ok(dispatch::encode_receipt_with_wake(&receipt, wake))
        }
    })();
    let closed = storage.close();
    let body = pending?;
    closed.map_err(|_| Fault::unavailable())?;
    if body.len() > OUTPUT_LIMIT {
        return Err(Fault::new(
            "API_OUTPUT_TOO_LARGE",
            "Response exceeds the API bound; request a smaller page.",
        ));
    }
    Ok(body)
}
fn request_error(error: tmt_core::request::RequestError<StorageError>) -> Fault {
    match error {
        tmt_core::request::RequestError::Invalid(_) => invalid(),
        tmt_core::request::RequestError::NotFound => {
            Fault::new("REQUEST_NOT_FOUND", "Request was not found.")
        }
        _ => Fault::unavailable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_and_nested_admission_preserve_strict_resource_decoders() {
        let bad = [
            r#"{"version":1,"operation":"rooms.write","identity":"A","input":{"roomId":"00000000-0000-4000-8000-000000000001","room":{"expectedRevision":0,"name":"A","name":"B","memberIds":[]}}}"#,
            r#"{"version":1,"operation":"capabilities","identity":"A","input":{}}"#,
            r#"{"version":1,"operation":"notes.read","input":{"identityId":"../../notes"}}"#,
            r#"{"version":1,"operation":"requests.list","input":{"limit":1}}"#,
            r#"{"version":1,"operation":"unknown","input":{}}"#,
        ];
        for body in bad {
            assert!(matches!(
                decode(body),
                Err(Fault {
                    code: "API_INPUT_INVALID",
                    ..
                })
            ));
        }
        let error = decode(r#"{"version":2,"operation":"capabilities","input":{}}"#)
            .err()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&error.encode()).unwrap()["error"]["supported"],
            json!({"min":1,"max":1})
        );
    }

    #[test]
    fn roster_is_a_read_with_a_strict_room_and_key_prefix() {
        let roster = |input: &str| {
            decode(&format!(
                r#"{{"version":1,"operation":"rooms.roster","input":{input}}}"#
            ))
        };
        let Request::Roster { room, prefix } =
            roster(r#"{"room":"Design room","metadataPrefix":"squad.a."}"#).unwrap()
        else {
            panic!("roster");
        };
        assert_eq!(
            (room.as_str(), prefix.as_deref()),
            ("Design room", Some("squad.a."))
        );
        assert!(matches!(
            roster(r#"{"room":"00000000-0000-4000-8000-000000000001"}"#),
            Ok(Request::Roster { prefix: None, .. })
        ));
        let long = format!(r#"{{"room":"{}"}}"#, "r".repeat(257));
        for input in [
            r#"{"room":""}"#,
            long.as_str(),
            r#"{"room":"A","metadataPrefix":""}"#,
            r#"{"room":"A","metadataPrefix":"Squad."}"#,
            r#"{"room":"A","metadataPrefix":"squad a"}"#,
            r#"{"room":"A","presence":true}"#,
            r#"{"room":"A","room":"B"}"#,
            r#"{}"#,
        ] {
            assert!(
                matches!(
                    roster(input),
                    Err(Fault {
                        code: "API_INPUT_INVALID",
                        ..
                    })
                ),
                "{input}"
            );
        }
        assert!(matches!(
            decode(
                r#"{"version":1,"operation":"rooms.roster","identity":"A","input":{"room":"A"}}"#
            ),
            Err(Fault {
                code: "API_INPUT_INVALID",
                ..
            })
        ));
        let capabilities: serde_json::Value = serde_json::from_slice(&capabilities()).unwrap();
        assert!(
            capabilities["operations"]
                .as_array()
                .unwrap()
                .contains(&json!("rooms.roster"))
        );
    }

    #[test]
    fn envelope_allows_canonical_maximum_message_after_json_escaping() {
        let message = "\u{1b}".repeat(tmt_core::exact_text::MAX_EXCHANGE_TEXT_BYTES);
        let body = json!({"version":1,"operation":"dispatch.create","identity":"Sender","input":{
            "operationId":"00000000-0000-4000-8000-000000000001",
            "recipientIds":["00000000-0000-4000-8000-000000000002"],"message":message
        }})
        .to_string();
        assert!(body.len() < INPUT_LIMIT);
        let Request::Dispatch { input, .. } = decode(&body).unwrap() else {
            panic!("dispatch");
        };
        assert_eq!(input.message, message);
    }
}
