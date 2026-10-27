//! The browser's adopted original settles in the invitation writer transaction.
use super::*;
use crate::invitations::port::{Committed, OwnerOperation, Snapshot};

const CONFIRM: &str = "remote.invites.confirm";

pub(crate) fn check(
    tx: &Transaction<'_>,
    client: &str,
    operation: &OwnerOperation,
    snapshot: &Snapshot,
    sending: bool,
    now: u64,
) -> Result<()> {
    if operation.offer != snapshot.context.offer
        || operation.snapshot != snapshot.digest
        || operation.sending != sending
        || operation.id == snapshot.context.offer
    {
        return Err(invalid());
    }
    let grant = grant_in(tx, client)?.ok_or_else(invalid)?;
    require_writable(tx, &grant)?;
    let row: Option<(String, String, Vec<u8>)> = tx
        .query_row(
            "SELECT client_id,operation,digest FROM management_receipts WHERE id=?1",
            [&operation.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(database)?;
    if row.as_ref().is_none_or(|(actor, kind, digest)| {
        actor != client || kind != CONFIRM || digest.as_slice() != operation.intent
    }) {
        return Err(conflict());
    }
    // This also checks the immutable lookup deadline and M2 actor incarnation.
    // A settled or differently owned original is never overwritten by an effect.
    if receipt(tx, client, &operation.id, now)? != unknown(&operation.id) {
        return Err(conflict());
    }
    Ok(())
}

pub(crate) fn settle(
    tx: &Transaction<'_>,
    operation: &OwnerOperation,
    committed: &Committed,
    persisted: &Grant,
) -> Result<()> {
    super::settle(
        tx,
        &operation.id,
        &json!({
            "operationId":operation.id,"state":"committed","sessionEnded":false,
            "result":{"invitation":{
                "offerId":committed.snapshot.context.offer,
                "clientId":committed.snapshot.candidate.machine,
                "memberId":committed.member,
                "state":"confirmed",
                "sendingEnabled":persisted.permits_scope("talk")
            }}
        }),
    )
}
fn conflict() -> RemoteError {
    RemoteError::new(
        "REMOTE_INTENT_CONFLICT",
        "Original invitation decision conflicts.",
    )
}
