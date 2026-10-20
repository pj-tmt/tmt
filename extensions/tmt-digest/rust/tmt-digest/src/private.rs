//! The admission rule for Digest's private state paths: owner-only regular
//! files and directories, never a symlink.

use std::{
    fs,
    io::{self, ErrorKind},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::Path,
};

/// Whether `path` is an owner-only regular file (or directory when `directory`).
pub fn is_private(path: &Path, directory: bool) -> io::Result<bool> {
    let meta = fs::symlink_metadata(path)?;
    Ok(!(meta.file_type().is_symlink()
        || meta.is_dir() != directory
        || (!directory && !meta.is_file())
        || meta.mode() & 0o077 != 0))
}

/// Creates a missing owner-only directory, then reports whether `path` is private.
pub fn ensure_dir(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(path)?;
        }
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    is_private(path, true)
}
