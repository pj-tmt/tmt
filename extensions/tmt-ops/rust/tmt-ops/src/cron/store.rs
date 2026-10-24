use super::{Error, Schedule};
use crate::state::{BUSY_RETRY as STATE_RETRY, Cause, Database, Refusal, Slot, StateFailure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const LIMIT: u64 = 16 * 1024 * 1024;
/// Code of the one failure a waiter reports once its budget is spent.
pub const STORE_BUSY: &str = "SQUAD_CRON_STORE_BUSY";
/// The single next step for [`STORE_BUSY`]; the caller attaches it to the message.
pub const BUSY_RETRY: &str = STATE_RETRY;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pause {
    pub by: String,
    pub at_ms: i64,
}

/// References are core UUIDs admitted by the CLI. This owner never resolves
/// names, tests presence or substitutes message content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    id: u64,
    pub squad: String,
    pub room_id: String,
    pub owner_id: Option<String>,
    pub message: String,
    pub schedule: Schedule,
    pub revision: u64,
    pub pause: Option<Pause>,
}

fn corrupt(message: impl Into<String>) -> Error {
    Error::new("SQUAD_CRON_STORE_INVALID", message)
}
fn io(error: impl std::fmt::Display) -> Error {
    Error::new("SQUAD_CRON_STORE_IO", error.to_string())
}
fn string(value: &Value, key: &str) -> Result<String, Error> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| corrupt(format!("Missing job {key}.")))
}

impl Job {
    pub fn new(
        squad: String,
        room_id: String,
        owner_id: String,
        message: String,
        schedule: Schedule,
    ) -> Self {
        Self {
            id: 0,
            squad,
            room_id,
            owner_id: Some(owner_id),
            message,
            schedule,
            revision: 1,
            pause: None,
        }
    }
    pub fn id(&self) -> String {
        format!("c{}", self.id)
    }
    pub fn state(&self) -> &'static str {
        if self.owner_id.is_none() {
            "no owner"
        } else if self.pause.is_some() {
            "paused"
        } else {
            "on"
        }
    }
    pub fn document(&self) -> Value {
        json!({"id":self.id(),"squad":self.squad,"roomId":self.room_id,"ownerId":self.owner_id,"message":self.message,"schedule":self.schedule.document(),"revision":self.revision,"state":self.state(),"pause":self.pause.as_ref().map(|p| json!({"by":p.by,"atMs":p.at_ms}))})
    }
    fn from_document(value: &Value) -> Result<Self, Error> {
        let id = string(value, "id")?
            .strip_prefix('c')
            .and_then(|n| n.parse().ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| corrupt("Invalid job id."))?;
        let pause = if value["pause"].is_null() {
            None
        } else {
            Some(Pause {
                by: string(&value["pause"], "by")?,
                at_ms: value["pause"]["atMs"]
                    .as_i64()
                    .ok_or_else(|| corrupt("Invalid pause time."))?,
            })
        };
        let job = Self {
            id,
            squad: string(value, "squad")?,
            room_id: string(value, "roomId")?,
            owner_id: if value["ownerId"].is_null() {
                None
            } else {
                Some(string(value, "ownerId")?)
            },
            message: string(value, "message")?,
            schedule: Schedule::from_document(&value["schedule"])
                .map_err(|e| corrupt(e.to_string()))?,
            revision: value["revision"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or_else(|| corrupt("Invalid job revision."))?,
            pause,
        };
        if value["state"] != job.state() {
            return Err(corrupt("Job state disagrees with its owner/pause."));
        }
        job.validate()?;
        Ok(job)
    }
    fn validate(&self) -> Result<(), Error> {
        if self.squad.is_empty()
            || self.room_id.is_empty()
            || self.owner_id.as_ref().is_some_and(|id| id.is_empty())
            || self.revision == 0
            || (self.owner_id.is_none() && self.pause.is_none())
            || self.pause.as_ref().is_some_and(|p| p.by.is_empty())
        {
            return Err(corrupt("Invalid job ownership/revision."));
        }
        if self.message.len() > 1048576 {
            return Err(corrupt("Job messages are at most 1 MiB of UTF-8."));
        }
        Ok(())
    }
}

