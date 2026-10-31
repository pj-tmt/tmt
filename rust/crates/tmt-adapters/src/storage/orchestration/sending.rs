//! Sending rules and talk limits inside the request preparation transaction (#4892).
//! The rules are `tmt_core::orchestration::sending`; this reads their facts at the
//! transaction's captured instant and records holds. No counter table: the pair count is
//! read from the stored requests.
use super::{ReceivingCalendar, StorageError, append, classify, corrupt, read_snapshot};
use rusqlite::{Connection, OptionalExtension, params};
use tmt_core::orchestration::{sending::*, *};

impl ReceivingCalendar {
    pub(in crate::storage) fn talk_admission(
        &self,
        db: &Connection,
        from: &str,
        to: &str,
        now: u64,
    ) -> Result<OrchTalkAdmission, StorageError> {
        let stored = read_snapshot(db, None, None)?;
        if stored.snapshot.revision == 0 {
            // A fresh store is Base only: anyone sends, unlimited.
            return Ok(OrchTalkAdmission::Allowed);
        }
        let zone = self
            .zone
            .as_ref()
            .filter(|_| self.configured_zone == stored.time_zone)
            .ok_or_else(|| {
                StorageError::new(
                    super::StorageErrorCode::Busy,
                    "Orchestration calendar selection changed; retry the operation.",
                )
            })?;
        let sender = effective(&stored.snapshot, from, now, zone).map_err(|_| corrupt())?;
        let leads = active_leads(db)?;
        let grant = db
            .query_row(
                "SELECT until_ms FROM orchestration_route_grants WHERE from_id=? AND to_id=?",
                params![from, to],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e| classify(e, "Read route grant"))?
            .map(|until| u64::try_from(until).map_err(|_| corrupt()))
            .transpose()?;
        if let OrchRoute::Restricted { sending, source } =
            route(&sender.sending, leads.iter().any(|id| id == to), grant, now)
        {
            return Ok(OrchTalkAdmission::Restricted(Box::new(OrchRouteRefusal {
                sender_id: from.into(),
                recipient_id: to.into(),
                sending,
                source,
                configuration_revision: stored.snapshot.revision,
                time_zone: zone.name().into(),
            })));
        }
        // A lead's requests are never counted or held.
        if leads.iter().any(|id| id == from) {
            return Ok(OrchTalkAdmission::Allowed);
        }
        let recipient = effective(&stored.snapshot, to, now, zone).map_err(|_| corrupt())?;
        let Some(limit) = pair_limit(sender.talk_limit.value, recipient.talk_limit.value) else {
            return Ok(OrchTalkAdmission::Allowed);
        };
        let start = talk_window_start(now, resumed_at(db, from, to)?);
        let counted = pair_count(db, from, to, start)?;
        Ok(if needs_person(counted, Some(limit)) {
            OrchTalkAdmission::NeedsPerson { counted, limit }
        } else {
            OrchTalkAdmission::Allowed
        })
    }

    /// Record the hold and its decision in the caller's transaction.
    pub(in crate::storage) fn hold_for_person(
        &self,
        db: &Connection,
        request_id: &str,
        from: &str,
        to: &str,
        now: u64,
    ) -> Result<(), StorageError> {
        db.execute(
            "INSERT INTO orchestration_talk_holds (request_id,from_id,to_id,held_ms) VALUES (?,?,?,?)",
            params![request_id, from, to, now as i64],
        )
        .map_err(|e| classify(e, "Hold request for a person"))?;
        let stored = read_snapshot(db, None, None)?;
        let zone = self.zone.as_ref().ok_or_else(corrupt)?;
        let source = effective(&stored.snapshot, from, now, zone)
            .map_err(|_| corrupt())?
            .talk_limit
            .source;
        append(
            db,
            &OrchDecision {
                recorded_at_ms: now,
                subject_kind: OrchSubjectKind::Request,
                subject_id: request_id.into(),
                actor_id: Some(from.into()),
                recipient_id: Some(to.into()),
                action: OrchDecisionAction::Delay,
                outcome: OrchDecisionOutcome::Held,
                reason: "talk_limit".into(),
                configuration_revision: stored.snapshot.revision,
                time_zone: zone.name().into(),
                sources: vec![OrchDecisionSource {
                    field: OrchPolicyField::TalkLimit,
                    source,
                }],
            },
        )?;
        Ok(())
    }
}

/// Stored leads whose identities are still active; a retired one drops out here, not by
/// rewriting the list.
pub(in crate::storage) fn active_leads(db: &Connection) -> Result<Vec<String>, StorageError> {
    let mut query = db
        .prepare(
            "SELECT l.identity_id FROM orchestration_leads l JOIN identities i ON i.id=l.identity_id \
             WHERE i.retired_at_ms IS NULL ORDER BY l.identity_id",
        )
        .map_err(|e| classify(e, "Prepare leads"))?;
    let rows = query
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| classify(e, "Read leads"))?;
    rows.map(|r| r.map_err(|e| classify(e, "Read lead")))
        .collect()
}

fn ordered<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
    if a < b { (a, b) } else { (b, a) }
}

fn resumed_at(db: &Connection, a: &str, b: &str) -> Result<Option<u64>, StorageError> {
    let (first, second) = ordered(a, b);
    db.query_row(
        "SELECT resumed_ms FROM orchestration_talk_resumes WHERE first_id=? AND second_id=?",
        params![first, second],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map_err(|e| classify(e, "Read talk resume"))?
    .map(|ms| u64::try_from(ms).map_err(|_| corrupt()))
    .transpose()
}

/// New requests between the two, both directions, prepared since `start`. Held requests
/// and requests from a current lead are not counted.
fn pair_count(db: &Connection, a: &str, b: &str, start: u64) -> Result<u64, StorageError> {
    db.query_row(
        "SELECT COUNT(*) FROM request_attempts r \
         WHERE r.request_kind='request' AND r.prepared_at_ms >= ?1 \
         AND ((r.originator_identity_id=?2 AND r.recipient_identity_id=?3) \
           OR (r.originator_identity_id=?3 AND r.recipient_identity_id=?2)) \
         AND NOT EXISTS (SELECT 1 FROM orchestration_talk_holds h WHERE h.request_id=r.request_id) \
         AND NOT EXISTS (SELECT 1 FROM orchestration_leads l JOIN identities i ON i.id=l.identity_id \
           WHERE l.identity_id=r.originator_identity_id AND i.retired_at_ms IS NULL)",
        params![start as i64, a, b],
        |r| r.get::<_, i64>(0),
    )
    .map_err(|e| classify(e, "Count pair requests"))
    .and_then(|n| u64::try_from(n).map_err(|_| corrupt()))
}
