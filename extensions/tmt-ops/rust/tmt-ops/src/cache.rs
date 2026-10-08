//! Squad's own cache files: UI state and regenerable values kept in the
//! user's cache directory. Losing or corrupting one only costs a refresh.

use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// `$XDG_CACHE_HOME/tmt-squad/<name>`, else `~/.cache/tmt-squad/<name>`.
pub fn directory(name: &str) -> Option<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let cache =
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")))?;
    Some(cache.join("tmt-squad").join(name))
}

/// Replaces `path` atomically with a file only the user can read, creating
/// its directory the same way, so a reader never sees half a file.
pub fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let directory = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    // A leftover from a process that had this id is stale.
    let _ = fs::remove_file(&temporary);
    // Readable only by the user, like its directory.
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut file| file.write_all(contents))
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
