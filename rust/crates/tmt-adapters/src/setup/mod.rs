//! Consented provider setup plans. Planning is pure; publication rechecks input.

mod document;
mod environment;
mod publication;

pub use environment::{SetupEnvironment, provider_settings};
pub use publication::{apply, read_settings};

use std::path::PathBuf;
pub use tmt_core::skill_provider::Provider;

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

pub const SUPPORTED_PROVIDERS: [Provider; 2] = [Provider::Claude, Provider::Codex];

fn hook_entry(provider: Provider, launcher: &str) -> Result<serde_json::Value, PlanError> {
    match provider {
        Provider::Claude => Ok(crate::runtime::claude::hook_entry(launcher)),
        Provider::Codex => Ok(crate::runtime::codex::hook_entry(launcher)),
        _ => Err(PlanError::UnsupportedProvider),
    }
}

/// Whether the provider's TMT SessionStart hook is installed in its user
/// settings. Read-only and bounded; an unreadable or invalid file counts as
/// not installed.
pub fn start_hook_installed(provider: Provider) -> bool {
    provider_settings(provider)
        .and_then(|path| read_settings(&path))
        .ok()
        .flatten()
        .is_some_and(|text| document::has_owned_start_hook(provider, &text))
}

pub fn claude_plan(
    path: PathBuf,
    before: Option<String>,
    launcher: PathBuf,
    removing: bool,
) -> Result<SetupPlan, PlanError> {
    plan(Provider::Claude, path, before, launcher, removing)
}

pub fn plan(
    provider: Provider,
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
        provider: provider.as_str(),
        launcher,
        removing,
        change: FileChange {
            path,
            before,
            after,
        },
    })
}
