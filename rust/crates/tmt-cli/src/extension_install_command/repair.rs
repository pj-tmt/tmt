//! Explicit extension recovery retains the native installer's existing provenance rules.

use super::{Human, InstalledExtension, failure, interruptible};
use crate::output::Failure;
use serde_json::json;
use std::{env, path::Path};
use tmt_adapters::native_install::{self, Product};
use tmt_cli_style::value;

pub(super) fn repair_extension(
    product: Product,
    prefix: &Path,
    archive: Option<&str>,
    manifest: Option<&str>,
) -> Result<InstalledExtension, Failure> {
    let verifier = crate::office_facade::release_verifier(product);
    let checkpoint = interruptible("Extension repair interrupted before activation.")?;
    let result = match (archive, manifest) {
        (Some(archive), Some(manifest)) => native_install::repair_product_from_archive(
            product,
            prefix,
            Path::new(archive),
            Path::new(manifest),
            verifier,
            checkpoint,
        ),
        (None, None) => native_install::repair_product(product, prefix, verifier, checkpoint),
        _ => {
            return Err(Failure::new(
                "USAGE_ERROR",
                "Archive and manifest must be supplied together.",
                1,
            ));
        }
    }
    .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    let report = result.installation;
    let mut human = if report.changed {
        Human::done(format!("Repaired {} {}.", product.as_str(), report.version))
    } else {
        Human::plain(format!(
            "{} {} is already verified.",
            product.as_str(),
            report.version
        ))
    };
    if let Some(path) = &result.retained_release {
        human.push(format!(
            "Kept damaged release at {}",
            value::home_path(path, env::home_dir().as_deref())
        ));
    }
    Ok(InstalledExtension {
        document: json!({"extension": product.as_str(), "installed": true, "changed": report.changed,
        "version": report.version, "executable": report.executable, "retainedRelease": result.retained_release}),
        human,
        executable: report.executable,
        previous: Vec::new(),
        changed: report.changed,
        notice: super::post_upgrade::PostUpgradeNotice::default(),
    })
}
