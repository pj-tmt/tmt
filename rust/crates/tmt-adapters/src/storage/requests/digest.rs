//! Digest row admission and bounded SQL inside the canonical request transaction.
use super::*;
use tmt_core::request::digest::{
    DeliveryPolicy, DigestChecklist, DigestItem, DigestKind, DigestPolicy, DigestSource,
    DigestState,
};

pub(super) fn policy(db: &Connection, id: &str) -> Result<Option<DigestPolicy>, StorageError> {
    db.query_row("SELECT revision,until_ms,owner_identity_id,setter_identity_id FROM focus_policies WHERE identity_id=?", [id], |r| {
        Ok(DigestPolicy {identity_id:id.into(),revision:rows::u64_at(r,0)?,until_ms:rows::u64_at(r,1)?,owner_identity_id:r.get(2)?,setter_identity_id:r.get(3)?})
    }).optional().map_err(|e| classify(e,"Read digest policy"))
}
pub(super) fn write_policy(db: &Connection, p: &DigestPolicy) -> Result<(), StorageError> {
    db.execute("INSERT INTO focus_policies(identity_id,revision,until_ms,owner_identity_id,setter_identity_id) VALUES(?1,?2,?3,?4,?5)
        ON CONFLICT(identity_id) DO UPDATE SET revision=excluded.revision,until_ms=excluded.until_ms,owner_identity_id=excluded.owner_identity_id,setter_identity_id=excluded.setter_identity_id",
        params![p.identity_id,checked_i64(p.revision,"Digest revision")?,checked_i64(p.until_ms,"Digest expiry")?,p.owner_identity_id,p.setter_identity_id])
        .map_err(|e|classify(e,"Write digest policy"))?;
    Ok(())
}
pub(super) fn delivery_policy(db: &Connection, id: &str) -> Result<DeliveryPolicy, StorageError> {
    Ok(db
        .query_row(
            "SELECT urgent,kind,automatic FROM request_delivery_policies WHERE request_id=?",
            [id],
            |r| {
                let urgent: i64 = r.get(0)?;
                let automatic: i64 = r.get(2)?;
                let kind = DigestKind::parse(&r.get::<_, String>(1)?)
                    .filter(|k| *k != DigestKind::Result)
                    .ok_or(rusqlite::Error::InvalidQuery)?;
                if ![0, 1].contains(&urgent) || ![0, 1].contains(&automatic) {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(DeliveryPolicy {
                    urgent: urgent == 1,
                    kind,
                    automatic: automatic == 1,
                })
            },
        )
        .optional()
        .map_err(|e| classify(e, "Read request delivery policy"))?
        .unwrap_or_default())
}
pub(super) fn write_delivery_policy(
    db: &Connection,
    id: &str,
    p: &DeliveryPolicy,
) -> Result<(), StorageError> {
    db.execute("INSERT INTO request_delivery_policies(request_id,urgent,kind,automatic) VALUES(?1,?2,?3,?4)
        ON CONFLICT(request_id) DO UPDATE SET urgent=excluded.urgent,kind=excluded.kind,automatic=excluded.automatic",
        params![id,i64::from(p.urgent),p.kind.as_str(),i64::from(p.automatic)]).map_err(|e|classify(e,"Write request delivery policy"))?;
    Ok(())
}
pub(super) fn hold(
    db: &Connection,
    identity: &str,
    request: &str,
    kind: DigestKind,
    source: DigestSource,
    now: u64,
) -> Result<(), StorageError> {
    // Read the stored observation through its driver owner, without refreshing or
    // doing process/filesystem I/O inside the canonical request transaction.
    use tmt_core::binding::BindingRecords;
    let preferences = crate::storage::bindings::BindingRows(db).session_preferences(identity)?;
    let usage = preferences
        .remembered
        .as_ref()
        .filter(|session| session.stale_at_ms.is_none() && session.resume_pending_at_ms.is_none())
        .and_then(|session| {
            crate::runtime::RuntimeRegistry::first_party().remembered_usage(session)
        })
        .filter(|usage| usage.observed_at_ms <= now);
    db.execute("INSERT INTO focus_items(identity_id,request_id,kind,source,created_at_ms,context_tokens_at_arrival,context_observed_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)
        ON CONFLICT(identity_id,request_id,source) DO NOTHING",params![identity,request,kind.as_str(),source.as_str(),checked_now(now,"Digest item clock")?,usage.map(|u|u.tokens as i64),usage.map(|u|u.observed_at_ms as i64)])
        .map_err(|e|classify(e,"Hold digest item"))?;
    if source == DigestSource::Result {
        // These frames have no external attempt. Their canonical result hint
        // and the Digest reference move under the same request transaction.
        db.execute(
            "DELETE FROM reply_notices WHERE request_id=? AND attempted=0",
            [request],
        )
        .map_err(|e| classify(e, "Move unattempted reply notice to Digest"))?;
    }
    Ok(())
}

// DigestItem visibility follows canonical metadata retention. A reference never
// extends the prompt, final or response-acceptance horizon.
const ITEM_SCOPE: &str =
    "i.identity_id=?1 AND ((?2 IS NULL AND i.checklist_id IS NULL) OR i.checklist_id=?2)
    AND a.retention_expires_at_ms>?3 AND i.sequence>?4";
pub(super) fn inventory(
    db: &Connection,
    identity: &str,
    batch: Option<&str>,
    after: u64,
    now: u64,
) -> Result<(u64, u64), StorageError> {
    db.query_row(&format!("SELECT COUNT(*),COALESCE(MAX(i.sequence),0) FROM focus_items i JOIN request_attempts a USING(request_id) WHERE {ITEM_SCOPE}"),
        params![identity,batch,checked_now(now,"Digest read clock")?,checked_i64(after,"Digest cursor")?],|r|Ok((rows::u64_at(r,0)?,rows::u64_at(r,1)?)))
        .map_err(|e|classify(e,"Read digest inventory"))
}
pub(super) fn counters(
    db: &Connection,
    identity: &str,
) -> Result<tmt_core::request::digest::DigestCounters, StorageError> {
    db.query_row(
        "SELECT due_through_sequence,delivered_digests FROM digest_counters WHERE identity_id=?",
        [identity],
        |row| {
            Ok(tmt_core::request::digest::DigestCounters {
                due_through_sequence: rows::u64_at(row, 0)?,
                delivered_digests: rows::u64_at(row, 1)?,
            })
        },
    )
    .optional()
    .map(|value| value.unwrap_or_default())
    .map_err(|e| classify(e, "Read digest counters"))
}

pub(super) fn mark_due(db: &Connection, identity: &str, through: u64) -> Result<(), StorageError> {
    db.execute("INSERT INTO digest_counters(identity_id,due_through_sequence) VALUES(?1,?2)
        ON CONFLICT(identity_id) DO UPDATE SET due_through_sequence=MAX(digest_counters.due_through_sequence,excluded.due_through_sequence)",
        params![identity,checked_i64(through,"Digest due watermark")?]).map_err(|e| classify(e,"Mark held digest range due"))?;
    Ok(())
}

pub(super) fn due_inventory(
    db: &Connection,
    identity: &str,
    through: u64,
    now: u64,
) -> Result<(u64, u64), StorageError> {
    db.query_row("SELECT COUNT(*),COALESCE(MAX(i.sequence),0) FROM focus_items i JOIN request_attempts a USING(request_id)
        WHERE i.identity_id=?1 AND i.checklist_id IS NULL AND i.sequence<=?2 AND a.retention_expires_at_ms>?3",
        params![identity,checked_i64(through,"Digest due watermark")?,checked_now(now,"Digest due clock")?],
        |r| Ok((rows::u64_at(r,0)?,rows::u64_at(r,1)?))).map_err(|e| classify(e,"Read due digest inventory"))
}

pub(super) fn oldest_held(
    db: &Connection,
    identity: &str,
    now: u64,
) -> Result<Option<u64>, StorageError> {
    db.query_row("SELECT MIN(i.created_at_ms) FROM focus_items i JOIN request_attempts a USING(request_id)
        LEFT JOIN focus_checklists c ON c.id=i.checklist_id WHERE i.identity_id=?1 AND a.retention_expires_at_ms>?2
        AND (i.checklist_id IS NULL OR c.state='claimed')", params![identity,checked_now(now,"Digest age clock")?],
        |row| row.get::<_,Option<i64>>(0)?.map(|n| u64::try_from(n).map_err(|_| rusqlite::Error::InvalidQuery)).transpose())
        .map_err(|e| classify(e,"Read oldest held digest timestamp"))
}

pub(super) fn items(
    db: &Connection,
    identity: &str,
    batch: Option<&str>,
    after: u64,
    limit: u64,
    now: u64,
) -> Result<Vec<DigestItem>, StorageError> {
    let mut q=db.prepare(&format!("SELECT i.sequence,i.request_id,i.kind,i.created_at_ms,i.checklist_id,i.source,i.context_tokens_at_arrival,i.context_observed_at_ms FROM focus_items i
        JOIN request_attempts a USING(request_id) WHERE {ITEM_SCOPE} ORDER BY i.sequence LIMIT ?5"))
        .map_err(|e|classify(e,"Prepare digest checklist read"))?;
    q.query_map(
        params![
            identity,
            batch,
            checked_now(now, "Digest read clock")?,
            checked_i64(after, "Digest cursor")?,
            checked_limit(limit)?
        ],
        |r| {
            Ok(DigestItem {
                sequence: rows::u64_at(r, 0)?,
                identity_id: identity.into(),
                request_id: r.get(1)?,
                kind: DigestKind::parse(&r.get::<_, String>(2)?)
                    .ok_or(rusqlite::Error::InvalidQuery)?,
                created_at_ms: rows::u64_at(r, 3)?,
                checklist_id: r.get(4)?,
                source: DigestSource::parse(&r.get::<_, String>(5)?)
                    .ok_or(rusqlite::Error::InvalidQuery)?,
                context_tokens_at_arrival: r
                    .get::<_, Option<i64>>(6)?
                    .map(|n| u64::try_from(n).map_err(|_| rusqlite::Error::InvalidQuery))
                    .transpose()?,
                context_observed_at_ms: r
                    .get::<_, Option<i64>>(7)?
                    .map(|n| u64::try_from(n).map_err(|_| rusqlite::Error::InvalidQuery))
                    .transpose()?,
            })
        },
    )
    .and_then(|r| r.collect())
    .map_err(|e| classify(e, "Read digest items"))
}
fn checklist_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DigestChecklist> {
    Ok(DigestChecklist {
        id: r.get(0)?,
        identity_id: r.get(1)?,
        attempt_token: r.get(2)?,
        through_sequence: rows::u64_at(r, 3)?,
        state: DigestState::parse(&r.get::<_, String>(4)?).ok_or(rusqlite::Error::InvalidQuery)?,
        created_at_ms: rows::u64_at(r, 5)?,
    })
}
const CHECKLIST_COLUMNS: &str = "id,identity_id,attempt_token,through_sequence,state,created_at_ms";
pub(super) fn checklist(
    db: &Connection,
    id: &str,
) -> Result<Option<DigestChecklist>, StorageError> {
    db.query_row(
        &format!("SELECT {CHECKLIST_COLUMNS} FROM focus_checklists WHERE id=?"),
        [id],
        checklist_row,
    )
    .optional()
    .map_err(|e| classify(e, "Read digest checklist attempt"))
}
pub(super) fn active(
    db: &Connection,
    identity: &str,
) -> Result<Option<DigestChecklist>, StorageError> {
    db.query_row(&format!("SELECT {CHECKLIST_COLUMNS} FROM focus_checklists WHERE identity_id=? AND state='claimed'"),[identity],checklist_row)
        .optional().map_err(|e|classify(e,"Read active digest checklist"))
}
pub(super) fn create_checklist(
    db: &Connection,
    b: &DigestChecklist,
    now: u64,
) -> Result<(), StorageError> {
    db.execute("INSERT INTO focus_checklists(id,identity_id,attempt_token,through_sequence,state,created_at_ms) VALUES(?1,?2,?3,?4,'claimed',?5)",
        params![b.id,b.identity_id,b.attempt_token,checked_i64(b.through_sequence,"Digest snapshot sequence")?,checked_now(b.created_at_ms,"Digest claim clock")?])
        .map_err(|e|classify(e,"Claim digest checklist"))?;
    db.execute("UPDATE focus_items SET checklist_id=?1 WHERE identity_id=?2 AND checklist_id IS NULL AND sequence<=?3
        AND EXISTS(SELECT 1 FROM request_attempts a WHERE a.request_id=focus_items.request_id AND a.retention_expires_at_ms>?4)",
        params![b.id,b.identity_id,checked_i64(b.through_sequence,"Digest snapshot sequence")?,checked_now(now,"Digest claim clock")?])
        .map_err(|e|classify(e,"Seal digest checklist membership"))?;
    Ok(())
}
pub(super) fn settle(
    db: &Connection,
    b: &DigestChecklist,
    state: DigestState,
) -> Result<(), StorageError> {
    let changed=db.execute("UPDATE focus_checklists SET state=?1 WHERE id=?2 AND identity_id=?3 AND attempt_token=?4 AND state='claimed'",
        params![state.as_str(),b.id,b.identity_id,b.attempt_token]).map_err(|e|classify(e,"Settle digest checklist"))?;
    if changed != 1 {
        return Err(StorageError::new(
            StorageErrorCode::Unknown,
            "Digest checklist claim changed",
        ));
    }
    if state == DigestState::Delivered {
        db.execute("INSERT INTO digest_counters(identity_id,delivered_digests) VALUES(?1,1)
            ON CONFLICT(identity_id) DO UPDATE SET delivered_digests=digest_counters.delivered_digests+1", [&b.identity_id])
            .map_err(|e| classify(e,"Count successful digest delivery"))?;
    }
    if state == DigestState::Unsent {
        db.execute(
            "UPDATE focus_items SET checklist_id=NULL WHERE checklist_id=?",
            [&b.id],
        )
        .map_err(|e| classify(e, "Release definitely-unsent digest membership"))?;
    }
    Ok(())
}

pub(super) fn has_item(
    db: &Connection,
    identity: &str,
    request: &str,
    source: DigestSource,
) -> Result<bool, StorageError> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM focus_items WHERE identity_id=?1 AND request_id=?2 AND source=?3)",params![identity,request,source.as_str()],|r|r.get(0)).map_err(|e|classify(e,"Read held digest reference"))
}

pub(super) fn prune_settled(db: &Connection, cutoff: u64, limit: u64) -> Result<(), StorageError> {
    // Canonical request cleanup cascades expired member links. Definite-unsent
    // settlement already released its members. Keep claimed/uncertain records
    // discoverable even after their members expire: age is no replay lease.
    db.execute(
        "DELETE FROM focus_checklists WHERE id IN (
            SELECT id FROM focus_checklists INDEXED BY focus_checklists_settled_cleanup
            WHERE state IN ('delivered','definitely_unsent') AND created_at_ms<=?1
              AND NOT EXISTS (SELECT 1 FROM focus_items WHERE checklist_id=focus_checklists.id)
            ORDER BY created_at_ms,id LIMIT ?2
        )",
        params![
            checked_i64(cutoff, "Digest bookkeeping cutoff")?,
            checked_limit(limit)?
        ],
    )
    .map_err(|error| classify(error, "Prune settled Digest checklists"))?;
    Ok(())
}
