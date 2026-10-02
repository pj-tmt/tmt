//! A launch-owned app-server process group and its private capability/log files.
//! The provider binds an ephemeral loopback port and announces the actual port;
//! TMT never reserves/releases a port and races another listener for it.

use super::{attachment::LaunchOptions, transport::Endpoint};
use crate::{
    process::{
        UnixCommandRunner,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::RuntimeCommand,
};
use std::{
    fs::{self, File},
    io::{self, Write},
    net::{Ipv4Addr, SocketAddrV4},
    os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use subprocess::{Exec, ExecExt, Job, JobExt, Redirection};
use tmt_core::endpoint::ProcessIncarnation;

const STARTUP_LIMIT: u64 = 64 * 1024;
/// The files a launch creates in its generation directory, and nothing else.
pub(super) const CAPABILITY_FILE: &str = "capability";
pub(super) const LOG_FILE: &str = "server.log";

pub struct OwnedServer {
    resources: Resources,
    pub incarnation: ProcessIncarnation,
    pub endpoint: Endpoint,
    token: String,
}

/// Carries teardown certainty across the start boundary. Callers may withdraw
/// enrollment only when a failed start confirms cleanup.
#[derive(Debug)]
pub struct StartError {
    error: io::Error,
    cleanup_error: Option<io::Error>,
}
impl StartError {
    pub fn cleanup_confirmed(&self) -> bool {
        self.cleanup_error.is_none()
    }
    pub fn kind(&self) -> io::ErrorKind {
        self.error.kind()
    }
    pub(super) fn after_cleanup(error: io::Error, cleanup: io::Result<()>) -> Self {
        Self {
            error,
            cleanup_error: cleanup.err(),
        }
    }
}
impl From<io::Error> for StartError {
    fn from(error: io::Error) -> Self {
        Self::after_cleanup(error, Ok(()))
    }
}
impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)?;
        if let Some(error) = &self.cleanup_error {
            write!(f, "; cleanup unconfirmed: {error}")?;
        }
        Ok(())
    }
}
impl std::error::Error for StartError {}

impl OwnedServer {
    pub fn start(
        command: &RuntimeCommand,
        options: &LaunchOptions,
        directory: &Path,
        deadline: Instant,
    ) -> Result<Self, StartError> {
        Self::start_with_environment(command, options, directory, deadline, &[])
    }

    pub(super) fn start_with_environment(
        command: &RuntimeCommand,
        options: &LaunchOptions,
        directory: &Path,
        deadline: Instant,
        environment: &[(std::ffi::OsString, std::ffi::OsString)],
    ) -> Result<Self, StartError> {
        let mut files = Files::create(directory).map_err(|error| {
            StartError::after_cleanup(
                error,
                Err(io::Error::other("Generation ownership was not acquired")),
            )
        })?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut capability = files.create_file(CAPABILITY_FILE)?;
        capability.write_all(token.as_bytes())?;
        capability.sync_all()?;
        let mut log = files.create_file(LOG_FILE)?;
        let mut args = options.server_arguments().to_vec();
        args.extend([
            "app-server".into(),
            "--listen".into(),
            "ws://127.0.0.1:0".into(),
            "--ws-auth".into(),
            "capability-token".into(),
            "--ws-token-file".into(),
            directory.join(CAPABILITY_FILE).into_os_string(),
        ]);
        let mut launch = Exec::cmd(&command.executable)
            .args(&args)
            .cwd(options.working_directory())
            .env("NO_COLOR", "1")
            .env("RUST_LOG", "error")
            .setpgid()
            .stdin(Redirection::Null)
            .stdout(Redirection::Null)
            .stderr(log.try_clone()?);
        for (name, value) in environment {
            launch = launch.env(name, value);
        }
        let job = launch.start()?;
        let mut resources = Resources {
            process: Process(Some(job)),
            files,
        };
        let initialized = (|| -> io::Result<_> {
            let address = wait_address(&mut log, deadline)?;
            let incarnation = match observe_runtime_process(
                &UnixCommandRunner,
                u64::from(resources.process.pid()),
                deadline,
            ) {
                Ok(ProcessObservation::Live(incarnation)) => incarnation,
                _ => {
                    return Err(io::Error::other(
                        "Owned Codex app-server could not be observed",
                    ));
                }
            };
            let endpoint = Endpoint::new(address, token.clone())
                .map_err(|_| io::Error::other("Invalid owned Codex listener"))?;
            Ok((incarnation, endpoint))
        })();
        let (incarnation, endpoint) = match initialized {
            Ok(value) => value,
            Err(error) => return Err(StartError::after_cleanup(error, resources.stop())),
        };
        Ok(Self {
            resources,
            incarnation,
            endpoint,
            token,
        })
    }

