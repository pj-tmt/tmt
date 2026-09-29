//! Capture provider setup paths without resolving a stable launcher symlink.

use std::{env, fs, io, os::unix::fs::PermissionsExt, path::PathBuf};

/// The user settings file where setup installs a provider's TMT hooks.
pub fn provider_settings(provider: super::Provider) -> io::Result<PathBuf> {
    let home =
        env::home_dir().ok_or_else(|| io::Error::other("Cannot determine the home directory."))?;
    match provider {
        super::Provider::Claude => Ok(home.join(".claude/settings.json")),
        super::Provider::Codex => Ok(env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("hooks.json")),
        _ => Err(io::Error::other(
            "This provider does not support lifecycle setup.",
        )),
    }
}

pub struct SetupEnvironment {
    pub claude_settings: PathBuf,
    pub codex_settings: PathBuf,
    pub launcher: PathBuf,
}

impl SetupEnvironment {
    pub fn capture() -> io::Result<Self> {
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
            claude_settings: provider_settings(super::Provider::Claude)?,
            codex_settings: provider_settings(super::Provider::Codex)?,
            launcher,
        })
    }

    pub fn settings_path(
        &self,
        provider: super::Provider,
    ) -> Result<&std::path::Path, super::PlanError> {
        match provider {
            super::Provider::Claude => Ok(&self.claude_settings),
            super::Provider::Codex => Ok(&self.codex_settings),
            _ => Err(super::PlanError::UnsupportedProvider),
        }
    }
}
