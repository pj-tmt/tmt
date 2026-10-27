//! Confirmation and its original receipt share the serving Store's transaction.
//! Provider I/O never runs here; journal claims precede publication outside it.
use super::{Context, invalid, port::*};
use crate::{
    error::RemoteError,
    members, pairing,
    store::{Store, database, grant_in, machine_in},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};

const OPERATION: &str = "remote.invite.confirm";
type Result<T> = std::result::Result<T, RemoteError>;

/// Composition supplies the existing serving owner and pinned native public key.
/// This adapter neither opens a database nor restores an unconfirmed invitation.
#[derive(Clone)]
pub struct StoreConfirmation {
    store: Arc<Mutex<Store>>,
    owner_key: [u8; 32],
}

/// Transaction-local backend additions. The common owner serializes persisted
/// policy first and refuses replacement of any of its receipt fields.
#[derive(Default)]
pub(crate) struct ConfirmationExtras {
    pub receipt_fields: serde_json::Map<String, serde_json::Value>,
    pub routes: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    original: Committed,
    claimed: Vec<Part>,
    acknowledged: Vec<Part>,
}

impl StoreConfirmation {
    pub fn new(store: Arc<Mutex<Store>>, owner_key: [u8; 32]) -> Result<Self> {
        crate::crypto::public_key(&owner_key).map_err(|_| invalid())?;
        Ok(Self { store, owner_key })
    }
    /// A backend observes the original publication in this journal, never a
    /// parallel provider-state cache. It cannot create or replace confirmation.
    pub(crate) fn captured(&self, context: &Context) -> Result<Option<Committed>> {
        let store = self.lock()?;
        let saved: Option<String> = store
            .connection
            .query_row(
                "SELECT outcome FROM management_receipts WHERE id=?1 AND operation=?2",
                params![context.offer, OPERATION],
                |r| r.get(0),
            )
            .optional()
            .map_err(database)?;
        let Some(saved) = saved else {
            return Ok(None);
        };
        let record: Record = serde_json::from_str(&saved).map_err(database)?;
        if record.original.snapshot.context != *context {
            return Err(conflict());
        }
        original(
            &store.connection,
            &record.original.snapshot,
            pairing::now_ms()?,
        )
        .map(|record| record.map(|record| record.original))
    }
    fn lock(&self) -> Result<MutexGuard<'_, Store>> {
        self.store.lock().map_err(|_| {
            RemoteError::new(
                "REMOTE_INVITATION_UNAVAILABLE",
                "Invitation Store is unavailable.",
            )
        })
    }
    fn owner(
        &self,
        connection: &Connection,
        context: &Context,
        actor: &OwnerActor,
        now: u64,
    ) -> Result<()> {
        context.validate()?;
        if machine_in(connection)?.id != context.owner || context.owner_key != self.owner_key {
            return Err(invalid());
        }
        let owner = match &actor.0 {
            Actor::LocalInteractive => members::Owner::LocalOwner,
            Actor::OwnerBrowser {
                client,
                key,
                revision,
            } => {
                if grant_in(connection, client)?.is_none_or(|grant| grant.public_key != *key) {
                    return Err(invalid());
                }
                members::Owner::DesignatedBrowser {
                    client_id: client,
                    revision: *revision,
                }
            }
            Actor::Automated => return Err(invalid()),
        };
        members::authorize_owner_in(connection, &owner, now)?;
        Ok(())
    }
    fn confirm_in(
        &self,
        tx: &rusqlite::Transaction<'_>,
        input: Confirmation<'_>,
        extend: impl FnOnce(
            &rusqlite::Transaction<'_>,
            &Confirmation<'_>,
            &members::Member,
        ) -> Result<ConfirmationExtras>,
    ) -> Result<Committed> {
        let now = pairing::now_ms()?;
        let snapshot = input.snapshot;
        self.owner(tx, &snapshot.context, input.actor, now)?;
        // Only the shared coordinator holds this verified, frozen candidate.
        // Recheck it after acquiring the writer, not just before waiting on it.
        if input.verified.context() != &snapshot.context
            || input.verified.candidate() != &snapshot.candidate
            || input.verified.digest() != &snapshot.digest
            || now < snapshot.context.issued_at_ms
            || now >= snapshot.context.expires_at_ms
        {
            return Err(invalid());
        }
        let browser_operation = match (&input.actor.0, &input.actor.1) {
            (_, None) => None,
            (Actor::OwnerBrowser { client, .. }, Some(operation)) => {
                crate::management::invitation_outcomes::check(
                    tx,
                    client,
                    operation,
                    snapshot,
                    input.sending,
                    now,
                )?;
                Some(operation)
            }
            _ => return Err(invalid()),
        };
        if let Some(record) = original(tx, snapshot, now)? {
            // Another presenter already committed this offer. A newly adopted
            // browser operation cannot claim that earlier confirmation as its effect.
            if browser_operation.is_some() {
                return Err(conflict());
            }
            return Ok(record.original);
        }
        let grant = input.grant;
        if grant.client_id != snapshot.candidate.machine
            || grant.public_key != snapshot.candidate.public_key
            || grant.name != snapshot.candidate.name
            || grant.kind != "cli"
            || grant.origin != "cli"
            || grant.agents != "all"
            || grant.mode != "direct"
            || grant.disabled
            || grant.revision != 1
            || grant.expires_at_ms.is_some()
            || grant.issued_at_ms > now
            || grant.scopes.iter().map(String::as_str).collect::<Vec<_>>()
                != crate::store::DEFAULT_SCOPES
        {
            return Err(invalid());
        }
        let operation_exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM operations WHERE id=?1)",
                [&snapshot.context.offer],
                |row| row.get(0),
            )
            .map_err(database)?;
        if operation_exists {
            return Err(conflict());
        }
        // Existing management identities are permanent bounded tombstones.
        // No expired identity is deleted or made eligible for a new effect.
        let (client, total): (i64, i64) = tx.query_row(
            "SELECT (SELECT COUNT(*) FROM management_receipts WHERE client_id=?1),COUNT(*) FROM management_receipts",
            [&grant.client_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(database)?;
        if client >= crate::journal::OPERATIONS || total >= 4 * crate::journal::OPERATIONS {
            return Err(RemoteError::new(
                "REMOTE_MANAGEMENT_CAPACITY",
                "Retained management identity limit reached.",
            ));
        }
        input.io.check().map_err(|_| invalid())?;
        let member = members::confirm_member_grant_in(tx, grant)?;
        if input.sending {
            members::sending_for_client(tx, &grant.client_id, true)?.ok_or_else(invalid)?;
        }
        let extras = extend(tx, &input, &member)?;
        let persisted = grant_in(tx, &grant.client_id)?.ok_or_else(invalid)?;
        let mut receipt: serde_json::Value = serde_json::from_slice(&pairing::receipt(
            &persisted,
            &snapshot.context.owner,
            &self.owner_key,
        )?)
        .map_err(database)?;
        let fields = receipt.as_object_mut().ok_or_else(invalid)?;
        for (name, value) in extras.receipt_fields {
            if fields.insert(name, value).is_some() {
                return Err(invalid());
            }
        }
        let receipt = serde_json::to_vec(&receipt).map_err(database)?;
        if receipt.len() > crate::limits::CLOUD_ENROLLMENT_BYTES
            || extras
                .routes
                .as_ref()
                .is_some_and(|bytes| bytes.len() > crate::limits::CLOUD_ENROLLMENT_BYTES)
        {
            return Err(invalid());
        }
        let committed = Committed {
            snapshot: snapshot.clone(),
            member: member.member_id,
            grant_revision: persisted.revision,
            proof: input.receipt_key.prove(&receipt)?,
            receipt,
            routes: extras.routes,
        };
        let record = Record {
            original: committed.clone(),
            claimed: Vec::new(),
            acknowledged: Vec::new(),
        };
        tx.execute(
            "INSERT INTO management_receipts VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                snapshot.context.offer,
                persisted.client_id,
                OPERATION,
                snapshot.digest.as_slice(),
                persisted.revision as i64,
                now as i64,
                snapshot.context.expires_at_ms as i64,
                encode(&record)?
            ],
        )
        .map_err(database)?;
        if let Some(operation) = browser_operation {
            crate::management::invitation_outcomes::settle(tx, operation, &committed, &persisted)?;
        }
        // An expired/cancelled writer rolls back member, grant and both receipts alike.
        input.io.check().map_err(|_| invalid())?;
        if pairing::now_ms()? >= snapshot.context.expires_at_ms {
            return Err(invalid());
        }
        Ok(committed)
    }
    pub(crate) fn confirm_with(
        &self,
        input: Confirmation<'_>,
        extend: impl FnOnce(
            &rusqlite::Transaction<'_>,
            &Confirmation<'_>,
            &members::Member,
        ) -> Result<ConfirmationExtras>,
    ) -> std::result::Result<Committed, ConfirmationFailure> {
        input
            .io
            .check()
            .map_err(|_| ConfirmationFailure::Refused(invalid()))?;
        let mut store = self.lock().map_err(ConfirmationFailure::Refused)?;
        let tx = store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)
            .map_err(ConfirmationFailure::Refused)?;
        match self.confirm_in(&tx, input, extend) {
            Ok(committed) => {
                tx.commit()
                    .map_err(database)
                    .map_err(ConfirmationFailure::CommitUnknown)?;
                Ok(committed)
            }
            Err(error) => match tx.rollback() {
                Ok(()) => Err(ConfirmationFailure::Refused(error)),
                Err(_) => Err(ConfirmationFailure::CommitUnknown(error)),
            },
        }
    }
}

