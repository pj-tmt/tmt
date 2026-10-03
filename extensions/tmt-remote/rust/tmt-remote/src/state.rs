//! Remote's private subtree `<dataRoot>/remote/`, using the extension-state leaf:
//! an owned 0700 directory, owned 0600 regular files opened without following
//! symlinks, create-only machine key publication and one foreground serve lock.
//! No core database, configuration or provider setting is touched.
use crate::error::RemoteError;
use ed25519_dalek::SigningKey;
use nix::fcntl::Flock;
use std::{fs::File, ops::Deref, path::Path};
use tmt_extension_state::Error as StateError;

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

fn state_error(error: StateError) -> RemoteError {
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
    pub fn file(&self, name: &str) -> Result<File, RemoteError> {
        self.shared.file(name).map_err(state_error)
    }
    /// One foreground remote per data root; held for the life of `serve`.
    /// The returned [`Serving`] is the only way to open remote state, so a
    /// second process cannot open the database while serve runs.
    pub fn serve_lock(&self) -> Result<Serving, RemoteError> {
        let lock = self.shared.lock("serve.lock").map_err(|error| {
            lock_error(
                error,
                "REMOTE_ALREADY_SERVING",
                "Remote is already serving.",
            )
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
    _lock: Flock<File>,
}
impl Serving {
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
