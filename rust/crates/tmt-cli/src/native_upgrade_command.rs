//! Native update composition; the new executable owns skill refresh and extensions.

mod database;
mod extensions;
mod rename;

use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{
    error::Error,
    fs,
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    drivers::claude::picker,
    native_install::{self, UpgradeReport, UpgradeRequest},
    process::{CommandFailure, CommandRequest, CommandRunner, UnixCommandRunner},
    skill_installation::ProviderEnvironment,
};
use tmt_core::{native_install::Channel, skill_catalog::Group};

pub fn execute_with_schema_consent(
    channel: Option<Channel>,
    exact: Option<&str>,
    unpin: bool,
    yes: bool,
    allow_schema_ahead: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    if allow_schema_ahead {
        writeln!(
            tmt_cli_style::stream::stderr(),
            "Warning: allowing a PR application schema ahead of latest alpha. Local data is never downgraded; returning to alpha may be unavailable until alpha catches up."
        )?;
    }
    let interrupt = match tmt_adapters::interrupt::Interrupt::install() {
        Ok(interrupt) => interrupt,
        Err(error) => {
            return publish(
                None,
                None,
                Some(Failure::new("NATIVE_UPGRADE_FAILED", error.to_string(), 1).caused_by(error)),
                mode,
            );
        }
    };
    let ran_from_moved_prefix = std::env::current_exe()
        .is_ok_and(|executable| native_install::in_moved_prefix(&executable));
    let result = (|| {
        let executable = std::env::current_exe()?;
        native_install::upgrade_product_with_schema_consent(
            tmt_core::native_install::Product::Cli,
            UpgradeRequest {
                executable: &executable,
                channel,
                exact,
                unpin,
            },
            allow_schema_ahead,
            None,
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
            let message = error.to_string();
            let code = error.code();
            let failure = Failure::new(code, message, status).caused_by(error);
            return publish(activated.as_deref(), None, Some(failure), mode);
        }
    };
    if report.skipped_pinned {
        drop(interrupt);
        return finish(&report, None, None, yes, ran_from_moved_prefix, None, mode);
    }
    if interrupt.is_interrupted() {
        return publish(
            Some(&report),
            None,
            Some(Failure::new(
                "NATIVE_UPGRADE_INTERRUPTED",
                "Native binary installation completed; managed skill refresh was interrupted.",
                130,
            )),
            mode,
        );
    }
    let skills = Some(skill_outcome(refresh(
        &report.installation.active_executable,
        &UnixCommandRunner,
    )));
    let mut failure = None;
    if interrupt.is_interrupted() {
        failure = Some(Failure::new(
            "NATIVE_UPGRADE_INTERRUPTED",
            "Native binary installation completed; update was interrupted during managed skill refresh. Inspect the skill report.",
            130,
        ));
    }
    drop(interrupt);
    // The activated executable renames the Core database (#2292), on every
    // completed unpinned upgrade; whatever happens there never fails the upgrade.
    let database = failure
        .is_none()
        .then(|| database::run(&report.installation.active_executable, &UnixCommandRunner));
    finish(
        &report,
        skills,
        failure,
        yes,
        ran_from_moved_prefix,
        database,
        mode,
    )
}

fn finish(
    report: &UpgradeReport,
    skills: Option<Value>,
    failure: Option<Failure>,
    yes: bool,
    ran_from_moved_prefix: bool,
    database: Option<Value>,
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
    let after_upgrade = failure.is_none() && !report.skipped_pinned;
    let rename_hints = if after_upgrade {
        rename::hints()
    } else {
        Vec::new()
    };
    let picker_hint = if after_upgrade {
        ProviderEnvironment::capture()
            .ok()
            .and_then(|environment| picker::missing_hint(&environment))
    } else {
        None
    };
    // The upgrade that moves the installation keeps the former prefix on
    // purpose; only a later one reports why it could not remove it.
    let former_hint = if after_upgrade && ran_from_moved_prefix {
        native_install::retained_former_hint(&report.installation.active_executable)
    } else {
        None
    };
    // A leftover Squad installation is not upgraded or replaced; say so on every
    // completed upgrade until it is removed.
    let squad_hint = failure
        .is_none()
        .then(|| {
            let prefix = native_install::release_prefix(&report.installation.active_executable)?;
            crate::extension_install_command::retired_squad_hint(prefix)
        })
        .flatten();
    let named_hints = picker_hint
        .map(|hint| ("pickerHint", hint))
        .into_iter()
        .chain(
            former_hint
                .as_deref()
                .map(|hint| ("formerPrefixHint", hint)),
        )
        .chain(squad_hint.as_deref().map(|hint| ("retiredSquadHint", hint)))
        .collect::<Vec<_>>();
    publish_products_with_hints(
        Some(report),
        skills,
        failure,
        extensions,
        Notices {
            pane_rename: &rename_hints,
            named: &named_hints,
            database: database.as_ref(),
        },
        mode,
    )
}

fn refresh(
    executable: &Path,
    runner: &impl CommandRunner,
) -> Result<Value, (Option<Value>, io::Error)> {
    let result = runner.execute(CommandRequest {
        program: executable.as_os_str(),
        args: &[
            "__native-refresh-skills".into(),
            "--managed".into(),
            "--json".into(),
        ],
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

fn skill_outcome(result: Result<Value, (Option<Value>, io::Error)>) -> Value {
    match result {
        Ok(mut document) => {
            document["status"] = json!("current");
            document
        }
        Err((document, cause)) => {
            let mut document = document
                .unwrap_or_else(|| json!({"refreshed": [], "skipped": [], "conflicts": []}));
            let cause = document["error"]["message"]
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| cause.to_string());
            document["status"] = json!("warning");
            document["warning"] = json!({"code": "NATIVE_UPGRADE_SKILLS_FAILED", "message": format!("Native binary installation completed, but managed skill refresh failed: {cause}")});
            document
                .as_object_mut()
                .expect("skill report object")
                .remove("error");
            document
        }
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
    publish_products_with_hints(
        report,
        skills,
        failure,
        extensions,
        Notices::default(),
        mode,
    )
}

/// What an upgrade tells the user beyond its products: pane option cutover
/// hints, named hints that also appear in the JSON (the missing prompt picker,
/// the retained former prefix) and the database rename outcome.
#[derive(Default)]
struct Notices<'a> {
    pane_rename: &'a [String],
    named: &'a [(&'a str, &'a str)],
    database: Option<&'a Value>,
}

fn publish_products_with_hints(
    report: Option<&UpgradeReport>,
    skills: Option<Value>,
    failure: Option<Failure>,
    extensions: Vec<Value>,
    notices: Notices<'_>,
    mode: OutputMode,
) -> io::Result<u8> {
    let Notices {
        pane_rename: rename_hints,
        named: named_hints,
        database,
    } = notices;
    let failure = failure.map(|mut failure| {
        if let Some(report) = report {
            if let Some(version) = &report.state.pinned_version {
                failure
                    .message
                    .push_str(&format!(" The pin to {version} is kept."));
            }
            failure.suggestion(retry_hint(report))
        } else {
            failure
        }
    });
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
        document["error"] = failure_document(failure)["error"].clone();
    }
    let mut products = product_rows(report, failure.as_ref(), extensions);
    products[0]["skills"] = document["skills"].clone();
    let extension_failed = products.iter().skip(1).any(|p| p["status"] == "failed");
    document["products"] = json!(products);
    if !rename_hints.is_empty() {
        document["paneRenameHints"] = json!(rename_hints);
    }
    for (key, hint) in named_hints {
        document[*key] = (*hint).into();
    }
    if let Some(database) = database {
        document["databaseRename"] = database.clone();
    }
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
                let conflicts = document["skills"]["conflicts"]
                    .as_array()
                    .expect("validated skill report");
                writeln!(stdout, "{}", skills_summary(&document["skills"]))?;
                for target in conflicts {
                    tmt_cli_style::message::hint(
                        &mut stdout,
                        terminal,
                        &crate::skill_refresh_command::conflict_hint(Path::new(
                            target.as_str().expect("validated skill path"),
                        )),
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
        if let Some(message) = document["skills"]["warning"]["message"].as_str() {
            tmt_cli_style::message::warning(&mut stdout, terminal, message, None)?;
        }
        for product in products.iter().skip(1) {
            write_extension_product(&mut stdout, terminal, product)?;
        }
        let database_hint = database.and_then(database::hint);
        for hint in rename_hints
            .iter()
            .map(String::as_str)
            .chain(named_hints.iter().map(|(_, hint)| *hint))
            .chain(database_hint.as_deref())
        {
            tmt_cli_style::message::hint(&mut stdout, terminal, hint)?;
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

fn write_extension_product(
    stdout: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    product: &Value,
) -> io::Result<()> {
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
        tmt_cli_style::message::hint(stdout, terminal, hint)?;
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
    if let Some(skills) = product["details"].get("skills") {
        writeln!(
            stdout,
            "Agent skills: {} published",
            skills["published"].as_array().map_or(0, Vec::len)
        )?;
        if let Some(message) = skills["warning"]["message"].as_str() {
            tmt_cli_style::message::warning(stdout, terminal, message, None)?;
        }
    }
    if let Some(hint) = product["details"]["restartHint"]
        .as_str()
        .or_else(|| product["error"]["suggestion"].as_str())
    {
        tmt_cli_style::message::hint(stdout, terminal, hint)?;
    }
    Ok(())
}

fn failure_document(failure: &Failure) -> Value {
    let mut document = failure.document();
    if failure.code == "NATIVE_UPGRADE_FAILED" {
        let cause = failure
            .source()
            .map(ToString::to_string)
            .filter(|cause| !cause.trim().is_empty())
            .unwrap_or_else(|| failure.message.clone());
        document["error"]["cause"] = cause.into();
    }
    document
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
        "error": failure.map(|failure| failure_document(failure)["error"].clone()),
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
        Some(version) => format!("tmt upgrade --to {version}"),
        None => "tmt upgrade".into(),
    }
}

/// The plain-text skills line. Only core's own skills that went missing are
/// worth the user's attention: a skill of an extension that is frozen or not
/// installed is not a fault, and the JSON report keeps every skipped path.
fn skills_summary(skills: &Value) -> String {
    let core = |target: &Value| {
        target
            .as_str()
            .and_then(|target| Path::new(target).file_name())
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                tmt_core::skill_catalog::BUNDLED
                    .iter()
                    .any(|skill| skill.name == name && skill.group == Group::Core)
            })
    };
    let missing = skills["skipped"]
        .as_array()
        .map_or(0, |skipped| skipped.iter().filter(|t| core(t)).count());
    let mut line = format!(
        "Managed skills: {} current/refreshed",
        skills["refreshed"].as_array().map_or(0, Vec::len)
    );
    if missing > 0 {
        line.push_str(&format!(", {missing} missing"));
    }
    let conflicts = skills["conflicts"].as_array().map_or(0, Vec::len);
    if conflicts > 0 {
        line.push_str(&format!(", {conflicts} conflicts preserved"));
    }
    line.push('.');
    line
}

#[cfg(test)]
#[path = "native_upgrade_command_tests.rs"]
mod tests;

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core("tmt upgrade", &[""], &[]),
    crate::cli_style_tests::HintSpec::core(
        "tmt upgrade --to {version}",
        &[""],
        &[("{version}", "0.2.0")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "reload or restart your agent to use updated guidance; existing conversations can read tmt learn --skill",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::skipped(
        "tmt {} at {}",
        "Installed-executable summary, not a command suggestion.",
    ),
];

#[cfg(test)]
pub(crate) use database::PRINTED_HINTS as DATABASE_HINTS;

#[cfg(test)]
pub(crate) use extensions::PRINTED_HINTS as EXTENSIONS_HINTS;

#[cfg(test)]
pub(crate) use rename::PRINTED_HINTS as RENAME_HINTS;
