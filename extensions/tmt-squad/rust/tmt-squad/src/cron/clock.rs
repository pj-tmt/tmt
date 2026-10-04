use super::Error;
use nix::fcntl::{Flock, FlockArg, OFlag};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 4096;
const LEASE_MS: i64 = 30_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub pane: Option<String>,
    pub pid: u32,
    pub since_ms: i64,
    pub expires_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClockStatus {
    Running(Holder),
    NoClock,
    Unknown,
}

fn io(error: impl std::fmt::Display) -> Error {
    Error::new("SQUAD_CRON_CLOCK_IO", error.to_string())
}

fn invalid() -> Error {
    Error::new("SQUAD_CRON_CLOCK_INVALID", "Invalid clock lease.")
}

impl Holder {
    fn live(&self, now_ms: i64) -> bool {
        self.since_ms <= now_ms && now_ms < self.expires_ms
    }

    fn document(&self) -> Value {
        json!({"version":1,"pane":self.pane,"pid":self.pid,"sinceMs":self.since_ms,"expiresMs":self.expires_ms})
    }

    fn parse(value: &Value) -> Result<Self, Error> {
        let holder = Self {
            pane: if value["pane"].is_null() {
                None
            } else {
                Some(value["pane"].as_str().ok_or_else(invalid)?.into())
            },
            pid: value["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
                .filter(|pid| *pid > 0)
                .ok_or_else(invalid)?,
            since_ms: value["sinceMs"].as_i64().ok_or_else(invalid)?,
            expires_ms: value["expiresMs"].as_i64().ok_or_else(invalid)?,
        };
        if value["version"] != 1
            || holder.expires_ms <= holder.since_ms
            || holder.pane.as_ref().is_some_and(|pane| {
                pane.is_empty() || pane.len() > 128 || pane.chars().any(char::is_control)
            })
        {
            return Err(invalid());
        }
        Ok(holder)
    }
}

/// One extension-local lease for every squad. Status never creates or cleans files.
#[derive(Clone)]
pub struct Clock {
    directory: PathBuf,
}

impl Clock {
    pub fn new(data_root: &Path) -> Result<Self, Error> {
        if !data_root.is_absolute() {
            return Err(io("storage.root must return an absolute dataRoot."));
        }
        Ok(Self {
            directory: data_root.join("squad/cron"),
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
            .open(self.directory.join("clock.lock"));
        let file = match file {
            Ok(file) => file,
            Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(io(error)),
        };
        if !file.metadata().map_err(io)?.is_file() {
            return Err(invalid());
        }
        Flock::lock(file, FlockArg::LockExclusiveNonblock)
            .map(Some)
            .map_err(|(_, error)| Error::new("SQUAD_CRON_CLOCK_BUSY", error.to_string()))
    }

    fn read(&self) -> Result<Option<Holder>, Error> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(self.directory.join("clock.json"));
        let file = match file {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io(error)),
        };
        if !file.metadata().map_err(io)?.is_file() {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(io)?;
        if bytes.len() as u64 > LIMIT {
            return Err(invalid());
        }
        let value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        Holder::parse(&value).map(Some)
    }

    fn publish(&self, holder: &Holder) -> Result<(), Error> {
        let temporary = self.directory.join("clock.tmp");
        // The stable lock excludes every publisher, including an interrupted one.
        match fs::remove_file(&temporary) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io(error)),
        }
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(holder.document().to_string().as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, self.directory.join("clock.json"))?;
            File::open(&self.directory)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(io)
    }

    pub fn status(&self, now_ms: i64) -> ClockStatus {
        let read = (|| {
            let Some(_lock) = self.lock(false)? else {
                return if self.directory.join("clock.json").try_exists().map_err(io)? {
                    Err(invalid())
                } else {
                    Ok(None)
                };
            };
            self.read()
        })();
        match read {
            Ok(Some(holder)) if holder.live(now_ms) => ClockStatus::Running(holder),
            Ok(_) => ClockStatus::NoClock,
            Err(_) => ClockStatus::Unknown,
        }
    }

    /// Expired evidence (including a rollback before since) may be replaced.
    /// No PID probe is necessary: every sender renews before dispatching.
    pub fn acquire(
        &self,
        now_ms: i64,
        pid: u32,
        pane: Option<String>,
    ) -> Result<Option<Lease>, Error> {
        let _lock = self.lock(true)?;
        if self.read()?.is_some_and(|holder| holder.live(now_ms)) {
            return Ok(None);
        }
        let holder = Holder {
            pane,
            pid,
            since_ms: now_ms,
            expires_ms: now_ms.checked_add(LEASE_MS).ok_or_else(invalid)?,
        };
        Holder::parse(&holder.document())?;
        self.publish(&holder)?;
        Ok(Some(Lease {
            clock: self.clone(),
            holder,
        }))
    }
}

/// A lease is evidence, not a dispatch fence. Stable slot UUIDs own idempotency.
pub struct Lease {
    clock: Clock,
    holder: Holder,
}

impl Lease {
    pub fn holder(&self) -> &Holder {
        &self.holder
    }

    pub fn renew(&mut self, now_ms: i64) -> Result<bool, Error> {
        let _lock = self.clock.lock(false)?.ok_or_else(invalid)?;
        if !self.holder.live(now_ms) || self.clock.read()?.as_ref() != Some(&self.holder) {
            return Ok(false);
        }
        let mut next = self.holder.clone();
        next.expires_ms = now_ms.checked_add(LEASE_MS).ok_or_else(invalid)?;
        self.clock.publish(&next)?;
        self.holder = next;
        Ok(true)
    }

    pub fn release(self) -> Result<(), Error> {
        let Some(_lock) = self.clock.lock(false)? else {
            return Ok(());
        };
        if self.clock.read()?.as_ref() == Some(&self.holder) {
            fs::remove_file(self.clock.directory.join("clock.json")).map_err(io)?;
            File::open(&self.clock.directory)
                .and_then(|file| file.sync_all())
                .map_err(io)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
