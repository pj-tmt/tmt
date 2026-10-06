//! Public update orchestration. Network acquisition never holds the install lock.

use super::{
    ActivatedInstallation, ActivationRequest, InstallReport, ReleaseVerifier, activate, artifact,
    release,
};
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};
use tmt_core::native_install::{
    Channel, InstalledVersion, PrVersionContext, UpgradeSelection, plan_candidate_version,
    select_upgrade,
};

pub struct UpgradeRequest<'a> {
    pub executable: &'a Path,
    pub channel: Option<Channel>,
    pub exact: Option<&'a str>,
    pub unpin: bool,
}

#[derive(Debug)]
pub struct UpgradeReport {
    pub installation: InstallReport,
    pub state: InstalledVersion,
    pub skipped_pinned: bool,
}

#[derive(Debug)]
pub struct UpgradeFailure {
    /// Present only when this invocation activated a release before failing.
    pub activated: Option<Box<UpgradeReport>>,
    cause: io::Error,
}

impl std::fmt::Display for UpgradeFailure {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.cause.fmt(output)
    }
}
impl std::error::Error for UpgradeFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
impl From<io::Error> for UpgradeFailure {
    fn from(cause: io::Error) -> Self {
        Self {
            activated: None,
            cause,
        }
    }
}
impl UpgradeFailure {
    pub fn code(&self) -> &'static str {
        use tmt_core::native_install::SchemaError;
        match self
            .cause
            .get_ref()
            .and_then(|error| error.downcast_ref::<SchemaError>())
        {
            Some(SchemaError::Unknown) => "NATIVE_SCHEMA_UNKNOWN",
            Some(SchemaError::AheadOfAlpha) => "NATIVE_SCHEMA_AHEAD",
            Some(SchemaError::DataDowngrade) => "NATIVE_SCHEMA_DOWNGRADE",
            None if self
                .cause
                .get_ref()
                .is_some_and(|error| error.is::<crate::release_http::MissingAuth>()) =>
            {
                "NATIVE_PR_AUTH_REQUIRED"
            }
            None if self.needs_new_installer() => "NATIVE_UPGRADE_INSTALLER_UNSUPPORTED",
            None => "NATIVE_UPGRADE_FAILED",
        }
    }
    pub fn needs_new_installer(&self) -> bool {
        self.cause
            .get_ref()
            .is_some_and(|error| error.is::<super::handoff::Unsupported>())
    }
    pub fn kind(&self) -> io::ErrorKind {
        self.cause.kind()
    }
}

pub fn upgrade(
    request: UpgradeRequest<'_>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<UpgradeReport, UpgradeFailure> {
    upgrade_product(super::Product::Cli, request, None, checkpoint)
}

