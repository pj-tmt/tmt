//! Managed extension observations and consented upgrades.

use super::skills::settle_skills;
use super::{
    ExtensionListRow, Human, INSTALLABLE_EXTENSIONS, Outcome, ask, extension, extension_list,
    failure, installed, interruptible, prefix, repair_command, repair_required,
    require_installable,
};
use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{
    env, fmt, fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tmt_adapters::native_install::{self, Product, UpgradeRequest};
use tmt_cli_style::value;
use tmt_core::native_install::Channel;

pub(super) fn upgrade_extension(
    name: String,
    selected: Option<String>,
    channel: Option<Channel>,
    to: Option<String>,
    unpin: bool,
    yes: bool,
    mode: OutputMode,
) -> Result<Outcome, Failure> {
    let product = extension(&name)?;
    require_installable(product)?;
    let prefix = prefix(selected.as_deref())?;
    if !installed(product, &prefix)? {
        return Err(Failure::new(
            "EXTENSION_NOT_INSTALLED",
            format!("{name} is not installed. Install it with: tmt extension install {name}"),
            1,
        ));
    }
    let affects = to
        .as_deref()
        .and_then(|version| version.parse().ok())
        .map(|version| native_install::affected(&prefix, product, Some(&version)))
        .unwrap_or_default();
    let mut question = format!("Update the {name} extension");
    if let Some(warning) = affects_warning(&affects) {
        question.push_str(&warning);
    }
    if !ask(yes, mode, &question)? {
        return Ok(None);
    }
    let mut outcome = upgrade_at(product, &prefix, channel, to.as_deref(), unpin, None)?;
    if let Some((document, _)) = outcome.as_mut() {
        record_affects(document, &affects);
    }
    Ok(outcome)
}

/// The installed extensions whose declared features stop working, as one
/// sentence for a consent question. A warning only: nothing is blocked.
pub(super) fn affects_warning(affects: &[native_install::Affected]) -> Option<String> {
    if affects.is_empty() {
        return None;
    }
    let features = affects
        .iter()
        .map(|used| format!("{}'s {}", used.extension.as_str(), used.label))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(". This stops {features} from working"))
}

pub(super) fn record_affects(document: &mut Value, affects: &[native_install::Affected]) {
    if !affects.is_empty() {
        document["affects"] = affects
            .iter()
            .map(|used| {
                json!({"extension": used.extension.as_str(), "feature": used.feature,
                    "label": used.label})
            })
            .collect::<Vec<_>>()
            .into();
    }
}

pub(super) fn upgrade_at(
    product: Product,
    prefix: &Path,
    channel: Option<Channel>,
    to: Option<&str>,
    unpin: bool,
    selected: Option<&str>,
) -> Result<Outcome, Failure> {
    let name = product.as_str();
    let current = native_install::inspect_product_prefix(product, prefix)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    let executable = current.active_executable;
    // The skills the replaced release carried: only those may be pruned.
    let previous = native_install::release_skill_names(product, &executable)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    let request = UpgradeRequest {
        executable: &executable,
        channel,
        exact: to,
        unpin,
    };
    let verifier = crate::office_facade::release_verifier(product);
    let checkpoint = interruptible("Extension update interrupted before activation.")?;
    let report = match selected {
        Some(version) => native_install::upgrade_product_selected(
            product, request, version, verifier, checkpoint,
        ),
        None => native_install::upgrade_product(product, request, verifier, checkpoint),
    }
    .map_err(|error| {
        failure(
            "EXTENSION_UPGRADE_FAILED",
            io::Error::new(error.kind(), error),
        )
    })?;
    let version = report.installation.version.clone();
    let human = if report.skipped_pinned {
        Human::plain(format!(
            "{name} {version} is pinned; nothing changed. Clear the pin with: tmt extension upgrade {name} --unpin"
        ))
    } else if report.installation.changed {
        Human::done(format!("Updated {name} to {version}."))
    } else {
        Human::plain(format!("{name} {version} is current."))
    };
    let mut document = json!({"extension": name, "installed": true,
        "changed": report.installation.changed, "version": version,
        "skippedPinned": report.skipped_pinned,
        "executable": report.installation.executable});
    let mut human = human;
    // A post-activation settlement failure can leave a verified former install
    // beside the current release. Retry only that recovery on an unpinned no-op.
    if report.installation.changed
        || (!report.skipped_pinned
            && native_install::inspect_former_product(product, prefix)
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?
                .is_some())
    {
        settle_skills(
            product,
            &report.installation.executable,
            &previous,
            false,
            None,
            &mut document,
            &mut human,
        )?;
    }
    Ok(Some((document, human)))
}

