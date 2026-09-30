use std::{
    ffi::OsStr,
    fmt,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    Missing,
    NotAbsolute(PathBuf),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "TMT_EXECUTABLE is required",
            Self::NotAbsolute(_) => "TMT_EXECUTABLE must be absolute",
        })
    }
}

impl std::error::Error for DiscoveryError {}

/// Supplied path only: no permission check, normalization or PATH fallback.
pub fn invoking_tmt() -> Result<PathBuf, DiscoveryError> {
    let path = std::env::var_os("TMT_EXECUTABLE")
        .map(PathBuf::from)
        .ok_or(DiscoveryError::Missing)?;
    if !path.is_absolute() {
        return Err(DiscoveryError::NotAbsolute(path));
    }
    Ok(path)
}

/// Callers decide whether to search PATH; relative candidates stay relative.
pub fn find_executable(name: &OsStr, search_path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(search_path)
        .map(|directory| directory.join(name))
        .find(|candidate| is_executable(candidate))
}

pub fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
