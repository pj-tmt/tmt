//! Private per-binding opt-in and ready records, not an identity registry.
//! Every mutation is serialized and withdrawal compares the exact launch lease.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Instant,
};
use tmt_core::{
    binding::session::RuntimeLiveness,
    endpoint::{ProcessIncarnation, ServerEvidence},
};

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

/// An owned fresh endpoint exists before its foreground creates a thread.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FreshEndpoint {
    pub server: Process,
    pub port: u16,
    pub cwd: PathBuf,
    pub thread: Option<String>,
}

/// Pre-spawn attribution copied from the launcher's claimed binding.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Attribution {
    pub identity_id: String,
    pub host: String,
    pub server_id: String,
    pub socket_path: String,
    pub server: Process,
    pub pane_id: String,
    pub pane_pid: u64,
}
impl Attribution {
    pub fn new(
        identity: &str,
        server: &ServerEvidence,
        pane_id: &str,
        pane_pid: u64,
    ) -> io::Result<Self> {
        let value = Self {
            identity_id: identity.into(),
            host: server.host.as_str().into(),
            server_id: server.server_id.clone(),
            socket_path: server.socket_path.clone(),
            server: Process {
                pid: server.server_pid,
                start: server.server_start_time.clone(),
            },
            pane_id: pane_id.into(),
            pane_pid,
        };
        if !value.valid() {
            return Err(invalid());
        }
        Ok(value)
    }
    fn valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.identity_id).is_ok()
            && tmt_core::endpoint::valid_server_id(&self.server_id)
            && tmt_core::host::HostKind::parse(&self.host).is_some()
            && self.server.incarnation().is_some()
            && !self.socket_path.is_empty()
            && self.socket_path.len() <= 4096
            && !self.socket_path.chars().any(char::is_control)
            && !self.pane_id.is_empty()
            && self.pane_id.len() <= 128
            && !self.pane_id.chars().any(char::is_control)
            && tmt_core::endpoint::valid_process_id(self.pane_pid)
    }
    pub fn matches(&self, server: &ServerEvidence, pane_id: &str, pane_pid: u64) -> bool {
        self.host == server.host.as_str()
            && self.server_id == server.server_id
            && self.socket_path == server.socket_path
            && self.server.pid == server.server_pid
            && self.server.start == server.server_start_time
            && self.pane_id == pane_id
            && self.pane_pid == pane_pid
    }
}

