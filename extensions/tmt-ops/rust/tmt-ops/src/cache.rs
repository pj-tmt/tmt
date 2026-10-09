//! Squad's own cache files: UI state and regenerable values kept in the
//! user's cache directory. Losing or corrupting one only costs a refresh.

use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

/// `$XDG_CACHE_HOME/tmt-ops/<name>`, else `~/.cache/tmt-ops/<name>`.
pub fn directory(name: &str) -> Option<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let cache =
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")))?;
    Some(cache.join("tmt-ops").join(name))
}

/// Replaces `path` atomically with a file only the user can read, creating
/// its directory the same way, so a reader never sees half a file.
pub fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let directory = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    replace_if(path, contents, || Ok(()))
}

/// Publishes in an existing directory, rechecking cancellation before rename.
/// Never recreates a missing parent owned by migration.
pub(crate) fn replace_if(
    path: &Path,
    contents: &[u8],
    before_publish: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // Readable only by the user, like its directory.
    // Never remove another writer's staging file, even after PID reuse.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let written = file
        .write_all(contents)
        .and_then(|()| before_publish())
        .and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_replaced_file_and_its_directory_are_the_user_s_alone() {
        let root = std::env::temp_dir().join(format!("squad-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let path = root.join("fields").join("p.json");
        replace(&path, b"one").unwrap();
        replace(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let entries: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(entries.len(), 1, "no temporary file is left behind");
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod publication_failures {
    use super::*;

    #[test]
    fn rename_failure_removes_only_the_exclusively_created_staging_file() {
        let root =
            std::env::temp_dir().join(format!("ops-cache-rename-failure-{}", std::process::id()));
        fs::create_dir_all(root.join("target.json")).unwrap();
        let foreign = root.join("foreign.tmp");
        fs::write(&foreign, b"preserve").unwrap();
        assert!(replace(&root.join("target.json"), b"candidate").is_err());
        assert!(root.join("target.json").is_dir());
        assert_eq!(fs::read(&foreign).unwrap(), b"preserve");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
