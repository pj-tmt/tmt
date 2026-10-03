//! Consented provider setup plans. Planning is pure; publication rechecks input.

mod document;
mod environment;
mod publication;
pub mod record;
pub mod removal;

pub use environment::{SetupEnvironment, provider_settings};
pub use publication::{BACKUP_DIRECTORY, apply, backup_directory, backup_warning, read_settings};

use crate::drivers::{DriverDefinition, HookFormat};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: String,
}

impl SetupPlan {
    /// The TMT hooks this plan writes or removes, for its preview.
    pub fn events(&self) -> &'static str {
        const ALL: &str = "SessionStart, SessionEnd, UserPromptSubmit and Stop (context usage, consumption and activity) hooks";
        const LIFECYCLE: &str = "SessionStart, SessionEnd and UserPromptSubmit hooks";
        match (self.removing, self.usage_before, self.usage) {
            (true, true, _) | (false, _, true) => ALL,
            (false, true, false) => "Stop (context usage, consumption and activity) hook",
            _ => LIFECYCLE,
        }
    }
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
    /// Whether the file holds the turn-end hook (#519) before and
    /// after the plan.
    pub usage_before: bool,
    pub usage: bool,
    pub change: FileChange,
}

pub use tmt_core::driver::descriptor::UsageHook;

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
        .is_some_and(|text| document::has_owned_hook(provider, &text, "SessionStart"))
}

/// Effective owned Stop installation is the existing collection consent evidence.
/// Legacy lifecycle-only and explicit opt-outs do not enable foreground sampling.
pub fn usage_hook_installed(provider: &DriverDefinition) -> bool {
    provider_settings(provider)
        .and_then(|path| read_settings(&path))
        .ok()
        .flatten()
        .is_some_and(|text| document::has_owned_hook(provider, &text, document::USAGE))
}

pub fn plan(
    provider: &DriverDefinition,
    path: PathBuf,
    before: Option<String>,
    launcher: PathBuf,
    removing: bool,
    usage: UsageHook,
) -> Result<SetupPlan, PlanError> {
    let selected = launcher
        .to_str()
        .filter(|value| launcher.is_absolute() && !value.chars().any(char::is_control))
        .ok_or(PlanError::InvalidLauncher)?;
    let text = before.as_deref().unwrap_or("{}");
    // Removal is the exact inverse of setup's own edits where it can be.
    let after = if removing {
        document::removed(provider, text)?.unwrap_or_else(|| text.to_owned())
    } else if usage == UsageHook::Remove {
        document::usage_removed(provider, text, selected)?
    } else {
        document::settings(provider, text, selected, false, usage)?
    };
    Ok(SetupPlan {
        provider: provider.name(),
        launcher,
        removing,
        usage_before: document::has_owned_hook(provider, text, document::USAGE),
        usage: document::has_owned_hook(provider, &after, document::USAGE),
        change: FileChange {
            path,
            before,
            after,
        },
    })
}
