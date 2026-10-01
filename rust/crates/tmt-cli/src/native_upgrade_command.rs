//! Native update composition; the new executable owns skill refresh and extensions.

mod extensions;

use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    native_install::{self, UpgradeReport, UpgradeRequest},
    process::{CommandFailure, CommandRequest, CommandRunner, UnixCommandRunner},
};
use tmt_core::native_install::Channel;

pub fn execute(
    channel: Option<Channel>,
    exact: Option<&str>,
    unpin: bool,
    yes: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    let interrupt = match tmt_adapters::interrupt::Interrupt::install() {
        Ok(interrupt) => interrupt,
        Err(error) => {
            return Failure::new("NATIVE_UPGRADE_FAILED", error.to_string(), 1)
                .caused_by(error)
                .publish(mode);
        }
    };
    let result = (|| {
        let executable = std::env::current_exe()?;
        native_install::upgrade(
            UpgradeRequest {
                executable: &executable,
                channel,
                exact,
                unpin,
            },
            || {
                if interrupt.is_interrupted() {
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Native update interrupted.",
                    ))
                } else {
                    Ok(())
                }
            },
        )
    })();
    let report = match result {
        Ok(report) => report,
        Err(mut error) => {
            let activated = error.activated.take();
            let status = if error.kind() == io::ErrorKind::Interrupted {
                130
            } else {
                1
            };
            let message = activated.as_deref().map_or_else(
                || error.to_string(),
                |report| format!("{} {}", error, retry_hint(report)),
            );
            let failure = Failure::new("NATIVE_UPGRADE_FAILED", message, status).caused_by(error);
            return publish(activated.as_deref(), None, Some(failure), mode);
        }
    };
    if report.skipped_pinned {
        drop(interrupt);
        return finish(&report, None, None, yes, mode);
    }
    if interrupt.is_interrupted() {
        return publish(
            Some(&report),
            None,
            Some(Failure::new(
                "NATIVE_UPGRADE_INTERRUPTED",
                format!(
                    "Native binary installation completed; managed skill refresh was interrupted. {}",
                    retry_hint(&report)
                ),
                130,
            )),
            mode,
        );
    }
    let refreshed =
        native_install::with_active_release(&report.installation.active_executable, || {
            refresh(&report.installation.active_executable, &UnixCommandRunner)
        })
        .unwrap_or_else(|error| Err((None, error)));
    let (skills, mut failure) = match refreshed {
        Ok(skills) => (Some(skills), None),
        Err((skills, cause)) => (skills, Some(Failure::new(
            "NATIVE_UPGRADE_SKILLS_FAILED",
            format!("Native binary installation completed, but managed skill refresh failed. User-owned skill content was not overwritten. {}", retry_hint(&report)), 1,
        ).caused_by(cause))),
    };
    if interrupt.is_interrupted() {
        failure = Some(Failure::new(
            "NATIVE_UPGRADE_INTERRUPTED",
            format!(
                "Native binary installation completed; update was interrupted during managed skill refresh. Inspect the skill report. {}",
                retry_hint(&report)
            ),
            130,
        ));
    }
    drop(interrupt);
    finish(&report, skills, failure, yes, mode)
}

fn finish(
    report: &UpgradeReport,
    skills: Option<Value>,
    failure: Option<Failure>,
    yes: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    let extensions = if failure.is_none() {
        extensions::upgrade(
            &report.installation.active_executable,
            yes,
            mode,
            &UnixCommandRunner,
        )
    } else {
        Vec::new()
    };
    publish_products(Some(report), skills, failure, extensions, mode)
}

fn refresh(
    executable: &Path,
    runner: &impl CommandRunner,
) -> Result<Value, (Option<Value>, io::Error)> {
    let result = runner.execute(CommandRequest {
        program: executable.as_os_str(),
        args: &["__native-refresh-skills".into(), "--json".into()],
        input: &[],
        deadline: Instant::now() + Duration::from_secs(30),
        max_output_bytes: 4 * 1024 * 1024,
    });
    let (output, failure) = match result {
        Ok(output) => (output, None),
        Err(mut error) => {
            if !matches!(
                error.kind,
                CommandFailure::Exit {
                    code: Some(1),
                    signal: None
                }
            ) || error.cleanup_failed()
            {
                return Err((None, io::Error::other(error)));
            }
            let Some(output) = error.output.take() else {
                return Err((None, io::Error::other(error)));
            };
            (output, Some(error))
        }
    };
    let document = crate::skill_refresh_command::parse_document(&output.stdout)
        .filter(|value| {
            output.stderr.is_empty() && value.get("error").is_some() == failure.is_some()
        })
        .ok_or_else(|| {
            (
                None,
                io::Error::other("New executable returned an invalid managed-skill report."),
            )
        })?;
    match failure {
        None => Ok(document),
        Some(error) => Err((Some(document), io::Error::other(error))),
    }
}

fn publish(
    report: Option<&UpgradeReport>,
    skills: Option<Value>,
    failure: Option<Failure>,
    mode: OutputMode,
) -> io::Result<u8> {
    publish_products(report, skills, failure, Vec::new(), mode)
}

