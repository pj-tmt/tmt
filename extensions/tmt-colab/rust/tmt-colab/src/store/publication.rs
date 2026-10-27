//! Confirmation metadata only: no pending payload store and no cloud enablement.
//! The admitted carrier owner supplies confirmations of the exact stored originals.
use super::{
    Fault, Namespace, Store, StoreResult, StreamScope, bounded_id, capacity, current, sequence,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use tmt_colab_model::{crypto, values};

/// Stable verified binding identity. Ephemeral channel/binding UUIDs and credentials
/// are deliberately absent, so a restart cannot manufacture or reset progress.
pub struct CloudBinding<'a> {
    pub project: &'a str,
    pub declaration_digest: &'a str,
    pub space: &'a str,
    pub uid: &'a str,
}
impl CloudBinding<'_> {
    fn digest(&self) -> StoreResult<[u8; 32]> {
        for (value, max) in [(self.project, 64), (self.uid, 128)] {
            if value.is_empty()
                || value.len() > max
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
            {
                return Err(Fault::Invalid);
            }
        }
        values::space_id(self.space).map_err(|_| Fault::Invalid)?;
        values::object_id(self.declaration_digest).map_err(|_| Fault::Invalid)?;
        let bytes = serde_json::to_vec(&serde_json::json!({
            "project":self.project,"extension":"colab",
            "declarationDigest":self.declaration_digest,"space":self.space,"uid":self.uid,
        }))
        .map_err(|_| Fault::Invalid)?;
        Ok(crypto::digest(&bytes))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicationProgress {
    /// This cut is pending at first tracking, never presumed confirmed.
    pub bootstrap_seq: u64,
    pub confirmed_seq: u64,
}
fn progress(
    c: &Connection,
    scope: StreamScope<'_>,
    binding: &CloudBinding<'_>,
) -> StoreResult<Option<PublicationProgress>> {
    let row: Option<(Vec<u8>, String, String, String)> = c.query_row(
        "SELECT binding,uid,bootstrap_seq,confirmed_seq FROM cloud_publications WHERE page=? AND epoch=? AND stream=?",
        params![scope.page, scope.epoch.to_string(), scope.stream],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional()?;
    let Some((digest, uid, bootstrap, confirmed)) = row else {
        return Ok(None);
    };
    if digest != binding.digest()? || uid != binding.uid {
        return Err(Fault::Conflict);
    }
    let initial: u64 = bootstrap.parse().map_err(|_| Fault::Invalid)?;
    let latest = paired(c, scope)?.unwrap_or(0);
    if latest < initial {
        return Err(Fault::ResyncRequired);
    }
    contiguous(c, scope, latest)?;
    Ok(Some(PublicationProgress {
        bootstrap_seq: latest,
        confirmed_seq: confirmed.parse().map_err(|_| Fault::Invalid)?,
    }))
}
pub(super) fn paired(c: &Connection, s: StreamScope<'_>) -> StoreResult<Option<u64>> {
    let seq: Option<String> = c.query_row(
        "SELECT max(a.seq) FROM checkpoints a JOIN checkpoints b
         ON a.page=b.page AND a.epoch=b.epoch AND a.stream=b.stream AND a.seq=b.seq
         JOIN receipts r ON r.page=a.page AND r.epoch=a.epoch AND r.stream=a.stream
         AND r.seq=a.seq AND r.hash=a.head AND r.hash=b.head
         WHERE a.page=? AND a.epoch=? AND a.stream=? AND a.namespace='content'
         AND b.namespace='own' AND a.payload IS NOT NULL AND b.payload IS NOT NULL",
        params![s.page, s.epoch.to_string(), s.stream],
        |r| r.get(0),
    )?;
    seq.map(|s| s.parse().map_err(|_| Fault::Invalid))
        .transpose()
}
fn contiguous(c: &Connection, s: StreamScope<'_>, cut: u64) -> StoreResult<()> {
    let mut q = c.prepare("SELECT seq,payload IS NOT NULL FROM receipts WHERE page=? AND epoch=? AND stream=? AND seq>? ORDER BY seq")?;
    let rows = q.query_map(
        params![s.page, s.epoch.to_string(), s.stream, sequence(cut)],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
    )?;
    let mut expected = cut;
    for row in rows {
        let (seq, retained) = row?;
        expected = expected.checked_add(1).ok_or(Fault::ResyncRequired)?;
        if seq.parse::<u64>().ok() != Some(expected) || !retained {
            return Err(Fault::ResyncRequired);
        }
    }
    Ok(())
}
impl Store {
    /// Begin tracking from the current paired checkpoint and its contiguous
    /// retained tail. No older pruned update is needed; the cut remains pending.
    pub fn track_publication(
        &mut self,
        scope: StreamScope<'_>,
        binding: &CloudBinding<'_>,
    ) -> StoreResult<PublicationProgress> {
        bounded_id(scope.page)?;
        bounded_id(scope.stream)?;
        let digest = binding.digest()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, scope)?;
        if let Some(progress) = progress(&tx, scope, binding)? {
            return Ok(progress);
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM cloud_publications WHERE page=?",
            [scope.page],
            |r| r.get(0),
        )?;
        if count >= crate::limits::SYNC_NAMESPACES as i64 {
            return Err(Fault::Capacity);
        }
        let cut = paired(&tx, scope)?.unwrap_or(0);
        contiguous(&tx, scope, cut)?;
        tx.execute(
            "INSERT INTO cloud_publications VALUES (?,?,?,?,?,?,?)",
            params![
                scope.page,
                scope.epoch.to_string(),
                scope.stream,
                digest.as_slice(),
                binding.uid,
                sequence(cut),
                sequence(0)
            ],
        )?;
        capacity(&tx, scope.page, 0, false)?;
        tx.commit()?;
        Ok(PublicationProgress {
            bootstrap_seq: cut,
            confirmed_seq: 0,
        })
    }
    /// Record, before the owner writes a link's cloud row, that the row may exist: its removal
    /// then survives a restart. Idempotent.
    pub fn track_link_publication(&mut self, page: &str, link: &str) -> StoreResult<()> {
        bounded_id(page)?;
        bounded_id(link)?;
        let count: i64 = self.connection.query_row(
            "SELECT count(*) FROM link_publications WHERE page=?",
            [page],
            |r| r.get(0),
        )?;
        if count >= crate::limits::SYNC_NAMESPACES as i64 {
            return Err(Fault::Capacity);
        }
        self.connection.execute(
            "INSERT OR IGNORE INTO link_publications VALUES (?,?)",
            params![page, link],
        )?;
        Ok(())
    }
    /// The links of a page whose cloud row may exist, by link ID.
    pub fn link_publications(&self, page: &str) -> StoreResult<Vec<String>> {
        let mut query = self
            .connection
            .prepare("SELECT link FROM link_publications WHERE page=? ORDER BY link")?;
        let rows = query
            .query_map([page], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
    /// The link's removal from the cloud is complete for this page.
    pub fn end_link_publication(&mut self, page: &str, link: &str) -> StoreResult<()> {
        self.connection.execute(
            "DELETE FROM link_publications WHERE page=? AND link=?",
            params![page, link],
        )?;
        Ok(())
    }
    /// The cloud confirmed the page's owner row for `epoch`, with `expires_at_ms` as its expiry.
    /// Only the page's current epoch is recorded: a confirmation that finished after the epoch
    /// moved changes nothing.
    pub fn confirm_page_row(
        &mut self,
        page: &str,
        epoch: u64,
        expires_at_ms: u64,
    ) -> StoreResult<()> {
        bounded_id(page)?;
        let expires = i64::try_from(expires_at_ms).map_err(|_| Fault::Invalid)?;
        self.connection.execute(
            "INSERT INTO cloud_page_rows(page,epoch,expires_at_ms)
             SELECT page,epoch,? FROM pages WHERE page=? AND epoch=?
             ON CONFLICT(page) DO UPDATE SET epoch=excluded.epoch,expires_at_ms=excluded.expires_at_ms",
            params![expires, page, epoch.to_string()],
        )?;
        Ok(())
    }
    pub fn publication_progress(
        &self,
        scope: StreamScope<'_>,
        binding: &CloudBinding<'_>,
    ) -> StoreResult<Option<PublicationProgress>> {
        current(&self.connection, scope)?;
        progress(&self.connection, scope, binding)
    }
    /// Only an exact acknowledged/read-back update can move the contiguous
    /// prefix; a checkpoint confirmation may separately cover its whole cut.
    pub fn confirm_publication(
        &mut self,
        scope: StreamScope<'_>,
        binding: &CloudBinding<'_>,
        seq: u64,
        hash: &[u8; 32],
    ) -> StoreResult<PublicationProgress> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, scope)?;
        let mut p = progress(&tx, scope, binding)?.ok_or(Fault::Invalid)?;
        let stored: Option<Vec<u8>> = tx
            .query_row(
                "SELECT hash FROM receipts WHERE page=? AND epoch=? AND stream=? AND seq=?",
                params![
                    scope.page,
                    scope.epoch.to_string(),
                    scope.stream,
                    sequence(seq)
                ],
                |r| r.get(0),
            )
            .optional()?;
        if stored.as_deref() != Some(hash.as_slice()) {
            return Err(Fault::Conflict);
        }
        if seq > p.confirmed_seq {
            if seq != p.confirmed_seq.checked_add(1).ok_or(Fault::Gap)? {
                return Err(Fault::Gap);
            }
            p.confirmed_seq = seq;
            set_confirmed(&tx, scope, seq)?;
        }
        let cut = paired(&tx, scope)?;
        if let Some(cut) = cut {
            prune(&tx, scope, &sequence(cut))?;
        }
        tx.commit()?;
        if cut.is_some() {
            super::schema::reclaim(&self.connection);
        }
        Ok(p)
    }
    /// Both namespace originals must be confirmed. Checksums here are compared
    /// to admitted stored bytes; the caller still owns cryptographic authority.
    pub fn confirm_checkpoint_publication(
        &mut self,
        scope: StreamScope<'_>,
        binding: &CloudBinding<'_>,
        seq: u64,
        content_digest: &[u8; 32],
        own_digest: &[u8; 32],
    ) -> StoreResult<PublicationProgress> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, scope)?;
        let mut p = progress(&tx, scope, binding)?.ok_or(Fault::Invalid)?;
        if seq == 0 {
            return Err(Fault::Invalid);
        }
        for (namespace, digest) in [
            (Namespace::Content, content_digest),
            (Namespace::Own, own_digest),
        ] {
            let stored: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT c.digest FROM checkpoints c JOIN receipts r
                 ON r.page=c.page AND r.epoch=c.epoch AND r.stream=c.stream
                 AND r.seq=c.seq AND r.hash=c.head
                 WHERE c.page=? AND c.epoch=? AND c.stream=? AND c.namespace=? AND c.seq=?",
                    params![
                        scope.page,
                        scope.epoch.to_string(),
                        scope.stream,
                        namespace.name(),
                        sequence(seq)
                    ],
                    |r| r.get(0),
                )
                .optional()?;
            if stored.as_deref() != Some(digest.as_slice()) {
                return Err(Fault::Conflict);
            }
        }
        if seq > p.confirmed_seq {
            p.confirmed_seq = seq;
            set_confirmed(&tx, scope, seq)?;
        }
        let cut = paired(&tx, scope)?;
        if let Some(cut) = cut {
            prune(&tx, scope, &sequence(cut))?;
        }
        tx.commit()?;
        if cut.is_some() {
            super::schema::reclaim(&self.connection);
        }
        Ok(p)
    }
}
fn set_confirmed(c: &Connection, s: StreamScope<'_>, seq: u64) -> StoreResult<()> {
    c.execute(
        "UPDATE cloud_publications SET confirmed_seq=? WHERE page=? AND epoch=? AND stream=?",
        params![sequence(seq), s.page, s.epoch.to_string(), s.stream],
    )?;
    Ok(())
}
// Both the read-only estimate and committed prune use these exact candidates.
const RECEIPT_CANDIDATES: &str =
    "page=?1 AND epoch=?2 AND stream=?3 AND seq<=?4 AND (seq<>?5 OR NOT ?6)";
