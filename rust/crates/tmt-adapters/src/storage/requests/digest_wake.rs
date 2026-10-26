//! SQL for current pending class bits and one persistent admission/pass clock.
use super::*;
use tmt_core::request::digest::{DigestPendingWake, DigestWakeClock};

pub(super) fn marker(
    db: &Connection,
    identity: &str,
) -> Result<Option<DigestPendingWake>, StorageError> {
    db.query_row("SELECT released_classes,released_at_ms,checklist_id FROM orchestration_pending_wakes WHERE identity_id=?", [identity], |r| Ok(DigestPendingWake {
        classes: r.get(0)?, held_at_ms: rows::u64_at(r,1)?, checklist_id: r.get(2)?,
    })).optional().map_err(|e|classify(e,"Read pending Incoming wake"))
}
pub(super) fn write_marker(
    db: &Connection,
    identity: &str,
    marker: Option<&DigestPendingWake>,
) -> Result<(), StorageError> {
    if let Some(marker) = marker {
        db.execute("INSERT INTO orchestration_pending_wakes(identity_id,released_classes,released_at_ms,checklist_id) VALUES(?1,?2,?3,?4)
            ON CONFLICT(identity_id) DO UPDATE SET released_classes=excluded.released_classes,released_at_ms=excluded.released_at_ms,checklist_id=excluded.checklist_id
            WHERE released_classes IS NOT excluded.released_classes OR released_at_ms IS NOT excluded.released_at_ms OR checklist_id IS NOT excluded.checklist_id",
            params![identity,marker.classes,checked_now(marker.held_at_ms,"Wake hold clock")?,marker.checklist_id])
            .map_err(|e|classify(e,"Record pending Incoming wake"))?;
    } else {
        db.execute(
            "DELETE FROM orchestration_pending_wakes WHERE identity_id=?",
            [identity],
        )
        .map_err(|e| classify(e, "Close pending Incoming wake"))?;
    }
    Ok(())
}
pub(super) fn clock(db: &Connection) -> Result<DigestWakeClock, StorageError> {
    Ok(db.query_row("SELECT next_admission_ms,scan_after_identity_id FROM orchestration_wake_clock WHERE id=1", [], |r| Ok(DigestWakeClock {
        next_admission_ms: rows::u64_at(r,0)?, scan_after_identity_id: r.get(1)?,
    })).optional().map_err(|e|classify(e,"Read Incoming wake clock"))?.unwrap_or_default())
}
pub(super) fn write_clock(db: &Connection, clock: &DigestWakeClock) -> Result<(), StorageError> {
    if self::clock(db)? == *clock {
        return Ok(());
    }
    db.execute("INSERT INTO orchestration_wake_clock(id,next_admission_ms,scan_after_identity_id) VALUES(1,?1,?2)
        ON CONFLICT(id) DO UPDATE SET next_admission_ms=excluded.next_admission_ms,scan_after_identity_id=excluded.scan_after_identity_id",
        params![checked_i64(clock.next_admission_ms,"Wake next admission")?,clock.scan_after_identity_id])
        .map_err(|e|classify(e,"Advance Incoming wake clock"))?;
    Ok(())
}
pub(super) fn candidates(
    db: &Connection,
    after: Option<&str>,
    limit: u64,
) -> Result<Vec<String>, StorageError> {
    let mut q=db.prepare("SELECT w.identity_id FROM orchestration_pending_wakes w JOIN identities i ON i.id=w.identity_id WHERE i.retired_at_ms IS NULL AND (?1 IS NULL OR w.identity_id>?1) ORDER BY w.identity_id COLLATE BINARY LIMIT ?2")
        .map_err(|e|classify(e,"Prepare pending Incoming wake pass"))?;
    q.query_map(params![after, checked_limit(limit)?], |r| r.get(0))
        .and_then(|r| r.collect())
        .map_err(|e| classify(e, "Read pending Incoming wake pass"))
}
