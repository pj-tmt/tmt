//! Removing every TMT lifecycle hook for a full uninstall: the recorded
//! installations, plus exact TMT hooks found without a record in each
//! driver's settings file (installed before the record existed). A hook that
//! differs from what setup generates is kept and reported.

use super::{PlanError, SetupPlan, apply, plan, publication::remove_lock, read_settings, record};
use crate::{drivers::Registry, skill_installation::ProviderEnvironment};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum HookStep {
    /// Remove TMT's hooks from this file with this plan.
    Remove(SetupPlan),
    /// Recorded, but nothing of TMT's is left in the file.
    Forget { driver: String, settings: PathBuf },
    /// Left alone: the file or a hook in it is not what setup wrote.
    Keep {
        driver: String,
        settings: PathBuf,
        reason: String,
    },
}

/// Read-only. Fails on an unreadable record; a settings file that cannot be
/// planned is kept, not an error.
pub fn plan_hook_removal(
    global: &Path,
    environment: &ProviderEnvironment,
    drivers: &Registry,
) -> io::Result<Vec<HookStep>> {
    let recorded = record::read(global)?;
    let mut candidates: Vec<(String, PathBuf)> = recorded
        .iter()
        .map(|entry| (entry.driver.clone(), entry.settings.clone()))
        .collect();
    for driver in drivers.with_hooks() {
        if let Some(settings) = environment.locations(driver).hook_settings {
            candidates.push((driver.name().to_owned(), settings));
        }
    }
    candidates.sort();
    candidates.dedup();
    let mut steps = Vec::new();
    for (name, settings) in candidates {
        let is_recorded = recorded
            .iter()
            .any(|entry| entry.driver == name && entry.settings == settings);
        let keep = |reason: &str| HookStep::Keep {
            driver: name.clone(),
            settings: settings.clone(),
            reason: reason.to_owned(),
        };
        let Some(driver) = drivers.find(&name) else {
            steps.push(keep("its driver is not known to this TMT"));
            continue;
        };
        let before = match read_settings(&settings) {
            Ok(before) => before,
            Err(error) => {
                steps.push(keep(&error.to_string()));
                continue;
            }
        };
        match plan(
            driver,
            settings.clone(),
            before,
            PathBuf::from("/"),
            true,
            super::UsageHook::Keep,
        ) {
            Ok(planned) if planned.change.changed() => steps.push(HookStep::Remove(planned)),
            Ok(_) if is_recorded => steps.push(HookStep::Forget {
                driver: name.clone(),
                settings: settings.clone(),
            }),
            Ok(_) => {}
            Err(PlanError::EditedHook) => steps.push(keep("a TMT hook in it was edited")),
            Err(error) => steps.push(keep(&error.to_string())),
        }
    }
    Ok(steps)
}

/// Carries out one step and updates the record. A file that held nothing but
/// TMT's hooks (`{}` once they are removed) is deleted rather than rewritten.
/// Once no TMT hook is left in a directory, setup's lock there goes too.
pub fn remove_hooks(global: &Path, step: &HookStep) -> io::Result<()> {
    match step {
        HookStep::Remove(planned) => {
            if planned.change.after == "{}" {
                if read_settings(&planned.change.path)? != planned.change.before {
                    return Err(io::Error::other(
                        "Provider settings changed after planning; run uninstall again.",
                    ));
                }
                fs::remove_file(&planned.change.path)?;
            } else {
                apply(planned)?;
            }
            record::forget(global, planned.provider, &planned.change.path)?;
            remove_lock(&planned.change.path)
        }
        HookStep::Forget { driver, settings } => {
            record::forget(global, driver, settings)?;
            remove_lock(settings)
        }
        HookStep::Keep { .. } => Ok(()),
    }
}