pub fn upgrade_product(
    product: super::Product,
    request: UpgradeRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<UpgradeReport, UpgradeFailure> {
    upgrade_product_with_schema_consent(product, request, false, verifier, checkpoint)
}

/// Schema-ahead consent is invocation-owned, never inherited from a receipt.
pub fn upgrade_product_with_schema_consent(
    product: super::Product,
    request: UpgradeRequest<'_>,
    allow_schema_ahead: bool,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<UpgradeReport, UpgradeFailure> {
    // Preserve the pinned no-network path, including no gh credential lookup.
    let current = super::inspect_product(product, request.executable)?;
    let selection = select_upgrade(
        &current.state,
        request.channel,
        request.exact,
        request.unpin,
    )
    .map_err(io::Error::other)?;
    if matches!(
        selection,
        UpgradeSelection::Fetch {
            channel: Channel::Pr(_),
            ..
        }
    ) {
        let deadline = Instant::now() + Duration::from_secs(60);
        let client = crate::release_http::Https::authenticated(
            deadline,
            &crate::process::UnixCommandRunner,
        )?;
        return upgrade_product_with_options(
            product,
            request,
            verifier,
            Acquisition {
                selected: None,
                allow_schema_ahead,
                deadline,
            },
            checkpoint,
            |url, accept, limit, deadline| {
                if url.ends_with("/zip") {
                    client.get_actions(url, limit, deadline)
                } else {
                    client.get(url, accept, limit, deadline)
                }
            },
            install_cli,
        );
    }
    let client = crate::release_http::Https::new();
    upgrade_product_with(
        product,
        request,
        verifier,
        None,
        checkpoint,
        |url, accept, limit, deadline| client.get(url, accept, limit, deadline),
        install_cli,
    )
}

#[cfg(test)]
fn upgrade_with(
    request: UpgradeRequest<'_>,
    checkpoint: impl FnMut() -> io::Result<()>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> Result<UpgradeReport, UpgradeFailure> {
    upgrade_product_with(
        super::Product::Cli,
        request,
        None,
        None,
        checkpoint,
        get,
        super::test_support::install_downloaded,
    )
}

/// Upgrade the version shown in a consent prompt without changing the recorded pin policy.
pub fn upgrade_product_selected(
    product: super::Product,
    request: UpgradeRequest<'_>,
    version: &str,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<UpgradeReport, UpgradeFailure> {
    let version = version
        .parse::<semver::Version>()
        .map_err(io::Error::other)?;
    let client = crate::release_http::Https::new();
    upgrade_product_with(
        product,
        request,
        verifier,
        Some(&version),
        checkpoint,
        |url, accept, limit, deadline| client.get(url, accept, limit, deadline),
        install_cli,
    )
}

fn upgrade_product_with(
    product: super::Product,
    request: UpgradeRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    selected: Option<&semver::Version>,
    checkpoint: impl FnMut() -> io::Result<()>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
    install_cli: impl FnOnce(
        &super::ManagedInstallation,
        &release::DownloadedRelease,
        Channel,
        tmt_core::native_install::PinAction,
        &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<InstallReport>,
) -> Result<UpgradeReport, UpgradeFailure> {
    upgrade_product_with_options(
        product,
        request,
        verifier,
        Acquisition {
            selected,
            allow_schema_ahead: false,
            deadline: Instant::now() + Duration::from_secs(60),
        },
        checkpoint,
        get,
        install_cli,
    )
}

struct Acquisition<'a> {
    selected: Option<&'a semver::Version>,
    allow_schema_ahead: bool,
    deadline: Instant,
}

fn upgrade_product_with_options(
    product: super::Product,
    request: UpgradeRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    acquisition: Acquisition<'_>,
    mut checkpoint: impl FnMut() -> io::Result<()>,
    mut get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
    install_cli: impl FnOnce(
        &super::ManagedInstallation,
        &release::DownloadedRelease,
        Channel,
        tmt_core::native_install::PinAction,
        &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<InstallReport>,
) -> Result<UpgradeReport, UpgradeFailure> {
    let Acquisition {
        selected,
        allow_schema_ahead,
        deadline,
    } = acquisition;
    checkpoint()?;
    let current = super::inspect_product(product, request.executable)?;
    let selection = select_upgrade(
        &current.state,
        request.channel,
        request.exact,
        request.unpin,
    )
    .map_err(io::Error::other)?;
    let UpgradeSelection::Fetch {
        channel,
        exact,
        pin,
    } = selection
    else {
        return Ok(UpgradeReport {
            installation: InstallReport {
                executable: current.executable,
                active_executable: current.active_executable,
                version: current.state.version.to_string(),
                changed: false,
            },
            state: current.state,
            skipped_pinned: true,
        });
    };
    let mut downloaded = if let Channel::Pr(pr) = channel {
        super::pr_resolver::download(
            super::pr_resolver::Request {
                product,
                pr,
                exact: selected.or(exact.as_ref()),
                target: &current.target,
                opt_in: allow_schema_ahead,
                deadline,
            },
            super::pr_resolver::Policy {
                repository_id: 1_118_285_740,
                producers: super::pr_resolver::APPROVED,
            },
            &mut get,
            || super::local_application_schema(product),
            || {
                u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_err(io::Error::other)?
                        .as_millis(),
                )
                .map_err(io::Error::other)
            },
        )?
    } else {
        release::download_product(
            product,
            channel,
            selected.or(exact.as_ref()),
            &current.target,
            deadline,
            &mut get,
        )?
    };
    downloaded.explicit_channel = request.channel.is_some();
    if matches!(current.state.channel, Channel::Pr(_)) && !matches!(channel, Channel::Pr(_)) {
        let schema = super::manifest_application_schema(product, &downloaded.manifest)?;
        let local = super::local_application_schema(product)?;
        super::pr_receipt::admit_databases(product, &schema, &schema, &local, false)?;
    }
    let current_identity = current
        .provenance
        .as_ref()
        .map(super::receipt::Provenance::pr_identity)
        .transpose()?
        .flatten();
    let candidate_identity = downloaded.provenance.pr_identity()?;
    let plan = plan_candidate_version(
        Some(&current.state),
        &downloaded.version,
        channel,
        pin,
        PrVersionContext {
            current: current_identity.as_ref(),
            candidate: candidate_identity.as_ref(),
            explicit_channel: downloaded.explicit_channel,
        },
    )
    .map_err(io::Error::other)?;
    checkpoint()?;
    let result = if product == super::Product::Cli {
        install_cli(&current, &downloaded, channel, pin, &mut checkpoint)
    } else {
        let artifact = artifact::acquire_bytes(
            product,
            &downloaded.manifest,
            &downloaded.archive_name,
            &downloaded.archive,
            &current.target,
        )?;
        activate(
            ActivationRequest {
                product,
                prefix: &current.prefix,
                channel,
                pin,
                expected: Some(current.id),
                provenance: Some(downloaded.provenance),
                verifier,
                explicit_channel: downloaded.explicit_channel,
                schema: if current_identity.is_some() {
                    Some(super::manifest_application_schema(
                        product,
                        &downloaded.manifest,
                    )?)
                } else {
                    None
                },
            },
            &artifact,
            &mut checkpoint,
        )
    };
    result
        .map(|installation| UpgradeReport {
            installation,
            state: plan.state.clone(),
            skipped_pinned: false,
        })
        .map_err(|cause: io::Error| {
            let activated = cause
                .get_ref()
                .and_then(|error| error.downcast_ref::<ActivatedInstallation>())
                .map(|error| {
                    Box::new(UpgradeReport {
                        installation: error.report.clone(),
                        state: plan.state.clone(),
                        skipped_pinned: false,
                    })
                });
            UpgradeFailure { activated, cause }
        })
}

fn install_cli(
    current: &super::ManagedInstallation,
    downloaded: &release::DownloadedRelease,
    channel: Channel,
    pin: tmt_core::native_install::PinAction,
    checkpoint: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<InstallReport> {
    super::handoff::upgrade(
        current,
        downloaded,
        channel,
        pin,
        checkpoint,
        &crate::process::UnixCommandRunner,
    )
}

#[cfg(test)]
#[path = "upgrade_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "upgrade_artifact_tests.rs"]
mod artifact_tests;