fn publish_products(
    report: Option<&UpgradeReport>,
    skills: Option<Value>,
    failure: Option<Failure>,
    extensions: Vec<Value>,
    mode: OutputMode,
) -> io::Result<u8> {
    let mut document = report.map_or_else(|| json!({"changed": false}), |report| json!({
        "executable": report.installation.executable, "version": report.installation.version,
        "changed": report.installation.changed, "channel": report.state.channel.as_str(),
        "pinnedVersion": report.state.pinned_version.as_ref().map(ToString::to_string),
        "pinned": report.state.pinned_version.is_some(),
        "skippedPinned": report.skipped_pinned,
    }));
    document["skills"] = skills.unwrap_or(Value::Null);
    let warning = report.and_then(|report| path_warning(&report.installation.executable));
    document["pathWarning"] = warning.clone().into();
    if let Some(failure) = &failure {
        document["error"] = failure.document()["error"].clone();
    }
    let products = product_rows(report, failure.as_ref(), extensions);
    let extension_failed = products.iter().skip(1).any(|p| p["status"] == "failed");
    document["products"] = json!(products);
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    let terminal = stdout.terminal();
    if mode.json {
        writeln!(stdout, "{document}")?;
    } else {
        if let Some(report) = report {
            let what = format!(
                "tmt {} at {}",
                report.installation.version,
                report.installation.executable.display()
            );
            if report.skipped_pinned {
                writeln!(stdout, "Pinned {what}")?;
            } else if report.installation.changed {
                tmt_cli_style::message::success(&mut stdout, terminal, &format!("Updated {what}"))?;
            } else {
                writeln!(stdout, "Current {what}")?;
            }
            if !report.skipped_pinned
                && let Some(refreshed) = document["skills"]["refreshed"].as_array()
            {
                let skipped = document["skills"]["skipped"].as_array().map_or(0, Vec::len);
                let conflicts = document["skills"]["conflicts"]
                    .as_array()
                    .expect("validated skill report");
                writeln!(
                    stdout,
                    "Managed skills: {} current/refreshed, {skipped} missing, {} conflicts preserved.",
                    refreshed.len(),
                    conflicts.len()
                )?;
                for target in conflicts {
                    tmt_cli_style::message::hint(
                        &mut stdout,
                        terminal,
                        &format!(
                            "resolve the skill conflict at {}",
                            target.as_str().expect("validated skill path")
                        ),
                    )?;
                }
                if !refreshed.is_empty() {
                    tmt_cli_style::message::hint(
                        &mut stdout,
                        terminal,
                        "reload or restart your agent to use updated guidance; existing conversations can read tmt learn --skill",
                    )?;
                }
            }
        }
        for product in products.iter().skip(1) {
            if let Some(message) = product["message"].as_str() {
                writeln!(stdout, "{message}")?;
            } else {
                writeln!(
                    stdout,
                    "{}: {}{}",
                    product["product"].as_str().unwrap_or("extension"),
                    product["status"].as_str().unwrap_or("failed"),
                    product["version"]
                        .as_str()
                        .map(|v| format!(" ({v})"))
                        .unwrap_or_default()
                )?;
            }
            if product["message"].is_null()
                && let Some(hint) = product["hint"].as_str()
            {
                tmt_cli_style::message::hint(&mut stdout, terminal, hint)?;
            }
            if product["status"] == "failed" {
                writeln!(
                    stdout,
                    "{}: {}",
                    product["error"]["code"]
                        .as_str()
                        .unwrap_or("EXTENSION_UPGRADE_FAILED"),
                    product["error"]["message"]
                        .as_str()
                        .unwrap_or("Extension update failed")
                )?;
            }
        }
        if let Some(warning) = warning {
            drop(stdout);
            let mut stderr = tmt_cli_style::stream::stderr();
            let terminal = stderr.terminal();
            tmt_cli_style::message::warning(&mut stderr, terminal, &warning, None)?;
        }
        if let Some(failure) = &failure {
            failure.publish(mode)?;
        }
    }
    Ok(failure.map_or(u8::from(extension_failed), |failure| failure.status))
}

fn product_rows(
    report: Option<&UpgradeReport>,
    failure: Option<&Failure>,
    extensions: Vec<Value>,
) -> Vec<Value> {
    let status = if failure.is_some() {
        "failed"
    } else if report.is_some_and(|report| report.skipped_pinned) {
        "skippedPinned"
    } else if report.is_some_and(|report| report.installation.changed) {
        "changed"
    } else {
        "unchanged"
    };
    let mut products = vec![json!({
        "product": "cli",
        "status": status,
        "version": report.map(|report| &report.installation.version),
        "error": failure.map(|failure| failure.document()["error"].clone()),
    })];
    products.extend(extensions);
    products
}

fn path_warning(executable: &Path) -> Option<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let selected = std::env::split_paths(&path)
        .map(|directory| directory.join("tmt"))
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        });
    let expected = fs::canonicalize(executable).ok();
    if expected.is_some() && selected.and_then(|path| fs::canonicalize(path).ok()) == expected {
        return None;
    }
    Some(format!(
        "PATH does not select this managed tmt. Use {} directly or put its bin directory first; no shell profile or package-manager files were changed.",
        executable.display()
    ))
}

fn retry_hint(report: &UpgradeReport) -> String {
    match &report.state.pinned_version {
        Some(version) => format!(
            "Run the current managed tmt with upgrade --to {version} to retry without clearing the pin."
        ),
        None => "Run the current managed tmt upgrade to retry.".into(),
    }
}

#[cfg(test)]
#[path = "native_upgrade_command_tests.rs"]
mod tests;
