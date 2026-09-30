//! Verified extension installation through the existing native installer.

use super::{Human, InstalledExtension, failure, installed, interruptible};
use crate::output::Failure;
use serde_json::json;
use std::{env, io, path::Path};
use tmt_adapters::native_install::{self, InstallRequest, Product, UpgradeRequest};
use tmt_core::native_install::{Channel, PinAction};

/// The installed extension's report and its active command link.
pub(super) fn install(
    product: Product,
    prefix: &Path,
    channel: Option<Channel>,
    archive: Option<&str>,
    manifest: Option<&str>,
) -> Result<InstalledExtension, Failure> {
    let name = product.as_str();
    let target = tmt_core::native_install::native_target(env::consts::OS, env::consts::ARCH)
        .ok_or_else(|| {
            Failure::new(
                "EXTENSION_UNSUPPORTED",
                format!("No {name} artifact supports this platform."),
                1,
            )
        })?;
    let executable = prefix.join("bin").join(product.executable());
    let current = if installed(product, prefix)? {
        Some(
            native_install::inspect_product_prefix(product, prefix)
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?,
        )
    } else {
        None
    };
    // The skills the replaced release carried: only those may be pruned.
    let previous = match &current {
        Some(_) => native_install::release_skill_names(product, &executable)
            .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?,
        None => Vec::new(),
    };
    let channel = channel
        .or_else(|| current.as_ref().map(|current| current.state.channel))
        .unwrap_or(Channel::Alpha);
    let checkpoint = interruptible("Extension installation interrupted before activation.")?;
    let result = match (archive, manifest) {
        (Some(archive), Some(manifest)) => native_install::install_product(
            product,
            InstallRequest {
                archive: Path::new(archive),
                manifest: Path::new(manifest),
                prefix,
                target,
                channel,
                pin: PinAction::Preserve,
            },
            crate::office_facade::release_verifier(product),
            checkpoint,
        ),
        (None, None) if current.is_some() => native_install::upgrade_product(
            product,
            UpgradeRequest {
                executable: &executable,
                channel: Some(channel),
                exact: None,
                unpin: false,
            },
            crate::office_facade::release_verifier(product),
            checkpoint,
        )
        .map(|report| report.installation)
        .map_err(|error| io::Error::new(error.kind(), error)),
        (None, None) => native_install::install_release(
            product,
            prefix,
            target,
            channel,
            crate::office_facade::release_verifier(product),
            checkpoint,
        ),
        _ => {
            return Err(Failure::new(
                "USAGE_ERROR",
                "Archive and manifest must be supplied together.",
                1,
            ));
        }
    };
    let report = result.map_err(|error| {
        failure(
            if archive.is_none() && error.kind() == io::ErrorKind::NotFound {
                "EXTENSION_RELEASE_UNAVAILABLE"
            } else {
                "EXTENSION_INSTALL_FAILED"
            },
            error,
        )
    })?;
    let human = if report.changed {
        Human::done(format!(
            "Installed {name} {} at {}.",
            report.version,
            report.executable.display()
        ))
    } else {
        Human::plain(format!("{name} {} is already installed.", report.version))
    };
    Ok(InstalledExtension {
        document: json!({"extension": name, "installed": true, "changed": report.changed,
            "version": report.version, "executable": report.executable}),
        human,
        executable: report.executable,
        previous,
        changed: report.changed,
    })
}
