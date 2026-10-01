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
        publish_secret::<32>(layout, "owner.key", ".owner-")?;
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
        let space_id = tmt_colab_model::crypto::space_id(&owner.verifying_key().to_bytes())?;
        Ok(Self { owner, space_id })
    }
    /// Root signatures are confined to the verified membership-log mutation owner.
    pub(crate) fn genesis(&self, payload: &[u8]) -> Result<tmt_colab_model::statement::Envelope> {
        Ok(tmt_colab_model::statement::sign(
            &self.space_id,
            None,
            "member.add",
            payload,
            &self.owner,
        )?)
    }
    pub fn owner_public(&self) -> [u8; 32] {
        self.owner.verifying_key().to_bytes()
    }
}
/// Distinct owner-member keys. Root ownership is never assigned as a member role.
pub(crate) struct MemberKeys {
    pub id: String,
    pub signing: SigningKey,
    pub encryption_public: [u8; 32],
    encryption_seed: [u8; 32],
}
impl MemberKeys {
    pub(crate) fn open(layout: &Layout) -> Result<Self> {
        let _publication = Flock::lock(
            layout.file("keyring.lock")?,
            FlockArg::LockExclusiveNonblock,
        )
        .map_err(|(_, e)| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        cleanup_temporaries(layout, ".member-", 80)?;
        publish_secret::<80>(layout, "member.key", ".member-")?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(layout.directory.join("member.key"))?;
        validate_file(&file)?;
        let mut bytes = Vec::new();
        file.take(81).read_to_end(&mut bytes)?;
        if bytes.len() != 80 {
            bytes.fill(0);
            return Err(StateFault::InvalidOwnerKey.into());
        }
        let mut signing_seed = [0; 32];
        signing_seed.copy_from_slice(&bytes[..32]);
        let mut encryption_seed = [0; 32];
        encryption_seed.copy_from_slice(&bytes[32..64]);
        let mut id = [0; 16];
        id.copy_from_slice(&bytes[64..]);
        id[6] = (id[6] & 0x0f) | 0x40;
        id[8] = (id[8] & 0x3f) | 0x80;
        let id = format!(
            "{}-{}-{}-{}-{}",
            hex(&id[..4]),
            hex(&id[4..6]),
            hex(&id[6..8]),
            hex(&id[8..10]),
            hex(&id[10..])
        );
        let signing = SigningKey::from_bytes(&signing_seed);
        signing_seed.fill(0);
        bytes.fill(0);
        let encryption_public = tmt_colab_model::keys::x25519_public(&encryption_seed);
        Ok(Self {
            id,
            signing,
            encryption_public,
            encryption_seed,
        })
    }
}
impl Drop for MemberKeys {
    fn drop(&mut self) {
        self.encryption_seed.fill(0);
    }
}

/// Caller holds the publication lock. Frozen key bytes are never replaced.
fn publish_secret<const N: usize>(layout: &Layout, name: &str, prefix: &str) -> Result<()> {
    let destination = layout.directory.join(name);
    if destination.try_exists()? {
        return Ok(());
    }
    let mut temporary_id = [0; 16];
    getrandom::fill(&mut temporary_id)?;
    let temporary = layout
        .directory
        .join(format!("{prefix}{}", hex(&temporary_id)));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let mut secret = [0; N];
    let result = (|| -> Result<()> {
        getrandom::fill(&mut secret)?;
        file.write_all(&secret)?;
        file.sync_all()?;
        match fs::hard_link(&temporary, &destination) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    })();
    secret.fill(0);
    // Creation succeeded before entering the fallible publication block.
    fs::remove_file(&temporary)?;
    File::open(&layout.directory)?.sync_all()?;
    result
}

fn cleanup_owner_temporaries(layout: &Layout) -> Result<()> {
    cleanup_temporaries(layout, ".owner-", 32)
}
fn cleanup_temporaries(layout: &Layout, prefix: &str, bound: u64) -> Result<()> {
    let mut removed = false;
    for entry in fs::read_dir(&layout.directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name
            .to_str()
            .and_then(|s| s.strip_prefix(prefix))
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
        if file.metadata()?.len() > bound {
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
