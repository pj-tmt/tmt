//! Append-only bounded local metadata, committed with its owning state change.
use crate::{admission, canonical, error::RemoteError, store::database};
use rusqlite::{Transaction, params};

pub const RECORDS: i64 = 100_000;
pub(crate) struct AuditMetadata<'a> {
    pub time: u64,
    pub client: &'a str,
    pub request: &'a str,
    pub operation: &'a str,
    pub operation_id: Option<&'a str>,
    pub resources: &'a [String],
    pub digest: &'a [u8],
    pub revision: u64,
    pub decision: &'a str,
    pub code: &'a str,
}
pub(crate) fn append(tx: &Transaction<'_>, event: AuditMetadata<'_>) -> Result<(), RemoteError> {
    let count: i64 = tx
        .query_row("SELECT COUNT(*) FROM audit", [], |r| r.get(0))
        .map_err(database)?;
    if count >= RECORDS {
        return Err(database("audit capacity"));
    }
    if !canonical::is_core_id(event.client)
        || !canonical::is_core_id(event.request)
        || event
            .operation_id
            .is_some_and(|id| !canonical::is_core_id(id))
        || event.resources.len() > 256
        || event.resources.iter().any(|id| !canonical::is_core_id(id))
        || event.digest.len() != 32
        || event.revision == 0
        || event.revision > 9_007_199_254_740_991
        || event.time > 9_007_199_254_740_991
        || !matches!(event.decision, "adopted" | "refused")
    {
        return Err(database("invalid audit metadata"));
    }
    let operation = if admission::scope(event.operation).is_some() {
        event.operation
    } else {
        "unsupported"
    };
    let code = if event.code.len() <= 64
        && event
            .code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b == b'_')
    {
        event.code
    } else {
        "REMOTE_STATE_UNAVAILABLE"
    };
    tx.execute("INSERT INTO audit(at_ms,client_id,envelope_id,operation,operation_id,resources_json,digest,grant_revision,decision,code) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![event.time as i64,event.client,event.request,operation,event.operation_id,serde_json::to_string(event.resources).expect("UUID list"),event.digest,event.revision as i64,event.decision,code]).map_err(database)?;
    Ok(())
}
