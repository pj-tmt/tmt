//! Remote's private subtree `<dataRoot>/remote/`, using the extension-state leaf:
//! an owned 0700 directory, owned 0600 regular files opened without following
//! symlinks, create-only machine key publication and one serving lifecycle lock.
//! No core database, configuration or provider setting is touched.
use crate::error::RemoteError;
use ed25519_dalek::SigningKey;
use std::{fs::File, ops::Deref, path::Path};
use tmt_extension_state::Error as StateError;

/// Private file names; anything else is refused.
const FILES: [&str; 10] = [
    "machine.key",
    "key.lock",
    "serve.lock",
    "remote.db",
    "objects.db",
    "settings.json",
    "settings.lock",
    "serve-error.json",
    "deploy.json",
    "deploy.lock",
];

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

pub(crate) fn state_error(error: StateError) -> RemoteError {
    match error {
        StateError::RootNotAbsolute => {
            RemoteError::new("REMOTE_ROOT_INVALID", "Data root must be absolute.")
        }
        StateError::UnsafeDirectory => unsafe_directory(),
        StateError::UnsafeFile | StateError::ReadOpen(_) => unsafe_file(),
        StateError::FileOpen(error) if error.raw_os_error() == Some(nix::libc::ELOOP) => {
            unsafe_file()
        }
        StateError::InvalidFileName => {
            RemoteError::new("REMOTE_STATE_NAME_INVALID", "Invalid private file name.")
        }
        StateError::InvalidLength => invalid_key(),
        StateError::FileOpen(error)
        | StateError::DirectoryMissing(error)
        | StateError::Io(error) => io(error),
        StateError::Lock(error) => io(error),
        StateError::Busy => io(nix::errno::Errno::EWOULDBLOCK),
    }
}
fn lock_error(error: StateError, code: &str, message: &str) -> RemoteError {
    match error {
        StateError::Busy => RemoteError::new(code, message),
        error => state_error(error),
    }
}

#[derive(Clone)]
pub struct Layout {
    shared: tmt_extension_state::Layout,
}
impl Deref for Layout {
    type Target = tmt_extension_state::Layout;

    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}
impl Layout {
    pub fn open(data_root: &Path) -> Result<Self, RemoteError> {
        tmt_extension_state::Layout::open(data_root, "remote", &FILES)
            .map(|shared| Self { shared })
            .map_err(state_error)
    }
    /// Lookup only; status must never initialize Remote state.
    pub fn existing(data_root: &Path) -> Result<Option<Self>, RemoteError> {
        tmt_extension_state::Layout::existing(data_root, "remote", &FILES)
            .map(|layout| layout.map(|shared| Self { shared }))
            .map_err(state_error)
    }
    pub fn read_file(&self, name: &str) -> Result<Option<File>, RemoteError> {
        match self.shared.read_file(name) {
            Ok(file) => Ok(Some(file)),
            Err(StateError::ReadOpen(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(state_error(error)),
        }
    }
    /// Take an existing lease without creating a lock file. A missing lease
    /// beside a database is damaged state, not evidence that serve stopped.
    pub fn existing_serve_lock(&self) -> Result<Option<Serving>, RemoteError> {
        match self.read_file("serve.lock")? {
            Some(lock) => self.lock_file(lock).map(Some),
            None if self.read_file("remote.db")?.is_none() => Ok(None),
            None => Err(RemoteError::new(
                "REMOTE_STATE_UNAVAILABLE",
                "Remote database has no lifecycle lock.",
            )),
        }
    }
    /// Confirm the original foreground has released its lease without opening
    /// state for writing or identifying/signalling any process. A concurrent
    /// successor can keep this busy; never stop it to satisfy this wait.
    pub fn wait_for_release(
        &self,
        deadline: std::time::Instant,
    ) -> Result<Option<Serving>, RemoteError> {
        loop {
            match self.existing_serve_lock() {
                Ok(lease) => return Ok(lease),
                Err(error) if error.code == "REMOTE_ALREADY_SERVING" => {}
                Err(error) => return Err(error),
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(RemoteError::new(
                    "REMOTE_STOP_TIMEOUT",
                    "Shutdown was requested, but Remote did not release its lifecycle lease within the stop deadline.",
                ));
            }
            std::thread::sleep(remaining.min(std::time::Duration::from_millis(50)));
        }
    }
    pub fn file(&self, name: &str) -> Result<File, RemoteError> {
        self.shared.file(name).map_err(state_error)
    }
    /// One Remote serving owner per data root; held for the life of `serve`.
    /// The returned [`Serving`] is the only way to open remote state, so a
    /// second process cannot open the database while serve runs.
    pub fn serve_lock(&self) -> Result<Serving, RemoteError> {
        self.lock_file(self.file("serve.lock")?)
    }
    fn lock_file(&self, lock: File) -> Result<Serving, RemoteError> {
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => {
                RemoteError::new("REMOTE_ALREADY_SERVING", "Remote is already serving.")
            }
            std::fs::TryLockError::Error(error) => io(error),
        })?;
        Ok(Serving {
            layout: self.clone(),
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
/// The machine's long-term Ed25519 key, a software file with no hardware claim.
pub struct MachineKey {
    key: SigningKey,
}
impl MachineKey {
    pub fn open(layout: &Layout) -> Result<Self, RemoteError> {
        let publication = layout
            .shared
            .publication("machine.key", "key.lock", ".machine-", 32)
            .map_err(|error| {
                lock_error(
                    error,
                    "REMOTE_KEY_BUSY",
                    "Machine key publication is already in progress.",
                )
            })?;
        if !publication.exists().map_err(io)? {
            let mut seed = [0; 32];
            getrandom::fill(&mut seed)
                .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain key entropy."))?;
            let mut name = [0; 16];
            getrandom::fill(&mut name)
                .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain key entropy."))?;
            // Preserve Remote's failure ordering, including failed staging:
            // attempt removal, sync the directory, then surface publication/removal errors.
            let result =
                (|| -> std::io::Result<()> { publication.stage(&name)?.write_and_link(&seed) })();
            seed.fill(0);
            let removed = publication.discard(&name);
            layout.shared.sync().map_err(io)?;
            result.map_err(io)?;
            removed.map_err(io)?;
        }
        let mut seed = layout.shared.read("machine.key", 33).map_err(state_error)?;
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
