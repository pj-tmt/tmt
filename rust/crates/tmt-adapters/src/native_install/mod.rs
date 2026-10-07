//! Verified native release acquisition and managed publication, without app state.

mod artifact;
pub mod handoff;
mod online;
mod remove;
pub use online::{default_install_prefix, install_release, latest_release_version};
pub use remove::{ProductRemoval, plan_product_removal, remove_product, uninstall_extension};
pub use tmt_core::native_install::Product;
mod managed;
pub use managed::{
    Companion, ManagedInstallation, active_companion, inspect, inspect_product,
    inspect_product_prefix, release_skill_names, release_skills, with_active_product,
    with_active_release,
};
#[cfg(test)]
mod artifact_tests;
#[cfg(test)]
mod interrupt_tests;
mod pr_catalog;
mod pr_json;
mod pr_receipt;
mod pr_resolver;
mod pr_zip;
mod publication;
#[cfg(test)]
mod publication_tests;
mod receipt;
mod repair;
pub use repair::{RepairReport, RepairRequired, repair_product, repair_product_from_archive};
mod release;
pub use release::ReleaseUnavailable;
mod skills_tree;
mod uses;
pub use uses::{
    Affected, CheckError, Unavailable, Use, UseStatus, affected, check as check_use,
    declared as declared_uses, extension_named, statuses as use_statuses, valid_feature,
};
mod upgrade;
pub use upgrade::{
    UpgradeFailure, UpgradeReport, UpgradeRequest, upgrade, upgrade_product,
    upgrade_product_selected, upgrade_product_with_schema_consent,
};
#[cfg(test)]
mod test_support;

use std::{
    io,
    path::{Path, PathBuf},
};
use tmt_core::native_install::{Channel, PinAction, PrVersionContext, plan_candidate_version};

const OFFICIAL_REPOSITORY: &str = "pj-tmt/tmt";

/// Model-free schema export for the reviewed publisher. The supplied source
/// identity must be verified against every compiled source digest by that owner.
/// This performs no storage initialization, migration or acquisition.
pub fn compiled_application_schema(source_sha: &str) -> io::Result<serde_json::Value> {
    if !pr_catalog::git_sha(source_sha) {
        return Err(invalid(
            "Schema export requires an exact lowercase source SHA.",
        ));
    }
    let compiled = crate::storage::Storage::compiled_schema();
    let record = pr_catalog::ApplicationSchema {
        schema_version: 1,
        product: "cli".into(),
        source_sha: source_sha.into(),
        databases: vec![pr_catalog::DatabaseSchema {
            domain: "tmt-core-db".into(),
            version: compiled.version,
        }],
        source_files: compiled
            .sources
            .into_iter()
            .map(|source| pr_catalog::SourceFile {
                path: source.path.into(),
                sha256: source.sha256,
            })
            .collect(),
    };
    record.validate("cli", source_sha)?;
    serde_json::to_value(record).map_err(io::Error::other)
}
/// Names recorded before the organization transfer (#1021) and rename (#334).
/// Read-only compatibility: lookups and new receipts use the current repository.
const LEGACY_REPOSITORIES: [&str; 2] = ["wkh237/tmt", "wkh237/tmux-team"];

/// Accept only this repository's current and historical receipt provenance.
fn official_repository(recorded: &serde_json::Value) -> bool {
    recorded == OFFICIAL_REPOSITORY || LEGACY_REPOSITORIES.iter().any(|name| recorded == *name)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
    provenance: Option<receipt::Provenance>,
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
    provenance: Option<receipt::Provenance>,
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
            explicit_channel: true,
            schema: None,
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
    provenance: Option<receipt::Provenance>,
    verifier: Option<ReleaseVerifier<'a>>,
    explicit_channel: bool,
    schema: Option<pr_catalog::ApplicationSchema>,
}