const CHECKPOINT_CANDIDATES: &str = "page=?1 AND epoch=?2 AND stream=?3 AND seq<?4 AND pinned=0";

struct PruneBounds {
    cut: String,
    latest: String,
    anchored: bool,
}
fn prune_bounds(
    c: &Connection,
    s: StreamScope<'_>,
    paired_seq: &str,
) -> StoreResult<Option<PruneBounds>> {
    let confirmed: Option<String> = c
        .query_row(
            "SELECT confirmed_seq FROM cloud_publications WHERE page=? AND epoch=? AND stream=?",
            params![s.page, s.epoch.to_string(), s.stream],
            |r| r.get(0),
        )
        .optional()?;
    let latest = if confirmed.is_some() {
        paired(c, s)?.map(sequence)
    } else {
        Some(paired_seq.to_owned())
    };
    let Some(latest) = latest else {
        return Ok(None);
    };
    let cut = confirmed
        .as_deref()
        .map_or(latest.as_str(), |seq| seq.min(latest.as_str()))
        .to_owned();
    // An owner fold binds the retained update at its sequence; keep that anchor.
    let anchored = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM checkpoints WHERE page=?1 AND epoch=?2 AND stream=?3 AND seq=?4
         AND signer IS NOT NULL AND payload IS NOT NULL)",
        params![s.page, s.epoch.to_string(), s.stream, latest], |r| r.get(0),
    )?;
    Ok(Some(PruneBounds {
        cut,
        latest,
        anchored,
    }))
}
fn candidate_history_bytes(
    c: &Connection,
    s: StreamScope<'_>,
    bounds: &PruneBounds,
) -> StoreResult<u64> {
    let receipt_sql = format!(
        "SELECT COALESCE(sum(length(payload)),0) FROM receipts WHERE {RECEIPT_CANDIDATES} AND namespace='content'"
    );
    let checkpoint_sql = format!(
        "SELECT COALESCE(sum(length(payload)),0) FROM checkpoints WHERE {CHECKPOINT_CANDIDATES} AND namespace='content'"
    );
    let receipts: i64 = c.query_row(
        &receipt_sql,
        params![
            s.page,
            s.epoch.to_string(),
            s.stream,
            bounds.cut,
            bounds.latest,
            bounds.anchored
        ],
        |r| r.get(0),
    )?;
    let checkpoints: i64 = c.query_row(
        &checkpoint_sql,
        params![s.page, s.epoch.to_string(), s.stream, bounds.latest],
        |r| r.get(0),
    )?;
    let sum = receipts.checked_add(checkpoints).ok_or(Fault::Capacity)?;
    u64::try_from(sum).map_err(|_| Fault::Invalid)
}
/// Whether a stream has an update after its latest checkpoint in that namespace.
pub(super) fn has_active_tail(c: &Connection, s: StreamScope<'_>) -> StoreResult<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM receipts r WHERE r.page=?1 AND r.epoch=?2 AND r.stream=?3 AND r.payload IS NOT NULL AND r.seq>COALESCE((SELECT max(c.seq) FROM checkpoints c WHERE c.page=r.page AND c.epoch=r.epoch AND c.stream=r.stream AND c.namespace=r.namespace), '00000000000000000000'))",
        params![s.page, s.epoch.to_string(), s.stream], |r| r.get(0),
    )?)
}

