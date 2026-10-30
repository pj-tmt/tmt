//! The browser's adopted original settles in the invitation writer transaction.
use super::*;
use crate::invitations::port::{Committed, OwnerDecision, OwnerOperation, Snapshot};

const CONFIRM: &str = "remote.invites.confirm";

pub(crate) fn check(
    tx: &Transaction<'_>,
    client: &str,
    operation: &OwnerOperation,
    snapshot: &Snapshot,
    decision: OwnerDecision,
    now: u64,
) -> Result<()> {
    if operation.offer != snapshot.context.offer
        || operation.snapshot != snapshot.digest
        || operation.matched != decision.matched
        || operation.sending != decision.enable_sending
        || operation.id == snapshot.context.offer
    {
        return Err(invalid());
    }
    let grant = grant_in(tx, client)?.ok_or_else(invalid)?;
    require_writable(tx, &grant)?;
    if adopted(tx, client, &operation.id, &operation.intent, now)? != unknown(&operation.id) {
        return Err(conflict());
    }
    Ok(())
}

fn adopted(
    tx: &Transaction<'_>,
    client: &str,
    id: &str,
    intent: &[u8; 32],
    now: u64,
) -> Result<Value> {
    adopted_for(tx, client, id, intent, CONFIRM, now)
}
fn adopted_for(
    tx: &Transaction<'_>,
    client: &str,
    id: &str,
    intent: &[u8; 32],
    expected: &str,
    now: u64,
) -> Result<Value> {
    let row: Option<(String, String, Vec<u8>)> = tx
        .query_row(
            "SELECT client_id,operation,digest FROM management_receipts WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(database)?;
    if row.as_ref().is_none_or(|(actor, operation, digest)| {
        actor != client || operation != expected || digest.as_slice() != intent
    }) {
        return Err(conflict());
    }
    // Existing public lookup owns the immutable deadline and actor-member fence.
    receipt(tx, client, id, now)
}
impl Store {
    /// Records only a proven no-effect original, without restoring lost authority.
    /// An already recorded outcome remains immutable; no grant or provider work runs.
    pub(crate) fn refuse_invitation_confirmation(
        &mut self,
        grant: &Grant,
        id: &str,
        intent: &[u8; 32],
        code: &str,
    ) -> Result<Value> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database)?;
        let previous = adopted(&tx, &grant.client_id, id, intent, now_ms()?)?;
        let outcome = if previous == unknown(id) {
            let outcome = refused(id, code);
            super::settle(&tx, id, &outcome)?;
            outcome
        } else {
            previous
        };
        tx.commit().map_err(database)?;
        Ok(outcome)
    }
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

pub(crate) fn decline(
    tx: &Transaction<'_>,
    operation: &OwnerOperation,
    snapshot: &Snapshot,
) -> Result<()> {
    super::settle(
        tx,
        &operation.id,
        &json!({
            "operationId":operation.id,"state":"committed","sessionEnded":false,
            "result":{"invitation":{"offerId":snapshot.context.offer,
                "clientId":snapshot.candidate.machine,"state":"refused"}}
        }),
    )
}

/// The local offer reservation and original status are atomic. No code/link is
/// accepted by this writer; any recovered or replayed outcome is metadata only.
pub(crate) fn issued(
    tx: &Transaction<'_>,
    client: &str,
    context: &crate::invitations::Context,
    id: &str,
    intent: &[u8; 32],
    now: u64,
) -> Result<()> {
    if context.offer == id
        || adopted_for(tx, client, id, intent, "remote.invites.create", now)? != unknown(id)
    {
        return Err(conflict());
    }
    super::settle(
        tx,
        id,
        &json!({"operationId":id,"state":"committed","sessionEnded":false,
        "result":{"invitation":{"offerId":context.offer,"state":"created","expiresAtMs":context.expires_at_ms}}}),
    )
}
impl Store {
    pub(crate) fn adopt_invitation_action(
        &mut self,
        grant: &Grant,
        id: &str,
        operation: &str,
        intent: &[u8; 32],
    ) -> Result<Option<Value>> {
        if grant.kind != "browser"
            || canonical::uuid(id).is_err()
            || !matches!(operation, "remote.invites.create" | "remote.invites.cancel")
        {
            return Err(invalid());
        }
        self.management_adopt(grant, id, operation, intent, now_ms()?, Access::Legacy)
    }
    pub(crate) fn settle_invitation_action(
        &mut self,
        grant: &Grant,
        id: &str,
        operation: &str,
        intent: &[u8; 32],
        outcome: Value,
    ) -> Result<Value> {
        let tx = self.authorized(grant, now_ms()?)?;
        require_writable(&tx, grant)?;
        let previous = adopted_for(&tx, &grant.client_id, id, intent, operation, now_ms()?)?;
        let result = if previous == unknown(id) {
            super::settle(&tx, id, &outcome)?;
            outcome
        } else {
            previous
        };
        tx.commit().map_err(database)?;
        Ok(result)
    }
}