pub(super) fn listing(
    prefix: &Path,
    check: bool,
    search: &std::ffi::OsStr,
) -> Result<(Value, Human), Failure> {
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    for product in Product::ALL
        .into_iter()
        .filter(|product| *product != Product::Cli)
    {
        let name = product.as_str();
        let mut state = None;
        let mut partial = false;
        let mut issue = None;
        match partial_removal(product, prefix) {
            Ok(true) => partial = true,
            Ok(false) => match installed(product, prefix) {
                Ok(true) => match native_install::inspect_product_prefix(product, prefix) {
                    Ok(installation) => state = Some(installation.state),
                    Err(error) => issue = Some(Issue::io(product, prefix, &error)),
                },
                Ok(false) => {}
                Err(failure) => issue = Some(Issue::new(product, prefix, &failure.message)),
            },
            Err(error) => issue = Some(Issue::io(product, prefix, &error)),
        }
        let frozen = !INSTALLABLE_EXTENSIONS.contains(&product);
        // A frozen product is only reported while its own installation exists.
        if frozen && issue.as_ref().is_some_and(|issue| issue.unmanaged) {
            issue = None;
        }
        if frozen && state.is_none() && !partial && issue.is_none() {
            continue;
        }
        let shadowed = shadowing_in(product, prefix, search);
        // Only an explicit --check touches the network; unreachable is unknown.
        let update = if check && frozen {
            Some("frozen")
        } else if check {
            let channel = state.as_ref().map_or(Channel::Alpha, |state| state.channel);
            Some(
                match native_install::latest_release_version(product, channel) {
                    Ok(latest) => match &state {
                        Some(state) if latest > state.version => "available",
                        Some(_) => "current",
                        None => "available",
                    },
                    Err(_) => "unknown",
                },
            )
        } else {
            None
        };
        let mut row = json!({
            "name": name,
            "installed": state.is_some(),
            "version": state.as_ref().map(|state| state.version.to_string()),
            "channel": state.as_ref().map(|state| state.channel.as_str()),
            "pinned": state.as_ref().and_then(|state| state.pinned_version.as_ref()).map(ToString::to_string),
            "commands": product.links(),
            "shadowedBy": shadowed,
        });
        if frozen {
            row["frozen"] = true.into();
        }
        if let Some(update) = update {
            row["update"] = update.into();
        }
        let hint = format!(
            "tmt extension rm {name} --yes --prefix {}",
            crate::output::shell_word(&prefix.to_string_lossy())
        );
        if partial {
            row["status"] = "partiallyRemoved".into();
            row["hint"] = hint.clone().into();
        }
        let mut notes = Vec::new();
        let explicit = non_default(prefix);
        if state.is_some() {
            // A declaration that cannot be read is simply not shown: the entry
            // itself is already verified above.
            let statuses = native_install::use_statuses(product, prefix).unwrap_or_default();
            if !statuses.is_empty() {
                row["uses"] = statuses
                    .iter()
                    .map(native_install::UseStatus::to_json)
                    .collect::<Vec<_>>()
                    .into();
                notes.extend(statuses.iter().map(|used| use_line(used, explicit)));
            }
        }
        if let Some(issue) = &issue {
            row["status"] = issue.status().into();
            row["path"] = issue.path.to_string_lossy().into_owned().into();
            row["detail"] = issue.detail.clone().into();
            row["hint"] = issue.hint(product, prefix).into();
            notes.push(issue.hint(product, prefix));
        }
        let status = if let Some(issue) = &issue {
            format!(
                "{}: {} ({})",
                issue.status(),
                value::home_path(&issue.path, env::home_dir().as_deref()),
                issue.detail
            )
        } else if partial {
            format!("partially removed; finish with: {hint}")
        } else {
            match &state {
                Some(state) => format!(
                    "{}{}",
                    state.version,
                    state
                        .pinned_version
                        .as_ref()
                        .map(|pinned| format!(" (pinned {pinned})"))
                        .unwrap_or_default()
                ),
                None => "not installed".into(),
            }
        };
        let status = if frozen {
            format!("{status} (frozen)")
        } else {
            status
        };
        lines.push(ExtensionListRow {
            name: name.into(),
            status,
            update,
            shadowed,
            notes,
        });
        rows.push(row);
    }
    Ok((
        json!({"extensions": rows}),
        extension_list(&lines, env::home_dir().as_deref()),
    ))
}

/// The prefix only when it is not the one `tmt extension` uses by default, so a
/// printed command carries `--prefix` exactly when it needs it.
fn non_default(prefix: &Path) -> Option<&Path> {
    let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let default = native_install::default_install_prefix().ok()?;
    (canonical(&default) != canonical(prefix)).then_some(prefix)
}