/// Proven currently eligible content bytes, evaluated inside the caller's read snapshot.
pub(super) fn eligible_history_bytes(
    c: &Connection,
    page: &str,
    allowed: Option<&std::collections::BTreeSet<String>>,
) -> StoreResult<u64> {
    bounded_id(page)?;
    let allowed_json = allowed
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| Fault::Invalid)?;
    let mut query = c.prepare(
        "SELECT s.epoch,s.stream FROM streams s JOIN pages p ON p.page=s.page AND p.epoch=s.epoch WHERE s.page=?1 AND (?2 IS NULL OR EXISTS(SELECT 1 FROM json_each(?2) AS allowed WHERE allowed.value=s.stream))"
    )?;
    let rows = query.query_map(params![page, allowed_json], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut total = 0u64;
    for row in rows {
        let (epoch, stream) = row?;
        let epoch = epoch.parse().map_err(|_| Fault::Invalid)?;
        let scope = StreamScope {
            page,
            epoch,
            stream: &stream,
        };
        if has_active_tail(c, scope)?
            && let Some(seq) = paired(c, scope)?
            && let Some(bounds) = prune_bounds(c, scope, &sequence(seq))?
        {
            total = total
                .checked_add(candidate_history_bytes(c, scope, &bounds)?)
                .ok_or(Fault::Capacity)?;
        }
    }
    Ok(total)
}
impl Store {
    /// Existing prune candidates belonging only to already-admitted compactable streams.
    pub(crate) fn eligible_history_for_streams(
        &self,
        page: &str,
        streams: &std::collections::BTreeSet<String>,
    ) -> StoreResult<u64> {
        let tx = self.connection.unchecked_transaction()?;
        let total = eligible_history_bytes(&tx, page, Some(streams))?;
        tx.commit()?;
        Ok(total)
    }

    /// Proven presently eligible content payloads; excludes speculative rebuild savings.
    pub fn eligible_history_bytes(&self, page: &str) -> StoreResult<u64> {
        let tx = self.connection.unchecked_transaction()?;
        let total = eligible_history_bytes(&tx, page, None)?;
        tx.commit()?;
        Ok(total)
    }
}
pub(super) fn prune(c: &Connection, s: StreamScope<'_>, paired_seq: &str) -> StoreResult<()> {
    let Some(bounds) = prune_bounds(c, s, paired_seq)? else {
        return Ok(());
    };
    let receipt_sql = format!("UPDATE receipts SET payload=NULL WHERE {RECEIPT_CANDIDATES}");
    c.execute(
        &receipt_sql,
        params![
            s.page,
            s.epoch.to_string(),
            s.stream,
            bounds.cut,
            bounds.latest,
            bounds.anchored
        ],
    )?;
    // The latest paired original and every pinned checkpoint stay retained.
    let checkpoint_sql =
        format!("UPDATE checkpoints SET payload=NULL WHERE {CHECKPOINT_CANDIDATES}");
    c.execute(
        &checkpoint_sql,
        params![s.page, s.epoch.to_string(), s.stream, bounds.latest],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
