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

pub struct OwnedServer {
    process: Process,
    files: Files,
    pub incarnation: ProcessIncarnation,
    pub endpoint: Endpoint,
    token: String,
}

impl OwnedServer {
    pub fn start(
        command: &RuntimeCommand,
        options: &LaunchOptions,
        directory: &Path,
        deadline: Instant,
    ) -> io::Result<Self> {
        let mut files = Files::create(directory)?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut capability = files.create_file("capability")?;
        capability.write_all(token.as_bytes())?;
        capability.sync_all()?;
        let mut log = files.create_file("server.log")?;
        let mut args = options.server_arguments().to_vec();
        args.extend([
            "app-server".into(),
            "--listen".into(),
            "ws://127.0.0.1:0".into(),
            "--ws-auth".into(),
            "capability-token".into(),
            "--ws-token-file".into(),
            directory.join("capability").into_os_string(),
        ]);
        let job = Exec::cmd(&command.executable)
            .args(&args)
            .cwd(options.working_directory())
            .env("NO_COLOR", "1")
            .env("RUST_LOG", "error")
            .setpgid()
            .stdin(Redirection::Null)
            .stdout(Redirection::Null)
            .stderr(log.try_clone()?)
            .start()?;
        let process = Process(Some(job));
        let address = wait_address(&mut log, deadline)?;
        let incarnation =
            match observe_runtime_process(&UnixCommandRunner, u64::from(process.pid()), deadline) {
                Ok(ProcessObservation::Live(incarnation)) => incarnation,
                _ => {
                    return Err(io::Error::other(
                        "Owned Codex app-server could not be observed",
                    ));
                }
            };
        let endpoint = Endpoint::new(address, token.clone())
            .map_err(|_| io::Error::other("Invalid owned Codex listener"))?;
        Ok(Self {
            process,
            files,
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
        self.process.stop()?;
        self.files.remove()
    }
}

impl Drop for OwnedServer {
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
