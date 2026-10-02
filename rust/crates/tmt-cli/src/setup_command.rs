//! Setup consent and presentation; provider file policy belongs to adapters.
//! `tmt setup` without a driver is the guided flow in [`guided`].

mod guided;

use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    drivers::Registry,
    setup::{self, SetupEnvironment, UsageHook, record},
};

pub fn execute(
    provider: Option<String>,
    remove: bool,
    usage: UsageHook,
    yes: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    match run(provider.as_deref(), remove, usage, yes, mode) {
        Ok(()) => Ok(0),
        Err(error) => error.publish(mode),
    }
}

/// What the opt-in usage hook reads, shown wherever setup plans it.
const USAGE_NOTE: &str = "Usage: after each turn, TMT reads token counts from the end of the agent's own transcript; no transcript content is stored.";

fn failure(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("SETUP_ERROR", error.to_string(), 1).caused_by(error)
}

fn run(
    provider: Option<&str>,
    remove: bool,
    usage: UsageHook,
    yes: bool,
    mode: OutputMode,
) -> Result<(), Failure> {
    let drivers = Registry::builtin();
    let environment = SetupEnvironment::capture(&drivers).map_err(failure)?;
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
            setup::plan(
                provider,
                path,
                before,
                environment.launcher.clone(),
                remove,
                usage,
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
    // An unreadable record is preserved and stops setup before any change.
    let global = ConfigPaths::discover().map_err(Failure::from)?.global_dir;
    record::read(&global).map_err(failure)?;
    // Removing only the usage hook is a removal too.
    let removing = remove || (plan.usage_before && !plan.usage);
    if !mode.json {
        writeln!(output, "{} {} {} in {}\nLauncher: {}\nContext: identity, role summary, existing notes path and unread X counts. No message bodies or permission changes. Provider hook trust review still applies.",
            if removing { "Remove TMT-owned" } else { "Configure" }, plan.provider, plan.events(), plan.change.path.display(), plan.launcher.display()).map_err(failure)?;
        if plan.usage {
            writeln!(output, "{USAGE_NOTE}").map_err(failure)?;
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
        return Ok(());
    }
    let backup = setup::apply(&plan).map_err(failure)?;
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
    if mode.json {
        let mut document = json!({"provider": plan.provider, "changed": plan.change.changed(),
            "removed": remove, "settingsPath": plan.change.path, "launcher": plan.launcher, "backup": backup
        });
        // Additive: present only while the usage hook is installed.
        if plan.usage {
            document["usage"] = json!(true);
        }
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
        } else {
            writeln!(output, "Already current; no changes made.").map_err(failure)?;
        }
        if let Some(backup) = backup {
            writeln!(output, "Recoverable backup: {}", backup.display()).map_err(failure)?;
        }
    }
    Ok(())
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        "run tmt setup {} again",
        &[" again"],
        &[("{}", "codex")],
    )];

#[cfg(test)]
pub(crate) use guided::PRINTED_HINTS as GUIDED_HINTS;
