//! Read-only durable change cursor projection.

use super::Fault;
use crate::storage::Storage;
use serde_json::json;

pub(super) fn cursor(storage: &Storage) -> Result<Vec<u8>, Fault> {
    // Two reads of one connection, `usage` first: a usage write between them can only
    // show as a later `usage`, never hide behind an older one.
    let usage = storage.usage_cursor().map_err(|_| Fault::unavailable())?;
    let cursor = storage.change_cursor().map_err(|_| Fault::unavailable())?;
    Ok(serde_json::to_vec(&json!({"cursor": cursor, "usage": usage})).expect("change cursor"))
}
