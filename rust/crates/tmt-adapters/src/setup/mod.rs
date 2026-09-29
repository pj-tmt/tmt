//! Consented provider setup plans. Planning is pure; publication rechecks input.

mod document;
mod environment;
mod publication;
pub mod record;

pub use environment::{SetupEnvironment, provider_settings};
pub use publication::{apply, read_settings};

use crate::drivers::{DriverDefinition, HookFormat};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: String,
}

impl FileChange {
    pub fn changed(&self) -> bool {
        self.before.as_deref().unwrap_or("{}") != self.after
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupPlan {
    pub provider: &'static str,
    pub launcher: PathBuf,
    pub removing: bool,
    pub change: FileChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    InvalidSettings,
    EditedHook,
    InvalidLauncher,
    TooLarge,
    UnsupportedProvider,
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidSettings => "Provider settings must be valid JSON with unambiguous hook objects and arrays.",
            Self::EditedHook => "A TMT hook was edited or duplicated; preserve it and resolve the conflict before setup.",
            Self::InvalidLauncher => "Setup needs an absolute stable TMT launcher path without control characters.",
            Self::TooLarge => "Provider settings exceed the 1 MiB setup limit.",
            Self::UnsupportedProvider => "This provider does not support lifecycle setup.",
        })
    }
}

impl std::error::Error for PlanError {}

pub const SETTINGS_LIMIT: usize = 1024 * 1024;

fn hook_entry(provider: &DriverDefinition, launcher: &str) -> Result<serde_json::Value, PlanError> {
    match provider.descriptor.hooks {
        Some(HookFormat::SessionHooksJson) => Ok(crate::runtime::hook_protocol::command_entry(
            provider.name(),
            launcher,
        )),
        None => Err(PlanError::UnsupportedProvider),
    }
}

/// Whether the provider's TMT SessionStart hook is installed in its user
/// settings. Read-only and bounded; an unreadable or invalid file counts as
/// not installed.
pub fn start_hook_installed(provider: &DriverDefinition) -> bool {
    provider_settings(provider)
        .and_then(|path| read_settings(&path))
        .ok()
        .flatten()
        .is_some_and(|text| document::has_owned_start_hook(provider, &text))
}

pub fn plan(
    provider: &DriverDefinition,
    path: PathBuf,
    before: Option<String>,
    launcher: PathBuf,
    removing: bool,
) -> Result<SetupPlan, PlanError> {
    let selected = launcher
        .to_str()
        .filter(|value| launcher.is_absolute() && !value.chars().any(char::is_control))
        .ok_or(PlanError::InvalidLauncher)?;
    let after = document::settings(
        provider,
        before.as_deref().unwrap_or("{}"),
        selected,
        removing,
    )?;
    Ok(SetupPlan {
        provider: provider.name(),
        launcher,
        removing,
        change: FileChange {
            path,
            before,
            after,
        },
    })
}
