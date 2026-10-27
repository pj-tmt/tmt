//! Setup consent and presentation; provider file policy belongs to adapters.
//! `tmt setup` without a driver is the guided flow in [`guided`].

pub(crate) mod guided;
pub(crate) mod refresh;

use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    drivers::Registry,
    setup::{
        self, SetupEnvironment, UsageHook,
        provider_mod::{self, ModStep, PendingMod},
        record,
    },
    skill_installation::ProviderEnvironment,
};

pub fn execute(
    provider: Option<String>,
    status: bool,
    remove: bool,
    usage: UsageHook,
    yes: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    match run(provider.as_deref(), status, remove, usage, yes, mode) {
        Ok(status) => Ok(status),
        Err(error) => error.publish(mode),
    }
}

/// What the usage hook reads, shown wherever setup plans it.
pub(crate) const USAGE_NOTE: &str = "Usage and recent responses: after each turn, TMT reads token counts and keeps this agent's answers on this computer (up to 1,000 per agent, 16 KiB each, for 7 days). Use --no-usage to stop collecting.";

/// What the turn-end hook keeps when usage collection is off: the agent's answers for the chat.
pub(crate) const ANSWERS_NOTE: &str = "Chat answers: after each turn, TMT keeps this agent's answers on this computer (up to 1,000 per agent, 16 KiB each, for 7 days). No usage data is read or stored.";

/// What setup says about the managed mod a driver keeps once usage is consented (#5187).
fn mod_note(pending: &PendingMod) -> String {
    let provider = pending.provider();
    let target = pending.target().display();
    match pending.step {
        ModStep::Install | ModStep::Refresh => format!(
            "{provider} mod: {}. TMT keeps a managed copy in {target}; turning usage off, --remove and uninstall remove it.",
            pending.purpose()
        ),
        ModStep::Remove => format!("Removes TMT's managed {provider} mod in {target}."),
        ModStep::Blocked => format!(
            "Leaves {target} as it is: it is not TMT's, so the {provider} mod is not installed."
        ),
    }
}

/// What setup adds once a mod was installed or refreshed (#5394): when it starts to work.
fn activation_note(
    pending: &PendingMod,
    outcome: provider_mod::ModOutcome,
) -> Option<&'static str> {
    matches!(
        outcome,
        provider_mod::ModOutcome::Installed | provider_mod::ModOutcome::Refreshed
    )
    .then(|| pending.activation())
    .flatten()
}

fn failure(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("SETUP_ERROR", error.to_string(), 1).caused_by(error)
}

