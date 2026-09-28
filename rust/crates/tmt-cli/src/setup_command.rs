//! Setup consent and presentation; provider file policy belongs to adapters.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::io::{self, IsTerminal, Write};
use tmt_adapters::{
    setup::{self, SetupEnvironment},
    skill_installation::ProviderEnvironment,
};

pub fn execute(
    provider: Option<String>,
    remove: bool,
    yes: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    match run(provider.as_deref(), remove, yes, mode) {
        Ok(()) => Ok(0),
        Err(error) => error.publish(mode),
    }
}

fn failure(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("SETUP_ERROR", error.to_string(), 1).caused_by(error)
}

fn run(provider: Option<&str>, remove: bool, yes: bool, mode: OutputMode) -> Result<(), Failure> {
    let environment = SetupEnvironment::capture().map_err(failure)?;
    let before = setup::read_settings(&environment.claude_settings).map_err(failure)?;
    let plan = setup::claude_plan(
        environment.claude_settings,
        before,
        environment.launcher,
        remove,
    )
    .map_err(failure)?;
    let mut output = io::stdout().lock();
    if provider.is_none() {
        let detected = ProviderEnvironment::capture().map_err(failure)?.detect();
        let providers: Vec<_> = detected.iter().map(|value| value.as_str()).collect();
        if mode.json {
            writeln!(
                output,
                "{}",
                json!({"detectedProviders": providers, "integrations": [{
                    "provider": plan.provider, "current": !plan.change.changed(),
                    "settingsPath": plan.change.path, "launcher": plan.launcher
                }]})
            )
            .map_err(failure)?;
        } else {
            writeln!(
                output,
                "Detected agents: {}",
                if providers.is_empty() {
                    "none".into()
                } else {
                    providers.join(", ")
                }
            )
            .map_err(failure)?;
            writeln!(
                output,
                "Claude hooks: {} ({})",
                if plan.change.changed() {
                    "not current; run tmt setup claude"
                } else {
                    "current"
                },
                plan.change.path.display()
            )
            .map_err(failure)?;
        }
        return Ok(());
    }
    if !mode.json {
        writeln!(output, "{} Claude SessionStart and SessionEnd hooks in {}\nLauncher: {}\nContext: identity, role summary, existing notes path and unread X counts. No message bodies or permission changes.",
            if remove { "Remove TMT-owned" } else { "Configure" }, plan.change.path.display(), plan.launcher.display()).map_err(failure)?;
    }
    if !yes {
        if mode.json || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(Failure::new(
                "SETUP_CONSENT_REQUIRED",
                "Review setup interactively, or pass --yes to approve the requested provider changes.",
                1,
            ));
        }
        write!(output, "Apply this plan? [y/N] ").map_err(failure)?;
        output.flush().map_err(failure)?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer).map_err(failure)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            writeln!(output, "No changes made.").map_err(failure)?;
            return Ok(());
        }
    }
    let backup = setup::apply(&plan).map_err(failure)?;
    if mode.json {
        writeln!(output, "{}", json!({"provider": plan.provider, "changed": plan.change.changed(),
            "removed": remove, "settingsPath": plan.change.path, "launcher": plan.launcher, "backup": backup
        })).map_err(failure)?;
    } else {
        writeln!(
            output,
            "{}",
            if plan.change.changed() {
                "Setup applied."
            } else {
                "Already current; no changes made."
            }
        )
        .map_err(failure)?;
        if let Some(backup) = backup {
            writeln!(output, "Recoverable backup: {}", backup.display()).map_err(failure)?;
        }
    }
    Ok(())
}
