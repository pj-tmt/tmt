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
    Channel, InstalledVersion, UpgradeSelection, plan_version, select_upgrade,
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
    let client = crate::release_http::Https::new();
    upgrade_product_with(
        product,
        request,
        verifier,
        None,
        checkpoint,
        |url, accept, limit, deadline| client.get(url, accept, limit, deadline),
    )
}

#[cfg(test)]
fn upgrade_with(
    request: UpgradeRequest<'_>,
    checkpoint: impl FnMut() -> io::Result<()>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<Vec<u8>>,
) -> Result<UpgradeReport, UpgradeFailure> {
    upgrade_product_with(super::Product::Cli, request, None, None, checkpoint, get)
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
    )
}

fn upgrade_product_with(
    product: super::Product,
    request: UpgradeRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    selected: Option<&semver::Version>,
    mut checkpoint: impl FnMut() -> io::Result<()>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<Vec<u8>>,
) -> Result<UpgradeReport, UpgradeFailure> {
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
    let downloaded = release::download_product(
        product,
        channel,
        selected.or(exact.as_ref()),
        &current.target,
        Instant::now() + Duration::from_secs(60),
        get,
    )?;
    let plan = plan_version(Some(&current.state), &downloaded.version, channel, pin)
        .map_err(io::Error::other)?;
    checkpoint()?;
    let artifact = artifact::acquire_bytes(
        product,
        &downloaded.manifest,
        &downloaded.archive_name,
        &downloaded.archive,
        &current.target,
    )?;
    let result = activate(
        ActivationRequest {
            product,
            prefix: &current.prefix,
            channel,
            pin,
            expected: Some(current.id),
            provenance: Some(downloaded.provenance),
            verifier,
        },
        &artifact,
        &mut checkpoint,
    );
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

#[cfg(test)]
#[path = "upgrade_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "upgrade_artifact_tests.rs"]
mod artifact_tests;
