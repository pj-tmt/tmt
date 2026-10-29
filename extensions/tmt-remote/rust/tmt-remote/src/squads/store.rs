//! Verified cache and publisher intent share the existing authoritative Remote Store.
use super::{
    Record, Summary, SummaryReader,
    publisher::{Prepared, Schedule},
};
use crate::{
    error::RemoteError,
    pairing::now_ms,
    store::{Grant, Store, database},
};
use rusqlite::{OptionalExtension, params};

pub(crate) fn invalid() -> RemoteError {
    RemoteError::new("REMOTE_SUMMARY_INVALID", "Invalid machine squad summary.")
}
fn integer(value: u64) -> Result<i64, RemoteError> {
    if value > crate::cloud_hpke::INTEGER_MAX {
        return Err(invalid());
    }
    Ok(value as i64)
}
fn natural(value: i64) -> Result<u64, RemoteError> {
    u64::try_from(value)
        .ok()
        .filter(|n| *n <= crate::cloud_hpke::INTEGER_MAX)
        .ok_or_else(invalid)
}
/// Release revoked snapshot bodies, retaining only their sequence watermark.
/// One fixed metadata row per existing grant prevents stale replay after re-pair.
fn prune(c: &rusqlite::Connection, now: u64) -> Result<(), RemoteError> {
    let ended = "SELECT g.client_id FROM grants g LEFT JOIN member_machines mm ON mm.client_id=g.client_id AND mm.remote_id=(SELECT machine_id FROM machine WHERE singleton=1) LEFT JOIN members m ON m.remote_id=mm.remote_id AND m.member_id=mm.member_id WHERE g.disabled=1 OR g.expires_at_ms<=?1 OR m.revoked_at_ms IS NOT NULL";
    c.execute(
        &format!("UPDATE squad_cache SET summary=NULL WHERE summary IS NOT NULL AND client_id IN ({ended})"),
        [integer(now)?],
    )
    .map_err(database)?;
    c.execute(&format!("UPDATE squad_publications SET desired=NULL,fingerprint=NULL,changed_ms=NULL,sent_hash=NULL,sent_ms=NULL,active_id=NULL,active_payload=NULL,active_revision=NULL WHERE desired IS NOT NULL AND client_id IN ({ended})"), [integer(now)?]).map_err(database)?;
    Ok(())
}
/// Sequence is persisted independently of clock time. Unknown fields are ignored;
/// normalized Summary bytes, never arbitrary sender JSON, enter the cache.
pub(crate) fn frame(bytes: &[u8]) -> Result<(u64, Summary), RemoteError> {
    if bytes.len() > crate::limits::SQUAD_SUMMARY_FRAME_BYTES {
        return Err(invalid());
    }
    let value = crate::wire::strict_json(bytes).ok_or_else(invalid)?;
    if value["version"] != 1 {
        return Err(crate::cloud_command::upgrade());
    }
    let sequence = value["sequence"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= crate::cloud_hpke::INTEGER_MAX)
        .ok_or_else(invalid)?;
    let summary = Summary::read(&serde_json::to_vec(&value["summary"]).map_err(database)?)?;
    Ok((sequence, summary))
}
fn require_read(c: &rusqlite::Connection, grant: &Grant) -> Result<Option<String>, RemoteError> {
    if !grant.permits_scope("agents.read") {
        return Err(invalid());
    }
    let member = crate::members::member_for_client_in(c, &grant.client_id)?;
    if member
        .as_ref()
        .is_some_and(|member| member.revoked_at_ms.is_some())
    {
        return Err(invalid());
    }
    Ok(member.map(|member| member.member_id))
}
struct ScheduleRecord {
    hash: Option<Vec<u8>>,
    sent: Option<i64>,
    changed: Option<i64>,
    retry: i64,
}
/// Join the existing carrier retention transaction: expiry may retire a snapshot,
/// never erase an attempted-ID fence while that ID remains active publisher intent.
pub(crate) fn expire_outgoing(
    c: &rusqlite::Connection,
    before: u64,
    now: u64,
) -> Result<(), RemoteError> {
    c.execute("UPDATE squad_publications SET active_id=NULL,active_payload=NULL,active_revision=NULL,sent_hash=NULL,changed_ms=COALESCE(changed_ms,?2),retry_ms=0 WHERE active_id IN (SELECT id FROM cloud_transport WHERE kind='outbox' AND created_ms<?1)",params![integer(before)?,integer(now)?]).map_err(database)?;
    Ok(())
}
pub(crate) struct Pending {
    pub id: String,
    pub payload: Vec<u8>,
    pub revision: u64,
}
impl Store {
    pub(crate) fn summary_received(
        &mut self,
        grant: &Grant,
        carrier_id: &str,
        payload: &[u8],
        now: u64,
    ) -> Result<(), RemoteError> {
        crate::canonical::uuid(carrier_id).map_err(|_| invalid())?;
        let (sequence, summary) = frame(payload)?;
        let bytes = summary.bytes()?;
        let (tx, _) = self.authorized_at(grant, || Ok(now))?;
        prune(&tx, now)?;
        let member = require_read(&tx, grant)?;
        // Only a validated native summary under its current authority can enter
        // prompt cleanup. Keep that intent beside the carrier until delete succeeds.
        let marked=tx.execute("UPDATE cloud_transport SET summary_metadata=1 WHERE peer_id=?1 AND kind='inbox' AND id=?2 AND response IS NULL AND refusal IS NULL", params![grant.client_id,carrier_id]).map_err(database)?;
        if marked != 1 {
            return Err(invalid());
        }
        let old: Option<(i64, Vec<u8>, bool)> = tx
            .query_row(
                "SELECT sequence,public_key,summary IS NOT NULL FROM squad_cache WHERE client_id=?1",
                [&grant.client_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(database)?;
        if old
            .as_ref()
            .is_some_and(|(old, key, _)| key == &grant.public_key && *old >= sequence as i64)
        {
            return tx.commit().map_err(database);
        }
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM squad_cache WHERE summary IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(database)?;
        if old.as_ref().is_none_or(|(_, _, has_body)| !has_body)
            && count >= crate::limits::SQUAD_SUMMARY_MACHINES as i64
        {
            return Err(invalid());
        }
        tx.execute("INSERT INTO squad_cache VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(client_id) DO UPDATE SET public_key=excluded.public_key,grant_revision=excluded.grant_revision,member_id=excluded.member_id,sequence=excluded.sequence,summary=excluded.summary,received_ms=excluded.received_ms",
            params![grant.client_id,grant.public_key.as_slice(),integer(grant.revision)?,member,integer(sequence)?,bytes,integer(now)?]).map_err(database)?;
        tx.commit().map_err(database)
    }
    pub(crate) fn summary_observed(
        &mut self,
        grant: &Grant,
        next: &Prepared,
        now: u64,
    ) -> Result<(), RemoteError> {
        Summary::read(&next.bytes)?;
        let (tx, _) = self.authorized_at(grant, || Ok(now))?;
        require_read(&tx, grant)?;
        prune(&tx, now)?;
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM squad_publications WHERE desired IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(database)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM squad_publications WHERE client_id=?1 AND desired IS NOT NULL)",
                [&grant.client_id],
                |r| r.get(0),
            )
            .map_err(database)?;
        if !exists && count >= crate::limits::SQUAD_SUMMARY_MACHINES as i64 {
            return Err(invalid());
        }
        let unchanged: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM squad_publications WHERE client_id=?1 AND grant_revision=?2 AND fingerprint=?3 AND desired IS NOT NULL)",params![grant.client_id,integer(grant.revision)?,next.fingerprint.as_slice()],|r| r.get(0)).map_err(database)?;
        if unchanged {
            return tx.commit().map_err(database);
        }
        let obsolete: Option<String> = tx.query_row("SELECT active_id FROM squad_publications WHERE client_id=?1 AND active_id IS NOT NULL AND (grant_revision<>?2 OR fingerprint IS NOT ?3) AND NOT EXISTS(SELECT 1 FROM cloud_transport t WHERE t.kind='outbox' AND t.id=squad_publications.active_id AND t.attempted=1)",params![grant.client_id,integer(grant.revision)?,next.fingerprint.as_slice()],|r|r.get(0)).optional().map_err(database)?;
        if let Some(id) = obsolete {
            tx.execute(
                "DELETE FROM cloud_transport WHERE kind='outbox' AND id=?1 AND attempted=0",
                [&id],
            )
            .map_err(database)?;
            tx.execute("UPDATE squad_publications SET active_id=NULL,active_payload=NULL,active_revision=NULL WHERE client_id=?1", [&grant.client_id]).map_err(database)?;
        }
        // Retain the active attempt through authority changes. It can only be
        // observed, never republished; the next frame uses the new projection.
        tx.execute("INSERT INTO squad_publications(client_id,grant_revision,desired,fingerprint,changed_ms) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(client_id) DO UPDATE SET desired=CASE WHEN fingerprint<>excluded.fingerprint OR grant_revision<>excluded.grant_revision THEN excluded.desired ELSE desired END, changed_ms=CASE WHEN sent_hash=excluded.fingerprint AND grant_revision=excluded.grant_revision THEN NULL ELSE COALESCE(changed_ms,excluded.changed_ms) END, sent_hash=CASE WHEN grant_revision<>excluded.grant_revision THEN NULL ELSE sent_hash END,sent_ms=CASE WHEN grant_revision<>excluded.grant_revision THEN NULL ELSE sent_ms END,fingerprint=excluded.fingerprint, grant_revision=excluded.grant_revision",
            params![grant.client_id,integer(grant.revision)?,next.bytes,next.fingerprint.as_slice(),integer(now)?]).map_err(database)?;

