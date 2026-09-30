//! Squad's own cache files: UI state and regenerable values kept in the
//! user's cache directory. Losing or corrupting one only costs a refresh.

use std::{
    fs,
    io::{self, Write},
    os::unix::fs::DirBuilderExt,
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

/// Replaces `path` atomically, creating its directory readable only by the
/// user, so a reader never sees half a file.
pub fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let directory = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let written = fs::File::create(&temporary)
        .and_then(|mut file| file.write_all(contents))
        .and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}
