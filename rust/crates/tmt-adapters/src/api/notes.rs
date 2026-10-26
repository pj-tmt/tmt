//! Saved-identity notebook admission and bounded reads.

use super::{Fault, Request, invalid};
use crate::{config::ConfigPaths, notes};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdentityInput {
    identity_id: String,
}

pub(super) fn decode(input: &[u8]) -> Result<Request, Fault> {
    Ok({
        let value: IdentityInput = serde_json::from_slice(input).map_err(|_| invalid())?;
        if !tmt_core::dispatch::canonical_id(&value.identity_id) {
            return Err(invalid());
        }
        Request::Notes(value.identity_id)
    })
}

pub(super) fn open_fault(error: crate::storage::StorageError) -> Fault {
    Fault::new(
        "NOTEBOOK_UNAVAILABLE",
        "Saved-identity notes could not be read.",
    )
    .with_storage_open(error)
}
pub(super) fn read(
    paths: &ConfigPaths,
    storage: &mut crate::storage::Storage,
    id: String,
) -> Result<Vec<u8>, Fault> {
    notes::read_with_storage(paths, storage, &id)
        .map(|note| notes::encode(&note))
        .map_err(|error| Fault::new(error.code(), "Saved-identity notes could not be read."))
}
