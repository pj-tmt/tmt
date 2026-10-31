//! Explicit immutable-file import. Core never opens the Ops database.
use super::*;
use crate::orchestration::jobs::JobScheduleDocument;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tmt_core::content_digest;
const LIMIT: usize = 16 * 1024 * 1024;
const DOMAIN: &[u8] = b"tmt:orchestration:legacy-jobs:v1\0";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDocument {
    version: u32,
    counters: BTreeMap<String, u64>,
    jobs: Vec<LegacyEntry>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyEntry {
    id: String,
    room_id: String,
    owner_id: Option<String>,
    message: String,
    schedule: JobScheduleDocument,
    revision: u64,
    state: String,
    pause: Option<LegacyPause>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyPause {
    by: String,
    at_ms: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobImportReference {
    pub room_id: String,
    pub cron_id: u64,
    pub job_id: String,
    pub display_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobImportReceipt {
    pub source_id: String,
    pub source_digest: String,
    pub jobs: Vec<JobImportReference>,
    pub counters: BTreeMap<String, u64>,
}
struct Prepared {
    jobs: Vec<OrchJob>,
    counters: BTreeMap<String, u64>,
    digest: String,
}
fn prepare(bytes: &[u8]) -> Result<Prepared, JobStoreError> {
    if bytes.len() > LIMIT {
        return Err(JobError::Limit.into());
    }
    let document: LegacyDocument = serde_json::from_slice(bytes).map_err(|_| JobError::Invalid)?;
    if document.version != 2
        || document
            .counters
            .iter()
            .any(|(room, n)| !canonical_id(room) || *n > MAX_JS_SAFE_INTEGER)
    {
        return Err(JobError::Invalid.into());
    }
    let mut keys = BTreeSet::new();
    let mut jobs = Vec::with_capacity(document.jobs.len());
    for entry in document.jobs {
        let id = entry
            .id
            .strip_prefix('c')
            .and_then(|id| id.parse::<u64>().ok())
            .filter(|id| *id > 0 && *id <= MAX_JS_SAFE_INTEGER)
            .ok_or(JobError::Invalid)?;
        if !keys.insert((entry.room_id.clone(), id))
            || document
                .counters
                .get(&entry.room_id)
                .is_none_or(|counter| *counter < id)
        {
            return Err(JobError::Invalid.into());
        }
        let state = if entry.owner_id.is_none() {
            "no owner"
        } else if entry.pause.is_some() {
            "paused"
        } else {
            "on"
        };
        if state != entry.state {
            return Err(JobError::Invalid.into());
        }
        let job = OrchJob {
            id: uuid::Uuid::new_v4().to_string(),
            number: 1,
            revision: entry.revision,
            owner_id: entry.owner_id,
            room_id: Some(entry.room_id.clone()),
            title: None,
            message: entry.message,
            schedule: entry.schedule.into(),
            pause: entry.pause.map(|p| JobPause {
                by: p.by,
                at_ms: p.at_ms,
            }),
            pending: None,
            legacy: Some(LegacyJob {
                room_id: entry.room_id,
                cron_id: id,
            }),
        };
        job.validate()?;
        crate::orchestration::jobs::calendar(&job.schedule)?;
        jobs.push(job);
    }
    Ok(Prepared {
        jobs,
        counters: document.counters,
        digest: content_digest::framed_sha256(DOMAIN, bytes),
    })
}
/// Validation/preflight has no storage side effect. The caller freezes this
/// exact digest before handing the immutable file to the import owner.
pub fn preview_job_import(bytes: &[u8]) -> Result<(String, usize), JobStoreError> {
    let prepared = prepare(bytes)?;
    Ok((prepared.digest, prepared.jobs.len()))
}
fn receipt(db: &Connection, source_id: &str) -> Result<Option<JobImportReceipt>, JobStoreError> {
    let row:Option<(String,String)>=db.query_row("SELECT source_digest,CASE WHEN length(CAST(receipt AS BLOB))<=16777216 THEN receipt END FROM orchestration_job_imports WHERE source_id=?",[source_id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|error|classify(error,"Read job import receipt"))?;
    row.map(|(digest, json)| {
        let receipt: JobImportReceipt = serde_json::from_str(&json).map_err(|_| corrupt())?;
        if receipt.source_id != source_id
            || receipt.source_digest != digest
            || !content_digest::is_sha256(&digest)
        {
            return Err(corrupt().into());
        }
        Ok(receipt)
    })
    .transpose()
}
impl Storage {
    pub fn orchestration_job_import_receipt(
        &self,
        source_id: &str,
    ) -> Result<Option<JobImportReceipt>, JobStoreError> {
        if !canonical_id(source_id) {
            return Err(JobError::Invalid.into());
        }
        receipt(self.connection()?, source_id)
    }
    /// Definitions, legacy counters/mappings, and the immutable receipt have a
    /// single Core linearization point. A lost response is recovered by source
    /// ID; a different document can never replace that accepted import.
    pub fn import_orchestration_jobs(
        &mut self,
        source_id: &str,
        expected_digest: &str,
        bytes: &[u8],
        now_ms: i64,
    ) -> Result<JobImportReceipt, JobStoreError> {
        if !canonical_id(source_id)
            || !content_digest::is_sha256(expected_digest)
            || now_ms.unsigned_abs() > MAX_JS_SAFE_INTEGER
        {
            return Err(JobError::Invalid.into());
        }
        let prepared = prepare(bytes)?;
        if prepared.digest != expected_digest {
            return Err(JobError::Conflict.into());
        }
        let tx = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| classify(error, "Begin job import"))?;
        if let Some(previous) = receipt(&tx, source_id)? {
            if previous.source_digest != expected_digest {
                return Err(JobError::Conflict.into());
            }
            return Ok(previous);
        }
        // A room/c-id mapping and counter have one adoption owner. Another
        // source cannot merge or overwrite a partially corresponding inventory.
        for room in prepared.counters.keys() {
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM orchestration_job_legacy_counters WHERE room_id=?)",[room],|row|row.get(0)).map_err(|error|classify(error,"Fence imported job room"))?;
            if exists {
                return Err(JobError::Conflict.into());
            }
        }
        let mut references = Vec::with_capacity(prepared.jobs.len());
        for mut job in prepared.jobs {
            if let Some(owner) = job.owner_id.clone()
                && !owner_exists(&tx, Some(&owner))?
            {
                job.owner_id = None;
                job.pause.get_or_insert(JobPause {
                    by: owner,
                    at_ms: now_ms,
                });
            }
            let job = insert_job(&tx, job)?;
            let legacy = job.legacy.ok_or_else(corrupt)?;
            references.push(JobImportReference {
                room_id: legacy.room_id,
                cron_id: legacy.cron_id,
                job_id: job.id,
                display_id: format!("j{}", job.number),
            });
        }
        for (room, counter) in &prepared.counters {
            tx.execute(
                "INSERT INTO orchestration_job_legacy_counters(room_id,last_id) VALUES(?,?)",
                params![room, *counter as i64],
            )
            .map_err(|error| classify(error, "Retain imported job counter"))?;
        }
        let accepted = JobImportReceipt {
            source_id: source_id.into(),
            source_digest: prepared.digest,
            jobs: references,
            counters: prepared.counters,
        };
        let encoded = serde_json::to_string(&accepted).map_err(|_| JobError::Invalid)?;
        if encoded.len() > LIMIT {
            return Err(JobError::Limit.into());
        }
        tx.execute(
            "INSERT INTO orchestration_job_imports(source_id,source_digest,receipt) VALUES(?,?,?)",
            params![source_id, expected_digest, encoded],
        )
        .map_err(|error| classify(error, "Publish job import receipt"))?;
        advance_configuration(&tx)?;
        tx.commit()
            .map_err(|error| classify(error, "Commit job import"))?;
        Ok(accepted)
    }
}