        tx.commit().map_err(database)
    }
    /// Restore persisted wall-clock ages onto a monotonic timeline. Future wall
    /// timestamps conservatively restart their interval instead of spinning.
    pub(crate) fn summary_schedule(
        &self,
        peer: &str,
        wall: u64,
        mono: u64,
    ) -> Result<Option<Schedule>, RemoteError> {
        let row: Option<ScheduleRecord> = self.connection.query_row("SELECT sent_hash,sent_ms,changed_ms,retry_ms FROM squad_publications WHERE client_id=?1", [peer], |r| Ok(ScheduleRecord { hash:r.get(0)?,sent:r.get(1)?,changed:r.get(2)?,retry:r.get(3)? })).optional().map_err(database)?;
        row.map(
            |ScheduleRecord {
                 hash,
                 sent,
                 changed,
                 retry,
             }| {
                let age = |time: i64| -> Result<u64, RemoteError> {
                    Ok(wall.saturating_sub(natural(time)?))
                };
                let published = match (hash, sent) {
                    (Some(hash), Some(sent)) => {
                        Some((hash.try_into().map_err(|_| invalid())?, age(sent)?))
                    }
                    _ => None,
                };
                Ok(Schedule::restore(
                    published,
                    changed.map(age).transpose()?,
                    natural(retry)?.saturating_sub(wall),
                    mono,
                ))
            },
        )
        .transpose()
    }
    pub(crate) fn summary_pending(
        &mut self,
        grant: &Grant,
        latest: &Prepared,
        now: u64,
    ) -> Result<Pending, RemoteError> {
        let (tx, _) = self.authorized_at(grant, || Ok(now))?;
        require_read(&tx, grant)?;
        let (revision, seq, id, payload, active_revision): (i64,i64,Option<String>,Option<Vec<u8>>,Option<i64>) = tx.query_row("SELECT grant_revision,sequence,active_id,active_payload,active_revision FROM squad_publications WHERE client_id=?1", [&grant.client_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(database)?;
        if let (Some(id), Some(payload)) = (id, payload) {
            return Ok(Pending {
                id,
                payload,
                revision: natural(active_revision.ok_or_else(invalid)?)?,
            });
        }
        let revision = natural(revision)?;
        let seq = natural(seq)?;
        if revision != grant.revision || seq >= crate::cloud_hpke::INTEGER_MAX {
            return Err(invalid());
        }
        let id = crate::store::uuid_v4()?;
        let payload = serde_json::to_vec(&serde_json::json!({"version":1,"sequence":seq+1,"summary":Summary::read(&latest.bytes)?})).map_err(database)?;
        tx.execute("UPDATE squad_publications SET active_id=?2,active_payload=?3,sequence=?4,active_revision=grant_revision WHERE client_id=?1",params![grant.client_id,id,payload,integer(seq+1)?]).map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(Pending {
            id,
            payload,
            revision,
        })
    }
    pub(crate) fn summary_published(
        &mut self,
        grant: &Grant,
        id: &str,
        payload: &[u8],
        now: u64,
    ) -> Result<(), RemoteError> {
        let (_, summary) = frame(payload)?;
        let hash = Prepared::published_fingerprint(&summary)?;
        let (tx, _) = self.authorized_at(grant, || Ok(now))?;
        require_read(&tx, grant)?;
        let changed = tx.execute("UPDATE squad_publications SET sent_hash=?3,sent_ms=?4,changed_ms=CASE WHEN fingerprint=?3 THEN NULL ELSE COALESCE(changed_ms,?4) END,active_id=NULL,active_payload=NULL,active_revision=NULL,retry_ms=0 WHERE client_id=?1 AND active_id=?2",params![grant.client_id,id,hash.as_slice(),integer(now)?]).map_err(database)?;
        if changed != 1 {
            return Err(invalid());
        }
        Store::cloud_compact_summary_outgoing_in(&tx, id)?;
        tx.commit().map_err(database)
    }
    /// An original-ID GET found no carrier. Its outcome stays unconfirmed; keep
    /// the attempted row immutable, but allow a later fresh observation/sequence.
    pub(crate) fn summary_unobserved(
        &mut self,
        grant: &Grant,
        id: &str,
        now: u64,
    ) -> Result<(), RemoteError> {
        let (tx, _) = self.authorized_at(grant, || Ok(now))?;
        require_read(&tx, grant)?;
        let changed = tx.execute("UPDATE squad_publications SET active_id=NULL,active_payload=NULL,active_revision=NULL,sent_hash=NULL,changed_ms=COALESCE(changed_ms,?3),retry_ms=?4 WHERE client_id=?1 AND active_id=?2 AND EXISTS(SELECT 1 FROM cloud_transport WHERE kind='outbox' AND id=?2 AND attempted=1)",params![grant.client_id,id,integer(now)?,integer(now.saturating_add(crate::limits::SQUAD_SUMMARY_HEARTBEAT_MS))?]).map_err(database)?;
        if changed != 1 {
            return Err(invalid());
        }
        Store::cloud_compact_summary_outgoing_in(&tx, id)?;
        tx.commit().map_err(database)
    }
    pub(crate) fn summary_defer(&self, peer: &str, until: u64) -> Result<(), RemoteError> {
        self.connection
            .execute(
                "UPDATE squad_publications SET retry_ms=MAX(retry_ms,?2) WHERE client_id=?1",
                params![peer, integer(until)?],
            )
            .map_err(database)?;
        Ok(())
    }
    pub(crate) fn summary_claim(
        &mut self,
        grant: &Grant,
        peer: &str,
        pending: &Pending,
        carrier: &[u8],
    ) -> Result<bool, RemoteError> {
        let (tx, _) = self.authorized_at(grant, now_ms)?;
        require_read(&tx, grant)?;
        if pending.revision != grant.revision {
            return Err(invalid());
        }
        let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM squad_publications WHERE client_id=?1 AND grant_revision=?2 AND active_id=?3 AND active_payload=?4)", params![grant.client_id,integer(grant.revision)?,pending.id,pending.payload], |r| r.get(0)).map_err(database)?;
        if !current {
            return Err(invalid());
        }
        let claimed = tx.execute("UPDATE cloud_transport SET attempted=1,summary_metadata=1 WHERE peer_id=?1 AND kind='outbox' AND id=?2 AND carrier=?3 AND attempted=0", params![peer,pending.id,carrier]).map_err(database)? == 1;
        if claimed {
            // A claimed create can commit despite a lost acknowledgement. Until
            // its exact-ID observation succeeds the previous content is uncertain.
            tx.execute(
                "UPDATE squad_publications SET sent_hash=NULL WHERE client_id=?1",
                [&grant.client_id],
            )
            .map_err(database)?;
        }
        tx.commit().map_err(database)?;
        Ok(claimed)
    }
}
impl SummaryReader for Store {
    fn read(&self) -> Result<Vec<Record>, RemoteError> {
        let now = now_ms()?;
        let mut result = Vec::new();
        for connection in self
            .cloud_connections()?
            .into_iter()
            .filter(|c| c.confirmed)
        {
            let Some(grant) = self.grant(&connection.owner)?.filter(|g| {
                g.live_at(now)
                    && g.public_key == connection.owner_key
                    && g.permits_scope("agents.read")
            }) else {
                continue;
            };
            let member = self.member_for_client(&grant.client_id)?;
            if member
                .as_ref()
                .is_some_and(|member| member.revoked_at_ms.is_some())
            {
                continue;
            }
            let member = member.map(|member| member.member_id);
            let row: Option<Option<Vec<u8>>> = self.connection.query_row("SELECT summary FROM squad_cache WHERE client_id=?1 AND public_key=?2 AND grant_revision=?3 AND member_id IS ?4", params![grant.client_id,grant.public_key.as_slice(),integer(grant.revision)?,member], |r| r.get(0)).optional().map_err(database)?;
            result.push(Record {
                machine_id: connection.owner,
                remote: connection.name,
                // Absence is local uncertainty, never a fabricated remote observation.
                summary: row.flatten().unwrap_or_default(),
            });
        }
        Ok(result)
    }
}

pub(crate) fn receive(permit: &crate::admission::MessagePermit) -> Result<Vec<u8>, RemoteError> {
    if !permit.cloud() {
        return Err(invalid());
    }
    permit.with_store(|store| {
        store.summary_received(
            &permit.grant,
            permit.message.envelope().id,
            &permit.message.payload,
            now_ms()?,
        )
    })?;
    permit.response(&serde_json::json!({"state":"accepted"}))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
