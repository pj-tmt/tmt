//! Extension-owned software keyring and private files; no core/config writes.
use crate::Result;
use ed25519_dalek::SigningKey;
use nix::{
    fcntl::{Flock, FlockArg, OFlag},
    unistd::Uid,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub struct Layout {
    pub directory: PathBuf,
}
impl Layout {
    pub fn open(data_root: &Path) -> Result<Self> {
        if !data_root.is_absolute() {
            return Err("Data root must be absolute.".into());
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
            return Err("Data root must be absolute.".into());
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
            return Err("Colab directory must be an owned 0700 directory, not a symlink.".into());
        }
        Ok(Some(Self { directory }))
    }
    pub fn file(&self, name: &str) -> Result<File> {
        if !["owner.key", "serve.lock", "space.db"].contains(&name) {
            return Err("Invalid private file name.".into());
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
            format!("Local space is already serving or lock is unavailable: {e}").into()
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
        return Err("Colab files must be owned regular 0600 files.".into());
    }
    Ok(())
}

pub struct Keyring {
    owner: SigningKey,
    pub space_id: String,
}
impl Keyring {
    pub fn open(layout: &Layout) -> Result<Self> {
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
            .map_err(|_| "Invalid owner key length; key was not replaced.")?;
        let owner = SigningKey::from_bytes(&seed_array);
        seed_array.fill(0);
        seed.fill(0);
        let space_id = space_id(&owner.verifying_key().to_bytes());
        Ok(Self { owner, space_id })
    }
    pub fn owner_public(&self) -> [u8; 32] {
        self.owner.verifying_key().to_bytes()
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
// L2a needs only trust-root naming. L2b replaces this with the merged L1 model API.
fn space_id(public: &[u8; 32]) -> String {
    let mut hash = Sha256::new();
    for bytes in [b"tmt-colab-space-id-v1".as_slice(), public.as_slice()] {
        hash.update((bytes.len() as u32).to_be_bytes());
        hash.update(bytes);
    }
    let digest = hash.finalize();
    let alphabet = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut result = String::new();
    let mut bits = 0u32;
    let mut count = 0;
    for byte in &digest[..20] {
        bits = (bits << 8) | u32::from(*byte);
        count += 8;
        while count >= 5 {
            count -= 5;
            result.push(alphabet[((bits >> count) & 31) as usize] as char);
        }
    }
    result
}
