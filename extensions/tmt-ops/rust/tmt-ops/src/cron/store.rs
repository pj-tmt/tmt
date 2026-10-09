use super::{Error, Schedule};
use nix::fcntl::{Flock, FlockArg, OFlag};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 16 * 1024 * 1024;

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

/// A durable extension subtree. The caller supplies the invocation-selected Ops or pending legacy root;
/// merely constructing or reading an absent Store creates nothing.
pub struct Store {
    directory: PathBuf,
}

impl Store {
    pub fn new(extension_root: &Path) -> Result<Self, Error> {
        if !extension_root.is_absolute() {
            return Err(io("The selected extension root must be absolute."));
        }
        Ok(Self {
            directory: extension_root.join("cron"),
        })
    }
    fn lock(&self, create: bool) -> Result<Option<Flock<File>>, Error> {
        if create {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.directory)
                .map_err(io)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .mode(0o600)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(self.directory.join("jobs.lock"));
        let file = match file {
            Ok(file) => file,
            Err(e) if !create && e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        if !file.metadata().map_err(io)?.is_file() {
            return Err(io("Cron lock is not a regular file."));
        }
        // Nonblocking keeps both the CLI and future clock bounded.
        Flock::lock(file, FlockArg::LockExclusiveNonblock)
            .map(Some)
            .map_err(|(_, e)| Error::new("SQUAD_CRON_STORE_BUSY", e.to_string()))
    }
    fn read_locked(&self) -> Result<Jobs, Error> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(self.directory.join("jobs.json"));
        let file = match file {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Jobs::default()),
            Err(e) => return Err(io(e)),
        };
        if !file.metadata().map_err(io)?.is_file() {
            return Err(io("Cron store is not a regular file."));
        }
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(io)?;
        if bytes.len() as u64 > LIMIT {
            return Err(corrupt("Cron store exceeds 16 MiB."));
        }
        let value = serde_json::from_slice(&bytes).map_err(|e| corrupt(e.to_string()))?;
        Jobs::from_document(&value)
    }
    pub fn read(&self) -> Result<Jobs, Error> {
        let Some(_lock) = self.lock(false)? else {
            if self.directory.join("jobs.json").try_exists().map_err(io)? {
                return Err(corrupt("Cron store exists without its stable lock."));
            }
            return Ok(Jobs::default());
        };
        self.read_locked()
    }
    /// On closure/validation/write failure no partial job edit is published.
    /// A directory-sync error after rename reports an uncertain commit; reread.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut Jobs) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let _lock = self.lock(true)?.expect("create obtains a lock or fails");
        let mut jobs = self.read_locked()?;
        let before = jobs.document();
        let result = change(&mut jobs)?;
        jobs.validate()?;
        let document = jobs.document();
        if document == before {
            return Ok(result);
        }
        let bytes = serde_json::to_vec(&document).map_err(io)?;
        if bytes.len() as u64 > LIMIT {
            return Err(corrupt("Cron store exceeds 16 MiB."));
        }
        // The stable jobs.lock serializes writers, so a stale temporary belongs
        // to an interrupted writer, never another live publication.
        let temporary = self.directory.join("jobs.tmp");
        match fs::remove_file(&temporary) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        let publish = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, self.directory.join("jobs.json"))?;
            File::open(&self.directory)?.sync_all()
        })();
        if publish.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        publish.map_err(io)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
