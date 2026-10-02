//! Extension-owned software keyring and private files; no core/config writes.
use crate::Result;
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

pub struct Layout {
    pub directory: PathBuf,
}
#[derive(Debug, PartialEq, Eq)]
pub enum StateFault {
    RootNotAbsolute,
    UnsafeDirectory,
    UnsafeFile,
    InvalidFileName,
    InvalidOwnerKey,
    AlreadyServing,
    KeyringBusy,
}
impl StateFault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RootNotAbsolute => "COLAB_ROOT_INVALID",
            Self::UnsafeDirectory | Self::UnsafeFile => "COLAB_STATE_UNSAFE",
            Self::InvalidFileName => "COLAB_STATE_NAME_INVALID",
            Self::InvalidOwnerKey => "COLAB_KEY_INVALID",
            Self::AlreadyServing => "COLAB_ALREADY_SERVING",
            Self::KeyringBusy => "COLAB_KEYRING_BUSY",
        }
    }
}
impl std::fmt::Display for StateFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RootNotAbsolute => "Data root must be absolute.",
            Self::UnsafeDirectory => {
                "Colab directory must be an owned 0700 directory, not a symlink."
            }
            Self::UnsafeFile => "Colab files must be owned regular 0600 files.",
            Self::InvalidFileName => "Invalid private file name.",
            Self::InvalidOwnerKey => "Invalid owner key length; key was not replaced.",
            Self::AlreadyServing => "Local space is already serving.",
            Self::KeyringBusy => "Owner keyring publication is already in progress.",
        })
    }
}
impl std::error::Error for StateFault {}
impl Layout {
    pub fn open(data_root: &Path) -> Result<Self> {
        if !data_root.is_absolute() {
            return Err(StateFault::RootNotAbsolute.into());
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(data_root)?;
        let data_root = fs::canonicalize(data_root)?;
        let directory = data_root.join("colab");
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => File::open(&data_root)?.sync_all()?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        Self::admit(data_root)?.ok_or_else(|| "Colab directory disappeared.".into())
    }
    pub fn existing(data_root: &Path) -> Result<Option<Self>> {
        if !data_root.is_absolute() {
            return Err(StateFault::RootNotAbsolute.into());
        }
        // Only the trusted core-selected root may contain aliases such as macOS /var.
        let data_root = match fs::canonicalize(data_root) {
            Ok(root) => root,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Self::admit(data_root)
    }
    fn admit(data_root: PathBuf) -> Result<Option<Self>> {
        let directory = data_root.join("colab");
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if !metadata.is_dir()
            || metadata.uid() != Uid::effective().as_raw()
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(StateFault::UnsafeDirectory.into());
        }
        Ok(Some(Self { directory }))
    }
    pub fn file(&self, name: &str) -> Result<File> {
        if !["owner.key", "serve.lock", "keyring.lock", "space.db"].contains(&name) {
            return Err(StateFault::InvalidFileName.into());
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(self.directory.join(name))?;
        validate_file(&file)?;
        Ok(file)
    }
    pub fn serve_lock(&self) -> Result<Flock<File>> {
        Flock::lock(self.file("serve.lock")?, FlockArg::LockExclusiveNonblock).map_err(|(_, e)| {
            if e == nix::errno::Errno::EWOULDBLOCK {
                StateFault::AlreadyServing.into()
            } else {
                e.into()
            }
        })
    }
    pub fn running(&self) -> Result<bool> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(self.directory.join("serve.lock"))
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        validate_file(&file)?;
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(_lock) => Ok(false),
            Err((_, nix::errno::Errno::EWOULDBLOCK)) => Ok(true),
            Err((_, e)) => Err(e.into()),
        }
    }
}
fn validate_file(file: &File) -> Result<()> {
    let m = file.metadata()?;
    if !m.is_file() || m.uid() != Uid::effective().as_raw() || m.mode() & 0o777 != 0o600 {
        return Err(StateFault::UnsafeFile.into());
    }
    Ok(())
}

pub struct Keyring {
    owner: SigningKey,
    pub space_id: String,
}
impl Keyring {
    pub fn open(layout: &Layout) -> Result<Self> {
        // Publication/cleanup share this short-lived lock; never remove an active writer's file.
        let _publication = Flock::lock(
            layout.file("keyring.lock")?,
            FlockArg::LockExclusiveNonblock,
        )
        .map_err(|(_, e)| -> Box<dyn std::error::Error + Send + Sync> {
            if e == nix::errno::Errno::EWOULDBLOCK {
                StateFault::KeyringBusy.into()
            } else {
                e.into()
            }
        })?;
        cleanup_owner_temporaries(layout)?;
        let destination = layout.directory.join("owner.key");
        if !destination.try_exists()? {
            let mut seed = [0; 32];
            getrandom::fill(&mut seed)?;
            let mut name = [0; 16];
            getrandom::fill(&mut name)?;
            let temporary = layout.directory.join(format!(".owner-{}", hex(&name)));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            let result = (|| -> Result<()> {
                file.write_all(&seed)?;
                file.sync_all()?;
                match fs::hard_link(&temporary, &destination) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.into()),
                }
                Ok(())
            })();
            seed.fill(0);
            fs::remove_file(&temporary)?;
            File::open(&layout.directory)?.sync_all()?;
            result?;
        }
        Self::read(layout)
    }
    pub fn read(layout: &Layout) -> Result<Self> {
        let destination = layout.directory.join("owner.key");
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(destination)?;
        validate_file(&file)?;
        let mut seed = Vec::new();
        file.take(33).read_to_end(&mut seed)?;
        let mut seed_array: [u8; 32] = seed
            .as_slice()
            .try_into()
            .map_err(|_| StateFault::InvalidOwnerKey)?;
        let owner = SigningKey::from_bytes(&seed_array);
        seed_array.fill(0);
        seed.fill(0);
        let space_id = tmt_colab_model::crypto::space_id(&owner.verifying_key().to_bytes())
            .map_err(|_| StateFault::InvalidOwnerKey)?;
        Ok(Self { owner, space_id })
    }
    pub fn owner_public(&self) -> [u8; 32] {
        self.owner.verifying_key().to_bytes()
    }
}
fn cleanup_owner_temporaries(layout: &Layout) -> Result<()> {
    let mut removed = false;
    for entry in fs::read_dir(&layout.directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name
            .to_str()
            .and_then(|s| s.strip_prefix(".owner-"))
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
            .open(entry.path())?;
        validate_file(&file)?;
        if file.metadata()?.len() > 32 {
            return Err(StateFault::InvalidOwnerKey.into());
        }
        fs::remove_file(entry.path())?;
        removed = true;
    }
    if removed {
        File::open(&layout.directory)?.sync_all()?;
    }
    Ok(())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
