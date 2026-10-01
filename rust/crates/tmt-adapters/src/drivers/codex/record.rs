//! Private per-binding opt-in and ready records, not an identity registry.
//! Every mutation is serialized and withdrawal compares the exact launch lease.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use tmt_core::{binding::session::RuntimeLiveness, endpoint::ProcessIncarnation};

const LIMIT: u64 = 8192;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Process {
    pub pid: u64,
    pub start: String,
}

impl Process {
    pub fn of(process: &ProcessIncarnation) -> Self {
        Self {
            pid: process.pid(),
            start: process.start_identity().to_owned(),
        }
    }
    pub fn incarnation(&self) -> Option<ProcessIncarnation> {
        ProcessIncarnation::new(self.pid, &self.start).ok()
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Ready {
    pub server: Process,
    pub port: u16,
    pub thread: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Record {
    pub version: u8,
    pub binding_id: String,
    pub generation: String,
    pub launch_owner: Process,
    pub ready: Option<Ready>,
}

impl Record {
    pub fn new(binding_id: &str, owner: &ProcessIncarnation) -> io::Result<Self> {
        uuid::Uuid::parse_str(binding_id).map_err(io::Error::other)?;
        Ok(Self {
            version: 1,
            binding_id: binding_id.into(),
            generation: uuid::Uuid::new_v4().to_string(),
            launch_owner: Process::of(owner),
            ready: None,
        })
    }

    fn same_lease(&self, other: &Self) -> bool {
        self.binding_id == other.binding_id
            && self.generation == other.generation
            && self.launch_owner == other.launch_owner
    }

    fn valid(&self, binding: &str) -> bool {
        self.version == 1
            && self.binding_id == binding
            && uuid::Uuid::parse_str(binding).is_ok()
            && uuid::Uuid::parse_str(&self.generation).is_ok()
            && self.launch_owner.incarnation().is_some()
            && self.ready.as_ref().is_none_or(|ready| {
                ready.port != 0
                    && ready.server.incarnation().is_some()
                    && uuid::Uuid::parse_str(&ready.thread).is_ok()
            })
    }
}

pub struct Store {
    directory: PathBuf,
}

impl Store {
    /// Existing private directories are verified, never chmodded into trust.
    pub fn open(channels: &Path) -> io::Result<Self> {
        if !channels.is_absolute() {
            return Err(invalid());
        }
        let directory = channels.join("codex");
        fs::create_dir_all(channels)?;
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(&directory)?;
        if !metadata.is_dir() || !private(&metadata) {
            return Err(invalid());
        }
        Ok(Self { directory })
    }

    pub fn at(channels: &Path) -> Self {
        Self {
            directory: channels.join("codex"),
        }
    }

    fn path(&self, binding: &str) -> io::Result<PathBuf> {
        uuid::Uuid::parse_str(binding).map_err(io::Error::other)?;
        Ok(self.directory.join(format!("{binding}.json")))
    }

    pub fn generation_directory(&self, record: &Record) -> io::Result<PathBuf> {
        if !record.valid(&record.binding_id) {
            return Err(invalid());
        }
        Ok(self.directory.join(&record.generation))
    }

    pub fn read(&self, binding: &str) -> io::Result<Option<Record>> {
        let path = self.path(binding)?;
        match fs::symlink_metadata(&self.directory) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Ok(metadata) if metadata.is_dir() && private(&metadata) => {}
            _ => return Err(invalid()),
        }
        let file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() || !private(&metadata) || metadata.len() > LIMIT {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(invalid());
        }
        let record: Record = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if !record.valid(binding) {
            return Err(invalid());
        }
        Ok(Some(record))
    }

    /// The launcher must first validate its claimed binding/session snapshot
    /// and its own live incarnation through the existing admission authority.
    /// Probe both incarnations again while holding the record lock. Old-owner
    /// absence alone never establishes authority for this new launch.
    pub fn create(
        &self,
        record: &Record,
        observe: impl Fn(&ProcessIncarnation) -> RuntimeLiveness,
    ) -> io::Result<()> {
        let _lock = self.lock(&record.binding_id)?;
        let owner = record.launch_owner.incarnation().ok_or_else(invalid)?;
        if observe(&owner) != RuntimeLiveness::Alive {
            return Err(invalid());
        }
        if let Some(previous) = self.read(&record.binding_id)? {
            if previous.same_lease(record) {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            let previous_owner = previous.launch_owner.incarnation().ok_or_else(invalid)?;
            if previous_owner != owner && observe(&previous_owner) != RuntimeLiveness::Gone {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
        }
        self.write(record)
    }

    pub fn ready(&self, expected: &Record, ready: Ready) -> io::Result<Record> {
        let _lock = self.lock(&expected.binding_id)?;
        let mut current = self.read(&expected.binding_id)?.ok_or_else(invalid)?;
        if !current.same_lease(expected) || current.ready.is_some() {
            return Err(invalid());
        }
        current.ready = Some(ready);
        self.write(&current)?;
        Ok(current)
    }

    pub fn withdraw(&self, expected: &Record) -> io::Result<bool> {
        let _lock = self.lock(&expected.binding_id)?;
        let Some(current) = self.read(&expected.binding_id)? else {
            return Ok(false);
        };
        if !current.same_lease(expected) {
            return Ok(false);
        }
        fs::remove_file(self.path(&expected.binding_id)?)?;
        fs::File::open(&self.directory)?.sync_all()?;
        Ok(true)
    }

    fn lock(&self, binding: &str) -> io::Result<nix::fcntl::Flock<fs::File>> {
        let path = self.path(binding)?.with_extension("lock");
        crate::file_lock::exclusive(&path)
    }

    fn write(&self, record: &Record) -> io::Result<()> {
        if !record.valid(&record.binding_id) {
            return Err(invalid());
        }
        crate::private_file::replace(
            &self.path(&record.binding_id)?,
            &serde_json::to_vec(record).map_err(io::Error::other)?,
        )
    }
}

fn private(metadata: &fs::Metadata) -> bool {
    metadata.uid() == nix::unistd::geteuid().as_raw() && metadata.mode() & 0o077 == 0
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid private Codex enrollment",
    )
}

#[cfg(test)]
mod tests;
