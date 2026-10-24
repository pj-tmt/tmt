//! Transport checkpoints and frozen bytes share the authoritative Store connection/lease.
//! This table never decides whether an operation may execute; journal adoption owns that.
use super::{Checkpoint, Replay};
use crate::{
    canonical,
    error::RemoteError,
    journal, limits,
    store::{Store, database},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Not a UUID: neither a peer nor the enrollment scan can own this checkpoint key.
const QUERY_CHECKPOINT: &str = "inbox-query";
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Continuation {
    pub(crate) from_ms: u64,
    pub(crate) read_ms: u64,
    pub(crate) cursor: super::document::Cursor,
}
impl Continuation {
    fn validate(&self) -> Result<(), RemoteError> {
        Checkpoint::new(self.from_ms)?;
        Checkpoint::new(self.read_ms)?;
        let indexed = super::document::timestamp(&self.cursor.created_at).map_err(database)?;
        canonical::uuid(&self.cursor.id).map_err(database)?;
        // Later pages can see documents newer than the earliest snapshot. Keep
        // that conservative snapshot for overlap without rejecting those arrivals.
        if self.from_ms > indexed || self.from_ms > self.read_ms {
            return Err(database("invalid cloud query continuation"));
        }
        Ok(())
    }
}
pub(crate) struct QueryCheckpoint {
    pub(crate) completed: Checkpoint,
    pub(crate) continuation: Option<Continuation>,
}

struct InboxRow {
    digest: Vec<u8>,
    created_ms: i64,
    response: Option<Vec<u8>>,
    refusal: Option<String>,
}
fn invalid() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_CONFLICT",
        "The operation ID already belongs to different cloud bytes.",
    )
}
fn validate(peer: &str, id: &str, bytes: &[u8], created_ms: u64) -> Result<(), RemoteError> {
    canonical::uuid(peer).map_err(|_| invalid())?;
    canonical::uuid(id).map_err(|_| invalid())?;
    Checkpoint::new(created_ms)?;
    if bytes.is_empty() || bytes.len() > limits::CLOUD_CARRIER_BYTES {
        return Err(invalid());
    }
    Ok(())
}
impl Store {
    pub(crate) fn cloud_begin(
        &mut self,
        peer: &str,
        id: &str,
        carrier: &[u8],
        created_ms: u64,
    ) -> Result<Replay, RemoteError> {
        validate(peer, id, carrier, created_ms)?;
        let digest = Sha256::digest(carrier);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let old:Option<InboxRow>=tx.query_row(
            "SELECT digest,created_ms,response,refusal FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",
            [peer,id],|row|Ok(InboxRow{digest:row.get(0)?,created_ms:row.get(1)?,response:row.get(2)?,refusal:row.get(3)?})).optional().map_err(database)?;
        if let Some(old) = old {
            if old.digest != digest.as_slice() || old.created_ms as u64 != created_ms {
                return Err(invalid());
            }
            return Ok(if old.refusal.is_some() {
                Replay::Refused(old.response)
            } else {
                old.response.map_or(Replay::Pending, Replay::Frozen)
            });
        }
        let (count,bytes):(i64,i64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(length(carrier)+COALESCE(length(response),0)),0) FROM cloud_transport WHERE peer_id=?1 AND kind<>'checkpoint'",[peer],|r|Ok((r.get(0)?,r.get(1)?))).map_err(database)?;
        if count >= journal::OPERATIONS
            || bytes.saturating_add(carrier.len() as i64) > journal::FROZEN_BYTES
        {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_CAPACITY",
                "Cloud recovery storage is full.",
            ));
        }
        tx.execute("INSERT INTO cloud_transport(peer_id,kind,id,digest,carrier,created_ms) VALUES (?1,'inbox',?2,?3,?4,?5)",params![peer,id,digest.as_slice(),carrier,created_ms as i64]).map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(Replay::Fresh)
    }
    /// Durable per-document terminal refusal; no operation is adopted and no effect is retried.
    /// Unknown peers have no authority revision: zero is refusal-only audit metadata.
    pub(crate) fn cloud_refuse(
        &mut self,
        peer: &str,
        id: &str,
        code: &str,
        response: Option<&[u8]>,
        now: u64,
    ) -> Result<(), RemoteError> {
        canonical::uuid(peer).map_err(|_| invalid())?;
        canonical::uuid(id).map_err(|_| invalid())?;
        if code.is_empty()
            || code.len() > 64
            || !code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        {
            return Err(invalid());
        }
        if let Some(bytes) = response {
            validate(peer, id, bytes, now)?;
        }
        let revision = self.grant(peer)?.map_or(0, |grant| grant.revision);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let (digest,old):(Vec<u8>,Option<String>)=tx.query_row("SELECT digest,refusal FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",[peer,id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(database)?;
        if old.is_some() {
            return Ok(());
        }
        if let Some(bytes) = response {
            let used:i64=tx.query_row("SELECT COALESCE(SUM(length(carrier)+COALESCE(length(response),0)),0) FROM cloud_transport WHERE peer_id=?1 AND kind<>'checkpoint'",[peer],|r|r.get(0)).map_err(database)?;
            if used.saturating_add(bytes.len() as i64) > journal::FROZEN_BYTES {
                return Err(RemoteError::new(
                    "REMOTE_CLOUD_CAPACITY",
                    "Cloud recovery storage is full.",
                ));
            }
        }
        crate::audit::append(
            &tx,
            crate::audit::AuditMetadata {
                time: now,
                client: peer,
                request: id,
                operation: "unsupported",
                operation_id: None,
                resources: &[],
                digest: &digest,
                revision,
                decision: "refused",
                code,
            },
        )?;
        tx.execute("UPDATE cloud_transport SET refusal=?3,response=?4 WHERE peer_id=?1 AND kind='inbox' AND id=?2",params![peer,id,code,response]).map_err(database)?;
        tx.commit().map_err(database)
    }
    pub(crate) fn cloud_freeze(
        &mut self,
        peer: &str,
        id: &str,
        response: &[u8],
    ) -> Result<(), RemoteError> {
        validate(peer, id, response, 0)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let old: Option<Option<Vec<u8>>> = tx
            .query_row(
                "SELECT response FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",
                [peer, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(database)?;
        match old {
            Some(Some(old)) if old == response => return Ok(()),
            Some(None) => {}
            _ => return Err(invalid()),
        }
        let used:i64=tx.query_row("SELECT COALESCE(SUM(length(carrier)+COALESCE(length(response),0)),0) FROM cloud_transport WHERE peer_id=?1 AND kind<>'checkpoint'",[peer],|r|r.get(0)).map_err(database)?;
        if used.saturating_add(response.len() as i64) > journal::FROZEN_BYTES {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_CAPACITY",
                "Cloud recovery storage is full.",
            ));
        }
        tx.execute(
            "UPDATE cloud_transport SET response=?3 WHERE peer_id=?1 AND kind='inbox' AND id=?2",
            params![peer, id, response],
        )
        .map_err(database)?;
        tx.commit().map_err(database)
    }
    /// Commit before HTTP. False means observation only, even if a previous call lost its reply.
    pub(crate) fn cloud_claim_publication(
        &mut self,
        peer: &str,
        id: &str,
    ) -> Result<bool, RemoteError> {
        let changed=self.connection.execute("UPDATE cloud_transport SET attempted=1 WHERE peer_id=?1 AND kind='inbox' AND id=?2 AND response IS NOT NULL AND attempted=0",[peer,id]).map_err(database)?;
        Ok(changed == 1)
    }
    /// A claimed publication may remain unconfirmed; it never permits another POST.
    pub(crate) fn cloud_publication_attempted(
        &self,
        peer: &str,
        id: &str,
    ) -> Result<bool, RemoteError> {
        self.connection.query_row(
            "SELECT attempted FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2 AND response IS NOT NULL",
            [peer, id], |row| row.get(0),
        ).map_err(database)
    }
    pub(crate) fn cloud_checkpoint(&self, peer: &str) -> Result<Checkpoint, RemoteError> {
        canonical::uuid(peer).map_err(|_| invalid())?;
        let time:Option<i64>=self.connection.query_row("SELECT created_ms FROM cloud_transport WHERE peer_id=?1 AND kind='checkpoint' AND id=''",[peer],|r|r.get(0)).optional().map_err(database)?;
        Checkpoint::new(time.unwrap_or(0) as u64)
    }
}

impl Store {
    pub(crate) fn cloud_query_checkpoint(&self) -> Result<Option<QueryCheckpoint>, RemoteError> {
        let row: Option<(i64, Vec<u8>)> = self.connection.query_row(
            "SELECT created_ms,carrier FROM cloud_transport WHERE peer_id=?1 AND kind='checkpoint' AND id=''",
            [QUERY_CHECKPOINT], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(database)?;
        row.map(|(completed, bytes)| {
            // One fixed-size cursor, never an unbounded provider value or carrier.
            if bytes.len() > limits::CLOUD_QUERY_CHECKPOINT_BYTES {
                return Err(database("oversized cloud query continuation"));
            }
            let continuation: Option<Continuation> = if bytes.is_empty() {
                None
            } else {
                let value: Continuation = serde_json::from_slice(&bytes).map_err(database)?;
                value.validate()?;
                Some(value)
            };
            Ok(QueryCheckpoint {
                completed: Checkpoint::new(completed as u64)?,
                continuation,
            })
        })
        .transpose()
    }
    pub(crate) fn cloud_progress(&mut self, progress: &Continuation) -> Result<(), RemoteError> {
        progress.validate()?;
        let bytes = serde_json::to_vec(progress).map_err(database)?;
        self.connection.execute(
            "INSERT INTO cloud_transport(peer_id,kind,id,carrier,created_ms) VALUES (?1,'checkpoint','',?2,0) ON CONFLICT(peer_id,kind,id) DO UPDATE SET carrier=excluded.carrier",
            params![QUERY_CHECKPOINT, bytes],
        ).map_err(database)?;
        Ok(())
    }
    /// A sizing hint only: every provider result still traverses the overlap query.
    /// Cached rows never authorize skipping a timestamp or operation ID.
    pub(crate) fn cloud_inbox_page_limit(&self, from_ms: u64) -> Result<usize, RemoteError> {
        Checkpoint::new(from_ms)?;
        let known: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM cloud_transport WHERE kind='inbox' AND created_ms>=?1 LIMIT ?2)",
            params![from_ms as i64, limits::CLOUD_INBOX_PAGE_DOCUMENTS as i64],
            |row| row.get(0),
        ).map_err(database)?;
        let known = usize::try_from(known).map_err(database)?;
        Ok(if known < 2 {
            1
        } else {
            (known + 1).min(limits::CLOUD_INBOX_PAGE_DOCUMENTS)
        })
    }
    pub(crate) fn cloud_finish_query(&mut self, read_ms: u64) -> Result<(), RemoteError> {
        self.cloud_complete(&[QUERY_CHECKPOINT.into()], read_ms)
    }
    pub(crate) fn cloud_peers(&self) -> Result<Vec<String>, RemoteError> {
        Ok(self
            .grants()?
            .into_iter()
            .filter(|g| g.kind == "cli" && g.origin == "cli")
            .map(|g| g.client_id)
            .collect())
    }
    /// Complete only the supplied stream keys. Enrollment uses the machine UUID;
    /// the inbox query uses its reserved non-UUID key, never a peer's checkpoint.
    pub(crate) fn cloud_complete(
        &mut self,
        peers: &[String],
        read_ms: u64,
    ) -> Result<(), RemoteError> {
        Checkpoint::new(read_ms)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        for peer in peers {
            tx.execute("INSERT INTO cloud_transport(peer_id,kind,id,carrier,created_ms) VALUES (?1,'checkpoint','',x'',?2) ON CONFLICT(peer_id,kind,id) DO UPDATE SET carrier=x'',created_ms=MAX(created_ms,excluded.created_ms)",params![peer,read_ms as i64]).map_err(database)?;
        }
        tx.commit().map_err(database)
    }
    pub(crate) fn cloud_forget(
        &mut self,
        peer: &str,
        id: &str,
        now: u64,
    ) -> Result<(), RemoteError> {
        self.connection.execute("DELETE FROM cloud_transport WHERE peer_id=?1 AND id=?2 AND kind<>'checkpoint' AND created_ms<=?3",params![peer,id,now.saturating_sub(journal::RECOVERY) as i64]).map_err(database)?;
        Ok(())
    }
}

