//! Bounded UUID reference admission and existing state projections.

use super::{Fault, REFERENCE_LIMIT, Request, invalid};
use crate::storage::Storage;
use serde::Deserialize;
use serde_json::json;
use tmt_core::room::RoomRepository;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReferencesInput {
    #[serde(default)]
    identity_ids: Vec<String>,
    #[serde(default)]
    room_ids: Vec<String>,
}

pub(super) fn decode(input: &[u8]) -> Result<Request, Fault> {
    Ok({
        let value: ReferencesInput = serde_json::from_slice(input).map_err(|_| invalid())?;
        if value.identity_ids.len() + value.room_ids.len() > REFERENCE_LIMIT
            || !value
                .identity_ids
                .iter()
                .chain(&value.room_ids)
                .all(|id| tmt_core::dispatch::canonical_id(id))
        {
            return Err(invalid());
        }
        Request::References {
            identities: value.identity_ids,
            rooms: value.room_ids,
        }
    })
}

pub(super) fn resolve(
    storage: &mut Storage,
    identities: Vec<String>,
    rooms: Vec<String>,
) -> Result<Vec<u8>, Fault> {
    let mut identity_states = Vec::with_capacity(identities.len());
    for id in &identities {
        let state = match storage
            .find_identity_by_id(id)
            .map_err(|_| Fault::unavailable())?
        {
            None => json!({"id": id, "found": false}),
            Some(identity) => {
                let retired = storage
                    .find_active_identity_by_id(id)
                    .map_err(|_| Fault::unavailable())?
                    .is_none();
                json!({"id": id, "found": true, "name": identity.name,
                    "lifetime": identity.lifetime.as_str(), "retired": retired})
            }
        };
        identity_states.push(state);
    }
    let mut room_states = Vec::with_capacity(rooms.len());
    for id in &rooms {
        let state = match storage.find_historical_meeting_room(id) {
            Ok(Some(room)) => json!({"id": id, "found": true, "retired": room.retired}),
            Ok(None) | Err(crate::storage::RoomStoreError::Invalid) => {
                json!({"id": id, "found": false})
            }
            Err(_) => return Err(Fault::unavailable()),
        };
        room_states.push(state);
    }
    Ok(
        serde_json::to_vec(&json!({"identities": identity_states, "rooms": room_states}))
            .expect("reference states"),
    )
}
