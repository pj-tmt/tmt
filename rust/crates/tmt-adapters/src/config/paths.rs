use std::{
    env,
    path::{Component, Path, PathBuf},
};

use super::ConfigError;
use tmt_core::identity::NotesIdentityId;

#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub global_dir: PathBuf,
    pub global_config: PathBuf,
    pub local_config: PathBuf,
    pub database: PathBuf,
}

impl ConfigPaths {
    pub fn office_directory(&self) -> PathBuf {
        self.global_dir.join("office")
    }

    /// Endpoint records and sockets of provider channel servers, keyed by binding.
    pub fn channel_directory(&self) -> PathBuf {
        self.global_dir.join("channels")
    }

    pub(crate) fn notes_layout(
        &self,
        identity_id: &NotesIdentityId,
    ) -> std::io::Result<(PathBuf, PathBuf, PathBuf)> {
        let global_dir = if self.global_dir.is_absolute() {
            normalize(&self.global_dir)
        } else {
            normalize(&env::current_dir()?.join(&self.global_dir))
        };
        let identity_dir = global_dir.join("notes").join(identity_id.as_str());
        let notes_file = identity_dir.join("notes.md");
        Ok((global_dir, identity_dir, notes_file))
    }

    pub fn discover() -> Result<Self, ConfigError> {
        let cwd = env::current_dir().map_err(|error| ConfigError::internal(error.to_string()))?;
        let home = env::home_dir()
            .ok_or_else(|| ConfigError::internal("Cannot determine the home directory"))?;
        Ok(Self::resolve(
            &cwd,
            &home,
            env::var_os("TMUX_TEAM_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .as_deref(),
            env::var_os("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .as_deref(),
        ))
    }

    /// Inputs are invocation-owned; tests need not mutate process-wide variables.
    pub fn resolve(cwd: &Path, home: &Path, explicit: Option<&Path>, xdg: Option<&Path>) -> Self {
        let global_dir = if let Some(explicit) = explicit {
            explicit.to_path_buf()
        } else if let Some(xdg) = xdg {
            normalize(&xdg.join("tmux-team"))
        } else {
            let preferred = normalize(&home.join(".config/tmux-team"));
            let legacy = normalize(&home.join(".tmux-team"));
            if preferred.exists() {
                if legacy.join("config.json").exists() && !preferred.join("config.json").exists() {
                    legacy
                } else {
                    preferred
                }
            } else if legacy.exists() {
                legacy
            } else {
                preferred
            }
        };
        let local_config = cwd
            .ancestors()
            .map(|directory| directory.join("tmux-team.json"))
            .find(|candidate| candidate.exists())
            .unwrap_or_else(|| cwd.join("tmux-team.json"));
        Self {
            global_config: normalize(&global_dir.join("config.json")),
            database: normalize(&global_dir.join("tmux-team.db")),
            global_dir,
            local_config,
        }
    }
}

/// Lexical normalization matches path.join without requiring the destination
/// (or a discarded parent component) to exist. Do not canonicalize symlinks.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            component => result.push(component.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}