/// One dim line per declared use: available, or the exact next step.
fn use_line(used: &native_install::UseStatus, prefix: Option<&Path>) -> String {
    if used.available() {
        format!(
            "{}: uses {} >={}, available",
            used.declared.label,
            used.declared.extension.as_str(),
            used.declared.minimum
        )
    } else {
        used.hint(prefix)
    }
}

/// One extension entry TMT cannot read as a managed installation. Listing
/// reports it beside the healthy entries instead of failing as a whole.
struct Issue {
    path: PathBuf,
    /// A command link with no TMT activation behind it: not TMT's file.
    unmanaged: bool,
    detail: String,
    /// The exact repair when verification found damage the installer can restore.
    repair: Option<String>,
}

impl Issue {
    fn new(product: Product, prefix: &Path, error: &dyn fmt::Display) -> Self {
        let unmanaged = !matches!(
            exists(&prefix.join(product.namespace()).join("current")),
            Ok(true)
        );
        Self {
            path: prefix.join("bin").join(product.executable()),
            unmanaged,
            detail: if unmanaged {
                "no TMT installation behind it".into()
            } else {
                error.to_string()
            },
            repair: None,
        }
    }

    fn io(product: Product, prefix: &Path, error: &io::Error) -> Self {
        let mut issue = Self::new(product, prefix, error);
        issue.repair = repair_required(error).map(repair_command);
        issue
    }

    fn status(&self) -> &'static str {
        if self.repair.is_some() {
            "repairRequired"
        } else if self.unmanaged {
            "unmanaged"
        } else {
            "invalid"
        }
    }

    /// A repair that never points back at the failing listing.
    fn hint(&self, product: Product, prefix: &Path) -> String {
        let name = product.as_str();
        let prefix = crate::output::shell_word(&prefix.to_string_lossy());
        if let Some(command) = &self.repair {
            format!("repair this release with: {command}")
        } else if self.unmanaged {
            format!(
                "move or remove {}, then install the managed one with: tmt extension install {name} --yes --prefix {prefix}",
                crate::output::shell_word(&self.path.to_string_lossy())
            )
        } else {
            format!(
                "remove its commands with: tmt extension rm {name} --yes --prefix {prefix}, then reinstall it"
            )
        }
    }
}

/// A retained activation with any missing command link is recoverable partial
/// removal. Listing reports it without inspecting a missing execution path.
fn partial_removal(product: Product, prefix: &Path) -> io::Result<bool> {
    if !exists(&prefix.join(product.namespace()).join("current"))? {
        return Ok(false);
    }
    for name in product.links() {
        if !exists(&prefix.join("bin").join(name))? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Every other `tmt-<name>` command on PATH that does not resolve to this
/// installation's file. Paths are canonicalized; nothing is executed.
pub(super) fn shadowing_in(
    product: Product,
    prefix: &Path,
    search: &std::ffi::OsStr,
) -> Vec<String> {
    let mut found = Vec::new();
    for link in product.links() {
        let ours = fs::canonicalize(prefix.join("bin").join(link)).ok();
        for directory in env::split_paths(search) {
            let candidate = directory.join(link);
            let Ok(metadata) = fs::metadata(&candidate) else {
                continue;
            };
            if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
            let Ok(resolved) = fs::canonicalize(&candidate) else {
                continue;
            };
            if Some(&resolved) != ours.as_ref() {
                let shown = candidate.display().to_string();
                if !found.contains(&shown) {
                    found.push(shown);
                }
            }
        }
    }
    found
}

fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "move or remove {}, then install the managed one with: tmt extension install {name} --yes --prefix {prefix}",
        &[""],
        &[
            ("{name}", "ops"),
            ("{prefix}", "/tmp/hint-prefix"),
            ("{}", "/tmp/hint-prefix/bin/tmt-ops"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "remove its commands with: tmt extension rm {name} --yes --prefix {prefix}, then reinstall it",
        &[", then"],
        &[("{name}", "ops"), ("{prefix}", "/tmp/hint-prefix")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt extension rm {name} --yes --prefix {}",
        &[""],
        &[
            ("{name}", "ops"),
            ("{SUGGESTED_EXTENSION}", "ops"),
            ("{}", "/tmp/hint-prefix"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{name} is not installed. Install it with: tmt extension install {name}",
        &[""],
        &[
            ("{name}", "ops"),
            ("{}", "ops"),
            ("{SUGGESTED_EXTENSION}", "ops"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{name} {version} is pinned; nothing changed. Clear the pin with: tmt extension upgrade {name} --unpin",
        &[""],
        &[
            ("{name}", "ops"),
            ("{}", "ops"),
            ("{SUGGESTED_EXTENSION}", "ops"),
        ],
    ),
];
