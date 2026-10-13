//! Bounded Remote-owned restart after actual replacement, using one prior observation.

use super::{InstallReport, Product};
use crate::process::{CommandFailure, CommandRequest, CommandRunner, UnixCommandRunner};
use serde_json::Value;
use std::{
    ffi::{OsStr, OsString},
    path::Path,
    time::{Duration, Instant},
};
use tmt_core::native_install::PostUpgradeCheck;

const RUNNING: &str = "Remote was upgraded while its door is running. Finish active pairing and held approvals, then restart: tmt remote stop && tmt remote serve.";
const UNKNOWN: &str = "Remote was upgraded, but its door status could not be confirmed. Finish active pairing and held approvals, then restart if running: tmt remote stop && tmt remote serve.";
const OUTDATED: &str = "Remote was upgraded while a legacy door is running. Finish active pairing and held approvals, then press Ctrl-C in its terminal and start it again with: tmt remote serve. Do not start a second door.";
const RESTARTED: &str = "Remote upgraded and restarted in the background.";
const BUDGET: Duration = Duration::from_secs(2);
// Existing Remote phases: stop, startup, startup cleanup, two core calls and
// two control replies. Core neither supervises a PID nor retries the effect.
pub const POST_UPGRADE_RESTART_BOUND: Duration = Duration::from_secs(40 + 35 + 40 + 2 * 15 + 2 * 5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostUpgradeObservation {
    #[default]
    NotApplicable,
    Running,
    Stopped,
    Legacy,
    Unknown,
}

/// Capture once through the verified existing executable before publication.
/// The CLI passes this observation through activation and partial-failure handling.
pub fn observe_before_upgrade(
    product: Product,
    executable: &Path,
    core_executable: Option<&Path>,
) -> PostUpgradeObservation {
    capture(product, executable, core_executable, &UnixCommandRunner)
}

fn capture(
    product: Product,
    executable: &Path,
    core_executable: Option<&Path>,
    runner: &impl CommandRunner,
) -> PostUpgradeObservation {
    if product.post_upgrade_check() != Some(PostUpgradeCheck::RemoteDoor) {
        return PostUpgradeObservation::NotApplicable;
    }
    let Some(core) = core_executable else {
        return PostUpgradeObservation::Unknown;
    };
    match invoke(executable, core, &["status", "--json"], BUDGET, runner) {
        Ok(output) => remote_status(&output.stdout),
        Err(error)
            if !error.cleanup_failed()
                && matches!(
                    error.kind,
                    CommandFailure::Exit {
                        code: Some(1),
                        signal: None
                    }
                )
                && error.output.as_ref().is_some_and(|output| {
                    serde_json::from_slice::<Value>(&output.stdout)
                        .is_ok_and(|value| value["error"]["code"] == "REMOTE_SERVE_OUTDATED")
                }) =>
        {
            PostUpgradeObservation::Legacy
        }
        Err(_) => PostUpgradeObservation::Unknown,
    }
}

/// No install lock is held. Only actual replacement of a previously observed
/// running Remote admits the single captured-executable restart command.
pub fn post_upgrade_hint(
    product: Product,
    report: &InstallReport,
    replacing: bool,
    core_executable: Option<&Path>,
    before: PostUpgradeObservation,
) -> Option<&'static str> {
    observe(
        product,
        report,
        replacing,
        core_executable,
        before,
        &UnixCommandRunner,
    )
}

fn observe(
    product: Product,
    report: &InstallReport,
    replacing: bool,
    core_executable: Option<&Path>,
    before: PostUpgradeObservation,
    runner: &impl CommandRunner,
) -> Option<&'static str> {
    if !replacing || !report.changed || product.post_upgrade_check().is_none() {
        return None;
    }
    match before {
        PostUpgradeObservation::NotApplicable | PostUpgradeObservation::Stopped => None,
        PostUpgradeObservation::Legacy => Some(OUTDATED),
        PostUpgradeObservation::Unknown => Some(UNKNOWN),
        PostUpgradeObservation::Running => {
            let Some(core) = core_executable else {
                return Some(UNKNOWN);
            };
            let hint = match invoke(
                &report.active_executable,
                core,
                &["restart", "--json", "--no-open"],
                POST_UPGRADE_RESTART_BOUND,
                runner,
            ) {
                Ok(output) => remote_restart(&output.stdout),
                Err(_) => Some(UNKNOWN),
            };
            if hint == Some(UNKNOWN) {
                let _ = invoke(
                    &report.active_executable,
                    core,
                    &["status", "--json"],
                    BUDGET,
                    runner,
                );
            }
            hint
        }
    }
}

fn invoke(
    executable: &Path,
    core: &Path,
    command: &[&str],
    budget: Duration,
    runner: &impl CommandRunner,
) -> Result<crate::process::CommandOutput, crate::process::CommandError> {
    let program = super::env_program_operand(executable)
        .map_err(|_| crate::process::CommandError::new(CommandFailure::Spawn))?;
    let mut core_argument = OsString::from("TMT_EXECUTABLE=");
    core_argument.push(core);
    let mut args = vec![
        "-u".into(),
        "GH_TOKEN".into(),
        "-u".into(),
        "GITHUB_TOKEN".into(),
        core_argument,
        program.to_owned(),
    ];
    args.extend(command.iter().map(OsString::from));
    runner.execute(CommandRequest {
        program: OsStr::new("/usr/bin/env"),
        args: &args,
        input: &[],
        deadline: Instant::now() + budget,
        max_output_bytes: 64 * 1024,
    })
}

fn remote_status(bytes: &[u8]) -> PostUpgradeObservation {
    let Ok(Value::Object(value)) = serde_json::from_slice(bytes) else {
        return PostUpgradeObservation::Unknown;
    };
    // Required members stay strict; status projections are additive.
    match value.get("running").and_then(Value::as_bool) {
        Some(true)
            if value.get("origin").is_some_and(Value::is_string)
                && value.get("path").is_some_and(Value::is_string) =>
        {
            PostUpgradeObservation::Running
        }
        Some(false)
            if value.get("lastPort").is_some_and(|port| {
                port.is_null()
                    || port
                        .as_u64()
                        .is_some_and(|port| (1..=65535).contains(&port))
            }) =>
        {
            PostUpgradeObservation::Stopped
        }
        _ => PostUpgradeObservation::Unknown,
    }
}

fn remote_restart(bytes: &[u8]) -> Option<&'static str> {
    let Ok(Value::Object(value)) = serde_json::from_slice(bytes) else {
        return Some(UNKNOWN);
    };
    // Required members are strict; additive members remain forward compatible.
    match (
        value.get("outcome").and_then(Value::as_str),
        value.get("reason"),
    ) {
        (Some("restarted"), Some(Value::Null)) => Some(RESTARTED),
        (Some("not-running"), Some(Value::Null)) => None,
        (Some("deferred"), Some(Value::String(reason))) if reason == "legacy" => Some(OUTDATED),
        (Some("deferred"), Some(Value::String(reason)))
            if matches!(reason.as_str(), "busy" | "foreground") =>
        {
            Some(RUNNING)
        }
        (Some("unconfirmed"), Some(Value::String(reason)))
            if matches!(reason.as_str(), "candidate" | "startup" | "unknown") =>
        {
            Some(UNKNOWN)
        }
        _ => Some(UNKNOWN),
    }
}

#[cfg(test)]
mod tests;
