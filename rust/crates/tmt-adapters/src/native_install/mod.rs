//! Verified native release acquisition and managed publication, without app state.

mod artifact;
mod online;
mod remove;
pub use online::{default_install_prefix, install_release, latest_release_version};
pub use remove::{ProductRemoval, plan_product_removal, remove_product, uninstall_extension};
pub use tmt_core::native_install::Product;
mod managed;
pub use managed::{
    ManagedInstallation, inspect, inspect_product, inspect_product_prefix, release_skill_names,
    release_skills, with_active_product, with_active_release,
};
#[cfg(test)]
mod artifact_tests;
#[cfg(test)]
mod interrupt_tests;
mod publication;
#[cfg(test)]
mod publication_tests;
mod receipt;
mod repair;
pub use repair::{RepairReport, RepairRequired, repair_product};
mod release;
mod skills_tree;
mod upgrade;
pub use upgrade::{UpgradeFailure, UpgradeReport, UpgradeRequest, upgrade, upgrade_product};
#[cfg(test)]
mod test_support;

use std::{
    io,
    path::{Path, PathBuf},
};
use tmt_core::native_install::{Channel, PinAction, plan_version};

const OFFICIAL_REPOSITORY: &str = "wkh237/tmt";
/// The same repository before its rename (#334). Receipts written by
/// v5.0.0-alpha.2 through alpha.6 and Office 0.1.0-alpha.1 through alpha.3
/// record it; GitHub redirects it and keeps its release IDs. Read-only: new
/// receipts always record `OFFICIAL_REPOSITORY`.
const LEGACY_REPOSITORY: &str = "wkh237/tmux-team";

/// Whether a receipt's recorded repository is the official one, under its
/// current or pre-rename name. Anything else is foreign provenance.
fn official_repository(recorded: &serde_json::Value) -> bool {
    [OFFICIAL_REPOSITORY, LEGACY_REPOSITORY]
        .iter()
        .any(|name| recorded == *name)
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub executable: PathBuf,
    pub active_executable: PathBuf,
    pub version: String,
    pub changed: bool,
}

#[derive(Debug)]
struct ActivatedInstallation {
    report: InstallReport,
    cause: io::Error,
}

impl std::fmt::Display for ActivatedInstallation {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.cause.fmt(output)
    }
}
impl std::error::Error for ActivatedInstallation {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// A product owner's check of a written release candidate, given its executable
/// and the release version. It runs before the receipt is written, so a failure
/// leaves the previous release current. Products whose row requires one cannot
/// be published without it.
pub type ReleaseVerifier<'a> = &'a dyn Fn(&Path, &semver::Version) -> io::Result<()>;

/// Offline installation. Application data and provider skills are separate owners.
pub struct InstallRequest<'a> {
    pub archive: &'a Path,
    pub manifest: &'a Path,
    pub prefix: &'a Path,
    pub target: &'a str,
    pub channel: Channel,
    pub pin: PinAction,
}

/// The caller owns cancellation. Checkpoints never run inside atomic activation.
pub fn install(
    request: InstallRequest<'_>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<InstallReport> {
    install_observed(request, None, None, checkpoint)
}

/// Install the explicitly selected fixed product; never discover one from archive data.
pub fn install_product(
    product: Product,
    request: InstallRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<InstallReport> {
    install_product_observed(product, request, None, None, verifier, checkpoint)
}

fn install_observed(
    request: InstallRequest<'_>,
    expected: Option<uuid::Uuid>,
    provenance: Option<receipt::GitHubProvenance>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<InstallReport> {
    install_product_observed(
        Product::Cli,
        request,
        expected,
        provenance,
        None,
        checkpoint,
    )
}

fn install_product_observed(
    product: Product,
    request: InstallRequest<'_>,
    expected: Option<uuid::Uuid>,
    provenance: Option<receipt::GitHubProvenance>,
    verifier: Option<ReleaseVerifier<'_>>,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<InstallReport> {
    checkpoint()?;
    let artifact =
        artifact::acquire_product(product, request.manifest, request.archive, request.target)?;
    activate(
        ActivationRequest {
            product,
            prefix: request.prefix,
            channel: request.channel,
            pin: request.pin,
            expected,
            provenance,
            verifier,
        },
        &artifact,
        checkpoint,
    )
}

struct ActivationRequest<'a> {
    product: Product,
    prefix: &'a Path,
    channel: tmt_core::native_install::Channel,
    pin: tmt_core::native_install::PinAction,
    expected: Option<uuid::Uuid>,
    provenance: Option<receipt::GitHubProvenance>,
    verifier: Option<ReleaseVerifier<'a>>,
}

fn activate(
    request: ActivationRequest<'_>,
    artifact: &artifact::Artifact,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<InstallReport> {
    if request.product.requires_release_verifier() && request.verifier.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "This product's releases require a verifier; refusing publication.",
        ));
    }
    plan_version(None, &artifact.version, request.channel, request.pin)
        .map_err(io::Error::other)?;
    checkpoint()?;
    let layout = publication::Layout::open_product(request.prefix, request.product)?;
    let _lock = crate::file_lock::exclusive(&layout.root.join("install.lock"))?;
    let current = layout.current()?;
    if let Some(expected) = request.expected
        && current.as_ref().map(|receipt| receipt.id) != Some(expected)
    {
        return Err(invalid(
            "The installed release or pin changed while downloading. Retry from the active executable.",
        ));
    }
    layout.check_links(current.is_some())?;
    let plan = plan_version(
        current.as_ref().map(|receipt| &receipt.state),
        &artifact.version,
        request.channel,
        request.pin,
    )
    .map_err(io::Error::other)?;
    if let Some(current) = &current
        && (current.target != artifact.target
            || (current.state.version == artifact.version
                && (current.archive_sha256 != artifact.sha256
                    || current.archive_name != artifact.name
                    || current.file_hashes != artifact.file_hashes())))
    {
        return Err(invalid(
            "Installed target or equal-version artifact integrity does not match.",
        ));
    }
    let active_id = if plan.changed {
        let mut receipt = receipt::Receipt::new(artifact, plan.state);
        receipt.provenance = request.provenance;
        if let Err(error) = layout.publish(
            artifact,
            &receipt,
            current.as_ref().map(|receipt| receipt.id),
            request.verifier,
            &mut checkpoint,
        ) {
            return Err(
                if error
                    .get_ref()
                    .is_some_and(|cause| cause.is::<publication::ActivatedError>())
                {
                    io::Error::new(
                        error.kind(),
                        ActivatedInstallation {
                            report: installed_report(
                                &layout,
                                &artifact.version.to_string(),
                                receipt.id,
                                true,
                            ),
                            cause: error,
                        },
                    )
                } else {
                    error
                },
            );
        }
        receipt.id
    } else {
        checkpoint()?;
        layout.ensure_links()?;
        current
            .as_ref()
            .expect("unchanged installation has a current receipt")
            .id
    };
    Ok(installed_report(
        &layout,
        &artifact.version.to_string(),
        active_id,
        plan.changed,
    ))
}

fn installed_report(
    layout: &publication::Layout,
    version: &str,
    active_id: uuid::Uuid,
    changed: bool,
) -> InstallReport {
    InstallReport {
        executable: layout.prefix.join("bin").join(layout.product.executable()),
        active_executable: layout
            .root
            .join("releases")
            .join(active_id.to_string())
            .join(layout.product.executable()),
        version: version.into(),
        changed,
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
