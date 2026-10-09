//! Client-scoped metadata streams and separate bounded recovery ownership.
use crate::{
    admission, audit, budgets, canonical, crypto,
    error::RemoteError,
    store::{GRANT_COLUMNS, Grant, Store, database, grant_row, uuid_v4},
    wire::SignedMessage,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const ENTRIES: i64 = 1000;
pub const OPERATIONS: i64 = 1000;
pub const DAY: u64 = 86_400_000;
pub const RECOVERY: u64 = 30 * DAY;
pub const FROZEN_BYTES: i64 = 64 * 1024 * 1024;
#[derive(Debug, Clone)]
pub struct Owned {
    pub id: String,
    pub(crate) operation: String,
    pub phase: String,
    pub receipt: Value,
    pub frozen: Option<Vec<u8>>,
    pub references: Vec<String>,
}
struct Stream {
    incarnation: String,
    key: Vec<u8>,
    tip: i64,
    floor: i64,
    observed: i64,
    acknowledged: i64,
    last_ms: i64,
}
fn stream(tx: &Transaction<'_>, client: &str) -> Result<Stream, RemoteError> {
    let mut key = [0; 32];
    getrandom::fill(&mut key).map_err(database)?;
    tx.execute(
        "INSERT OR IGNORE INTO streams(client_id,incarnation,key) VALUES (?1,?2,?3)",
        params![client, uuid_v4()?, key.as_slice()],
    )
    .map_err(database)?;
    let value = tx.query_row("SELECT incarnation,key,tip,floor,observed,acknowledged,last_ms FROM streams WHERE client_id=?1", [client], |r| {
        Ok(Stream{incarnation:r.get(0)?,key:r.get(1)?,tip:r.get(2)?,floor:r.get(3)?,observed:r.get(4)?,acknowledged:r.get(5)?,last_ms:r.get(6)?})
    }).map_err(database)?;
    if value.key.len() != 32
        || !canonical::is_core_id(&value.incarnation)
        || value.floor < 0
        || value.observed < 0
        || value.acknowledged < 0
        || value.last_ms < 0
        || value.floor > value.tip
        || value.observed > value.tip
        || value.acknowledged > value.observed
    {
        return Err(database("invalid stream state"));
    }
    Ok(value)
}
fn cursor_bytes(
    tx: &Transaction<'_>,
    client: &str,
    stream: &Stream,
    header: &[u8],
) -> Result<Vec<u8>, RemoteError> {
    let machine: String = tx
        .query_row(
            "SELECT machine_id FROM machine WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(database)?;
    canonical::framed(&[
        b"tmt-remote-cursor-v1",
        machine.as_bytes(),
        client.as_bytes(),
        stream.incarnation.as_bytes(),
        header,
    ])
    .map_err(database)
}
fn cursor(
    tx: &Transaction<'_>,
    client: &str,
    stream: &Stream,
    position: i64,
    time: u64,
) -> Result<String, RemoteError> {
    let mut bytes = Vec::from(position.to_be_bytes());
    bytes.extend(time.to_be_bytes());
    bytes.extend(crypto::mac(
        &stream.key,
        &cursor_bytes(tx, client, stream, &bytes)?,
    ));
    Ok(canonical::base64url(&bytes))
}
fn position(
    tx: &Transaction<'_>,
    client: &str,
    stream: &Stream,
    token: &str,
    now: u64,
) -> Result<i64, RemoteError> {
    let invalid = || {
        RemoteError::new(
            "REMOTE_CURSOR_EXPIRED",
            "Cursor unavailable; recover owned state.",
        )
    };
    let bytes = canonical::base64url_bytes(token, 48).map_err(|_| invalid())?;
    crypto::verify_mac(
        &stream.key,
        &cursor_bytes(tx, client, stream, &bytes[..16])?,
        &bytes[16..],
    )
    .map_err(|_| invalid())?;
    let position = i64::from_be_bytes(bytes[..8].try_into().expect("fixed length"));
    let time = u64::from_be_bytes(bytes[8..16].try_into().expect("fixed length"));
    if position < stream.floor || position > stream.tip || now.saturating_sub(time) >= DAY {
        return Err(invalid());
    }
    Ok(position)
}
pub(crate) fn prune(tx: &Transaction<'_>, client: &str, now: u64) -> Result<(), RemoteError> {
    let expired: Option<i64> = tx
        .query_row(
            "SELECT MAX(position) FROM entries WHERE client_id=?1 AND at_ms<=?2",
            params![client, now.checked_sub(DAY).map_or(-1, |time| time as i64)],
            |r| r.get(0),
        )
        .map_err(database)?;
    tx.execute(
        "UPDATE streams SET floor=MAX(floor,acknowledged,?2) WHERE client_id=?1",
        params![client, expired.unwrap_or(0)],
    )
    .map_err(database)?;
    tx.execute("DELETE FROM entries WHERE client_id=?1 AND position<=(SELECT floor FROM streams WHERE client_id=?1)",[client]).map_err(database)?;
    // Expired ownership remains a bounded ID fence; expiry never authorizes re-adoption.
    Ok(())
}
/// Drop the oldest entries so one more fits. A notification entry is delivery metadata, not
/// the once-only fence (the ownership records are), so a full stream never refuses work:
/// a cursor behind the new floor takes the existing `REMOTE_CURSOR_EXPIRED` recovery path.
pub(crate) fn make_room_for_entry(tx: &Transaction<'_>, client: &str) -> Result<(), RemoteError> {
    let entries: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM entries WHERE client_id=?1",
            [client],
            |r| r.get(0),
        )
        .map_err(database)?;
    if entries < ENTRIES {
        return Ok(());
    }
    let last_dropped: i64 = tx
        .query_row(
            "SELECT position FROM entries WHERE client_id=?1 ORDER BY position LIMIT 1 OFFSET ?2",
            params![client, entries - ENTRIES],
            |r| r.get(0),
        )
        .map_err(database)?;
    tx.execute(
        "UPDATE streams SET floor=MAX(floor,?2) WHERE client_id=?1",
        params![client, last_dropped],
    )
    .map_err(database)?;
    tx.execute("DELETE FROM entries WHERE client_id=?1 AND position<=(SELECT floor FROM streams WHERE client_id=?1)",[client]).map_err(database)?;
    Ok(())
}
/// Drop ownership records that can no longer matter so one more fits: rows left by reads
/// (only `dispatch.create` is an owned operation), finished records past the recovery
/// horizon and, over the limit, the oldest finished ones. Held, dispatching and uncertain
/// records are live work and never dropped. A send under a dropped ID is a new adoption that
/// asks core for that operation ID before it creates anything, so dropping cannot resend.
fn make_room_for_operation(
    tx: &Transaction<'_>,
    client: &str,
    now: u64,
) -> Result<(), RemoteError> {
    const FINISHED: &str = "phase IN ('accepted','cancelled','refused')";
    tx.execute(
        "DELETE FROM operations WHERE client_id=?1 AND operation<>'dispatch.create'",
        [client],
    )
    .map_err(database)?;
    tx.execute(
        &format!("DELETE FROM operations WHERE client_id=?1 AND {FINISHED} AND updated_ms<=?2"),
        params![client, now.saturating_sub(RECOVERY) as i64],
    )
    .map_err(database)?;
    let operations: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE client_id=?1",
            [client],
            |r| r.get(0),
        )
        .map_err(database)?;
    if operations >= OPERATIONS {
        tx.execute(
            &format!("DELETE FROM operations WHERE id IN (SELECT id FROM operations WHERE client_id=?1 AND {FINISHED} ORDER BY updated_ms,id LIMIT ?2)"),
            params![client, operations - OPERATIONS + 1],
        )
        .map_err(database)?;
    }
    Ok(())
}
fn authority(tx: &Transaction<'_>, grant: &Grant, now: u64) -> Result<(), RemoteError> {
    let current = tx
        .query_row(
            &format!("SELECT {GRANT_COLUMNS} FROM grants WHERE client_id=?1"),
            [&grant.client_id],
            grant_row,
        )
        .optional()
        .map_err(database)?;
    if !current.is_some_and(|current| current.revision == grant.revision && current.live_at(now)) {
        return Err(RemoteError::new("REMOTE_CLOSED", "Device authority ended."));
    }
    Ok(())
}

