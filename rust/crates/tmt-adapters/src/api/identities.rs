//! Bounded identity status admission and directory projection.

use super::{Fault, REFERENCE_LIMIT, Request, invalid};
use crate::storage::Storage;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdentityStatusesInput {
    identity_ids: Vec<String>,
}

pub(super) fn decode(input: &[u8]) -> Result<Request, Fault> {
    Ok({
        let value: IdentityStatusesInput = serde_json::from_slice(input).map_err(|_| invalid())?;
        if value.identity_ids.len() > REFERENCE_LIMIT
            || !value
                .identity_ids
                .iter()
                .all(|id| tmt_core::dispatch::canonical_id(id))
        {
            return Err(invalid());
        }
        Request::IdentityStatuses {
            identities: value.identity_ids,
        }
    })
}

pub(super) fn status(storage: &Storage, identities: Vec<String>) -> Result<Vec<u8>, Fault> {
    // One directory read; expiry (`stale`) is applied here, by core.
    let statuses = storage
        .list_active_identity_statuses()
        .map_err(|_| Fault::unavailable())?;
    let now = crate::request_runtime::wall_time_ms();
    let mut entries = Vec::with_capacity(identities.len());
    for id in &identities {
        entries.push(
            match storage
                .find_identity_by_id(id)
                .map_err(|_| Fault::unavailable())?
            {
                None => json!({"id": id, "found": false}),
                Some(_) => json!({"id": id, "found": true,
                    "status": crate::identity_status::status_value(statuses.get(id), now)}),
            },
        );
    }
    Ok(serde_json::to_vec(&json!({"identities": entries})).expect("identity statuses"))
}