impl ConfirmationSink for StoreConfirmation {
    fn authorize(&self, context: &Context, actor: &OwnerActor) -> Result<()> {
        self.owner(&self.lock()?.connection, context, actor, pairing::now_ms()?)
    }

    fn confirm(
        &self,
        input: Confirmation<'_>,
    ) -> std::result::Result<Committed, ConfirmationFailure> {
        self.confirm_with(input, |_, _, _| Ok(ConfirmationExtras::default()))
    }

    fn original(&self, snapshot: &Snapshot) -> Result<Option<Committed>> {
        original(&self.lock()?.connection, snapshot, pairing::now_ms()?)
            .map(|record| record.map(|record| record.original))
    }

    fn claim(&self, committed: &Committed, part: Part) -> Result<Claim> {
        publication_part(part)?;
        let mut store = self.lock()?;
        let tx = store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let now = pairing::now_ms()?;
        let Some(mut record) = original(&tx, &committed.snapshot, now)? else {
            return Ok(Claim::Stale);
        };
        if record.original != *committed || !current(&tx, committed, now)? {
            return Ok(Claim::Stale);
        }
        let result = if record.claimed.contains(&part) {
            Claim::ObserveOnly
        } else {
            record.claimed.push(part);
            save(&tx, &record)?;
            Claim::FirstAttempt
        };
        tx.commit().map_err(database)?;
        Ok(result)
    }

