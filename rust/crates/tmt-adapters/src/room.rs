//! Strict local meeting-room wire values; membership remains storage-owned.

use crate::{
    identity_projection::{identity_value, metadata_value},
    identity_status::status_value,
    storage::RoomRoster,
};
use serde::{Deserialize, Serialize};
use tmt_core::room::{MeetingRoom, RoomWrite};

pub const INPUT_LIMIT: usize = 8192;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WriteWire {
    expected_revision: u64,
    name: String,
    member_ids: Vec<String>,
}

pub fn decode_write(bytes: &[u8]) -> Option<RoomWrite> {
    if bytes.len() > INPUT_LIMIT {
        return None;
    }
    let input: WriteWire = serde_json::from_slice(bytes).ok()?;
    RoomWrite {
        expected_revision: input.expected_revision,
        name: input.name,
        member_ids: input.member_ids,
    }
    .normalize()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetireWire {
    expected_revision: u64,
}

pub fn decode_retire(bytes: &[u8]) -> Option<u64> {
    if bytes.len() > INPUT_LIMIT {
        return None;
    }
    let input: RetireWire = serde_json::from_slice(bytes).ok()?;
    (input.expected_revision > 0
        && input.expected_revision <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
        .then_some(input.expected_revision)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomWire<'a> {
    id: &'a str,
    name: &'a str,
    revision: u64,
    retired: bool,
    member_ids: &'a [String],
}
impl<'a> From<&'a MeetingRoom> for RoomWire<'a> {
    fn from(room: &'a MeetingRoom) -> Self {
        Self {
            id: &room.id,
            name: &room.name,
            revision: room.revision,
            retired: room.retired,
            member_ids: &room.member_ids,
        }
    }
}
pub fn encode_room(room: &MeetingRoom) -> Vec<u8> {
    serde_json::to_vec(&RoomWire::from(room)).expect("serializable room values")
}
pub fn encode_rooms(rooms: &[MeetingRoom]) -> Vec<u8> {
    serde_json::to_vec(&rooms.iter().map(RoomWire::from).collect::<Vec<_>>())
        .expect("serializable room list")
}

/// Members extend the shared identity projection with metadata and status.
/// Presence is deliberately absent: it needs host observation, not storage.
pub fn encode_roster(roster: &RoomRoster, now_ms: u64) -> Vec<u8> {
    let members: Vec<serde_json::Value> = roster
        .members
        .iter()
        .map(|member| {
            let mut value = identity_value(&member.identity);
            value["metadata"] = metadata_value(&member.metadata);
            value["status"] = status_value(member.status.as_ref(), now_ms);
            value
        })
        .collect();
    serde_json::to_vec(
        &serde_json::json!({"room": RoomWire::from(&roster.room), "members": members}),
    )
    .expect("serializable room roster")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn room_wire_admits_empty_rosters_and_rejects_invalid_or_unknown_fields() {
        let valid = json!({"expectedRevision":0,"name":"Review room","memberIds":[]});
        assert!(decode_write(&serde_json::to_vec(&valid).unwrap()).is_some());
        for (key, value) in [
            ("expectedRevision", json!(-1)),
            ("name", json!("\n")),
            ("name", json!("x".repeat(81))),
            ("memberIds", json!(["alice"])),
            ("everyone", json!(true)),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(decode_write(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
        assert!(
            decode_write(br#"{"expectedRevision":0,"name":"A","name":"B","memberIds":[]}"#)
                .is_none()
        );
    }
}
