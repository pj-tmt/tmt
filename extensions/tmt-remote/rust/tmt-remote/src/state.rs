//! Remote's private subtree `<dataRoot>/remote/`, relocated from the colab keyring:
//! an owned 0700 directory, owned 0600 regular files opened without following
//! symlinks, create-only machine key publication and one foreground serve lock.
//! No core database, configuration or provider setting is touched.
use crate::error::RemoteError;
use ed25519_dalek::SigningKey;
use nix::{
    fcntl::{Flock, FlockArg, OFlag},
    unistd::Uid,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// Private file names; anything else is refused.
const FILES: [&str; 4] = ["machine.key", "key.lock", "serve.lock", "remote.db"];

fn unsafe_directory() -> RemoteError {
    RemoteError::new(
        "REMOTE_STATE_UNSAFE",
        "Remote state must be an owned 0700 directory, not a symlink.",
    )
}
fn unsafe_file() -> RemoteError {
    RemoteError::new(
        "REMOTE_STATE_UNSAFE",
        "Remote state files must be owned regular 0600 files.",
    )
}
fn invalid_key() -> RemoteError {
    RemoteError::new(
        "REMOTE_KEY_INVALID",
        "Invalid machine key length; the key was not replaced.",
    )
}
fn io(error: impl std::fmt::Display) -> RemoteError {
    RemoteError::new("REMOTE_IO", &format!("Remote state I/O failed: {error}."))
}

pub struct Layout {
    pub directory: PathBuf,
}
impl Layout {
    pub fn open(data_root: &Path) -> Result<Self, RemoteError> {
        if !data_root.is_absolute() {
            return Err(RemoteError::new(
                "REMOTE_ROOT_INVALID",
                "Data root must be absolute.",
            ));
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(data_root)
            .map_err(io)?;
        // Only the trusted core-selected root may contain aliases such as macOS /var.
        let data_root = fs::canonicalize(data_root).map_err(io)?;
        let directory = data_root.join("remote");
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => File::open(&data_root)
                .and_then(|root| root.sync_all())
                .map_err(io)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io(e)),
        }
        let metadata = fs::symlink_metadata(&directory).map_err(io)?;
        if !metadata.is_dir()
            || metadata.uid() != Uid::effective().as_raw()
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(unsafe_directory());
        }
        Ok(Self { directory })
    }
    pub fn file(&self, name: &str) -> Result<File, RemoteError> {
        if !FILES.contains(&name) {
            return Err(RemoteError::new(
                "REMOTE_STATE_NAME_INVALID",
                "Invalid private file name.",
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(self.directory.join(name))
            .map_err(|e| {
                if e.raw_os_error() == Some(nix::libc::ELOOP) {
                    unsafe_file()
                } else {
                    io(e)
                }
            })?;
        validate_file(&file)?;
        Ok(file)
    }
    /// One foreground remote per data root; held for the life of `serve`.
    /// The returned [`Serving`] is the only way to open remote state, so a
    /// second process cannot open the database while serve runs.
    pub fn serve_lock(&self) -> Result<Serving, RemoteError> {
        let lock = self.file("serve.lock")?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => {
                RemoteError::new("REMOTE_ALREADY_SERVING", "Remote is already serving.")
            }
            std::fs::TryLockError::Error(error) => io(error),
        })?;
        Ok(Serving {
            layout: Layout {
                directory: self.directory.clone(),
            },
            _lock: lock,
        })
    }
}
/// Proof that this process holds the data root's serve lock. Serve is the only
/// writer and opener of remote state while it runs; every other path (pairing,
/// device management) reaches that state through serve's control socket.
pub struct Serving {
    layout: Layout,
    _lock: File,
}
impl Serving {
    /// Keep the lease until the last actual invocation child closes it, even
    /// after owner death or unconfirmed cleanup. Closing never explicitly unlocks.
    pub fn retain_for_invocations(&self) -> Result<(), RemoteError> {
        nix::fcntl::fcntl(
            &self._lock,
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
        )
        .map_err(io)?;
        Ok(())
    }
    pub fn layout(&self) -> &Layout {
        &self.layout
    }
}
fn validate_file(file: &File) -> Result<(), RemoteError> {
    let m = file.metadata().map_err(io)?;
    if !m.is_file() || m.uid() != Uid::effective().as_raw() || m.mode() & 0o777 != 0o600 {
        return Err(unsafe_file());
    }
    Ok(())
}

