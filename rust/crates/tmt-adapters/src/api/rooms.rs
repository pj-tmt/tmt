//! Conditional room operations and snapshot roster projection.

use super::{Envelope, Fault, Request, identity, invalid};
use crate::{
    request_runtime::wall_time_ms,
    room,
    storage::{RosterError, Storage},
};
use serde::Deserialize;
use serde_json::{json, value::RawValue};
use tmt_core::{
    identity_metadata::MetadataKey,
    room::{RoomRepository, RoomWrite},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomInput {
    room_id: String,
    room: Box<RawValue>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomRetireInput {
    room_id: String,
    expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RosterInput {
    room: String,
    #[serde(default)]
    metadata_prefix: Option<String>,
}

pub(super) fn decode_write(wire: Envelope) -> Result<Request, Fault> {
    let input = wire.input.get().as_bytes();
    Ok({
        let value: RoomInput = serde_json::from_slice(input).map_err(|_| invalid())?;
        if !tmt_core::dispatch::canonical_id(&value.room_id) {
            return Err(invalid());
        }
        Request::Room {
            identity: wire.identity,
            id: value.room_id,
            input: room::decode_write(value.room.get().as_bytes()).ok_or_else(invalid)?,
        }
    })
}

pub(super) fn decode_retire(wire: Envelope) -> Result<Request, Fault> {
    let input = wire.input.get().as_bytes();
    Ok({
        let value: RoomRetireInput = serde_json::from_slice(input).map_err(|_| invalid())?;
        if !tmt_core::dispatch::canonical_id(&value.room_id) {
            return Err(invalid());
        }
        Request::RoomRetire {
            identity: wire.identity,
            id: value.room_id,
            expected_revision: room::decode_retire(
                json!({"expectedRevision": value.expected_revision})
                    .to_string()
                    .as_bytes(),
            )
            .ok_or_else(invalid)?,
        }
    })
}

pub(super) fn decode_roster(input: &[u8]) -> Result<Request, Fault> {
    Ok({
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
    })
}

pub(super) fn roster(
    storage: &Storage,
    room: String,
    prefix: Option<String>,
) -> Result<Vec<u8>, Fault> {
    storage
        .room_roster(&room, prefix.as_deref())
        .map(|roster| room::encode_roster(&roster, wall_time_ms()))
        .map_err(|error| match error {
            RosterError::NotFound => Fault::new("ROOM_NOT_FOUND", "Active room was not found."),
            RosterError::Ambiguous => Fault::new(
                "ROOM_AMBIGUOUS",
                "Room name is not unique; select the room by UUID.",
            ),
            RosterError::Storage(_) => Fault::unavailable(),
        })
}

pub(super) fn write(
    storage: &mut Storage,
    selector: Option<String>,
    id: String,
    input: RoomWrite,
) -> Result<Vec<u8>, Fault> {
    if let Some(selector) = &selector {
        identity(storage, selector)?;
    }
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

pub(super) fn retire(
    storage: &mut Storage,
    selector: Option<String>,
    id: String,
    expected_revision: u64,
) -> Result<Vec<u8>, Fault> {
    if let Some(selector) = &selector {
        identity(storage, selector)?;
    }
    storage
        .retire_meeting_room(&id, expected_revision)
        .map(|value| room::encode_room(&value))
        .map_err(|error| {
            Fault::new(
                error.code(),
                "Room retirement failed; refresh its revision before retrying.",
            )
        })
}