    pub fn capability(&self) -> &str {
        &self.token
    }

    /// Kill the exact unreaped owned process group before any wait can recycle
    /// its leader PID. Cleanup never signals an observed or inherited group.
    pub fn stop(&mut self) -> io::Result<()> {
        self.resources.stop()
    }
}

// One owner orders process teardown before deleting its capability/diagnostics,
// including startup errors before OwnedServer can be returned.
struct Resources {
    process: Process,
    files: Files,
}
impl Resources {
    fn stop(&mut self) -> io::Result<()> {
        if self.files.retained {
            return Err(io::Error::other(
                "Codex files retained after unverified process cleanup",
            ));
        }
        if let Err(error) = self.process.stop() {
            self.files.retained = true;
            return Err(error);
        }
        self.files.remove()
    }
}
impl Drop for Resources {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("Codex channel cleanup failed: {error}");
        }
    }
}

struct Process(Option<Job>);
impl Process {
    fn pid(&self) -> u32 {
        self.0.as_ref().expect("owned server before stop").pid()
    }
    fn stop(&mut self) -> io::Result<()> {
        let Some(job) = self.0.take() else {
            return Ok(());
        };
        let signal = job.send_signal_group(nix::sys::signal::Signal::SIGKILL as i32);
        match job.wait_timeout(Duration::from_secs(2)) {
            Ok(Some(_)) => {
                if signal
                    .as_ref()
                    .is_err_and(|error| error.raw_os_error() != Some(nix::libc::ESRCH))
                {
                    return signal;
                }
                Ok(())
            }
            _ => {
                job.detach();
                Err(io::Error::other(
                    "Owned Codex app-server cleanup could not be verified",
                ))
            }
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("Codex process cleanup failed: {error}");
        }
    }
}

/// Only the created directory inode and created regular-file inodes are ours.
/// No recursive deletion and no deletion of a replacement generation.
struct Files {
    directory: PathBuf,
    identity: (u64, u64),
    files: Vec<(String, (u64, u64))>,
    removed: bool,
    retained: bool,
}
impl Files {
    fn create(directory: &Path) -> io::Result<Self> {
        fs::DirBuilder::new().mode(0o700).create(directory)?;
        let metadata = fs::symlink_metadata(directory)?;
        Ok(Self {
            directory: directory.into(),
            identity: identity(&metadata),
            files: Vec::new(),
            removed: false,
            retained: false,
        })
    }
    fn create_file(&mut self, name: &str) -> io::Result<File> {
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .read(true)
            .mode(0o600)
            .open(self.directory.join(name))?;
        self.files.push((name.into(), identity(&file.metadata()?)));
        Ok(file)
    }
    fn remove(&mut self) -> io::Result<()> {
        if self.removed {
            return Ok(());
        }
        let current = match fs::symlink_metadata(&self.directory) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.removed = true;
                return Ok(());
            }
            result => result?,
        };
        if !current.is_dir() || identity(&current) != self.identity {
            return Err(io::Error::other("Codex cleanup directory was replaced"));
        }
        for (name, expected) in &self.files {
            let path = self.directory.join(name);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Ok(metadata) if metadata.is_file() && identity(&metadata) == *expected => {
                    fs::remove_file(path)?
                }
                _ => return Err(io::Error::other("Codex cleanup file was replaced")),
            }
        }
        fs::remove_dir(&self.directory)?;
        self.removed = true;
        Ok(())
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        if self.retained {
            return;
        }
        if let Err(error) = self.remove() {
            eprintln!("Codex file cleanup failed: {error}");
        }
    }
}
fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

fn wait_address(log: &mut File, deadline: Instant) -> io::Result<SocketAddrV4> {
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Codex listener startup timed out",
            ));
        }
        let length = log.metadata()?.len();
        if length > STARTUP_LIMIT {
            return Err(io::Error::other("Codex startup output exceeded its bound"));
        }
        let mut bytes = vec![0; length as usize];
        let read = log.read_at(&mut bytes, 0)?;
        bytes.truncate(read);
        if let Some(address) = announced_address(&bytes) {
            return Ok(address);
        }
        thread::sleep(
            Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn announced_address(bytes: &[u8]) -> Option<SocketAddrV4> {
    let text = std::str::from_utf8(bytes).ok()?;
    for line in text.lines() {
        let Some(address) = line.trim().strip_prefix("listening on: ws://") else {
            continue;
        };
        let address: SocketAddrV4 = address.parse().ok()?;
        if *address.ip() == Ipv4Addr::LOCALHOST && address.port() != 0 {
            return Some(address);
        }
    }
    None
}

#[cfg(test)]
mod tests;
