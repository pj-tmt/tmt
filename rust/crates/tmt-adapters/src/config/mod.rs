//! Configuration filesystem boundary. One raw-document implementation preserves
//! opaque user fields while shared core policy validates every known setting.

mod document;
mod paths;

#[cfg(test)]
mod initialization_tests;
#[cfg(test)]
mod theme_tests;

pub use document::ThemeProblem;
pub use paths::ConfigPaths;
pub(crate) use paths::normalize;
use std::{fmt, path::Path};
pub use tmt_core::settings::Scope;
use tmt_core::settings::{LocalClear, ResolvedSettings, Setting};

#[derive(Debug)]
pub struct ConfigError {
    pub code: &'static str,
    pub message: String,
}

impl ConfigError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "INTERNAL_ERROR",
            message: message.into(),
        }
    }

    fn validation(path: &Path, field: &str, expected: &str) -> Self {
        Self {
            code: "CONFIG_ERROR",
            message: format!(
                "Invalid configuration in {} ({field}): must be {expected}.",
                path.display()
            ),
        }
    }

    fn parse(path: &Path, cause: impl fmt::Display) -> Self {
        Self {
            code: "CONFIG_ERROR",
            message: format!("Invalid JSON in {}: {cause}", path.display()),
        }
    }

    fn initialization(path: &Path, cause: impl fmt::Display) -> Self {
        Self {
            code: "ERROR",
            message: format!("Could not initialize {}: {cause}", path.display()),
        }
    }

    fn already_initialized(path: &Path) -> Self {
        Self {
            code: "ERROR",
            message: format!(
                "{} already exists. Remove it first if you want to reinitialize.",
                path.display()
            ),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

pub struct ConfigFiles {
    pub paths: ConfigPaths,
}

impl ConfigFiles {
    /// Global advisory policy, independent of the hook worker's project cwd.
    pub fn notes_compaction_reminder(&self) -> Result<bool, ConfigError> {
        let path = &self.paths.global_config;
        let value = document::read(path, Scope::Global)?;
        Ok(ResolvedSettings::from_layers(
            document::project(&value, path, Scope::Global)?,
            Vec::new(),
        )
        .settings
        .notes_compaction_reminder)
    }

    pub fn notification_settings(&self) -> Result<(u64, u64), ConfigError> {
        document::notification_settings(&self.paths.global_config)
    }

    /// Create the workspace-local settings file without reading or initializing
    /// any other configuration or storage state.
    pub fn initialize_local(&self) -> Result<(), ConfigError> {
        initialize_local_file(&self.paths.local_config)
    }

    pub fn load(&self) -> Result<ResolvedSettings, ConfigError> {
        let read_layer = |scope| {
            let path = self.path(scope);
            let document = document::read(path, scope)?;
            document::project(&document, path, scope)
        };
        Ok(ResolvedSettings::from_layers(
            read_layer(Scope::Global)?,
            read_layer(Scope::Local)?,
        ))
    }

    /// The global file's `theme` settings, as written. The outer error is
    /// the file itself (unreadable or not JSON), as for every setting; the
    /// inner one a wrongly shaped theme, which never affects loading the
    /// other settings.
    pub fn theme(&self) -> Result<Result<Vec<(String, String)>, ThemeProblem>, ConfigError> {
        let path = self.path(Scope::Global);
        Ok(document::theme(&document::read(path, Scope::Global)?))
    }

    pub fn set(&self, setting: Setting, scope: Scope) -> Result<(), ConfigError> {
        let path = self.path(scope);
        let mut value = document::read(path, scope)?;
        document::set(&mut value, setting, scope)?;
        document::write(path, &value, scope)
    }

    /// Write only the presentation base, validated by the CLI style registry.
    /// Theme errors must remain independent of runtime settings validation.
    pub fn set_theme_base(&self, base: &str) -> Result<(), ConfigError> {
        let path = self.path(Scope::Global);
        let mut value = document::read(path, Scope::Global)?;
        document::set_theme_base(&mut value, base, path)?;
        document::write(path, &value, Scope::Global)
    }

    pub fn clear_local(&self, clear: LocalClear) -> Result<(), ConfigError> {
        let path = self.path(Scope::Local);
        let mut value = document::read(path, Scope::Local)?;
        if document::clear(&mut value, clear.key()) {
            document::write(path, &value, Scope::Local)?;
        }
        Ok(())
    }

    fn path(&self, scope: Scope) -> &Path {
        match scope {
            Scope::Global => &self.paths.global_config,
            Scope::Local => &self.paths.local_config,
        }
    }
}

fn initialize_local_file(path: &Path) -> Result<(), ConfigError> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ConfigError::already_initialized(path));
        }
        Err(error) => return Err(ConfigError::initialization(path, error)),
    };

    if let Err(error) = file.write_all(b"{}\n") {
        return Err(finish_initialization_failure(path, &file, error));
    }
    if let Err(error) = file.sync_all() {
        return Err(finish_initialization_failure(path, &file, error));
    }
    drop(file);
    Ok(())
}

fn finish_initialization_failure(
    path: &Path,
    file: &std::fs::File,
    error: std::io::Error,
) -> ConfigError {
    match remove_owned_partial_file(path, file) {
        Ok(()) => ConfigError::initialization(path, error),
        Err(cleanup) => ConfigError::initialization(
            path,
            format!("{error}; could not remove partial file: {cleanup}"),
        ),
    }
}

#[cfg(unix)]
fn remove_owned_partial_file(path: &Path, file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let owned = file.metadata()?;
    let current = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if current.dev() == owned.dev() && current.ino() == owned.ino() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn remove_owned_partial_file(_path: &Path, _file: &std::fs::File) -> std::io::Result<()> {
    // Native installation targets are Unix. On other targets preserve the
    // path rather than risk deleting a replacement that arrived concurrently.
    Ok(())
}
