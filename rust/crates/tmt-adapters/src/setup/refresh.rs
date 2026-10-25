//! Refresh only the provider scopes the user already approved through setup.
use super::{UsageHook, apply, document, plan_with_choice, read_settings, record};
use crate::drivers::Registry;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefreshStatus {
    Unchanged,
    Updated,
    NeedsApproval,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshReason {
    RefreshUnavailable,
    Current,
    Refreshed,
    InvalidRecord,
    UnsupportedDriver,
    MissingSettings,
    SettingsUnavailable,
    UnownedSettings,
    InvalidSettings,
    EditedHook,
    PublicationFailed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct IntegrationRefresh {
    pub provider: String,
    pub settings_path: PathBuf,
    pub status: RefreshStatus,
    pub reason: RefreshReason,
    pub message: String,
    pub next_step: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct IntegrationRefreshFailure {
    pub reason: RefreshReason,
    pub message: String,
    pub next_step: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationRefreshReport {
    pub integrations: Vec<IntegrationRefresh>,
    pub error: Option<IntegrationRefreshFailure>,
}

fn bounded(message: String) -> String {
    message.chars().take(2048).collect()
}

/// The record grants only its original file, launcher and usage scope. No
/// filesystem detection, implicit adoption or new-provider installation occurs.
/// Each provider retains setup's input recheck and recoverable backup; a failure
/// is reported independently and never authorizes a broader publication.
pub fn refresh_recorded(global: &Path) -> IntegrationRefreshReport {
    let entries = match record::read(global) {
        Ok(entries) => entries,
        Err(error) => {
            return IntegrationRefreshReport {
                integrations: Vec::new(),
                error: Some(IntegrationRefreshFailure {
                    reason: RefreshReason::InvalidRecord,
                    message: bounded(error.to_string()),
                    next_step: "tmt setup".into(),
                }),
            };
        }
    };
    let registry = Registry::builtin();
    let integrations = entries
        .iter()
        .map(|entry| refresh_one(global, entry, &entries, &registry))
        .collect();
    IntegrationRefreshReport {
        integrations,
        error: None,
    }
}

fn refresh_one(
    global: &Path,
    entry: &record::RecordedHooks,
    entries: &[record::RecordedHooks],
    registry: &Registry,
) -> IntegrationRefresh {
    let mut row = IntegrationRefresh {
        provider: entry.driver.clone(),
        settings_path: entry.settings.clone(),
        status: RefreshStatus::Skipped,
        reason: RefreshReason::UnsupportedDriver,
        message: "The recorded driver does not support provider setup.".into(),
        next_step: Some("tmt setup".into()),
    };
    let Some(driver) = registry
        .find(&entry.driver)
        .filter(|driver| driver.descriptor.hooks.is_some())
    else {
        return row;
    };
    row.next_step = Some(format!("tmt setup {}", driver.name()));
    let before = match read_settings(&entry.settings) {
        Ok(Some(before)) => before,
        Ok(None) => {
            row.status = RefreshStatus::NeedsApproval;
            row.reason = RefreshReason::MissingSettings;
            row.message =
                "Recorded settings are missing; setup approval is required to recreate them."
                    .into();
            return row;
        }
        Err(error) => {
            row.reason = RefreshReason::SettingsUnavailable;
            row.message = bounded(error.to_string());
            return row;
        }
    };
    let choice = record::usage_policy(entries, driver.name(), &entry.settings, UsageHook::Default);
    let choice = if choice == UsageHook::Keep
        && !document::has_owned_hook(driver, &before, document::USAGE)
    {
        UsageHook::Remove
    } else {
        choice
    };
    let plan = match plan_with_choice(
        driver,
        entry.settings.clone(),
        Some(before.clone()),
        entry.launcher.clone(),
        false,
        choice,
        record::usage_opted_out(entries, driver.name(), &entry.settings),
    ) {
        Ok(plan) => plan,
        Err(error) => {
            row.status = RefreshStatus::NeedsApproval;
            row.reason = if error == super::PlanError::EditedHook {
                RefreshReason::EditedHook
            } else {
                RefreshReason::InvalidSettings
            };
            row.message = error.to_string();
            return row;
        }
    };
    // A record alone must not resurrect hooks deliberately removed by the
    // user. At least one exact owned entry must still establish the scope.
    let installed = document::lifecycle_events(driver)
        .iter()
        .chain(plan.activity.iter())
        .any(|event| document::has_owned_hook(driver, &before, event))
        || plan.stop_before;
    if !installed {
        row.status = RefreshStatus::NeedsApproval;
        row.reason = RefreshReason::UnownedSettings;
        row.message = "No exact TMT hooks remain in the recorded settings; review setup before installing them again.".into();
        return row;
    }
    match record::refresh(global, entry, plan.usage, || apply(&plan)) {
        Ok(_) => {
            row.status = if plan.change.changed() {
                RefreshStatus::Updated
            } else {
                RefreshStatus::Unchanged
            };
            row.reason = if plan.change.changed() {
                RefreshReason::Refreshed
            } else {
                RefreshReason::Current
            };
            row.message.clear();
            row.next_step = None;
        }
        Err(error) => {
            row.reason = RefreshReason::PublicationFailed;
            row.message = bounded(error.to_string());
        }
    }
    row
}
