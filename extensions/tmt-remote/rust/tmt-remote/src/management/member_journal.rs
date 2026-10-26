//! Actor-member binding in the existing journal; no historical row or public outcome is rewritten.
use super::*;

pub(super) fn stored(c: &Connection, grant: &Grant, outcome: Value) -> Result<Value> {
    let actor = crate::members::member_for_client_in(c, &grant.client_id)?.ok_or_else(invalid)?;
    Ok(json!({"actorMemberId":actor.member_id,"outcome":outcome}))
}
pub(super) fn public(c: &Connection, client: &str, stored: Value) -> Result<Value> {
    let actor = crate::members::member_for_client_in(c, client)?.ok_or_else(unavailable)?;
    let object = stored.as_object().ok_or_else(unavailable)?;
    if object.contains_key("actorMemberId") {
        if object.len() != 2
            || object.get("actorMemberId").and_then(Value::as_str) != Some(actor.member_id.as_str())
            || object.get("outcome").is_none_or(|v| !v.is_object())
        {
            return Err(unavailable());
        }
        return Ok(stored["outcome"].clone());
    }
    // Before member-bound envelopes, a legacy browser's member ID was its client
    // ID. Unknown historical ownership must never cross a changed association.
    if actor.member_id != client {
        return Err(unavailable());
    }
    Ok(stored)
}
pub(super) fn settle(c: &Connection, id: &str, outcome: &Value) -> Result<Value> {
    let text: String = c
        .query_row(
            "SELECT outcome FROM management_receipts WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(database)?;
    let mut stored: Value = serde_json::from_str(&text).map_err(database)?;
    if stored.get("actorMemberId").is_none() {
        return Ok(outcome.clone());
    }
    if !stored["actorMemberId"].is_string() {
        return Err(unavailable());
    }
    stored["outcome"] = outcome.clone();
    Ok(stored)
}
fn unavailable() -> RemoteError {
    RemoteError::new(
        "REMOTE_MANAGEMENT_UNAVAILABLE",
        "Original management outcome is unavailable for this member incarnation.",
    )
}
