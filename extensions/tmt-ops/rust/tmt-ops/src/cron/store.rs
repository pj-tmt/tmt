mod cutover;
pub use cutover::FrozenJobs;

use super::{Error, Schedule};
use crate::state::{BUSY_RETRY as STATE_RETRY, Cause, Database, Refusal, Slot, StateFailure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Write,
    os::unix::fs::OpenOptionsExt,
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
    pub fn new(room_id: String, owner_id: String, message: String, schedule: Schedule) -> Self {
        Self {
            id: 0,
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
        json!({"id":self.id(),"roomId":self.room_id,"ownerId":self.owner_id,"message":self.message,"schedule":self.schedule.document(),"revision":self.revision,"state":self.state(),"pause":self.pause.as_ref().map(|p| json!({"by":p.by,"atMs":p.at_ms}))})
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
        if self.room_id.is_empty()
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

struct LegacyJobs {
    counters: BTreeMap<String, u64>,
    jobs: Vec<(String, Job)>,
}

impl Jobs {
    /// Convert jobs using their frozen room IDs. Counter-only names use the
    /// public room inventory; unresolved names without jobs are discarded.
    pub fn migrate_v1(value: &Value, rooms: &[(String, String)]) -> Result<Value, Error> {
        let (document, conflicts) = Self::convert_v1(value, rooms)?;
        if conflicts {
            return Err(corrupt(
                "Conflicting legacy timers require a retained export.",
            ));
        }
        Ok(document)
    }

    fn convert_v1(value: &Value, rooms: &[(String, String)]) -> Result<(Value, bool), Error> {
        let legacy = Self::from_v1_document(value)?;
        let mut counters = BTreeMap::<String, u64>::new();
        let mut name_rooms = BTreeMap::<String, std::collections::BTreeSet<String>>::new();
        for (name, job) in &legacy.jobs {
            name_rooms
                .entry(name.clone())
                .or_default()
                .insert(job.room_id.clone());
        }
        for (name, counter) in &legacy.counters {
            let mapped = if let Some(owned) = name_rooms.get(name) {
                owned.clone()
            } else {
                let found = rooms
                    .iter()
                    .filter(|(n, _)| n == name)
                    .map(|(_, id)| id.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                if found.len() == 1 {
                    found
                } else {
                    Default::default()
                }
            };
            for room in mapped {
                let entry = counters.entry(room).or_default();
                *entry = (*entry).max(*counter);
            }
        }
        let mut occurrences = BTreeMap::<(String, u64), usize>::new();
        for (_, job) in &legacy.jobs {
            *occurrences
                .entry((job.room_id.clone(), job.id))
                .or_default() += 1;
        }
        let mut conflicts = false;
        let mut jobs = Vec::new();
        for (name, job) in &legacy.jobs {
            if name_rooms[name].len() != 1 || occurrences[&(job.room_id.clone(), job.id)] != 1 {
                conflicts = true;
            } else {
                jobs.push(job.document());
            }
        }
        Ok((
            json!({"version": 2, "counters": counters, "jobs": jobs}),
            conflicts,
        ))
    }

    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }
    pub fn find_mut(&mut self, squad: &str, id: &str) -> Option<&mut Job> {
        self.jobs
            .iter_mut()
            .find(|j| j.room_id == squad && j.id() == id)
    }
    pub fn remove(&mut self, squad: &str, id: &str) -> Option<Job> {
        let index = self
            .jobs
            .iter()
            .position(|j| j.room_id == squad && j.id() == id)?;
        Some(self.jobs.remove(index))
    }
    pub fn insert(&mut self, mut job: Job) -> Result<String, Error> {
        job.validate()?;
        let counter = self.counters.entry(job.room_id.clone()).or_default();
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

    pub fn document(&self) -> Value {
        json!({"version":2,"counters":self.counters,"jobs":self.jobs.iter().map(Job::document).collect::<Vec<_>>()})
    }
    fn from_document(value: &Value) -> Result<Self, Error> {
        if value["version"] != 2 {
            if value["version"].as_u64().is_some_and(|version| version > 2) {
                return Err(Error::new(
                    "SQUAD_CRON_STORE_NEWER",
                    "Cron timers were written by a newer Ops. Upgrade Ops, then retry.",
                ));
            }
            return Err(corrupt("Unsupported cron store version."));
        }
        let jobs = Self::parse_document(value)?;
        jobs.validate()?;
        Ok(jobs)
    }

    fn from_v1_document(value: &Value) -> Result<LegacyJobs, Error> {
        if value["version"] != 1 {
            return Err(corrupt("Unsupported cron store version."));
        }
        let parsed = Self::parse_document(value)?;
        let entries = value["jobs"]
            .as_array()
            .ok_or_else(|| corrupt("Missing jobs."))?;
        let mut keys = std::collections::BTreeSet::new();
        let mut jobs = Vec::with_capacity(parsed.jobs.len());
        for (value, job) in entries.iter().zip(parsed.jobs) {
            let squad = string(value, "squad")?;
            if squad.is_empty()
                || job.id == 0
                || parsed.counters.get(&squad).is_none_or(|n| *n < job.id)
                || !keys.insert((squad.clone(), job.id))
            {
                return Err(corrupt("Duplicate job or invalid counter."));
            }
            jobs.push((squad, job));
        }
        Ok(LegacyJobs {
            counters: parsed.counters,
            jobs,
        })
    }

    fn parse_document(value: &Value) -> Result<Self, Error> {
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
        Ok(Self { counters, jobs })
    }
    fn validate(&self) -> Result<(), Error> {
        if self
            .counters
            .keys()
            .any(|room| !crate::operation::uuid_like(room))
        {
            return Err(corrupt("Cron counter keys must be room UUIDs."));
        }
        let mut keys = std::collections::BTreeSet::new();
        for job in &self.jobs {
            job.validate()?;
            if job.id == 0
                || self.counters.get(&job.room_id).is_none_or(|n| *n < job.id)
                || !keys.insert((&job.room_id, job.id))
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

    /// Inspect the current format without creating an absent store.
    pub fn needs_v1_migration(&self) -> Result<bool, Error> {
        let slot = self.slot();
        let Some(mut database) =
            Database::open_existing(&self.root, &slot.legacy).map_err(refused)?
        else {
            return Ok(false);
        };
        database
            .inspect(&slot, |bytes| {
                let Some(bytes) = bytes else {
                    return Ok(false);
                };
                let value: Value =
                    serde_json::from_slice(bytes).map_err(|error| corrupt(error.to_string()))?;
                match value["version"].as_u64() {
                    Some(1) => Ok(true),
                    Some(2) => {
                        Jobs::from_document(&value)?;
                        Ok(false)
                    }
                    Some(version) if version > 2 => Err(Error::new(
                        "SQUAD_CRON_STORE_NEWER",
                        "Cron timers were written by a newer Ops. Upgrade Ops, then retry.",
                    )),
                    _ => Err(corrupt("Unsupported cron store version.")),
                }
            })
            .map_err(failed)
    }

    /// Convert the currently stored legacy snapshot, if any. The room inventory
    /// is a public observation acquired by the caller before the transaction;
    /// the document itself is read again inside the write transaction.
    pub fn migrate_v1(&self, rooms: &[(String, String)]) -> Result<(), Error> {
        let slot = self.slot();
        let Some(mut database) =
            Database::open_existing(&self.root, &slot.legacy).map_err(refused)?
        else {
            return Ok(());
        };
        database
            .update(
                &slot,
                |bytes| {
                    let Some(bytes) = bytes else {
                        return Ok((None, None));
                    };
                    let value: Value = serde_json::from_slice(bytes)
                        .map_err(|error| corrupt(error.to_string()))?;
                    if value["version"] == 2 {
                        Jobs::from_document(&value)?;
                        return Ok((None, None));
                    }
                    let (converted, conflicts) = Jobs::convert_v1(&value, rooms)?;
                    Jobs::from_document(&converted)?;
                    let export = if conflicts { Some(self.export_conflicts(bytes)?) } else { None };
                    let bytes = serde_json::to_vec(&converted).map_err(io)?;
                    if bytes.len() as u64 > LIMIT {
                        return Err(corrupt("Cron store exceeds 16 MiB."));
                    }
                    Ok((Some(bytes), export))
                },
                |export, transaction| {
                    if let Some(path) = export {
                        let notice = json!({"path":path, "cli":true, "board":true});
                        let body = serde_json::to_vec(&notice).map_err(io)?;
                        transaction.execute(
                            "INSERT INTO documents(kind, key, body) VALUES('cron-migration-notice', '', ?1) ON CONFLICT(kind, key) DO UPDATE SET body = excluded.body",
                            [&body],
                        ).map_err(io)?;
                    }
                    Ok(())
                },
            )
            .map(|_| ())
            .map_err(failed)
    }

    /// Retain the entire original document before the database transaction publishes v2.
    fn export_conflicts(&self, bytes: &[u8]) -> Result<String, Error> {
        let layout =
            tmt_extension_state::Layout::open(&self.root, "cron", &[]).map_err(|error| {
                io(format!(
                    "Cannot create private cron export directory: {error:?}"
                ))
            })?;
        let stamp = jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .strftime("%Y%m%dT%H%M%SZ")
            .to_string();
        for attempt in 0u32.. {
            let path = layout
                .directory
                .join(format!("jobs-conflict-{stamp}-{attempt}.json"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits())
                .open(&path)
            {
                Ok(mut file) => {
                    let result = file.write_all(bytes).and_then(|_| file.sync_all());
                    if let Err(error) = result {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        let _ = layout.sync();
                        return Err(io(error));
                    }
                    layout.sync().map_err(io)?;
                    return Ok(path.to_string_lossy().into_owned());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io(error)),
            }
        }
        unreachable!()
    }
    /// Claim one delivery channel independently; the clock never claims notices.
    pub fn take_migration_notice(&self, board: bool) -> Result<Option<String>, Error> {
        let slot = Slot {
            kind: "cron-migration-notice",
            key: String::new(),
            legacy: self.root.join("cron").join("no-legacy-notice"),
            limit: 4096,
        };
        let Some(mut database) =
            Database::open_existing(&self.root, &slot.legacy).map_err(refused)?
        else {
            return Ok(None);
        };
        let channel = if board { "board" } else { "cli" };
        let pending = database
            .inspect(&slot, |bytes| -> Result<bool, Error> {
                let Some(bytes) = bytes else {
                    return Ok(false);
                };
                let value: Value = serde_json::from_slice(bytes).map_err(io)?;
                Ok(value[channel] == true)
            })
            .map_err(failed)?;
        if !pending {
            return Ok(None);
        }
        database
            .update(
                &slot,
                |bytes| {
                    let Some(bytes) = bytes else {
                        return Ok((None, None));
                    };
                    let mut value: Value = serde_json::from_slice(bytes).map_err(io)?;
                    let channel = if board { "board" } else { "cli" };
                    if value[channel] != true {
                        return Ok((None, None));
                    }
                    let path = value["path"]
                        .as_str()
                        .ok_or_else(|| corrupt("Invalid migration notice."))?
                        .to_owned();
                    value[channel] = json!(false);
                    Ok((Some(serde_json::to_vec(&value).map_err(io)?), Some(path)))
                },
                |_, _| Ok(()),
            )
            .map_err(failed)
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
