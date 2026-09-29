//! Capture provider setup paths without resolving a stable launcher symlink.

use crate::{
    drivers::{DriverDefinition, Registry},
    skill_installation::ProviderEnvironment,
};
use std::{
    env, fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

/// The user settings file where setup installs a provider's TMT hooks.
pub fn provider_settings(provider: &DriverDefinition) -> io::Result<PathBuf> {
    settings_in(&ProviderEnvironment::capture()?, provider)
}

fn settings_in(
    environment: &ProviderEnvironment,
    provider: &DriverDefinition,
) -> io::Result<PathBuf> {
    environment
        .locations(provider)
        .hook_settings
        .ok_or_else(|| io::Error::other("This provider does not support lifecycle setup."))
}

pub struct SetupEnvironment {
    /// Each hook-capable driver's settings file, in registry order.
    pub settings: Vec<(&'static DriverDefinition, PathBuf)>,
    pub launcher: PathBuf,
}

impl SetupEnvironment {
    pub fn capture(registry: &Registry) -> io::Result<Self> {
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
        let environment = ProviderEnvironment::capture()?;
        Ok(Self {
            settings: registry
                .with_hooks()
                .map(|driver| Ok((driver, settings_in(&environment, driver)?)))
                .collect::<io::Result<_>>()?,
            launcher,
        })
    }

    pub fn settings_path(&self, provider: &DriverDefinition) -> Result<&Path, super::PlanError> {
        self.settings
            .iter()
            .find(|(driver, _)| *driver == provider)
            .map(|(_, path)| path.as_path())
            .ok_or(super::PlanError::UnsupportedProvider)
    }
}