/// Counters survive removal of the last job; a human c-id is never reused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Jobs {
    counters: BTreeMap<String, u64>,
    jobs: Vec<Job>,
}

impl Jobs {
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }
    pub fn find_mut(&mut self, squad: &str, id: &str) -> Option<&mut Job> {
        self.jobs
            .iter_mut()
            .find(|j| j.squad == squad && j.id() == id)
    }
    pub fn remove(&mut self, squad: &str, id: &str) -> Option<Job> {
        let index = self
            .jobs
            .iter()
            .position(|j| j.squad == squad && j.id() == id)?;
        Some(self.jobs.remove(index))
    }
    pub fn insert(&mut self, mut job: Job) -> Result<String, Error> {
        job.validate()?;
        let counter = self.counters.entry(job.squad.clone()).or_default();
        let id = counter
            .checked_add(1)
            .ok_or_else(|| corrupt("Job id space exhausted."))?;
        job.id = id;
        *counter = id;
        self.jobs.push(job);
        Ok(format!("c{id}"))
    }
    /// Pause departure jobs in the same store transaction. A pre-existing pause
    /// retains its author/time; only explicit service Resume can clear it.
    pub fn pause_owner(
        &mut self,
        room_id: &str,
        identity_id: &str,
        now_ms: i64,
    ) -> Result<Vec<String>, Error> {
        let mut paused = Vec::new();
        for job in &mut self.jobs {
            if job.room_id != room_id || job.owner_id.as_deref() != Some(identity_id) {
                continue;
            }
            if job.pause.is_none() {
                job.revision = job
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| corrupt("Job revision space exhausted."))?;
                job.pause = Some(Pause {
                    by: identity_id.into(),
                    at_ms: now_ms,
                });
            }
            paused.push(job.id());
        }
        Ok(paused)
    }

    /// Called under Store::update after the room CAS. Retain deleted-job counters
    /// as well as live job IDs. A destination namespace is never silently merged.
    /// Repeating the completed step is a no-op for the explicit repair path.
    pub fn rename_squad(&mut self, old: &str, new: &str, room_id: &str) -> Result<bool, Error> {
        if old == new {
            return Ok(false);
        }
        if self
            .jobs
            .iter()
            .any(|job| job.squad == old && job.room_id != room_id)
        {
            return Err(Error::new(
                "SQUAD_CRON_ROOM_CONFLICT",
                "The old timer namespace belongs to a different room.",
            ));
        }
        if !self.counters.contains_key(old) && !self.jobs.iter().any(|job| job.squad == old) {
            return Ok(false);
        }
        if self.counters.contains_key(new) || self.jobs.iter().any(|job| job.squad == new) {
            return Err(Error::new(
                "SQUAD_CRON_NAMESPACE_CONFLICT",
                "The destination timer namespace already exists; choose a different squad name.",
            ));
        }
        if let Some(counter) = self.counters.remove(old) {
            self.counters.insert(new.into(), counter);
        }
        for job in &mut self.jobs {
            if job.squad == old {
                job.squad = new.into();
            }
        }
        Ok(true)
    }

    pub fn document(&self) -> Value {
        json!({"version":1,"counters":self.counters,"jobs":self.jobs.iter().map(Job::document).collect::<Vec<_>>()})
    }
    fn from_document(value: &Value) -> Result<Self, Error> {
        if value["version"] != 1 {
            return Err(corrupt("Unsupported cron store version."));
        }
        let counters = value["counters"]
            .as_object()
            .ok_or_else(|| corrupt("Missing job counters."))?
            .iter()
            .map(|(key, value)| {
                value
                    .as_u64()
                    .map(|n| (key.clone(), n))
                    .ok_or_else(|| corrupt("Invalid job counter."))
            })
            .collect::<Result<_, _>>()?;
        let jobs = value["jobs"]
            .as_array()
            .ok_or_else(|| corrupt("Missing jobs."))?
            .iter()
            .map(Job::from_document)
            .collect::<Result<_, _>>()?;
        let jobs = Self { counters, jobs };
        jobs.validate()?;
        Ok(jobs)
    }
    fn validate(&self) -> Result<(), Error> {
        let mut keys = std::collections::BTreeSet::new();
        for job in &self.jobs {
            job.validate()?;
            if job.id == 0
                || self.counters.get(&job.squad).is_none_or(|n| *n < job.id)
                || !keys.insert((&job.squad, job.id))
            {
                return Err(corrupt("Duplicate job or invalid counter."));
            }
        }
        Ok(())
    }
}

