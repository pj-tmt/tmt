//! Verified extension installation through the existing native installer.

use super::{
    Human, InstalledExtension, failure, installed, interruptible, post_upgrade::PostUpgradeNotice,
};
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
    let current = if installed(product, prefix)? {
        Some(
            native_install::inspect_product_prefix(product, prefix)
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?,
        )
    } else {
        None
    };
    let executable = current.as_ref().map_or_else(
        || prefix.join("bin").join(product.executable()),
        |current| current.active_executable.clone(),
    );
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
    let previous_version = current
        .as_ref()
        .map(|current| current.state.version.to_string());
    let report = result.map_err(|error| {
        let notice = PostUpgradeNotice::after_failure(product, &error, previous_version.as_deref());
        notice.retain(installation_failure(error))
    })?;
    let notice = PostUpgradeNotice::observe(product, &report, previous_version.as_deref());
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
        notice,
    })
}

fn installation_failure(error: io::Error) -> Failure {
    if error
        .get_ref()
        .is_some_and(|cause| cause.is::<native_install::ReleaseUnavailable>())
    {
        return Failure::new(
            "EXTENSION_RELEASE_UNAVAILABLE",
            format!("{error} No installation was created or changed."),
            1,
        )
        .caused_by(error);
    }
    failure("EXTENSION_INSTALL_FAILED", error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_published_extension_release_is_availability_not_installation_damage() {
        for product in [Product::Remote, Product::Colab, Product::Digest] {
            let error = installation_failure(io::Error::new(
                io::ErrorKind::NotFound,
                native_install::ReleaseUnavailable {
                    product,
                    channel: Channel::Alpha,
                },
            ));
            assert_eq!(error.code, "EXTENSION_RELEASE_UNAVAILABLE");
            assert_eq!(error.status, 1);
            assert_eq!(
                error.message,
                format!(
                    "No published {} release yet in the alpha channel. No installation was created or changed.",
                    product.as_str()
                )
            );
            assert!(!error.message.contains("Inspect with"));
        }
    }

    #[test]
    fn missing_local_archive_and_invalid_published_bytes_remain_installation_failures() {
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::InvalidData] {
            let error = installation_failure(io::Error::new(kind, "bad archive"));
            assert_eq!(error.code, "EXTENSION_INSTALL_FAILED");
            assert!(error.message.contains("bad archive"));
        }
    }
}