/// Unknown is durable before spawn; launcher death never proves child absence.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    content = "process",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Foreground {
    #[default]
    Unknown,
    Known(Process),
}
impl Foreground {
    fn valid(&self) -> bool {
        match self {
            Self::Known(process) => process.incarnation().is_some(),
            Self::Unknown => true,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    pub foreground: Foreground,
    #[serde(default)]
    pub attribution: Option<Attribution>,
    pub version: u8,
    pub binding_id: String,
    pub generation: String,
    pub launch_owner: Process,
    pub ready: Option<Ready>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fresh: Option<FreshEndpoint>,
}

impl Record {
    pub fn new(binding_id: &str, owner: &ProcessIncarnation) -> io::Result<Self> {
        uuid::Uuid::parse_str(binding_id).map_err(io::Error::other)?;
        Ok(Self {
            version: 1,
            foreground: Foreground::Unknown,
            attribution: None,
            binding_id: binding_id.into(),
            generation: uuid::Uuid::new_v4().to_string(),
            launch_owner: Process::of(owner),
            ready: None,
            fresh: None,
        })
    }

    /// A ready server can precede foreground spawn, so Unknown is never ended.
    pub fn ended(&self, observe: &impl Fn(&ProcessIncarnation) -> RuntimeLiveness) -> bool {
        let Foreground::Known(foreground) = &self.foreground else {
            return false;
        };
        let gone = |process: &Process| {
            process
                .incarnation()
                .is_some_and(|process| observe(&process) == RuntimeLiveness::Gone)
        };
        gone(foreground)
            && gone(&self.launch_owner)
            && self.ready.as_ref().is_none_or(|ready| gone(&ready.server))
            && self.fresh.as_ref().is_none_or(|fresh| gone(&fresh.server))
    }

    fn same_lease(&self, other: &Self) -> bool {
        self.binding_id == other.binding_id
            && self.generation == other.generation
            && self.launch_owner == other.launch_owner
    }

    fn valid(&self, binding: &str) -> bool {
        self.version == 1
            && self.foreground.valid()
            && self.attribution.as_ref().is_none_or(Attribution::valid)
            && self.binding_id == binding
            && uuid::Uuid::parse_str(binding).is_ok()
            && uuid::Uuid::parse_str(&self.generation).is_ok()
            && self.launch_owner.incarnation().is_some()
            && self.fresh.as_ref().is_none_or(|fresh| {
                fresh.port != 0
                    && fresh.server.incarnation().is_some()
                    && fresh.cwd.is_absolute()
                    && fresh
                        .thread
                        .as_ref()
                        .is_none_or(|id| uuid::Uuid::parse_str(id).is_ok())
                    && self.ready.as_ref().is_none_or(|ready| {
                        ready.server == fresh.server
                            && ready.port == fresh.port
                            && fresh.thread.as_ref() == Some(&ready.thread)
                    })
            })
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

/// What `Store::recover` did.
pub enum Removal<T> {
    /// No record is on file.
    Absent,
    /// The record on file is no longer the observed one; nothing was touched.
    Changed,
    /// `cleanup` could not remove this path, so the record was kept and the same
    /// recovery can run again.
    Retained(PathBuf),
    Removed(T),
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

    pub(super) fn path(&self, binding: &str) -> io::Result<PathBuf> {
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
            let process_gone = |process: &Process| {
                process
                    .incarnation()
                    .is_some_and(|process| observe(&process) == RuntimeLiveness::Gone)
            };
            let foreground_gone = match &previous.foreground {
                Foreground::Known(process) => process_gone(process),
                Foreground::Unknown => {
                    previous.attribution.is_some() && previous.attribution == record.attribution
                }
            };
            if !foreground_gone
                || previous
                    .ready
                    .as_ref()
                    .is_some_and(|ready| !process_gone(&ready.server))
                || previous
                    .fresh
                    .as_ref()
                    .is_some_and(|fresh| !process_gone(&fresh.server))
                || observe(&previous_owner) != RuntimeLiveness::Gone
            {
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

    pub fn fresh_endpoint(&self, expected: &Record, endpoint: FreshEndpoint) -> io::Result<Record> {
        let _lock = self.lock(&expected.binding_id)?;
        let mut current = self.read(&expected.binding_id)?.ok_or_else(invalid)?;
        if !current.same_lease(expected) || current.ready.is_some() || current.fresh.is_some() {
            return Err(invalid());
        }
        current.fresh = Some(endpoint);
        self.write(&current)?;
        Ok(current)
    }

    pub fn fresh_thread(&self, expected: &Record, thread: &str) -> io::Result<Record> {
        let _lock = self.lock(&expected.binding_id)?;
        let mut current = self.read(&expected.binding_id)?.ok_or_else(invalid)?;
        if !current.same_lease(expected) || current.foreground != expected.foreground {
            return Err(invalid());
        }
        let fresh = current.fresh.as_mut().ok_or_else(invalid)?;
        if fresh.thread.is_some() || current.ready.is_some() {
            return Err(invalid());
        }
        fresh.thread = Some(thread.into());
        self.write(&current)?;
        Ok(current)
    }

    /// Publication follows launcher admission; recheck every original process
    /// while holding the generation lock. Discovery never authorizes delivery.
    pub fn admit_fresh(
        &self,
        expected: &Record,
        foreground: &ProcessIncarnation,
        observe: impl Fn(&ProcessIncarnation) -> RuntimeLiveness,
    ) -> io::Result<Record> {
        let _lock = self.lock(&expected.binding_id)?;
        let mut current = self.read(&expected.binding_id)?.ok_or_else(invalid)?;
        let fresh = current.fresh.as_ref().ok_or_else(invalid)?;
        if !current.same_lease(expected)
            || current != *expected
            || current.foreground != Foreground::Known(Process::of(foreground))
            || current.ready.is_some()
            || [
                &current.launch_owner,
                &fresh.server,
                &Process::of(foreground),
            ]
            .iter()
            .any(|p| {
                p.incarnation()
                    .is_none_or(|p| observe(&p) != RuntimeLiveness::Alive)
            })
        {
            return Err(invalid());
        }
        current.ready = Some(Ready {
            server: fresh.server.clone(),
            port: fresh.port,
            thread: fresh.thread.clone().ok_or_else(invalid)?,
        });
        self.write(&current)?;
        Ok(current)
    }

    /// The launcher publishes only its original owned child's incarnation.
    pub fn foreground(
        &self,
        expected: &Record,
        process: &ProcessIncarnation,
    ) -> io::Result<Record> {
        let _lock = self.lock(&expected.binding_id)?;
        let mut current = self.read(&expected.binding_id)?.ok_or_else(invalid)?;
        if !current.same_lease(expected) || !matches!(current.foreground, Foreground::Unknown) {
            return Err(invalid());
        }
        current.foreground = Foreground::Known(Process::of(process));
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

    /// User-requested recovery of the exact `observed` record: under the record lock
    /// and only while the record on file is still exactly it, runs `cleanup` (for
    /// the generation directory) and then removes the record, unless `cleanup`
    /// names a path it could not remove: the record then stays. The caller has
    /// already proven every recorded process gone; an exact incarnation that is
    /// gone never returns, so an unchanged record keeps that proof valid.
    pub fn recover<T>(
        &self,
        observed: &Record,
        cleanup: impl FnOnce() -> Result<T, PathBuf>,
    ) -> io::Result<Removal<T>> {
        let _lock = self.lock(&observed.binding_id)?;
        match self.read(&observed.binding_id)? {
            None => return Ok(Removal::Absent),
            Some(current) if current != *observed => return Ok(Removal::Changed),
            Some(_) => {}
        }
        let cleaned = match cleanup() {
            Ok(cleaned) => cleaned,
            Err(path) => return Ok(Removal::Retained(path)),
        };
        fs::remove_file(self.path(&observed.binding_id)?)?;
        fs::File::open(&self.directory)?.sync_all()?;
        Ok(Removal::Removed(cleaned))
    }

    /// Only enrollment mutates ended records. Read-only pane queries never prune.
    /// Probe snapshots without excluding live startup publications. Lock only
    /// ended candidates, then compare the exact snapshot and prove ended again.
    /// Unknown, replacements and unreadable records remain untouched.
    pub fn prune(
        &self,
        deadline: Instant,
        observe: impl Fn(&ProcessIncarnation) -> RuntimeLiveness,
    ) -> io::Result<usize> {
        let mut removed = 0;
        for entry in fs::read_dir(&self.directory)? {
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Some(binding) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if uuid::Uuid::parse_str(binding).is_err() {
                continue;
            }
            let Ok(Some(candidate)) = self.read(binding) else {
                continue;
            };
            if !candidate.ended(&observe) {
                continue;
            }
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let Ok(_lock) = self.lock(binding) else {
                continue;
            };
            let Ok(Some(current)) = self.read(binding) else {
                continue;
            };
            if current != candidate || !current.ended(&observe) {
                continue;
            }
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            fs::remove_file(self.path(binding)?)?;
            removed += 1;
        }
        if removed != 0 {
            fs::File::open(&self.directory)?.sync_all()?;
        }
        Ok(removed)
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

/// This is guidance only. Delivery never executes recovery or pastes afterward;
/// `tmt channel recover` applies the contract's recovery rule.
pub fn recovery(path: &Path, record: &Record) -> String {
    let pane = record.attribution.as_ref().map_or_else(
        || "the original pane (attribution is unavailable)".to_owned(),
        |attribution| {
            format!(
                "pane {:?} (pid {}) on server {:?} at {:?}",
                attribution.pane_id,
                attribution.pane_pid,
                attribution.server_id,
                attribution.socket_path
            )
        },
    );
    let foreground = match &record.foreground {
        Foreground::Unknown => {
            "the original foreground (its incarnation was not published)".to_owned()
        }
        Foreground::Known(process) => format!(
            "the original foreground pid {} start {:?}",
            process.pid, process.start
        ),
    };
    format!(
        "Enrollment record {path:?}. Verify {pane} no longer runs the opted-in session, and that {foreground}, the launch owner and the owned app-server are gone. Only after verification, recover it with: {}. It refuses while a recorded process runs or cannot be observed, and sends or pastes nothing.",
        crate::runtime::channel::recover_command(&record.binding_id, &record.generation)
    )
}
pub(super) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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