pub fn intent(message: &SignedMessage) -> Result<[u8; 32], RemoteError> {
    Ok(Sha256::digest(
        canonical::framed(&[message.envelope().operation.as_bytes(), &message.payload])
            .map_err(database)?,
    )
    .into())
}
fn event<'a>(
    grant: &'a Grant,
    message: &'a SignedMessage,
    digest: &'a [u8],
    now: u64,
    decision: &'a str,
    code: &'a str,
) -> audit::AuditMetadata<'a> {
    audit::AuditMetadata {
        time: now,
        client: &grant.client_id,
        request: message.envelope().id,
        operation: message.envelope().operation,
        operation_id: (message.envelope().operation == "dispatch.create")
            .then_some(message.envelope().id),
        resources: &[],
        digest,
        revision: grant.revision,
        decision,
        code,
    }
}
struct Stored {
    client: String,
    digest: Vec<u8>,
    owned: Owned,
    time: u64,
}
fn json_column<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let bytes: String = row.get(index)?;
    serde_json::from_str(&bytes).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}
fn read_owned(tx: &Transaction<'_>, id: &str) -> Result<Option<Stored>, RemoteError> {
    tx.query_row("SELECT client_id,operation,digest,phase,receipt,frozen,references_json,updated_ms FROM operations WHERE id=?1",[id],|row| {
        let time:i64=row.get(7)?;
        Ok(Stored{client:row.get(0)?,digest:row.get(2)?,
            owned:Owned{id:id.into(),operation:row.get(1)?,phase:row.get(3)?,receipt:json_column(row,4)?,frozen:row.get(5)?,references:json_column(row,6)?},
            time:u64::try_from(time).map_err(|_|rusqlite::Error::IntegralValueOutOfRange(7,time))?})
    }).optional().map_err(database)
}
impl Store {
    fn begin(&mut self) -> Result<Transaction<'_>, RemoteError> {
        self.connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)
    }
    pub(crate) fn authorized(
        &mut self,
        grant: &Grant,
        now: u64,
    ) -> Result<Transaction<'_>, RemoteError> {
        self.authorized_at(grant, || Ok(now)).map(|value| value.0)
    }
    pub(crate) fn authorized_at(
        &mut self,
        grant: &Grant,
        clock: impl FnOnce() -> Result<u64, RemoteError>,
    ) -> Result<(Transaction<'_>, u64), RemoteError> {
        let tx = self.begin()?;
        let now = clock()?;
        authority(&tx, grant, now)?;
        Ok((tx, now))
    }
    pub fn charge_call(&mut self, client: &str, now: u64) -> Result<(), RemoteError> {
        let tx = self.begin()?;
        budgets::charge(&tx, client, budgets::Budget::Call, now)?;
        tx.commit().map_err(database)
    }
    pub fn charge_approval(&mut self, recipient: &str, now: u64) -> Result<(), RemoteError> {
        if !canonical::is_core_id(recipient) {
            return Err(database("invalid approval recipient"));
        }
        let tx = self.begin()?;
        budgets::charge(&tx, recipient, budgets::Budget::Approval, now)?;
        tx.commit().map_err(database)
    }
    /// MessagePermit owns the live session fence; this owner rechecks the persisted grant.
    pub(crate) fn adopt(
        &mut self,
        grant: &Grant,
        message: &SignedMessage,
        frozen: Option<&[u8]>,
        resources: &[String],
        metadata: &[u8],
        now: u64,
    ) -> Result<Owned, RemoteError> {
        let input = message.envelope();
        let digest = intent(message)?;
        if input.client_id != grant.client_id
            || input.kind != "request"
            || admission::scope(input.operation).is_none()
            || admission::RemoteOperation::parse(input.operation)
                .is_some_and(|operation| operation.class() != admission::OperationClass::Effect)
            || (input.operation == "dispatch.create") != frozen.is_some()
            || resources.len() > 256
            || resources.iter().any(|id| !canonical::is_core_id(id))
            || metadata.len() > crate::limits::METADATA_BYTES
        {
            return Err(database("invalid adoption"));
        }
        let tx = self.authorized(grant, now)?;
        if tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM management_receipts WHERE id=?1)",
                [input.id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database)?
        {
            return Err(RemoteError::new(
                "REMOTE_INTENT_CONFLICT",
                "Request ID belongs to a management intent.",
            ));
        }
        if let Some(Stored {
            client,
            digest: old,
            owned,
            time,
        }) = read_owned(&tx, input.id)?
        {
            if client != grant.client_id || owned.operation != input.operation || old != digest {
                return Err(RemoteError::new(
                    "REMOTE_INTENT_CONFLICT",
                    "Request ID or intent conflicts.",
                ));
            }
            if now.saturating_sub(time) >= RECOVERY {
                return Err(database("recovery horizon"));
            }
            return Ok(owned);
        }
        let mut stream = stream(&tx, &grant.client_id)?;
        prune(&tx, &grant.client_id, now)?;
        make_room_for_entry(&tx, &grant.client_id)?;
        make_room_for_operation(&tx, &grant.client_id, now)?;
        let (operations,bytes):(i64,i64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(length(frozen)),0) FROM operations WHERE client_id=?1",[&grant.client_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(database)?;
        let total: i64 = tx
            .query_row(
                "SELECT COALESCE(SUM(length(frozen)),0) FROM operations",
                [],
                |r| r.get(0),
            )
            .map_err(database)?;
        if total.saturating_add(frozen.map_or(0, |b| b.len() as i64)) > 4 * FROZEN_BYTES
            || operations >= OPERATIONS
            || bytes.saturating_add(frozen.map_or(0, |b| b.len() as i64)) > FROZEN_BYTES
        {
            return Err(database("journal capacity"));
        }
        let phase = if input.operation == "dispatch.create" {
            budgets::charge(&tx, &grant.client_id, budgets::Budget::Send, now)?;
            if grant.mode == "hold" {
                let held: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM operations WHERE client_id=?1 AND phase='held'",
                        [&grant.client_id],
                        |r| r.get(0),
                    )
                    .map_err(database)?;
                if held >= budgets::HELDS {
                    return Err(RemoteError::new(
                        "REMOTE_RATE_LIMITED",
                        "Outstanding hold capacity exhausted.",
                    ));
                }
                "held"
            } else {
                "dispatching"
            }
        } else {
            "observed"
        };
        let receipt =
            json!({"requestEnvelopeId":input.id,"operation":input.operation,"state":phase});
        let mut audit_event = event(grant, message, &digest, now, "adopted", "");
        audit_event.resources = resources;
        audit::append(&tx, audit_event)?;
        stream.tip = stream
            .tip
            .checked_add(1)
            .ok_or_else(|| database("stream exhausted"))?;
        tx.execute("INSERT INTO operations(id,client_id,operation,digest,frozen,phase,receipt,updated_ms,references_json,session_id,grant_revision) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![input.id,grant.client_id,input.operation,digest.as_slice(),frozen,phase,receipt.to_string(),now as i64,serde_json::to_string(resources).map_err(database)?,input.session_id,grant.revision as i64]).map_err(database)?;
        let at_ms = (now as i64).max(stream.last_ms);
        tx.execute(
            "INSERT INTO entries VALUES (?1,?2,?3,?4)",
            params![grant.client_id, stream.tip, metadata, at_ms],
        )
        .map_err(database)?;
        tx.execute(
            "UPDATE streams SET tip=?2,last_ms=?3 WHERE client_id=?1",
            params![grant.client_id, stream.tip, at_ms],
        )
        .map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(Owned {
            id: input.id.into(),
            operation: input.operation.into(),
            phase: phase.into(),
            receipt,
            frozen: frozen.map(Vec::from),
            references: resources.to_vec(),
        })
    }
    pub fn owned(&mut self, grant: &Grant, id: &str, now: u64) -> Result<Owned, RemoteError> {
        let tx = self.authorized(grant, now)?;
        let Some(Stored {
            client,
            owned,
            time,
            ..
        }) = read_owned(&tx, id)?
        else {
            return Err(database("owned state absent"));
        };
        if client != grant.client_id || now.saturating_sub(time) >= RECOVERY {
            return Err(database("owned state unavailable"));
        }
        Ok(owned)
    }
    pub fn page(
        &mut self,
        grant: &Grant,
        token: Option<&str>,
        limit: usize,
        now: u64,
    ) -> Result<Value, RemoteError> {
        if !(1..=50).contains(&limit) {
            return Err(RemoteError::new(
                "REMOTE_INPUT_INVALID",
                "Invalid subscribe limit.",
            ));
        }
        let tx = self.authorized(grant, now)?;
        prune(&tx, &grant.client_id, now)?;
        let stream = stream(&tx, &grant.client_id)?;
        let start = token.map_or(Ok(stream.floor), |token| {
            position(&tx, &grant.client_id, &stream, token, now)
        })?;
        let rows = {
            let mut query=tx.prepare("SELECT position,envelope FROM entries WHERE client_id=?1 AND position>?2 ORDER BY position LIMIT ?3").map_err(database)?;
            query
                .query_map(params![grant.client_id, start, limit as i64], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
                })
                .map_err(database)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database)?
        };
        let mut entries = Vec::new();
        let mut last = start;
        let mut next =
            token
                .map(str::to_owned)
                .unwrap_or(cursor(&tx, &grant.client_id, &stream, start, now)?);
        for (position, envelope) in rows {
            last = position;
            next = cursor(&tx, &grant.client_id, &stream, position, now)?;
            entries.push(json!({"cursor":next,"envelope":serde_json::from_slice::<Value>(&envelope).map_err(database)?}));
        }
        tx.execute(
            "UPDATE streams SET observed=MAX(observed,?2) WHERE client_id=?1",
            params![grant.client_id, last],
        )
        .map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(
            json!({"entries":entries,"nextCursor":next,"reason":if last>start {"changed"}else{"timeout"},"hasMore":last<stream.tip}),
        )
    }
    pub fn ack(&mut self, grant: &Grant, token: &str, now: u64) -> Result<Value, RemoteError> {
        let tx = self.authorized(grant, now)?;
        let stream = stream(&tx, &grant.client_id)?;
        let position = position(&tx, &grant.client_id, &stream, token, now)?;
        if position > stream.observed {
            return Err(RemoteError::new(
                "REMOTE_INPUT_INVALID",
                "Cannot acknowledge unseen entries.",
            ));
        }
        tx.execute(
            "UPDATE streams SET acknowledged=MAX(acknowledged,?2) WHERE client_id=?1",
            params![grant.client_id, position],
        )
        .map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(json!({"cursor":token}))
    }
    pub(crate) fn record_refusal(
        &mut self,
        grant: &Grant,
        message: &SignedMessage,
        code: &str,
        now: u64,
    ) -> Result<(), RemoteError> {
        let digest = intent(message)?;
        let tx = self.authorized(grant, now)?;
        audit::append(&tx, event(grant, message, &digest, now, "refused", code))?;
        tx.commit().map_err(database)
    }
}

#[cfg(test)]
mod tests;
