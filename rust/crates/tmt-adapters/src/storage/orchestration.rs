//! Configuration snapshots/publication and the transaction-local decision journal.
mod receiving;
mod sending;
mod write;
use super::{Storage, StorageError, StorageErrorCode, errors::classify};
use crate::orchestration::wire::{self, OrchPlanDocument, OrchSubjectDocument};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
pub use sending::active_leads;
pub use sending::route_refusal_decision;
pub(super) use sending::{append_route_refusal, held_for_person};
use serde_json::{Value, json};
use tmt_core::{
    dispatch::canonical_id,
    limits::MAX_JS_SAFE_INTEGER,
    orchestration::{receiving::OrchReceivingPolicy, *},
};

pub const ORCH_JOURNAL_RETENTION_MS: u64 = 30 * 86_400_000;
pub struct OrchStoredState {
    pub snapshot: OrchSnapshot,
    pub time_zone: Option<String>,
}
/// Load calendar data before entering a request transaction. The configured
/// zone is checked again with the policy snapshot before it can grant admission.
pub(super) struct ReceivingCalendar {
    configured_zone: Option<String>,
    zone: Option<crate::orchestration::OrchZone>,
}
impl ReceivingCalendar {
    /// Sending rules (#4892) need the zone even before Digest publishes receiving.
    pub(super) fn load(db: &Connection) -> Result<Self, StorageError> {
        let configured_zone: Option<String> = db
            .query_row(
                "SELECT time_zone FROM orchestration_settings WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| classify(e, "Read receiving calendar selection"))?
            .flatten();
        // Resolve failures only matter if a non-person operation needs this
        // policy; loading must not remove the person's immunity.
        let zone = crate::orchestration::OrchZone::load(configured_zone.as_deref()).ok();
        Ok(Self {
            configured_zone,
            zone,
        })
    }
    pub(super) fn policy(
        &self,
        db: &Connection,
        identity: &str,
        now: u64,
    ) -> Result<Option<OrchReceivingPolicy>, StorageError> {
        let Some(window) =
            super::requests::receiving_window(db, identity)?.filter(|p| p.active(now))
        else {
            return Ok(None);
        };
        self.resolved_receiving(db, identity, now, Some(window))
    }
    /// Resolve the published policy independently of a live hold lease. Release
    /// admission needs this view after the lease ends; `held_until` still owns
    /// the finite-window rule and grants no hold without that lease.
    pub(super) fn resolved_receiving(
        &self,
        db: &Connection,
        identity: &str,
        now: u64,
        window: Option<tmt_core::request::digest::DigestPolicy>,
    ) -> Result<Option<OrchReceivingPolicy>, StorageError> {
        let stored = read_snapshot(db, None, Some(identity))?;
        if stored.snapshot.digest_revision == 0 {
            return Ok(None);
        }
        let zone = self
            .zone
            .as_ref()
            .filter(|_| self.configured_zone == stored.time_zone)
            .ok_or_else(|| {
                StorageError::new(
                    StorageErrorCode::Busy,
                    "Receiving calendar selection changed; retry the operation.",
                )
            })?;
        let resolved = effective(&stored.snapshot, identity, now, zone).map_err(|_| corrupt())?;
        Ok(Some(OrchReceivingPolicy {
            receiving: resolved.receiving,
            configuration_revision: stored.snapshot.revision,
            time_zone: zone.name().into(),
            next_change_at_ms: resolved.next_change_at_ms,
            legacy_window: window,
        }))
    }
}
pub struct OrchJournalPage {
    pub items: Vec<Value>,
    pub next_before: Option<u64>,
    pub retained_after_ms: u64,
}
fn corrupt() -> StorageError {
    StorageError::new(
        StorageErrorCode::Corrupt,
        "Orchestration storage contains invalid data",
    )
}
fn unsigned(v: i64) -> Result<u64, StorageError> {
    u64::try_from(v)
        .ok()
        .filter(|v| *v <= MAX_JS_SAFE_INTEGER)
        .ok_or_else(corrupt)
}
fn document<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, StorageError> {
    if s.len() > wire::ORCH_DOCUMENT_BYTES {
        return Err(corrupt());
    }
    serde_json::from_str(s).map_err(|_| corrupt())
}
fn text_rows(c: &Connection, sql: &str, key: &str) -> Result<Vec<String>, StorageError> {
    let mut q = c
        .prepare(sql)
        .map_err(|e| classify(e, "Prepare orchestration rows"))?;
    q.query_map([key], |r| r.get(0))
        .map_err(|e| classify(e, "Read orchestration rows"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| classify(e, "Decode orchestration rows"))
}
fn read_snapshot(
    tx: &Connection,
    member_query: Option<&str>,
    identity_id: Option<&str>,
) -> Result<OrchStoredState, StorageError> {
    let normalized = member_query.map(tmt_core::names::normalize_name);
    let member_query = normalized.as_deref();
    let (revision, time_zone): (i64, Option<String>) = tx
        .query_row(
            "SELECT revision,time_zone FROM orchestration_settings WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| classify(e, "Read orchestration settings"))?
        .unwrap_or((0, None));
    let mut snapshot = OrchSnapshot {
        revision: unsigned(revision)?,
        ..Default::default()
    };
    let mut q=tx.prepare("SELECT id,name FROM identities WHERE retired_at_ms IS NULL AND (?1 IS NULL OR instr(canonical_name,?1)>0) AND (?2 IS NULL OR id=?2) ORDER BY CASE WHEN ?1 IS NOT NULL AND substr(canonical_name,1,length(?1))=?1 THEN 0 ELSE 1 END,canonical_name COLLATE BINARY LIMIT 257").map_err(|e|classify(e,"Select orchestration identities"))?;
    snapshot.identities = q
        .query_map(params![member_query, identity_id], |r| {
            Ok(OrchIdentity {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })
        .map_err(|e| classify(e, "Read orchestration identities"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| classify(e, "Decode orchestration identities"))?;
    drop(q);
    let mut q=tx.prepare("SELECT id,name,revision,priority,enabled,CASE WHEN length(CAST(schedule AS BLOB))<=32768 THEN schedule END,CASE WHEN length(CAST(fields AS BLOB))<=32768 THEN fields END FROM orchestration_plans ORDER BY priority LIMIT 129").map_err(|e|classify(e,"Select orchestration plans"))?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i32>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(|e| classify(e, "Read orchestration plans"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| classify(e, "Decode orchestration plans"))?;
    drop(q);
    for (id, name, revision, priority, enabled, schedule, fields) in rows {
        let targets=text_rows(tx,"SELECT target FROM orchestration_plan_targets WHERE plan_id=? ORDER BY position LIMIT 257",&id)?.iter().map(|s|document(s)).collect::<Result<Vec<_>,_>>()?;
        let mut q=tx.prepare("SELECT identity_id,fields FROM orchestration_member_overrides WHERE plan_id=? ORDER BY identity_id LIMIT 257").map_err(|e|classify(e,"Select orchestration member fields"))?;
        let member_rows = q
            .query_map([&id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| classify(e, "Read orchestration member fields"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| classify(e, "Decode orchestration member fields"))?;
        let members = member_rows
            .iter()
            .map(|(id, fields)| {
                Ok(wire::OrchMemberDocument {
                    identity_id: id.clone(),
                    fields: document(fields)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        let plan = OrchPlanDocument {
            id,
            name,
            revision: unsigned(revision)?,
            priority,
            enabled,
            schedule: document(&schedule)?,
            fields: document(&fields)?,
            targets,
            members,
        };
        snapshot.plans.push(plan.to_core().map_err(|_| corrupt())?);
    }
    let mut q=tx.prepare("SELECT f.identity_id,f.revision,f.start_ms,f.end_ms FROM orchestration_focus f JOIN identities i ON i.id=f.identity_id WHERE i.retired_at_ms IS NULL AND (?1 IS NULL OR instr(i.canonical_name,?1)>0) AND (?2 IS NULL OR f.identity_id=?2) ORDER BY f.identity_id LIMIT 257").map_err(|e|classify(e,"Select personal focus"))?;
    let rows = q
        .query_map(params![member_query, identity_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| classify(e, "Read personal focus"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| classify(e, "Decode personal focus"))?;
    for (identity_id, revision, start_ms, end_ms) in rows {
        let f = OrchFocus {
            identity_id,
            revision: unsigned(revision)?,
            start_ms: unsigned(start_ms)?,
            end_ms: unsigned(end_ms)?,
        };
        if !canonical_id(&f.identity_id)
            || f.revision == 0
            || f.start_ms == 0
            || f.start_ms >= f.end_ms
        {
            return Err(corrupt());
        }
        snapshot.focus.push(f);
    }
    drop(q);
    // Leads and grants are whole-store configuration: never narrowed by a member query.
    snapshot.leads = text_rows(
        tx,
        "SELECT identity_id FROM orchestration_leads WHERE ?1=?1 ORDER BY identity_id LIMIT 257",
        "",
    )?;
    let mut q = tx
        .prepare("SELECT from_id,to_id,until_ms,granted_by FROM orchestration_route_grants ORDER BY from_id,to_id LIMIT 4097")
        .map_err(|e| classify(e, "Select route grants"))?;
    snapshot.route_grants = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| classify(e, "Read route grants"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| classify(e, "Decode route grants"))?
        .into_iter()
        .map(|(from_id, to_id, until_ms, granted_by)| {
            Ok(tmt_core::orchestration::sending::OrchRouteGrant {
                from_id,
                to_id,
                until_ms: unsigned(until_ms)?,
                granted_by,
            })
        })
        .collect::<Result<_, StorageError>>()?;
    drop(q);
    if let Some((revision, settings)) = receiving::read(tx)? {
        settings.apply(&mut snapshot, revision);
    }
    Ok(OrchStoredState {
        snapshot,
        time_zone,
    })
}
impl Storage {
    pub fn orchestration_snapshot(
        &self,
        member_query: Option<&str>,
    ) -> Result<OrchStoredState, StorageError> {
        let tx = self
            .connection()?
            .unchecked_transaction()
            .map_err(|e| classify(e, "Read orchestration snapshot"))?;
        let snapshot = read_snapshot(&tx, member_query, None)?;
        tx.commit()
            .map_err(|e| classify(e, "Finish orchestration snapshot"))?;
        Ok(snapshot)
    }
    /// Future writers share `append` in their own transaction. This standalone
    /// owner is useful for non-request decisions and does not observe transport.
    pub fn append_orchestration_decision(
        &mut self,
        event: &OrchDecision,
    ) -> Result<u64, StorageError> {
        let tx = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| classify(e, "Begin orchestration decision"))?;
        let id = append(&tx, event)?;
        tx.commit()
            .map_err(|e| classify(e, "Commit orchestration decision"))?;
        Ok(id)
    }
    pub fn orchestration_journal(
        &self,
        subject: &OrchSubjectDocument,
        before: Option<u64>,
        limit: u16,
        now: u64,
    ) -> Result<OrchJournalPage, StorageError> {
        if !subject.valid()
            || !(1..=100).contains(&limit)
            || !wire::valid_ms(now)
            || before.is_some_and(|v| !wire::valid_ms(v))
        {
            return Err(corrupt());
        }
        let tx = self
            .connection()?
            .unchecked_transaction()
            .map_err(|e| classify(e, "Read orchestration journal"))?;
        let mut q=tx.prepare("SELECT id,recorded_ms,subject_kind,subject_id,actor_id,recipient_id,action,outcome,reason,configuration_revision,time_zone,sources FROM orchestration_decisions WHERE subject_kind=?1 AND subject_id=?2 AND id<?3 AND expires_ms>?4 ORDER BY id DESC LIMIT ?5").map_err(|e|classify(e,"Select orchestration journal"))?;
        let rows = q
            .query_map(
                params![
                    subject.kind,
                    subject.id,
                    before.unwrap_or(MAX_JS_SAFE_INTEGER) as i64,
                    now as i64,
                    i64::from(limit) + 1
                ],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, i64>(9)?,
                        r.get::<_, String>(10)?,
                        r.get::<_, String>(11)?,
                    ))
                },
            )
            .map_err(|e| classify(e, "Read orchestration journal rows"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| classify(e, "Decode orchestration journal rows"))?;
        let mut items = Vec::new();
        for (
            id,
            time,
            kind,
            subject,
            actor,
            recipient,
            action,
            outcome,
            reason,
            revision,
            zone,
            sources,
        ) in rows
        {
            let sources = wire::decode_sources(&sources).map_err(|_| corrupt())?;
            items.push(json!({"id":unsigned(id)?,"recordedAtMs":unsigned(time)?,"subject":{"kind":kind,"id":subject},"actorId":actor,"recipientId":recipient,"action":action,"outcome":outcome,"reason":reason,"configurationRevision":unsigned(revision)?,"timeZone":zone,"sources":sources}));
        }
        let more = items.len() > usize::from(limit);
        if more {
            items.pop();
        }
        let next_before = if more {
            items.last().and_then(|v| v["id"].as_u64())
        } else {
            None
        };
        drop(q);
        tx.commit()
            .map_err(|e| classify(e, "Finish orchestration journal read"))?;
        Ok(OrchJournalPage {
            items,
            next_before,
            retained_after_ms: now.saturating_sub(ORCH_JOURNAL_RETENTION_MS),
        })
    }
}

pub(super) fn append(c: &Connection, event: &OrchDecision) -> Result<u64, StorageError> {
    let kind = match event.subject_kind {
        OrchSubjectKind::Request => "request",
        OrchSubjectKind::Identity => "identity",
        OrchSubjectKind::Job => "job",
    };
    let subject = OrchSubjectDocument {
        kind: kind.into(),
        id: event.subject_id.clone(),
    };
    if !subject.valid()
        || !wire::valid_ms(event.recorded_at_ms)
        || event.configuration_revision > MAX_JS_SAFE_INTEGER
        || event.actor_id.as_deref().is_some_and(|v| !canonical_id(v))
        || event
            .recipient_id
            .as_deref()
            .is_some_and(|v| !canonical_id(v))
        || event.reason.trim().is_empty()
        || event.reason.len() > 256
        || event.reason.chars().any(char::is_control)
        || event.sources.len() > 5
        || event
            .sources
            .iter()
            .any(|source| !wire::source_valid(&source.source))
    {
        return Err(corrupt());
    }
    // Zone names are frozen evidence from the calendar owner. Do not load zone
    // files or reinterpret historical rules inside a caller's writer transaction.
    if event.time_zone.is_empty()
        || event.time_zone.len() > 128
        || event.time_zone.starts_with('/')
        || event.time_zone.ends_with('/')
        || event.time_zone.contains("//")
        || !event
            .time_zone
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_/-+".contains(&b))
        || event
            .sources
            .iter()
            .map(|s| s.field)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != event.sources.len()
    {
        return Err(corrupt());
    }
    let sources: Vec<_> = event
        .sources
        .iter()
        .map(wire::decision_source_value)
        .collect();
    let sources = serde_json::to_string(&sources).map_err(|_| corrupt())?;
    if sources.len() > 8192 {
        return Err(corrupt());
    }
    let expiry = event
        .recorded_at_ms
        .saturating_add(ORCH_JOURNAL_RETENTION_MS)
        .min(MAX_JS_SAFE_INTEGER);
    if expiry <= event.recorded_at_ms {
        return Err(corrupt());
    }
    let action = match event.action {
        OrchDecisionAction::Block => "block",
        OrchDecisionAction::Delay => "delay",
        OrchDecisionAction::Send => "send",
        OrchDecisionAction::JobRun => "jobRun",
    };
    let outcome = match event.outcome {
        OrchDecisionOutcome::Allowed => "allowed",
        OrchDecisionOutcome::Held => "held",
        OrchDecisionOutcome::Sent => "sent",
        OrchDecisionOutcome::Uncertain => "uncertain",
        OrchDecisionOutcome::Failed => "failed",
        OrchDecisionOutcome::Completed => "completed",
    };
    c.execute("DELETE FROM orchestration_decisions WHERE id IN (SELECT id FROM orchestration_decisions WHERE expires_ms<=? ORDER BY expires_ms,id LIMIT 500)",[event.recorded_at_ms as i64]).map_err(|e|classify(e,"Prune expired orchestration decisions"))?;
    c.execute("INSERT INTO orchestration_decisions(recorded_ms,expires_ms,subject_kind,subject_id,actor_id,recipient_id,action,outcome,reason,configuration_revision,time_zone,sources) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![event.recorded_at_ms as i64,expiry as i64,kind,event.subject_id,event.actor_id,event.recipient_id,action,outcome,event.reason,event.configuration_revision as i64,event.time_zone,sources]).map_err(|e|classify(e,"Append orchestration decision"))?;
    unsigned(c.last_insert_rowid())
}

#[cfg(test)]
mod tests;
