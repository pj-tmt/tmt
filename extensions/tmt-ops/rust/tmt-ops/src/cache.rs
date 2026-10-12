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
    cache_root().map(|cache| cache.join("tmt-ops").join(name))
}

#[cfg(not(test))]
fn cache_root() -> Option<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")))
}

/// A test build never resolves the host's cache (TEST_ISOLATION): every test in
/// the process shares one directory private to it, so none reads or writes the
/// user's cache and the many entry points that resolve it need no injection.
#[cfg(test)]
fn cache_root() -> Option<PathBuf> {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    Some(ROOT.get_or_init(test_root::create).clone())
}

#[cfg(test)]
mod test_root;

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

    #[test]
    fn a_test_build_never_resolves_the_host_cache() {
        let path = directory("staleness").unwrap();
        assert!(path.starts_with(std::env::temp_dir()), "{path:?}");
        assert!(path.ends_with("tmt-ops/staleness"), "{path:?}");
        // The host's cache roots, when the process has them, are never the answer.
        let host = [
            std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")),
        ];
        for root in host.into_iter().flatten().filter(|root| root.is_absolute()) {
            assert!(!path.starts_with(&root), "{path:?} is under {root:?}");
        }
    }

    #[test]
    fn every_resolution_in_a_test_shares_one_private_root_that_is_writable() {
        let first = directory("isolation-check").unwrap();
        assert_eq!(first.parent(), directory("limits").unwrap().parent());
        let file = first.join("squad.json");
        replace(&file, b"{}").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"{}");
        fs::remove_dir_all(first).unwrap();
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
