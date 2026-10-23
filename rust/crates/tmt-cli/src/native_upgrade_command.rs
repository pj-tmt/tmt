//! Native update composition; the new executable owns skill refresh and extensions.

mod database;
mod extensions;
mod rename;
mod summary;

use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
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
use tmt_core::native_install::Channel;

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
    let current = std::env::current_exe().ok();
    let prior = Prior {
        repeat: repeat_command(channel, exact, unpin, yes, allow_schema_ahead),
        ran_from_moved_prefix: current
            .as_deref()
            .is_some_and(native_install::in_moved_prefix),
        versions: current
            .as_deref()
            .map(installed_versions)
            .unwrap_or_default(),
    };
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
        return finish(&report, None, None, yes, &prior, None, mode);
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
    finish(&report, skills, failure, yes, &prior, database, mode)
}

/// What the upgrade started from: where it ran and which versions were installed, for the
/// table's `from → to`.
struct Prior {
    /// The command that repeats this invocation.
    repeat: String,
    ran_from_moved_prefix: bool,
    versions: BTreeMap<String, String>,
}

/// The bare retry the extension rows suggest; a repeated invocation replaces it.
const GENERIC_RETRY: &str = "tmt upgrade";

/// The command line that repeats this invocation, for the retry after a component failed.
fn repeat_command(
    channel: Option<Channel>,
    exact: Option<&str>,
    unpin: bool,
    yes: bool,
    allow_schema_ahead: bool,
) -> String {
    let mut command = String::from(GENERIC_RETRY);
    if let Some(channel) = channel {
        command.push_str(&format!(" --channel {}", channel.as_str()));
    }
    if let Some(exact) = exact {
        command.push_str(&format!(" --to {exact}"));
    }
    for (set, flag) in [
        (unpin, " --unpin"),
        (yes, " --yes"),
        (allow_schema_ahead, " --allow-schema-ahead"),
    ] {
        if set {
            command.push_str(flag);
        }
    }
    command
}

/// The installed version of the CLI and of every installed extension, by product name.
fn installed_versions(executable: &Path) -> BTreeMap<String, String> {
    let Some(prefix) = native_install::release_prefix(executable) else {
        return BTreeMap::new();
    };
    std::iter::once(native_install::Product::Cli)
        .chain(
            crate::extension_install_command::INSTALLABLE_EXTENSIONS
                .iter()
                .copied(),
        )
        .filter_map(|product| {
            let installed = native_install::inspect_product_prefix(product, prefix).ok()?;
            Some((
                product.as_str().to_owned(),
                installed.state.version.to_string(),
            ))
        })
        .collect()
}

fn finish(
    report: &UpgradeReport,
    skills: Option<Value>,
    failure: Option<Failure>,
    yes: bool,
    prior: &Prior,
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
    let former_hint = if after_upgrade && prior.ran_from_moved_prefix {
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
            before: Some(&prior.versions),
            repeat: Some(&prior.repeat),
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
    /// Versions installed before the upgrade, by product name.
    before: Option<&'a BTreeMap<String, String>>,
    /// The command that repeats this invocation.
    repeat: Option<&'a str>,
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
        before,
        repeat,
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
        let conflict_hints = document["skills"]["conflicts"]
            .as_array()
            .filter(|_| report.is_some_and(|report| !report.skipped_pinned))
            .map(|conflicts| {
                conflicts
                    .iter()
                    .map(|target| {
                        crate::skill_refresh_command::conflict_hint(Path::new(
                            target.as_str().expect("validated skill path"),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let reload = report.is_some_and(|report| !report.skipped_pinned);
        let required = rename_hints
            .iter()
            .cloned()
            .chain(database.and_then(database::hint))
            .collect::<Vec<_>>();
        let optional = named_hints
            .iter()
            .map(|(_, hint)| *hint)
            .collect::<Vec<_>>();
        summary::write(
            &mut stdout,
            terminal,
            &summary::Account {
                products: &products,
                has_report: report.is_some(),
                skipped_pinned: report.is_some_and(|report| report.skipped_pinned),
                before,
                skills: &document["skills"],
                repeat,
                followups: summary::Followups {
                    required: &required,
                    optional: &optional,
                    conflict_hints,
                    reload_hint: reload.then_some(
                        "Reload or restart your agent to use updated guidance; existing conversations can read tmt learn --skill",
                    ),
                },
            },
        )?;
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
        "Reload or restart your agent to use updated guidance; existing conversations can read tmt learn --skill",
        &[""],
        &[],
    ),
];

#[cfg(test)]
pub(crate) use database::PRINTED_HINTS as DATABASE_HINTS;

#[cfg(test)]
pub(crate) use extensions::PRINTED_HINTS as EXTENSIONS_HINTS;

#[cfg(test)]
pub(crate) use rename::PRINTED_HINTS as RENAME_HINTS;