    fn acknowledge(&self, committed: &Committed, part: Part) -> Result<bool> {
        publication_part(part)?;
        let mut store = self.lock()?;
        let tx = store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let now = pairing::now_ms()?;
        let Some(mut record) = original(&tx, &committed.snapshot, now)? else {
            return Ok(false);
        };
        if record.original != *committed
            || !record.claimed.contains(&part)
            || !current(&tx, committed, now)?
        {
            return Ok(false);
        }
        if !record.acknowledged.contains(&part) {
            record.acknowledged.push(part);
            save(&tx, &record)?;
        }
        tx.commit().map_err(database)?;
        Ok(true)
    }
}

fn original(connection: &Connection, snapshot: &Snapshot, now: u64) -> Result<Option<Record>> {
    let saved: Option<(String, Vec<u8>, String, i64)> = connection
        .query_row(
            "SELECT operation,digest,outcome,deadline_ms FROM management_receipts WHERE id=?1",
            [&snapshot.context.offer],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(database)?;
    let Some((operation, digest, outcome, deadline)) = saved else {
        return Ok(None);
    };
    if operation != OPERATION || digest.as_slice() != snapshot.digest {
        return Err(conflict());
    }
    if now >= snapshot.context.expires_at_ms || now as i64 >= deadline {
        return Err(RemoteError::new(
            "REMOTE_RESULT_EXPIRED",
            "The original invitation receipt is no longer available; its identity remains reserved.",
        ));
    }
    let record: Record = serde_json::from_str(&outcome).map_err(database)?;
    if record.original.snapshot != *snapshot {
        return Err(conflict());
    }
    Ok(Some(record))
}
fn current(connection: &Connection, record: &Committed, now: u64) -> Result<bool> {
    let Some(grant) = grant_in(connection, &record.snapshot.candidate.machine)? else {
        return Ok(false);
    };
    let member: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM member_machines mm JOIN members m USING(remote_id,member_id) WHERE mm.remote_id=?1 AND mm.member_id=?2 AND mm.client_id=?3 AND m.revoked_at_ms IS NULL)",
        params![record.snapshot.context.owner,record.member,grant.client_id], |row| row.get(0),
    ).map_err(database)?;
    Ok(member
        && grant.live_at(now)
        && grant.revision == record.grant_revision
        && grant.public_key == record.snapshot.candidate.public_key)
}
fn encode(record: &Record) -> Result<String> {
    serde_json::to_string(record).map_err(database)
}
fn save(connection: &Connection, record: &Record) -> Result<()> {
    connection
        .execute(
            "UPDATE management_receipts SET outcome=?2 WHERE id=?1",
            params![record.original.snapshot.context.offer, encode(record)?],
        )
        .map_err(database)?;
    Ok(())
}
fn publication_part(part: Part) -> Result<()> {
    if matches!(part, Part::Routes | Part::Receipt) {
        Ok(())
    } else {
        Err(invalid())
    }
}
fn conflict() -> RemoteError {
    RemoteError::new(
        "REMOTE_ID_CONFLICT",
        "This invitation ID already names a different action.",
    )
}
