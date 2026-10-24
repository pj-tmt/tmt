//! Consented provider setup plans. Planning is pure; publication rechecks input.

mod document;
mod environment;
mod publication;
pub mod record;
pub mod removal;

pub(crate) use document::{compose_launch_hooks, owned_event};
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
    pub fn events(&self) -> String {
        const USAGE: &str = "Stop (context usage, consumption, activity and recent responses)";
        const ANSWERS: &str = "Stop (turn end: activity and chat answers, no usage data)";
        let stop = |usage: bool| if usage { USAGE } else { ANSWERS };
        let lifecycle = crate::drivers::Registry::builtin()
            .find(self.provider)
            .map(document::lifecycle_events)
            .unwrap_or(&["SessionStart", "SessionEnd", "UserPromptSubmit"])
            .iter()
            .copied()
            .map(str::to_owned)
            .chain(
                self.activity
                    .iter()
                    .map(|event| format!("{event} (activity only)")),
            );
        let names: Vec<String> = if self.removing {
            lifecycle
                .chain(self.stop_before.then(|| stop(self.usage_before).to_owned()))
                .collect()
        } else {
            lifecycle.chain([stop(self.usage).to_owned()]).collect()
        };
        match names.split_last() {
            Some((last, [])) => format!("{last} hook"),
            Some((last, rest)) => format!("{} and {last} hooks", rest.join(", ")),
            None => String::new(),
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
    /// Whether usage collection (#519) is on before and after the plan: the turn-end hook is in
    /// the file and the user has not turned usage off. The hook itself also carries the agent's
    /// answers with usage off.
    pub usage_before: bool,
    pub usage: bool,
    /// Whether the file holds the turn-end hook before the plan, whatever the usage choice.
    pub stop_before: bool,
    /// The driver's activity-only events, and whether this plan adds one.
    pub activity: &'static [&'static str],
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
        Some(HookFormat::NamedHooksJson) => Ok(crate::runtime::hook_protocol::flat_command_entry(
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

/// Whether the provider's TMT turn-end (`Stop`) hook is installed. It carries the agent's answers
/// whatever the usage choice (#4914).
pub fn stop_hook_installed(provider: &DriverDefinition) -> bool {
    provider_settings(provider)
        .and_then(|path| read_settings(&path))
        .ok()
        .flatten()
        .is_some_and(|text| document::has_owned_hook(provider, &text, document::USAGE))
}

/// Usage collection is consented: the turn-end hook is installed and the setup record does not
/// say the user turned usage off for this settings file. Legacy lifecycle-only installs have no
/// hook, and an unreadable record counts as an opt-out, so neither reads usage data.
pub fn usage_consented(provider: &DriverDefinition) -> bool {
    let Ok(settings) = provider_settings(provider) else {
        return false;
    };
    stop_hook_installed(provider)
        && crate::config::ConfigPaths::discover()
            .ok()
            .and_then(|paths| record::read(&paths.global_dir).ok())
            .is_some_and(|entries| !record::usage_opted_out(&entries, provider.name(), &settings))
}

pub fn plan(
    provider: &DriverDefinition,
    path: PathBuf,
    before: Option<String>,
    launcher: PathBuf,
    removing: bool,
    usage: UsageHook,
) -> Result<SetupPlan, PlanError> {
    plan_with_choice(provider, path, before, launcher, removing, usage, false)
}

/// [`plan`] for a settings file whose setup record says whether the user had turned usage off.
pub fn plan_with_choice(
    provider: &DriverDefinition,
    path: PathBuf,
    before: Option<String>,
    launcher: PathBuf,
    removing: bool,
    usage: UsageHook,
    usage_opted_out_before: bool,
) -> Result<SetupPlan, PlanError> {
    let selected = launcher
        .to_str()
        .filter(|value| launcher.is_absolute() && !value.chars().any(char::is_control))
        .ok_or(PlanError::InvalidLauncher)?;
    let text = before.as_deref().unwrap_or("{}");
    // Removal is the exact inverse of setup's own edits where it can be.
    let after = if removing {
        document::removed(provider, text)?.unwrap_or_else(|| text.to_owned())
    } else {
        document::settings(provider, text, selected, false, usage)?
    };
    // Usage collection is the user's choice, held in the setup record; the turn-end hook is there
    // either way (it carries the agent's answers, #4914). Usage was on before when the hook was
    // there and the user had not turned it off.
    let usage_before =
        document::has_owned_hook(provider, text, document::USAGE) && !usage_opted_out_before;
    let usage_after = !removing
        && match usage {
            UsageHook::Default | UsageHook::Install => true,
            UsageHook::Remove => false,
            UsageHook::Keep => usage_before,
        };
    Ok(SetupPlan {
        provider: provider.name(),
        launcher,
        removing,
        usage_before,
        usage: usage_after,
        stop_before: document::has_owned_hook(provider, text, document::USAGE),
        activity: document::activity_events(provider),
        change: FileChange {
            path,
            before,
            after,
        },
    })
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use crate::drivers::{agy, claude, codex};

    fn events(
        provider: &DriverDefinition,
        before: &str,
        removing: bool,
        usage: UsageHook,
    ) -> String {
        plan(
            provider,
            PathBuf::from("/settings.json"),
            Some(before.into()),
            PathBuf::from("/stable/tmt"),
            removing,
            usage,
        )
        .unwrap()
        .events()
    }

    /// The preview names each driver-owned activity observer separately from usage consent.
    #[test]
    fn the_preview_names_the_codex_interrupt_observer_only_for_codex() {
        const ALL: &str = "SessionStart, SessionEnd, UserPromptSubmit, Notification (activity only), PostToolUse (activity only), PostToolUseFailure (activity only) and Stop (context usage, consumption, activity and recent responses) hooks";
        const CODEX_ALL: &str = "SessionStart, SessionEnd, UserPromptSubmit, Interrupt (activity only) and Stop (context usage, consumption, activity and recent responses) hooks";
        const ANSWERS: &str = "Stop (turn end: activity and chat answers, no usage data)";
        const AGY_ALL: &str = "SessionStart, PreInvocation and Stop (context usage, consumption, activity and recent responses) hooks";
        assert_eq!(
            events(&claude::DRIVER, "{}", false, UsageHook::Default),
            ALL
        );
        assert_eq!(
            events(&codex::DRIVER, "{}", false, UsageHook::Default),
            CODEX_ALL
        );
        // Turning usage off keeps the turn-end hook and says what it still does.
        assert_eq!(
            events(&agy::DRIVER, "{}", false, UsageHook::Default),
            AGY_ALL
        );
        assert_eq!(
            events(&agy::DRIVER, "{}", false, UsageHook::Remove),
            format!("SessionStart, PreInvocation and {ANSWERS} hooks")
        );
        assert_eq!(
            events(&claude::DRIVER, "{}", false, UsageHook::Remove),
            format!(
                "SessionStart, SessionEnd, UserPromptSubmit, Notification (activity only), PostToolUse (activity only), PostToolUseFailure (activity only) and {ANSWERS} hooks"
            )
        );
        assert_eq!(
            events(&codex::DRIVER, "{}", false, UsageHook::Remove),
            format!(
                "SessionStart, SessionEnd, UserPromptSubmit, Interrupt (activity only) and {ANSWERS} hooks"
            )
        );
        let installed = document::settings(
            &codex::DRIVER,
            "{}",
            "/stable/tmt",
            false,
            UsageHook::Install,
        )
        .unwrap();
        assert_eq!(
            events(&codex::DRIVER, &installed, true, UsageHook::Keep),
            CODEX_ALL
        );
        // An install that already holds every hook shows the same words with usage off.
        assert_eq!(
            events(&codex::DRIVER, &installed, false, UsageHook::Remove),
            format!(
                "SessionStart, SessionEnd, UserPromptSubmit, Interrupt (activity only) and {ANSWERS} hooks"
            )
        );
        // An install from before the observer gains it, and keeps the turn-end hook.
        let legacy = {
            let mut value: serde_json::Value = serde_json::from_str(&installed).unwrap();
            value["hooks"].as_object_mut().unwrap().remove("Interrupt");
            value.to_string()
        };
        let migrated = document::settings(
            &codex::DRIVER,
            &legacy,
            "/stable/tmt",
            false,
            UsageHook::Remove,
        )
        .unwrap();
        assert!(document::has_owned_hook(
            &codex::DRIVER,
            &migrated,
            "Interrupt"
        ));
        assert!(document::has_owned_hook(&codex::DRIVER, &migrated, "Stop"));
    }

    /// Usage is the setup record's choice; the turn-end hook is in the file either way.
    #[test]
    fn usage_follows_the_recorded_choice_while_the_turn_end_hook_stays() {
        let on = document::settings(
            &claude::DRIVER,
            "{}",
            "/stable/tmt",
            false,
            UsageHook::Install,
        )
        .unwrap();
        let lifecycle_only =
            document::settings(&claude::DRIVER, "{}", "/stable/tmt", false, UsageHook::Keep)
                .unwrap();
        assert!(!document::has_owned_hook(
            &claude::DRIVER,
            &lifecycle_only,
            "Stop"
        ));
        let planned = |before: &str, usage: UsageHook, opted_out: bool| {
            plan_with_choice(
                &claude::DRIVER,
                PathBuf::from("/settings.json"),
                Some(before.into()),
                PathBuf::from("/stable/tmt"),
                false,
                usage,
                opted_out,
            )
            .unwrap()
        };
        // Usage on, then turned off: nothing is removed, only the choice changes.
        let off = planned(&on, UsageHook::Remove, false);
        assert!(off.usage_before && !off.usage && off.stop_before);
        assert!(!off.change.changed(), "the hook stays");
        // Opted out earlier: a rerun without a choice keeps it off, and `--usage` turns it on.
        let kept = planned(&on, UsageHook::Keep, true);
        assert!(!kept.usage_before && !kept.usage && !kept.change.changed());
        let again = planned(&on, UsageHook::Install, true);
        assert!(!again.usage_before && again.usage && !again.change.changed());
        // A first setup with usage off installs the hook for the answers alone.
        let first = planned("{}", UsageHook::Remove, false);
        assert!(!first.usage_before && !first.usage && !first.stop_before);
        assert!(document::has_owned_hook(
            &claude::DRIVER,
            &first.change.after,
            "Stop"
        ));
        // A legacy lifecycle-only install stays without the hook until usage is asked for.
        let legacy = planned(&lifecycle_only, UsageHook::Keep, false);
        assert!(!legacy.usage && !legacy.change.changed());
        assert!(!document::has_owned_hook(
            &claude::DRIVER,
            &legacy.change.after,
            "Stop"
        ));
    }
}
