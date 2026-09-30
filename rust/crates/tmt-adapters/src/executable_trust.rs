//! Whether an executable TMT runs on the user's behalf is still the one they
//! approved. Extension hooks and host drivers share this: approval records
//! the file's digest and metadata fingerprint after checking that the user
//! owns it and nothing else can write it, and each use checks both again.

use serde::{Deserialize, Serialize};
use std::{fs, io, os::unix::fs::MetadataExt, path::Path};

/// Metadata that changes whenever the executable is replaced, rewritten or
/// re-permissioned (the change time moves on every such operation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Fingerprint {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub modified_ns: i64,
    pub changed_ns: i64,
    pub uid: u32,
    pub mode: u32,
}

impl Fingerprint {
    pub fn of(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified_ns: metadata.mtime() * 1_000_000_000 + metadata.mtime_nsec(),
            changed_ns: metadata.ctime() * 1_000_000_000 + metadata.ctime_nsec(),
            uid: metadata.uid(),
            mode: metadata.mode(),
        }
    }
}

#[derive(Debug)]
pub enum TrustError {
    /// Not an executable file, or writable by someone other than its owner.
    Unsafe(String),
    Io(io::Error),
}

impl From<io::Error> for TrustError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A trusted executable is a regular executable file owned by the current
/// user, and neither it nor its directory may be writable by anyone else.
pub fn verify_ownership(executable: &Path) -> Result<fs::Metadata, TrustError> {
    let metadata = fs::metadata(executable)?;
    let uid = nix::unistd::getuid().as_raw();
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err(TrustError::Unsafe(format!(
            "{} is not an executable file.",
            executable.display()
        )));
    }
    if metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
        return Err(TrustError::Unsafe(format!(
            "{} must be owned by you and not writable by others.",
            executable.display()
        )));
    }
    let directory = executable.parent().unwrap_or(Path::new("/"));
    let parent = fs::metadata(directory)?;
    if parent.mode() & 0o022 != 0 {
        return Err(TrustError::Unsafe(format!(
            "{} must not be writable by others.",
            directory.display()
        )));
    }
    Ok(metadata)
}

/// Whether the approved executable is unchanged: still safely owned, with
/// the fingerprint recorded at approval. A stat, cheap enough for each use.
pub fn unchanged(executable: &Path, approved: &Fingerprint) -> bool {
    verify_ownership(executable).is_ok_and(|metadata| Fingerprint::of(&metadata) == *approved)
}

/// The executable's content digest, as recorded at approval.
pub fn digest(executable: &Path) -> io::Result<String> {
    Ok(tmt_core::content_digest::sha256(&fs::read(executable)?))
}
