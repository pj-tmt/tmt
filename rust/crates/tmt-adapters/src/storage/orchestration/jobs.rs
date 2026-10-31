//! Canonical job definitions. Transport is never performed while this writer is held.
mod run;
pub use run::{JobRunInput, JobRunResult};

use super::{Storage, StorageError, StorageErrorCode, classify, unsigned};
use crate::orchestration::jobs::JobScheduleDocument as ScheduleDocument;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use tmt_core::{dispatch::canonical_id, limits::MAX_JS_SAFE_INTEGER, orchestration::jobs::*};

#[derive(Debug)]
pub enum JobStoreError {
    Job(JobError),
    Storage(StorageError),
    Dispatch(crate::storage::DispatchError),
}
impl From<StorageError> for JobStoreError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}
impl From<crate::storage::DispatchError> for JobStoreError {
    fn from(error: crate::storage::DispatchError) -> Self {
        Self::Dispatch(error)
    }
}
impl From<JobError> for JobStoreError {
    fn from(error: JobError) -> Self {
        Self::Job(error)
    }
}
impl std::fmt::Display for JobStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Job(error) => write!(f, "Job operation refused: {error:?}"),
            Self::Storage(error) => error.fmt(f),
            Self::Dispatch(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for JobStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Dispatch(error) => Some(error),
            Self::Job(_) => None,
        }
    }
}

fn corrupt() -> StorageError {
    StorageError::new(StorageErrorCode::Corrupt, "Invalid stored job definition")
}
fn encode_schedule(schedule: &JobSchedule) -> Result<String, JobStoreError> {
    let encoded =
        serde_json::to_string(&ScheduleDocument::from(schedule)).map_err(|_| corrupt())?;
    if encoded.len() > 4096 {
        return Err(JobError::Limit.into());
    }
    Ok(encoded)
}
fn decode_schedule(encoded: &str) -> Result<JobSchedule, StorageError> {
    if encoded.len() > 4096 {
        return Err(corrupt());
    }
    serde_json::from_str::<ScheduleDocument>(encoded)
        .map(Into::into)
        .map_err(|_| corrupt())
}
const COLUMNS: &str = "j.id,j.number,j.revision,j.owner_id,j.room_id,j.title,j.message,j.schedule,j.pause_by,j.pause_ms,j.pending_revision,j.pending_ms,l.room_id,l.cron_id";
fn decode_row(row: &Row<'_>, full_body: bool) -> rusqlite::Result<Result<OrchJob, StorageError>> {
    let number: i64 = row.get(1)?;
    let revision: i64 = row.get(2)?;
    let schedule: String = row.get(7)?;
    let pause_by: Option<String> = row.get(8)?;
    let pause_ms: Option<i64> = row.get(9)?;
    let pending_revision: Option<i64> = row.get(10)?;
    let pending_ms: Option<i64> = row.get(11)?;
    let legacy_room: Option<String> = row.get(12)?;
    let legacy_number: Option<i64> = row.get(13)?;
    // Decode SQL types first so malformed rows remain storage failures.
    let id = row.get(0)?;
    let owner_id = row.get(3)?;
    let room_id = row.get(4)?;
    let title = row.get(5)?;
    let message = row.get(6)?;
    Ok((|| {
        let job = OrchJob {
            id,
            number: unsigned(number)?,
            revision: unsigned(revision)?,
            owner_id,
            room_id,
            title,
            message,
            schedule: decode_schedule(&schedule)?,
            pause: match (pause_by, pause_ms) {
                (Some(by), Some(at_ms)) => Some(JobPause { by, at_ms }),
                (None, None) => None,
                _ => return Err(corrupt()),
            },
            pending: match (pending_revision, pending_ms) {
                (Some(revision), Some(at_ms)) => Some(HeldJobSlot {
                    revision: unsigned(revision)?,
                    at_ms,
                }),
                (None, None) => None,
                _ => return Err(corrupt()),
            },
            legacy: match (legacy_room, legacy_number) {
                (Some(room_id), Some(cron_id)) => Some(LegacyJob {
                    room_id,
                    cron_id: unsigned(cron_id)?,
                }),
                (None, None) => None,
                _ => return Err(corrupt()),
            },
        };
        if full_body {
            job.validate()
        } else {
            job.validate_metadata()
        }
        .map_err(|_| corrupt())?;
        Ok(job)
    })())
}
pub(super) fn read_job(db: &Connection, id: &str) -> Result<Option<OrchJob>, StorageError> {
    db.query_row(&format!("SELECT {COLUMNS} FROM orchestration_jobs j LEFT JOIN orchestration_job_legacy l ON l.job_id=j.id WHERE j.id=? AND j.removed=0"), [id], |row| decode_row(row, true))
        .optional().map_err(|error| classify(error, "Read job definition"))?.transpose()
}
fn owner_exists(db: &Connection, owner: Option<&str>) -> Result<bool, StorageError> {
    match owner {
        None => Ok(true),
        Some(id) => db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM identities WHERE id=? AND retired_at_ms IS NULL)",
                [id],
                |row| row.get(0),
            )
            .map_err(|error| classify(error, "Check job owner")),
    }
}