/// The machine's long-term Ed25519 key, a software file with no hardware claim.
pub struct MachineKey {
    key: SigningKey,
}
impl MachineKey {
    pub fn open(layout: &Layout) -> Result<Self, RemoteError> {
        // Publication and cleanup share this lock; never remove an active writer's file.
        let _publication = Flock::lock(layout.file("key.lock")?, FlockArg::LockExclusiveNonblock)
            .map_err(|(_, e)| {
            if e == nix::errno::Errno::EWOULDBLOCK {
                RemoteError::new(
                    "REMOTE_KEY_BUSY",
                    "Machine key publication is already in progress.",
                )
            } else {
                io(e)
            }
        })?;
        cleanup_temporaries(layout)?;
        let destination = layout.directory.join("machine.key");
        if !destination.try_exists().map_err(io)? {
            let mut seed = [0; 32];
            getrandom::fill(&mut seed)
                .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain key entropy."))?;
            let mut name = [0; 16];
            getrandom::fill(&mut name)
                .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain key entropy."))?;
            let temporary = layout.directory.join(format!(".machine-{}", hex(&name)));
            let result = (|| -> std::io::Result<()> {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temporary)?;
                file.write_all(&seed)?;
                file.sync_all()?;
                // Create-only: a concurrent publisher's key wins and ours is discarded.
                match fs::hard_link(&temporary, &destination) {
                    Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => Err(e),
                    _ => Ok(()),
                }
            })();
            seed.fill(0);
            let removed = fs::remove_file(&temporary);
            File::open(&layout.directory)
                .and_then(|d| d.sync_all())
                .map_err(io)?;
            result.map_err(io)?;
            removed.map_err(io)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(destination)
            .map_err(|_| unsafe_file())?;
        validate_file(&file)?;
        let mut seed = Vec::new();
        file.take(33).read_to_end(&mut seed).map_err(io)?;
        let mut bytes: [u8; 32] = seed.as_slice().try_into().map_err(|_| invalid_key())?;
        let key = SigningKey::from_bytes(&bytes);
        bytes.fill(0);
        seed.fill(0);
        Ok(Self { key })
    }
    pub fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
    /// Sign machine-authored bytes, such as a response envelope's signature input.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;
        self.key.sign(message).to_bytes()
    }
}
fn cleanup_temporaries(layout: &Layout) -> Result<(), RemoteError> {
    let mut removed = false;
    for entry in fs::read_dir(&layout.directory).map_err(io)? {
        let entry = entry.map_err(io)?;
        let name = entry.file_name();
        if !name
            .to_str()
            .and_then(|s| s.strip_prefix(".machine-"))
            .is_some_and(|s| {
                s.len() == 32
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            continue;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(entry.path())
            .map_err(|_| unsafe_file())?;
        validate_file(&file)?;
        if file.metadata().map_err(io)?.len() > 32 {
            return Err(invalid_key());
        }
        fs::remove_file(entry.path()).map_err(io)?;
        removed = true;
    }
    if removed {
        File::open(&layout.directory)
            .and_then(|d| d.sync_all())
            .map_err(io)?;
    }
    Ok(())
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod lease_tests {
    use super::*;
    #[test]
    fn closing_owner_preserves_a_child_copy_until_its_final_close() {
        let root = std::path::PathBuf::from(format!(
            "/tmp/t1055-lease-{}",
            crate::store::uuid_v4().unwrap()
        ));
        let layout = Layout::open(&root).unwrap();
        let serving = layout.serve_lock().unwrap();
        serving.retain_for_invocations().unwrap();
        let child_copy = serving._lock.try_clone().unwrap();
        drop(serving);
        assert!(matches!(layout.serve_lock(), Err(error) if error.code=="REMOTE_ALREADY_SERVING"));
        drop(child_copy);
        drop(layout.serve_lock().unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