fn activate(
    request: ActivationRequest<'_>,
    artifact: &artifact::Artifact,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<InstallReport> {
    let product = request.product;
    activate_with_local_schema(request, artifact, checkpoint, || {
        local_application_schema(product)
    })
}

fn activate_with_local_schema(
    request: ActivationRequest<'_>,
    artifact: &artifact::Artifact,
    mut checkpoint: impl FnMut() -> io::Result<()>,
    mut local_schema: impl FnMut() -> io::Result<Vec<pr_catalog::DatabaseSchema>>,
) -> io::Result<InstallReport> {
    if request.product.requires_release_verifier() && request.verifier.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "This product's releases require a verifier; refusing publication.",
        ));
    }
    let candidate_identity = request
        .provenance
        .as_ref()
        .map(receipt::Provenance::pr_identity)
        .transpose()?
        .flatten();
    let preliminary = plan_candidate_version(
        None,
        &artifact.version,
        request.channel,
        request.pin,
        PrVersionContext {
            current: None,
            candidate: candidate_identity.as_ref(),
            explicit_channel: true,
        },
    )
    .map_err(io::Error::other)?;
    if let Some(receipt::Provenance::Pr(proof)) = &request.provenance {
        proof.validate(
            request.product,
            &artifact.target,
            &preliminary.state,
            &artifact.name,
            &artifact.sha256,
        )?;
    }
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
    let current_identity = current
        .as_ref()
        .and_then(|receipt| receipt.provenance.as_ref())
        .map(receipt::Provenance::pr_identity)
        .transpose()?
        .flatten();
    let plan = plan_candidate_version(
        current.as_ref().map(|receipt| &receipt.state),
        &artifact.version,
        request.channel,
        request.pin,
        PrVersionContext {
            current: current_identity.as_ref(),
            candidate: candidate_identity.as_ref(),
            explicit_channel: request.explicit_channel,
        },
    )
    .map_err(io::Error::other)?;
    if let Some(current) = &current
        && (current.target != artifact.target
            || (current.state.version == artifact.version
                && !(request.explicit_channel
                    && current.state.channel != request.channel
                    && (current_identity.is_some() || candidate_identity.is_some()))
                && (current.archive_sha256 != artifact.sha256
                    || current.archive_name != artifact.name
                    || current.file_hashes != artifact.file_hashes())))
    {
        return Err(invalid(
            "Installed target or equal-version artifact integrity does not match.",
        ));
    }
    if let Some(receipt::Provenance::Pr(proof)) = &request.provenance {
        let local = local_schema()?;
        pr_receipt::admit_databases(
            request.product,
            &proof.candidate.application_schema,
            &proof.admission.latest_alpha.application_schema,
            &local,
            proof.admission.schema_ahead_opt_in,
        )?;
    } else if current_identity.is_some() {
        let schema = request
            .schema
            .as_ref()
            .ok_or_else(|| io::Error::other(tmt_core::native_install::SchemaError::Unknown))?;
        let local = local_schema()?;
        pr_receipt::admit_databases(request.product, schema, schema, &local, false)?;
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

fn local_application_schema(product: Product) -> io::Result<Vec<pr_catalog::DatabaseSchema>> {
    if product != Product::Cli {
        return Err(io::Error::other(
            tmt_core::native_install::SchemaError::Unknown,
        ));
    }
    let paths = crate::config::ConfigPaths::discover().map_err(io::Error::other)?;
    let version = crate::storage::Storage::application_schema(&paths.database)
        .map_err(|_| io::Error::other(tmt_core::native_install::SchemaError::Unknown))?;
    Ok(vec![pr_catalog::DatabaseSchema {
        domain: "tmt-core-db".into(),
        version,
    }])
}

fn manifest_application_schema(
    product: Product,
    bytes: &[u8],
) -> io::Result<pr_catalog::ApplicationSchema> {
    let unknown = || io::Error::other(tmt_core::native_install::SchemaError::Unknown);
    let value: serde_json::Value = pr_json::parse(bytes, artifact::MANIFEST_LIMIT)?;
    let schema: pr_catalog::ApplicationSchema = serde_json::from_value(
        value
            .get("tmt_application_schema")
            .ok_or_else(unknown)?
            .clone(),
    )
    .map_err(|_| unknown())?;
    schema.validate(product.as_str(), &schema.source_sha)?;
    Ok(schema)
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
