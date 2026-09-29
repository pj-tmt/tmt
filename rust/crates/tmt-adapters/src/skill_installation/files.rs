//! Installer-local file publication and path guards. No command execution.

use crate::config::normalize;
use nix::fcntl::Flock;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub(super) fn lock(global: &Path) -> io::Result<Flock<File>> {
    fs::create_dir_all(global)?;
    crate::file_lock::exclusive(&global.join("skill-install.lock")).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Cannot acquire the skill installation lock: {error}"),
        )
    })
}

pub(super) fn with_lock<T>(
    global: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let lock = lock(global)?;
    let pending = operation();
    let released = lock.unlock().map_err(|(guard, error)| {
        drop(guard);
        io::Error::other(format!(
            "Could not release skill installation lock: {error}"
        ))
    });
    match (pending, released) {
        (Err(primary), Err(cleanup)) => Err(io::Error::other(format!("{primary}; {cleanup}"))),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(file)) => {
            drop(file);
            Ok(value)
        }
    }
}

/// Resolve existing ancestors while retaining a nonexistent suffix. Lexical
/// normalization alone cannot detect source overlap through symlink aliases.
pub(super) fn resolved(path: &Path) -> io::Result<PathBuf> {
    let absolute = normalize(&std::env::current_dir()?.join(path));
    let mut ancestor = absolute.as_path();
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(mut real) => {
                for component in suffix.iter().rev() {
                    real.push(component);
                }
                return Ok(real);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                suffix.push(ancestor.file_name().ok_or(error)?);
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| io::Error::other("Cannot resolve installation path."))?;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn safe_target(source_root: &Path, target: &Path) -> io::Result<()> {
    // Publication replaces the leaf entry, not its symlink destination. Resolve
    // its parent so a correct managed link remains a valid no-op destination.
    let target_location = entry_location(target)?;
    let source = resolved(source_root)?;
    if source.starts_with(&target_location) || target_location.starts_with(&source) {
        return Err(io::Error::other(format!(
            "Skill target overlaps bundled source: {}",
            target.display()
        )));
    }
    Ok(())
}

pub(super) fn entry_location(target: &Path) -> io::Result<PathBuf> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("Skill target has no parent."))?;
    let leaf = target
        .file_name()
        .ok_or_else(|| io::Error::other("Skill target has no name."))?;
    Ok(resolved(parent)?.join(leaf))
}

pub(crate) fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn backup(target: &Path) -> io::Result<PathBuf> {
    let parent = target
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("Skill backup location has no parent."))?;
    let directory = parent.join(".tmt-skill-backups");
    fs::create_dir_all(&directory)?;
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::other("Skill target has no name."))?;
    let mut filename = name.to_os_string();
    filename.push(format!(
        ".backup-{}-{}",
        crate::request_runtime::wall_time_ms(),
        Uuid::new_v4()
    ));
    let destination = directory.join(filename);
    if exists(&destination)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Skill backup path is occupied.",
        ));
    }
    fs::rename(target, &destination)?;
    Ok(destination)
}

pub(crate) fn atomic_write(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("Publication path has no parent."))?;
    fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".tmt-install-{}", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stage)?;
    let pending = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&stage, destination)
    })();
    finish_stage(&stage, pending)
}

pub(super) fn link(destination: &Path, source: &Path) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("Skill target has no parent."))?;
    fs::create_dir_all(parent)?;
    if !exists(destination)? {
        return std::os::unix::fs::symlink(source, destination);
    }
    let stage = parent.join(format!(".tmt-install-{}", Uuid::new_v4()));
    std::os::unix::fs::symlink(source, &stage)?;
    let pending = fs::rename(&stage, destination);
    finish_stage(&stage, pending)
}

fn finish_stage(stage: &Path, pending: io::Result<()>) -> io::Result<()> {
    match fs::remove_file(stage) {
        Ok(()) => pending,
        Err(error) if error.kind() == io::ErrorKind::NotFound => pending,
        Err(error) => Err(io::Error::other(format!(
            "{}; could not remove owned staging entry {}: {error}",
            pending
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "Publication completed".into()),
            stage.display()
        ))),
    }
}