fn run(
    provider: Option<&str>,
    status: bool,
    remove: bool,
    usage: UsageHook,
    yes: bool,
    mode: OutputMode,
) -> Result<u8, Failure> {
    let drivers = Registry::builtin();
    if status {
        return show_status(&drivers, provider, mode).map(|()| 0);
    }
    let environment = SetupEnvironment::capture(&drivers).map_err(failure)?;
    let global = ConfigPaths::discover().map_err(Failure::from)?.global_dir;
    let recorded = record::read(&global).map_err(failure)?;
    if provider.is_none() {
        let mut output = tmt_cli_style::stream::stdout(mode.json);
        let terminal = output.terminal();
        return guided::run(
            &drivers,
            &environment,
            usage,
            yes,
            mode,
            &mut output,
            terminal,
        );
    }
    let selected = provider.and_then(|name| drivers.find(name));
    let plans = selected
        .map_or_else(|| drivers.with_hooks().collect(), |value| vec![value])
        .into_iter()
        .map(|provider| {
            let path = environment
                .settings_path(provider)
                .map_err(failure)?
                .to_path_buf();
            let before = setup::read_settings(&path).map_err(failure)?;
            let opted_out = record::usage_opted_out(&recorded, provider.name(), &path);
            let usage = record::usage_policy(&recorded, provider.name(), &path, usage);
            setup::plan_with_choice(
                provider,
                path,
                before,
                environment.launcher.clone(),
                remove,
                usage,
                opted_out,
            )
            .map_err(failure)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let terminal = output.terminal();
    let plan = plans
        .into_iter()
        .next()
        .expect("one selected provider plan");
    // The record was validated before planning any settings changes.
    let removing = remove;
    let provider_mod =
        provider_mod::pending(plan.provider, &plan.change.path, !remove && plan.usage)
            .map_err(failure)?;
    if !mode.json {
        writeln!(output, "{} {} {} in {}\nLauncher: {}\nContext: identity, role summary, existing notes path and unread X counts. No message bodies or permission changes. Provider hook trust review still applies.",
            if removing { "Remove TMT-owned" } else { "Configure" }, plan.provider, plan.events(), plan.change.path.display(), plan.launcher.display()).map_err(failure)?;
        if plan.change.before.is_some() && plan.change.changed() {
            writeln!(
                output,
                "Recoverable backups: {} (warning above 32; manual cleanup only).",
                setup::backup_directory(&plan.change.path)
                    .expect("settings parent")
                    .display()
            )
            .map_err(failure)?;
        }
        if plan.usage {
            writeln!(output, "{USAGE_NOTE}").map_err(failure)?;
        } else if !remove {
            writeln!(output, "{ANSWERS_NOTE}").map_err(failure)?;
            writeln!(output, "{}", collection_off(plan.provider)).map_err(failure)?;
        }
        if let Some(pending) = &provider_mod {
            writeln!(output, "{}", mod_note(pending)).map_err(failure)?;
        }
    }
    if !crate::consent::ask(
        &mut output,
        yes,
        mode,
        crate::consent::Consent {
            code: "SETUP_CONSENT_REQUIRED",
            refusal: "Review setup interactively, or pass --yes to approve the requested provider changes.",
            question: "Apply this plan",
            declined: "No changes made.",
        },
        failure,
    )? {
        return Ok(0);
    }
    let backup = setup::apply(&plan).map_err(failure)?;
    let warning = setup::backup_warning(&plan.change.path);
    // Recorded after publication; exact hooks already present are adopted.
    let recorded = if remove {
        record::forget(&global, plan.provider, &plan.change.path)
    } else {
        record::remember(
            &global,
            record::RecordedHooks {
                driver: plan.provider.to_owned(),
                settings: plan.change.path.clone(),
                launcher: plan.launcher.clone(),
                usage: Some(plan.usage),
            },
        )
    };
    recorded.map_err(|error| {
        Failure::new(
            "SETUP_ERROR",
            format!(
                "The provider settings were updated, but the setup record could not be: {error}"
            ),
            1,
        )
        .suggestion(format!("run tmt setup {} again", plan.provider))
    })?;
    // The hooks are in place; the mod follows them and its failure leaves them alone.
    let mod_outcome = provider_mod
        .as_ref()
        .map(|pending| pending.apply())
        .transpose()
        .map_err(|error| {
            Failure::new(
                "SETUP_ERROR",
                format!(
                    "The {} hooks were updated, but the mod could not be: {error}",
                    plan.provider
                ),
                1,
            )
            .suggestion(format!("run tmt setup {} again", plan.provider))
        })?;
    let mod_changed = provider_mod
        .as_ref()
        .zip(mod_outcome)
        .is_some_and(|(pending, outcome)| {
            pending.changes() && outcome != provider_mod::ModOutcome::Blocked
        });
    if mode.json {
        let mut document = json!({"provider": plan.provider, "changed": plan.change.changed(),
            "removed": remove, "settingsPath": plan.change.path, "launcher": plan.launcher, "backup": backup
        });
        if let (Some(pending), Some(outcome)) = (&provider_mod, mod_outcome) {
            document["mod"] = json!({"path": pending.target(), "status": outcome.label()});
            if let Some(note) = activation_note(pending, outcome) {
                document["mod"]["note"] = json!(note);
            }
        }
        if let Some(warning) = &warning {
            document["warnings"] = json!([warning]);
        }
        collection_document(&mut document, plan.provider, plan.usage);
        writeln!(output, "{document}").map_err(failure)?;
    } else {
        if plan.change.changed() {
            tmt_cli_style::message::success(
                &mut output,
                terminal,
                &format!(
                    "{} {} hooks",
                    if remove { "Removed" } else { "Configured" },
                    plan.provider
                ),
            )
            .map_err(failure)?;
        } else if !mod_changed {
            writeln!(output, "Already current; no changes made.").map_err(failure)?;
        }
        if let (Some(pending), Some(outcome)) = (&provider_mod, mod_outcome) {
            let provider = pending.provider();
            match outcome {
                provider_mod::ModOutcome::Blocked => tmt_cli_style::message::warning(
                    &mut output,
                    terminal,
                    &format!(
                        "Left {} as it is: it is not TMT's, so the {provider} mod was not installed",
                        pending.target().display()
                    ),
                    None,
                )
                .map_err(failure)?,
                done => {
                    tmt_cli_style::message::success(
                        &mut output,
                        terminal,
                        &format!("{} the {provider} mod", capitalized(done.label())),
                    )
                    .map_err(failure)?;
                    if let Some(note) = activation_note(pending, done) {
                        writeln!(output, "{note}").map_err(failure)?;
                    }
                }
            }
        }
        if let Some(warning) = &warning {
            tmt_cli_style::message::warning(&mut output, terminal, warning, None)
                .map_err(failure)?;
        }
        if let Some(backup) = backup {
            writeln!(output, "Recoverable backup: {}", backup.display()).map_err(failure)?;
        }
    }
    Ok(0)
}

fn capitalized(word: &str) -> String {
    let mut letters = word.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

fn collection_off(provider: &str) -> String {
    format!(
        "{provider}: consumption collection is off; enable it with: tmt setup {provider} --usage"
    )
}

fn collection_document(document: &mut serde_json::Value, provider: &str, enabled: bool) {
    document["usage"] = json!(enabled);
    if !enabled {
        document["diagnostic"] = json!(collection_off(provider));
    }
}

/// Settings-only inspection: no consent, provider execution, record adoption or DB.
fn show_status(
    drivers: &Registry,
    provider: Option<&str>,
    mode: OutputMode,
) -> Result<(), Failure> {
    let selected: Vec<_> = match provider {
        Some(name) => drivers.find(name).into_iter().collect(),
        None => drivers.with_hooks().collect(),
    };
    let mut integrations = Vec::new();
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let recorded = record::read(&paths.global_dir).map_err(failure)?;
    for driver in selected {
        let path = setup::provider_settings(driver).map_err(failure)?;
        let before = setup::read_settings(&path).map_err(failure)?;
        // Keep validates installed entries without proposing a usage opt-in; the turn-end hook
        // may be there with usage turned off, which only the setup record says.
        let plan = setup::plan_with_choice(
            driver,
            path.clone(),
            before,
            std::path::PathBuf::from("/"),
            false,
            UsageHook::Keep,
            record::usage_opted_out(&recorded, driver.name(), &path),
        )
        .map_err(failure)?;
        let mut row = json!({"provider": driver.name(), "settingsPath": path});
        collection_document(&mut row, driver.name(), plan.usage_before);
        if let Some(state) = provider_mod::state(driver.name(), &path).map_err(failure)? {
            row["mod"] = json!({"state": state.label()});
            // Read-only: an installed mod that has pushed nothing yet is not broken, it is
            // waiting for a provider session to start with it.
            if plan.usage_before
                && state.installed()
                && ProviderEnvironment::capture().is_ok_and(|environment| {
                    provider_mod::waiting_for_first_session(
                        driver.name(),
                        &environment,
                        &paths.global_dir,
                    )
                })
            {
                row["mod"]["waitingForFirstSession"] = json!(true);
            }
        }
        integrations.push(row);
    }
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(output, "{}", json!({"integrations": integrations})).map_err(failure)?;
    } else {
        for row in integrations {
            let provider = row["provider"].as_str().expect("driver name");
            if row["usage"] == true {
                writeln!(output, "{provider}: consumption collection is on").map_err(failure)?;
            } else {
                writeln!(output, "{}", collection_off(provider)).map_err(failure)?;
            }
            if row["mod"]["waitingForFirstSession"] == true {
                writeln!(
                    output,
                    "{provider}: waiting for the first {} session since setup",
                    capitalized(provider)
                )
                .map_err(failure)?;
            }
        }
    }
    Ok(())
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "{provider}: consumption collection is off; enable it with: tmt setup {provider} --usage",
        &[""],
        &[("{provider}", "codex")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "run tmt setup {} again",
        &[" again"],
        &[("{}", "codex")],
    ),
];

#[cfg(test)]
pub(crate) use guided::PRINTED_HINTS as GUIDED_HINTS;

#[cfg(test)]
pub(crate) use guided::EXTENSION_HINTS as GUIDED_EXTENSION_HINTS;
