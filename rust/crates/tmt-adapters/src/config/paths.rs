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
    /// Private recovery snapshots; never identity or presence authority.
    pub fn workspace_directory(&self, socket: &str) -> PathBuf {
        self.global_dir
            .join("workspace")
            .join(tmt_core::content_digest::sha256(socket.as_bytes()))
    }

    pub fn office_directory(&self) -> PathBuf {
        self.global_dir.join("office")
    }

    /// Endpoint records and sockets of provider channel servers, keyed by binding.
    pub fn channel_directory(&self) -> PathBuf {
        Self::channel_directory_in(&self.global_dir)
    }

    /// The same directory, absolute, for a caller that holds only the global
    /// directory. Every enrollment and every evidence lookup uses this one path.
    pub fn channel_directory_in(global_dir: &Path) -> PathBuf {
        let directory = global_dir.join("channels");
        std::path::absolute(&directory).unwrap_or(directory)
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

    /// Startup file selected by the shell's own conventional environment.
    #[cfg(unix)]
    pub fn shell_startup(shell: crate::completion_install::Shell) -> Result<PathBuf, ConfigError> {
        use crate::completion_install::Shell;
        let home = env::home_dir()
            .ok_or_else(|| ConfigError::internal("Cannot determine the home directory"))?;
        let variable = |name| {
            env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let path = match shell {
            Shell::Bash => home.join(".bashrc"),
            Shell::Zsh => variable("ZDOTDIR").unwrap_or(home).join(".zshrc"),
            Shell::Fish => variable("XDG_CONFIG_HOME")
                .unwrap_or_else(|| home.join(".config"))
                .join("fish/config.fish"),
        };
        std::path::absolute(path).map_err(|error| ConfigError::internal(error.to_string()))
    }

    pub fn discover() -> Result<Self, ConfigError> {
        let cwd = env::current_dir().map_err(|error| ConfigError::internal(error.to_string()))?;
        let home = env::home_dir()
            .ok_or_else(|| ConfigError::internal("Cannot determine the home directory"))?;
        let explicit = env::var_os("TMT_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let xdg = env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let paths = Self::resolve(&cwd, &home, explicit.as_deref(), xdg.as_deref());
        super::rename::prepare(
            &paths.global_dir,
            &home,
            explicit.as_deref(),
            xdg.as_deref(),
        )?;
        Ok(paths)
    }

    /// Inputs are invocation-owned; tests need not mutate process-wide variables.
    pub fn resolve(cwd: &Path, home: &Path, explicit: Option<&Path>, xdg: Option<&Path>) -> Self {
        let global_dir = if let Some(explicit) = explicit {
            explicit.to_path_buf()
        } else if let Some(xdg) = xdg {
            normalize(&xdg.join("tmt"))
        } else {
            normalize(&home.join(".config/tmt"))
        };
        let local_config = cwd
            .ancestors()
            .map(|directory| directory.join("tmt.json"))
            .find(|candidate| candidate.exists())
            .unwrap_or_else(|| cwd.join("tmt.json"));
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