fn advance_configuration(db: &Connection) -> Result<(), JobStoreError> {
    let changed=db.execute("INSERT INTO orchestration_settings(id,revision,time_zone) VALUES(1,1,NULL) ON CONFLICT(id) DO UPDATE SET revision=revision+1 WHERE revision<9007199254740991",[])
        .map_err(|error|classify(error,"Advance job configuration revision"))?;
    if changed != 1 {
        return Err(JobError::Limit.into());
    }
    Ok(())
}

/// Bounded metadata only. Message bodies are loaded only by show/admission.
#[derive(Debug)]
pub struct JobListRow {
    pub id: String,
    pub number: u64,
    pub revision: u64,
    pub owner_id: Option<String>,
    pub owner_name: Option<String>,
    pub room_id: Option<String>,
    pub title: Option<String>,
    pub message_preview: String,
    pub schedule: JobSchedule,
    pub pause: Option<JobPause>,
    pub pending: Option<HeldJobSlot>,
    pub legacy: Option<LegacyJob>,
    pub owner_retired: bool,
}
#[derive(Default)]
pub struct JobListQuery<'a> {
    pub room_id: Option<&'a str>,
    pub owner_id: Option<&'a str>,
    pub job_id: Option<&'a str>,
    pub after_number: u64,
    pub limit: u16,
}
pub struct JobListPage {
    pub items: Vec<JobListRow>,
    pub next_after: Option<u64>,
}
impl Storage {
    pub fn orchestration_job(&self, id: &str) -> Result<Option<OrchJob>, JobStoreError> {
        if !canonical_id(id) {
            return Err(JobError::Invalid.into());
        }
        Ok(read_job(self.connection()?, id)?)
    }
    pub fn create_orchestration_job(
        &mut self,
        definition: JobDefinition,
    ) -> Result<OrchJob, JobStoreError> {
        let mut job = definition.with_identity(uuid::Uuid::new_v4().to_string(), 1);
        job.validate()?;
        crate::orchestration::jobs::calendar(&job.schedule)?;
        let encoded = encode_schedule(&job.schedule)?;
        let tx = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| classify(error, "Begin job creation"))?;
        if !owner_exists(&tx, job.owner_id.as_deref())? {
            return Err(JobError::NoOwner.into());
        }
        tx.execute("INSERT INTO orchestration_jobs(id,revision,owner_id,room_id,title,message,schedule,pause_by,pause_ms) VALUES(?1,1,?2,?3,?4,?5,?6,?7,?8)",
            params![job.id,job.owner_id,job.room_id,job.title,job.message,encoded,job.pause.as_ref().map(|p|&p.by),job.pause.as_ref().map(|p|p.at_ms)])
            .map_err(|error| classify(error, "Create job definition"))?;
        job.number = unsigned(tx.last_insert_rowid())?;
        advance_configuration(&tx)?;
        tx.commit()
            .map_err(|error| classify(error, "Commit job creation"))?;
        Ok(job)
    }
    /// The authority/preview owner executes its bounded mutation after the
    /// exact-row fence. No callback may perform transport or other external I/O.
    pub fn change_orchestration_job(
        &mut self,
        id: &str,
        revision: u64,
        change: impl FnOnce(&mut OrchJob) -> Result<(), JobStoreError>,
    ) -> Result<OrchJob, JobStoreError> {
        if !canonical_id(id) || revision == 0 || revision >= MAX_JS_SAFE_INTEGER {
            return Err(JobError::Invalid.into());
        }
        let tx = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| classify(error, "Begin job change"))?;
        let mut job = read_job(&tx, id)?.ok_or(JobError::NotFound)?;
        if job.revision != revision {
            return Err(JobError::Conflict.into());
        }
        let before = job.clone();
        change(&mut job)?;
        if job.id != before.id
            || job.number != before.number
            || job.legacy != before.legacy
            || job.revision != before.revision
            || job.pending != before.pending
        {
            return Err(JobError::Invalid.into());
        }
        if job == before {
            return Ok(job);
        }
        job.revision = revision + 1;
        job.pending = None; // A changed intent never inherits an older held slot.
        job.validate()?;
        if !owner_exists(&tx, job.owner_id.as_deref())? {
            return Err(JobError::NoOwner.into());
        }
        tx.execute("UPDATE orchestration_jobs SET revision=?2,owner_id=?3,room_id=?4,title=?5,message=?6,schedule=?7,pause_by=?8,pause_ms=?9,pending_revision=NULL,pending_ms=NULL WHERE id=?1",
            params![id,job.revision as i64,job.owner_id,job.room_id,job.title,job.message,encode_schedule(&job.schedule)?,job.pause.as_ref().map(|p|&p.by),job.pause.as_ref().map(|p|p.at_ms)])
            .map_err(|error| classify(error, "Change job definition"))?;
        advance_configuration(&tx)?;
        tx.commit()
            .map_err(|error| classify(error, "Commit job change"))?;
        Ok(job)
    }
    pub fn remove_orchestration_job(
        &mut self,
        id: &str,
        revision: u64,
    ) -> Result<(), JobStoreError> {
        if !canonical_id(id) || revision == 0 || revision >= MAX_JS_SAFE_INTEGER {
            return Err(JobError::Invalid.into());
        }
        let tx = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| classify(error, "Begin job removal"))?;
        let changed = tx.execute("UPDATE orchestration_jobs SET removed=1,revision=revision+1,pending_revision=NULL,pending_ms=NULL WHERE id=? AND revision=? AND removed=0",params![id,revision as i64])
            .map_err(|error| classify(error,"Remove job definition"))?;
        if changed != 1 {
            return Err(JobError::Conflict.into());
        }
        advance_configuration(&tx)?;
        tx.commit()
            .map_err(|error| classify(error, "Commit job removal"))?;
        Ok(())
    }
    pub fn list_orchestration_jobs(
        &self,
        query: JobListQuery<'_>,
    ) -> Result<JobListPage, JobStoreError> {
        if !(1..=128).contains(&query.limit)
            || query.after_number > MAX_JS_SAFE_INTEGER
            || [query.room_id, query.owner_id, query.job_id]
                .into_iter()
                .flatten()
                .any(|id| !canonical_id(id))
        {
            return Err(JobError::Invalid.into());
        }
        // Filtering precedes LIMIT. NULL room rows are included only without a
        // room filter; they never acquire association from present membership.
        let sql = format!(
            "SELECT {},i.name,i.retired_at_ms FROM orchestration_jobs j LEFT JOIN orchestration_job_legacy l ON l.job_id=j.id LEFT JOIN identities i ON i.id=j.owner_id WHERE j.removed=0 AND (?1 IS NULL OR j.room_id=?1) AND (?2 IS NULL OR j.owner_id=?2) AND (?3 IS NULL OR j.id=?3) AND j.number>?4 ORDER BY j.number LIMIT ?5",
            COLUMNS.replace("j.message,", "substr(j.message,1,256),")
        );
        let mut statement = self
            .connection()?
            .prepare(&sql)
            .map_err(|error| classify(error, "Prepare job page"))?;
        let rows = statement
            .query_map(
                params![
                    query.room_id,
                    query.owner_id,
                    query.job_id,
                    query.after_number as i64,
                    i64::from(query.limit) + 1
                ],
                |row| {
                    Ok((
                        decode_row(row, false)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, Option<i64>>(15)?,
                    ))
                },
            )
            .map_err(|error| classify(error, "Read job page"))?;
        let mut items = Vec::new();
        for row in rows {
            let (job, name, retired) = row.map_err(|error| classify(error, "Decode job page"))?;
            let job = job?;
            let mut preview = job.message;
            let mut end = preview.len().min(256);
            while !preview.is_char_boundary(end) {
                end -= 1;
            }
            preview.truncate(end);
            items.push(JobListRow {
                id: job.id,
                number: job.number,
                revision: job.revision,
                owner_id: job.owner_id,
                owner_name: if retired.is_some() { None } else { name },
                room_id: job.room_id,
                title: job.title,
                message_preview: preview,
                schedule: job.schedule,
                pause: job.pause,
                pending: job.pending,
                legacy: job.legacy,
                owner_retired: retired.is_some(),
            });
        }
        let next_after = if items.len() > usize::from(query.limit) {
            items.pop();
            items.last().map(|job| job.number)
        } else {
            None
        };
        Ok(JobListPage { items, next_after })
    }
}

#[cfg(test)]
mod tests;
