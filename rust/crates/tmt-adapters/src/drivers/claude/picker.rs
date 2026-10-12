//! Whether the user's Claude Code has the `tmt-picker` plugin. The plugin is
//! a user-installed Claude Code plugin, so TMT only notices when it is missing.
//! The check is read-only and bounded: it asks Claude only when `claude` is on
//! `PATH`, and any failure to learn the answer stays silent.

use crate::{
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
    skill_installation::{InstallReport, ProviderEnvironment},
};
use serde_json::Value;
use std::time::{Duration, Instant};

const BUDGET: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: usize = 256 * 1024;
const PLUGIN_ID_PREFIX: &str = "tmt-picker@";

/// The one line shown when Claude Code lacks the picker.
pub const MISSING_HINT: &str = "Claude Code is installed without the tmt-picker plugin. Add it: claude plugin marketplace add pj-tmt/tmt && claude plugin install tmt-picker@tmt";

/// [`MISSING_HINT`] when Claude Code is installed and its plugin list is
/// readable but holds no `tmt-picker`; otherwise nothing.
pub fn missing_hint(environment: &ProviderEnvironment) -> Option<&'static str> {
    missing_hint_with(environment, &UnixCommandRunner)
}

/// [`missing_hint`] for a skill installation that published into Claude Code's
/// own roots; an installation that did not reach Claude says nothing about it.
pub fn missing_hint_after_install(
    environment: &ProviderEnvironment,
    report: &InstallReport,
) -> Option<&'static str> {
    reached_claude(report)
        .then(|| missing_hint(environment))
        .flatten()
}

fn reached_claude(report: &InstallReport) -> bool {
    report.installed.iter().any(|item| {
        item.agent
            .is_some_and(|agent| std::ptr::eq(agent, &super::DRIVER))
    })
}

fn missing_hint_with(
    environment: &ProviderEnvironment,
    runner: &impl CommandRunner,
) -> Option<&'static str> {
    let claude = super::DRIVER
        .descriptor
        .executables
        .iter()
        .find_map(|name| environment.find_command(name))?;
    let output = runner
        .execute(CommandRequest {
            program: claude.as_os_str(),
            args: &["plugin".into(), "list".into(), "--json".into()],
            input: &[],
            deadline: Instant::now() + BUDGET,
            max_output_bytes: OUTPUT_LIMIT,
        })
        .ok()?;
    let Value::Array(plugins) = serde_json::from_slice(&output.stdout).ok()? else {
        return None;
    };
    // A list whose entries carry no id is a shape this check does not know.
    if !plugins.is_empty() && !plugins.iter().any(|plugin| plugin["id"].is_string()) {
        return None;
    }
    let installed = plugins.iter().any(|plugin| {
        plugin["id"]
            .as_str()
            .is_some_and(|id| id.starts_with(PLUGIN_ID_PREFIX))
    });
    (!installed).then_some(MISSING_HINT)
}

#[cfg(test)]
mod tests;
