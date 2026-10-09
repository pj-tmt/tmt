//! Read-only, bounded notices after a product's actual release replacement.

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
const BUDGET: Duration = Duration::from_secs(2);

/// The caller supplies a verified activation report and whether its version
/// replaced an existing one. No install lock is held here; initial installs and no-ops
/// do not probe. Use the captured immutable executable, never PATH discovery.
pub fn post_upgrade_hint(
    product: Product,
    report: &InstallReport,
    replacing: bool,
    core_executable: Option<&Path>,
) -> Option<&'static str> {
    observe(
        product,
        report,
        replacing,
        core_executable,
        &UnixCommandRunner,
    )
}

fn observe(
    product: Product,
    report: &InstallReport,
    replacing: bool,
    core_executable: Option<&Path>,
    runner: &impl CommandRunner,
) -> Option<&'static str> {
    if !replacing || !report.changed {
        return None;
    }
    let check = product.post_upgrade_check()?;
    match check {
        PostUpgradeCheck::RemoteDoor => {
            let Some(core_executable) = core_executable else {
                return Some(UNKNOWN);
            };
            let mut core = OsString::from("TMT_EXECUTABLE=");
            core.push(core_executable);
            let args = [
                core,
                report.active_executable.as_os_str().to_owned(),
                "status".into(),
                "--json".into(),
            ];
            match runner.execute(CommandRequest {
                program: OsStr::new("/usr/bin/env"),
                args: &args,
                input: &[],
                deadline: Instant::now() + BUDGET,
                max_output_bytes: 64 * 1024,
            }) {
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
                            serde_json::from_slice::<Value>(&output.stdout).is_ok_and(|value| {
                                value["error"]["code"] == "REMOTE_SERVE_OUTDATED"
                            })
                        }) =>
                {
                    Some(OUTDATED)
                }
                Err(_) => Some(UNKNOWN),
            }
        }
    }
}

fn remote_status(bytes: &[u8]) -> Option<&'static str> {
    let Ok(Value::Object(value)) = serde_json::from_slice(bytes) else {
        return Some(UNKNOWN);
    };
    match value.get("running").and_then(Value::as_bool) {
        Some(true)
            if value.len() == 3
                && value.get("origin").is_some_and(Value::is_string)
                && value.get("path").is_some_and(Value::is_string) =>
        {
            Some(RUNNING)
        }
        Some(false)
            if value.len() == 2
                && value.get("lastPort").is_some_and(|port| {
                    port.is_null()
                        || port
                            .as_u64()
                            .is_some_and(|port| (1..=65535).contains(&port))
                }) =>
        {
            None
        }
        _ => Some(UNKNOWN),
    }
}

#[cfg(test)]
mod tests;