pub(crate) struct Outgoing {
    pub attempted: bool,
    pub carrier: Vec<u8>,
    pub created_ms: u64,
}
impl Store {
    pub(crate) fn cloud_outgoing(
        &self,
        peer: &str,
        id: &str,
        intent: &[u8],
        created_at_ms: u64,
    ) -> Result<Option<Outgoing>, RemoteError> {
        let old:Option<(Vec<u8>,Vec<u8>,bool,i64)>=self.connection.query_row("SELECT digest,carrier,attempted,created_ms FROM cloud_transport WHERE peer_id=?1 AND kind='outbox' AND id=?2",[peer,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(database)?;
        old.map(|(digest, carrier, attempted, created_ms)| {
            if digest != Sha256::digest(intent).as_slice() || created_ms as u64 != created_at_ms {
                return Err(invalid());
            }
            Ok(Outgoing {
                attempted,
                carrier,
                created_ms: created_ms as u64,
            })
        })
        .transpose()
    }
    pub(crate) fn cloud_freeze_outgoing(
        &mut self,
        peer: &str,
        id: &str,
        intent: &[u8],
        carrier: &[u8],
        now: u64,
    ) -> Result<Outgoing, RemoteError> {
        validate(peer, id, carrier, now)?;
        if let Some(old) = self.cloud_outgoing(peer, id, intent, now)? {
            return Ok(old);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let (count,bytes):(i64,i64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(length(carrier)+COALESCE(length(response),0)),0) FROM cloud_transport WHERE peer_id=?1 AND kind<>'checkpoint'",[peer],|r|Ok((r.get(0)?,r.get(1)?))).map_err(database)?;
        if count >= journal::OPERATIONS
            || bytes.saturating_add(carrier.len() as i64) > journal::FROZEN_BYTES
        {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_CAPACITY",
                "Cloud recovery storage is full.",
            ));
        }
        tx.execute("INSERT INTO cloud_transport(peer_id,kind,id,digest,carrier,created_ms) VALUES (?1,'outbox',?2,?3,?4,?5)",params![peer,id,Sha256::digest(intent).as_slice(),carrier,now as i64]).map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(Outgoing {
            attempted: false,
            carrier: carrier.to_vec(),
            created_ms: now,
        })
    }
    pub(crate) fn cloud_claim_outgoing(
        &mut self,
        peer: &str,
        id: &str,
    ) -> Result<bool, RemoteError> {
        self.connection.execute("UPDATE cloud_transport SET attempted=1 WHERE peer_id=?1 AND kind='outbox' AND id=?2 AND attempted=0",[peer,id]).map(|n|n==1).map_err(database)
    }
    /// The existing serving maintenance pass drops only elapsed sender recovery windows.
    pub(crate) fn cloud_prune_outgoing(&mut self, now: u64) -> Result<(), RemoteError> {
        let keep = journal::RECOVERY + crate::session::CLOCK_SKEW.as_millis() as u64;
        if let Some(before) = now.checked_sub(keep) {
            self.connection
                .execute(
                    "DELETE FROM cloud_transport WHERE kind='outbox' AND created_ms<?1",
                    [before as i64],
                )
                .map_err(database)?;
        }
        Ok(())
    }
    pub(crate) fn cloud_expired(&self, now: u64) -> Result<Vec<(String, String)>, RemoteError> {
        let mut statement=self.connection.prepare("SELECT peer_id,id FROM cloud_transport WHERE kind='inbox' AND created_ms<=?1 ORDER BY created_ms LIMIT ?2").map_err(database)?;
        let rows = statement
            .query_map(
                params![
                    now.saturating_sub(journal::RECOVERY) as i64,
                    journal::OPERATIONS
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(database)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(database)
    }
}

/// Private association for Core reply capabilities; never part of a carrier or audit.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReplyOrigin {
    pub machine: String,
    pub remote: String,
    pub sender_id: String,
    pub reply: Option<crate::cloud_command::Reply>,
}

impl Store {
    pub(crate) fn cloud_bind_outgoing(
        &mut self,
        peer: &str,
        operation_id: &str,
        binding: &ReplyOrigin,
    ) -> Result<(), RemoteError> {
        canonical::uuid(&binding.machine).map_err(|_| invalid())?;
        canonical::uuid(&binding.sender_id).map_err(|_| invalid())?;
        if !crate::remote_catalog::name_ok(&binding.remote) {
            return Err(invalid());
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT binding FROM cloud_transport WHERE peer_id=?1 AND kind='outbox' AND id=?2",
                [peer, operation_id],
                |r| r.get(0),
            )
            .map_err(database)?;
        if let Some(old) = old {
            let old: ReplyOrigin = serde_json::from_str(&old).map_err(database)?;
            if old != *binding {
                return Err(RemoteError::new(
                    "REMOTE_INTENT_CONFLICT",
                    "The request belongs to another remote route.",
                ));
            }
        } else {
            tx.execute("UPDATE cloud_transport SET binding=?3 WHERE peer_id=?1 AND kind='outbox' AND id=?2",
                params![peer, operation_id, serde_json::to_string(binding).map_err(database)?]).map_err(database)?;
        }
        tx.commit().map_err(database)
    }
}
