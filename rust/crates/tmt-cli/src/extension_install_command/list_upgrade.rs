//! Managed extension observations and consented upgrades.

use super::skills::settle_skills;
use super::{
    ExtensionListRow, Human, Outcome, ask, extension, extension_list, failure, installed,
    interruptible, prefix,
};
use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{env, fs, io, os::unix::fs::PermissionsExt, path::Path};
use tmt_adapters::native_install::{self, Product, UpgradeRequest};
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
    let prefix = prefix(selected.as_deref())?;
    if !installed(product, &prefix)? {
        return Err(Failure::new(
            "EXTENSION_NOT_INSTALLED",
            format!("{name} is not installed. Install it with: tmt extension install {name}"),
            1,
        ));
    }
    if !ask(yes, mode, &format!("Update the {name} extension"))? {
        return Ok(None);
    }
    let executable = prefix.join("bin").join(product.executable());
    native_install::inspect_product_prefix(product, &prefix)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    // The skills the replaced release carried: only those may be pruned.
    let previous = native_install::release_skill_names(product, &executable)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    let report = native_install::upgrade_product(
        product,
        UpgradeRequest {
            executable: &executable,
            channel,
            exact: to.as_deref(),
            unpin,
        },
        crate::office_facade::release_verifier(product),
        interruptible("Extension update interrupted before activation.")?,
    )
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
    if report.installation.changed {
        settle_skills(
            product,
            &executable,
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
        let state = match installed(product, prefix) {
            Ok(true) => native_install::inspect_product_prefix(product, prefix)
                .map(|installation| Some(installation.state))
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?,
            Ok(false) => None,
            Err(error) => return Err(error),
        };
        let shadowed = shadowing_in(product, prefix, search);
        // Only an explicit --check touches the network; unreachable is unknown.
        let update = if check {
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
        if let Some(update) = update {
            row["update"] = update.into();
        }
        let status = match &state {
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
        };
        lines.push(ExtensionListRow {
            name: name.into(),
            status,
            update,
            shadowed,
        });
        rows.push(row);
    }
    Ok((
        json!({"extensions": rows}),
        extension_list(&lines, env::home_dir().as_deref()),
    ))
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
