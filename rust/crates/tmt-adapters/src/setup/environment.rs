//! Capture provider setup paths without resolving a stable launcher symlink.

use std::{env, fs, io, os::unix::fs::PermissionsExt, path::PathBuf};

pub struct SetupEnvironment {
    pub claude_settings: PathBuf,
    pub launcher: PathBuf,
}

impl SetupEnvironment {
    pub fn capture() -> io::Result<Self> {
        let home = env::home_dir()
            .ok_or_else(|| io::Error::other("Cannot determine the home directory."))?;
        let cwd = env::current_dir()?;
        let search = env::var_os("PATH").unwrap_or_default();
        let launcher = env::split_paths(&search)
            .map(|directory| crate::config::normalize(&cwd.join(directory).join("tmt")))
            .find(|candidate| {
                fs::metadata(candidate).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
            .ok_or_else(|| io::Error::other("Put the stable tmt launcher on PATH before setup."))?;
        Ok(Self {
            claude_settings: home.join(".claude/settings.json"),
            launcher,
        })
    }
}
