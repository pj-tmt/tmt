//! Read-only durable change cursor projection.

use super::Fault;
use crate::storage::Storage;
use serde_json::json;

pub(super) fn cursor(storage: &Storage) -> Result<Vec<u8>, Fault> {
    storage
        .change_cursor()
        .map(|cursor| serde_json::to_vec(&json!({"cursor": cursor})).expect("change cursor"))
        .map_err(|_| Fault::unavailable())
}