/// The cron jobs document in the Ops state database. The caller supplies the
/// invocation-selected Ops or pending legacy root; constructing or reading an absent Store
/// creates nothing. Admission, the write queue and commits belong to [`crate::state`].
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(extension_root: &Path) -> Result<Self, Error> {
        if !extension_root.is_absolute() {
            return Err(io("The selected extension root must be absolute."));
        }
        Ok(Self {
            root: extension_root.into(),
        })
    }
    fn slot(&self) -> Slot {
        Slot {
            kind: "cron",
            key: String::new(),
            legacy: self.root.join("cron").join("jobs.json"),
            limit: LIMIT,
        }
    }
    pub fn read(&self) -> Result<Jobs, Error> {
        let slot = self.slot();
        let Some(mut database) =
            Database::open_existing(&self.root, &slot.legacy).map_err(refused)?
        else {
            return Ok(Jobs::default());
        };
        database.read(&slot, decode).map_err(failed)
    }
    /// On closure/validation/write failure no partial job edit is saved.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut Jobs) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let slot = self.slot();
        let mut database = Database::open(&self.root).map_err(refused)?;
        database
            .update(
                &slot,
                |bytes| {
                    let mut jobs = decode(bytes)?;
                    let before = jobs.document();
                    let result = change(&mut jobs)?;
                    jobs.validate()?;
                    let document = jobs.document();
                    if document == before {
                        return Ok((None, result));
                    }
                    let bytes = serde_json::to_vec(&document).map_err(io)?;
                    if bytes.len() as u64 > LIMIT {
                        return Err(corrupt("Cron store exceeds 16 MiB."));
                    }
                    Ok((Some(bytes), result))
                },
                |_, _| Ok(()),
            )
            .map_err(failed)
    }

    /// Pause a departing owner's jobs, run `departure`, then pause once more. The first
    /// pause is committed before `departure` starts, so a departure that fails, or a
    /// process that dies during it, still leaves the jobs paused. Nothing is held across
    /// `departure` (it runs a core command), so a job its owner adds meanwhile is caught
    /// by the second pause, which is best effort: the departure is already done.
    pub fn pause_owner_then<U>(
        &self,
        room_id: &str,
        identity_id: &str,
        now_ms: i64,
        departure: impl FnOnce(&[String]) -> U,
    ) -> Result<(Vec<String>, U), Error> {
        let paused = self.update(|jobs| jobs.pause_owner(room_id, identity_id, now_ms))?;
        let effect = departure(&paused);
        let _ = self.update(|jobs| jobs.pause_owner(room_id, identity_id, now_ms));
        Ok((paused, effect))
    }
}

fn decode(bytes: Option<&[u8]>) -> Result<Jobs, Error> {
    let Some(bytes) = bytes else {
        return Ok(Jobs::default());
    };
    let value = serde_json::from_slice(bytes).map_err(|e| corrupt(e.to_string()))?;
    Jobs::from_document(&value)
}
fn refused(refusal: Refusal) -> Error {
    // A commit that timed out may have been saved: repeating the change could add it twice.
    if refusal.may_not_be_saved {
        return Error::new(
            "SQUAD_CRON_STORE_UNCERTAIN",
            format!(
                "{} List the jobs before repeating the change.",
                refusal.message
            ),
        );
    }
    let code = match refusal.cause {
        Cause::Busy => STORE_BUSY,
        Cause::Newer => "SQUAD_CRON_STORE_NEWER",
        Cause::Failed => "SQUAD_CRON_STORE_IO",
    };
    Error::new(code, refusal.message)
}
fn failed(failure: StateFailure<Error>) -> Error {
    match failure {
        StateFailure::Refused(refusal) => refused(refusal),
        StateFailure::Change(error) => error,
    }
}

#[cfg(test)]
mod tests;
