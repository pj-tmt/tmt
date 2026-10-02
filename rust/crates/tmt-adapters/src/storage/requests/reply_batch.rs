//! SQL for the request-notification owner's fixed pane windows and claims.
use super::*;
use tmt_core::{
    endpoint::ProcessIncarnation,
    operation::new_operation_id,
    request::notification::{
        OriginatorHint,
        batch::{Batch, MAX_NOTICES, Notice, SendClaim},
    },
};

const COLUMNS: &str = "id, originator_id, binding_id, due_ms, window_ms, quiet_ms, worker_pid, worker_start, sending, EXISTS (SELECT 1 FROM reply_notices n WHERE n.batch_id=reply_notice_batches.id AND n.attempted=0)";
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Batch> {
    let pid: Option<i64> = row.get(6)?;
    let start: Option<String> = row.get(7)?;
    let worker = match (pid, start) {
        (Some(pid), Some(start)) => Some(
            ProcessIncarnation::new(
                u64::try_from(pid).map_err(|_| rusqlite::Error::InvalidQuery)?,
                &start,
            )
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        ),
        (None, None) => None,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Batch {
        id: row.get(0)?,
        originator_id: row.get(1)?,
        binding_id: row.get(2)?,
        due_ms: rows::u64_at(row, 3)?,
        window_ms: rows::u64_at(row, 4)?,
        quiet_ms: rows::u64_at(row, 5)?,
        worker,
        sending: row.get(8)?,
        pending: row.get(9)?,
    })
}
fn read(db: &Connection, id: &str) -> Result<Option<Batch>, StorageError> {
    db.query_row(
        &format!("SELECT {COLUMNS} FROM reply_notice_batches WHERE id = ?"),
        [id],
        row,
    )
    .optional()
    .map_err(|e| classify(e, "Read reply notice batch"))
}
impl Storage {
    /// Duplicate request IDs retain their original window. No transport runs in
    /// this immediate transaction; final acceptance already owns the reply body.
    pub fn queue_reply_notice(
        &mut self,
        hint: &OriginatorHint,
        binding_id: &str,
        text: &str,
        window_ms: u64,
        quiet_ms: u64,
        now: u64,
    ) -> Result<Batch, StorageError> {
        with_immediate_transaction(self, "reply notice enqueue", |db| {
            let claimed: bool = db.query_row("SELECT EXISTS (SELECT 1 FROM request_notifications WHERE request_id=? AND reply_state='claimed')", [&hint.request_id], |row| row.get(0))
                .map_err(|error| classify(error, "Verify reply notice claim"))?;
            if !claimed {
                return Err(StorageError::new(
                    StorageErrorCode::Unknown,
                    "Reply notice is not claimed",
                ));
            }
            // Empty completed shells are bookkeeping; bound cleanup per enqueue.
            db.execute("DELETE FROM reply_notice_batches WHERE id IN (SELECT id FROM reply_notice_batches b WHERE NOT EXISTS (SELECT 1 FROM reply_notices n WHERE n.batch_id=b.id) LIMIT 32)",[])
                .map_err(|e|classify(e,"Clean empty reply batches"))?;
            let existing: Option<String> = db
                .query_row(
                    "SELECT batch_id FROM reply_notices WHERE request_id=?",
                    [&hint.request_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| classify(e, "Find queued reply notice"))?;
            if let Some(id) = existing {
                return read(db, &id)?.ok_or_else(|| {
                    StorageError::new(StorageErrorCode::Unknown, "Missing reply batch")
                });
            }
            let id: Option<String> = if window_ms == 0 {
                None
            } else {
                db.query_row("SELECT id FROM reply_notice_batches b WHERE binding_id=? AND originator_id=? AND sending=0 AND window_ms>0 AND (SELECT count(*) FROM reply_notices n WHERE n.batch_id=b.id) < ? ORDER BY due_ms LIMIT 1",
                params![binding_id,hint.originator_id,MAX_NOTICES as i64],|r|r.get(0)).optional().map_err(|e|classify(e,"Find open reply batch"))?
            };
            let id = match id {
                Some(id) => id,
                None => {
                    let id = new_operation_id();
                    db.execute("INSERT INTO reply_notice_batches (id,originator_id,binding_id,due_ms,window_ms,quiet_ms) VALUES (?,?,?,?,?,?)",
                        params![id,hint.originator_id,binding_id,checked_now(now.saturating_add(window_ms),"Reply batch deadline")?,checked_i64(window_ms,"Reply batch window")?,checked_i64(quiet_ms,"Typing quiet period")?])
                        .map_err(|e|classify(e,"Create reply batch"))?;
                    id
                }
            };
            db.execute(
                "INSERT INTO reply_notices (request_id,batch_id,text) VALUES (?,?,?)",
                params![hint.request_id, id, text],
            )
            .map_err(|e| classify(e, "Append reply notice"))?;
            read(db, &id)?
                .ok_or_else(|| StorageError::new(StorageErrorCode::Unknown, "Missing reply batch"))
        })
    }
    pub fn reply_notice_batch(&self, id: &str) -> Result<Option<Batch>, StorageError> {
        read(self.connection()?, id)
    }
    /// The caller independently proves that an incumbent is gone. CAS protects
    /// a worker elected while that observation was in flight.
    pub fn claim_reply_notice_worker(
        &mut self,
        batch: &Batch,
        worker: &ProcessIncarnation,
    ) -> Result<bool, StorageError> {
        with_immediate_transaction(self, "reply worker claim", |db| {
            let changed=db.execute("UPDATE reply_notice_batches SET worker_pid=?,worker_start=? WHERE id=? AND worker_pid IS ? AND worker_start IS ? AND EXISTS (SELECT 1 FROM reply_notices WHERE batch_id=? AND attempted=0)",
                params![checked_now(worker.pid(),"Reply worker PID")?,worker.start_identity(),batch.id,batch.worker.as_ref().map(|p|checked_now(p.pid(),"Previous reply worker PID")).transpose()?,batch.worker.as_ref().map(ProcessIncarnation::start_identity),batch.id])
                .map_err(|e|classify(e,"Claim reply worker"))?;
            if changed == 1 {
                retire_attempted(db, &batch.id)?;
                db.execute(
                    "UPDATE reply_notice_batches SET sending=0 WHERE id=?",
                    [&batch.id],
                )
                .map_err(|error| classify(error, "Resume untouched reply notices"))?;
            }
            Ok(changed == 1)
        })
    }
    /// Seal membership before choosing transport. The delivery owner separately
    /// marks each driver frame, or every joined host frame, before external input.
    pub fn claim_reply_notice_send(
        &mut self,
        id: &str,
        worker: &ProcessIncarnation,
    ) -> Result<SendClaim, StorageError> {
        with_immediate_transaction(self, "reply batch send claim", |db| {
            let Some(batch) = read(db, id)? else {
                return Ok(SendClaim::Lost);
            };
            if batch.sending || batch.worker.as_ref() != Some(worker) {
                return Ok(SendClaim::Lost);
            }
            let active = db.query_row(
                &format!("SELECT {COLUMNS} FROM reply_notice_batches WHERE binding_id=? AND sending=1"),
                [&batch.binding_id], row,
            ).optional().map_err(|error| classify(error, "Read pane reply sender"))?;
            if let Some(active) = active {
                return Ok(SendClaim::Waiting(active));
            }
            db.execute("UPDATE reply_notice_batches SET sending=1 WHERE id=?", [id])
                .map_err(|error| classify(error, "Claim reply batch transport"))?;
            let mut query=db.prepare("SELECT request_id,text FROM reply_notices WHERE batch_id=? AND attempted=0 ORDER BY request_id").map_err(|e|classify(e,"Read batch members"))?;
            let values = query
                .query_map([id], |r| {
                    Ok(Notice {
                        request_id: r.get(0)?,
                        text: r.get(1)?,
                    })
                })
                .and_then(|r| r.collect())
                .map_err(|e| classify(e, "Decode batch members"))?;
            Ok(SendClaim::Ready(values))
        })
    }
    /// Call only after independently proving this exact worker Gone. A stale
    /// observation cannot release a newly elected sender. Attempted input is
    /// uncertain; untouched members remain available to their elected worker.
    pub fn release_reply_notice_send(&mut self, batch: &Batch) -> Result<bool, StorageError> {
        let Some(worker) = &batch.worker else {
            return Ok(false);
        };
        with_immediate_transaction(self, "dead reply sender release", |db| {
            let changed = db.execute(
                "UPDATE reply_notice_batches SET sending=0 WHERE id=? AND sending=1 AND worker_pid=? AND worker_start=?",
                params![batch.id, checked_now(worker.pid(), "Reply worker PID")?, worker.start_identity()],
            ).map_err(|error| classify(error, "Release dead reply sender"))?;
            if changed == 1 {
                retire_attempted(db, &batch.id)?;
                db.execute("DELETE FROM reply_notice_batches WHERE id=? AND NOT EXISTS (SELECT 1 FROM reply_notices WHERE batch_id=?)", params![batch.id,batch.id])
                    .map_err(|error| classify(error, "Remove dead sender's empty batch"))?;
            }
            Ok(changed == 1)
        })
    }

    /// A subsequent reply can restart only still-untouched work for this pane.
    pub fn pending_reply_notice_batches(
        &self,
        binding_id: &str,
    ) -> Result<Vec<Batch>, StorageError> {
        let db = self.connection()?;
        let mut statement=db.prepare(&format!("SELECT {COLUMNS} FROM reply_notice_batches WHERE binding_id=? AND EXISTS (SELECT 1 FROM reply_notices WHERE batch_id=reply_notice_batches.id AND attempted=0) ORDER BY due_ms LIMIT 4"))
            .map_err(|error|classify(error,"Read resumable pane notice batches"))?;
        statement
            .query_map([binding_id], row)
            .and_then(|rows| rows.collect())
            .map_err(|error| classify(error, "Decode resumable pane notice batches"))
    }

    pub fn mark_reply_notice_attempted(
        &mut self,
        id: &str,
        request_id: &str,
        worker: &ProcessIncarnation,
    ) -> Result<bool, StorageError> {
        with_immediate_transaction(self, "reply frame attempt", |db| {
            let changed=db.execute("UPDATE reply_notices SET attempted=1 WHERE batch_id=? AND request_id=? AND attempted=0 AND EXISTS (SELECT 1 FROM reply_notice_batches WHERE id=? AND sending=1 AND worker_pid=? AND worker_start=?)",params![id,request_id,id,checked_now(worker.pid(),"Reply worker PID")?,worker.start_identity()])
                .map_err(|error|classify(error,"Claim reply notice frame"))?;
            Ok(changed == 1)
        })
    }

    /// The first driver frame may already be claimed but proved NotSent or
    /// Unsupported within this same attempt. Joined fallback claims the rest.
    pub fn mark_reply_notice_fallback_attempted(
        &mut self,
        id: &str,
        worker: &ProcessIncarnation,
    ) -> Result<bool, StorageError> {
        with_immediate_transaction(self, "reply host attempt", |db| {
            let owned:bool=db.query_row("SELECT EXISTS (SELECT 1 FROM reply_notice_batches WHERE id=? AND sending=1 AND worker_pid=? AND worker_start=?)",params![id,checked_now(worker.pid(),"Reply worker PID")?,worker.start_identity()],|row|row.get(0))
                .map_err(|error|classify(error,"Verify reply host owner"))?;
            if owned {
                db.execute(
                    "UPDATE reply_notices SET attempted=1 WHERE batch_id=?",
                    [id],
                )
                .map_err(|error| classify(error, "Claim joined reply notice input"))?;
            }
            Ok(owned)
        })
    }

    pub fn settle_reply_notice_member(
        &mut self,
        id: &str,
        request_id: &str,
        outcome: WakeState,
    ) -> Result<(), StorageError> {
        validate_outcome(outcome)?;
        with_immediate_transaction(self, "reply frame settlement", |db| {
            db.execute("UPDATE request_notifications SET reply_state=? WHERE request_id=? AND reply_state='claimed' AND EXISTS (SELECT 1 FROM reply_notices WHERE batch_id=? AND request_id=? AND attempted=1)",params![outcome.as_str(),request_id,id,request_id])
                .map_err(|error|classify(error,"Settle reply frame"))?;
            db.execute(
                "DELETE FROM reply_notices WHERE batch_id=? AND request_id=? AND attempted=1",
                params![id, request_id],
            )
            .map_err(|error| classify(error, "Remove settled reply frame"))?;
            Ok(())
        })
    }

    /// Keep untouched driver frames queued; completed shells need no worker.
    pub fn finish_reply_notice_batch(&mut self, id: &str) -> Result<(), StorageError> {
        self.connection()?.execute("DELETE FROM reply_notice_batches WHERE id=? AND NOT EXISTS (SELECT 1 FROM reply_notices WHERE batch_id=?)",params![id,id])
            .map_err(|error|classify(error,"Remove completed reply batch"))?;
        Ok(())
    }

    pub fn settle_reply_notice_batch(
        &mut self,
        id: &str,
        outcome: WakeState,
    ) -> Result<(), StorageError> {
        validate_outcome(outcome)?;
        with_immediate_transaction(
            self,
            "reply batch settlement",
            |db| -> Result<(), StorageError> {
                db.execute("UPDATE request_notifications SET reply_state=? WHERE reply_state='claimed' AND request_id IN (SELECT n.request_id FROM reply_notices n JOIN reply_notice_batches b ON b.id=n.batch_id WHERE b.id=? AND b.sending=1 AND n.attempted=1)",params![outcome.as_str(),id])
                .map_err(|e|classify(e,"Settle reply notice members"))?;
                db.execute(
                    "DELETE FROM reply_notices WHERE batch_id=? AND attempted=1",
                    [id],
                )
                .map_err(|e| classify(e, "Remove settled reply batch"))?;
                Ok(())
            },
        )?;
        self.finish_reply_notice_batch(id)
    }
}

/// A dead worker cannot prove attempted frames absent; never replay them.
fn retire_attempted(db: &Connection, id: &str) -> Result<(), StorageError> {
    db.execute("UPDATE request_notifications SET reply_state='uncertain' WHERE reply_state='claimed' AND request_id IN (SELECT request_id FROM reply_notices WHERE batch_id=? AND attempted=1)", [id])
        .map_err(|error| classify(error, "Retire dead worker notice claims"))?;
    db.execute(
        "DELETE FROM reply_notices WHERE batch_id=? AND attempted=1",
        [id],
    )
    .map_err(|error| classify(error, "Remove terminal worker notice claims"))?;
    Ok(())
}

fn validate_outcome(outcome: WakeState) -> Result<(), StorageError> {
    if matches!(
        outcome,
        WakeState::Sent | WakeState::Unavailable | WakeState::Uncertain
    ) {
        Ok(())
    } else {
        Err(StorageError::new(
            StorageErrorCode::Unknown,
            "Invalid reply notice outcome",
        ))
    }
}
