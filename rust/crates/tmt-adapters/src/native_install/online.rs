//! First installation through the same release verifier and activation owner as updates.

use super::{
    ActivationRequest, InstallReport, Product, ReleaseVerifier, activate, artifact, release,
};
use semver::Version;
use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_core::native_install::{Channel, PinAction};

pub fn default_install_prefix() -> io::Result<PathBuf> {
    std::env::home_dir()
        .map(|home| home.join(".local"))
        .ok_or_else(|| io::Error::other("Cannot determine the native installation home directory."))
}

pub fn install_release(
    product: Product,
    prefix: &Path,
    target: &str,
    channel: Channel,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<InstallReport> {
    let client = crate::release_http::Https::new();
    install_release_with(
        product,
        prefix,
        target,
        channel,
        verifier,
        checkpoint,
        |url, accept, limit, deadline| client.get(url, accept, limit, deadline),
    )
}

/// The newest published version of `product` in `channel`, by bounded
/// metadata discovery only: nothing is downloaded or installed.
pub fn latest_release_version(product: Product, channel: Channel) -> io::Result<Version> {
    let client = crate::release_http::Https::new();
    release::discover_latest(
        product,
        channel,
        Instant::now() + Duration::from_secs(10),
        &mut |url, accept, limit, deadline| client.get(url, accept, limit, deadline),
    )
    .map(|(_, version)| version)
}

fn install_release_with(
    product: Product,
    prefix: &Path,
    target: &str,
    channel: Channel,
    verifier: Option<ReleaseVerifier<'_>>,
    mut checkpoint: impl FnMut() -> io::Result<()>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<InstallReport> {
    checkpoint()?;
    let downloaded = release::download_product(
        product,
        channel,
        None,
        target,
        Instant::now() + Duration::from_secs(60),
        get,
    )?;
    checkpoint()?;
    let artifact = artifact::acquire_bytes(
        product,
        &downloaded.manifest,
        &downloaded.archive_name,
        &downloaded.archive,
        target,
    )?;
    activate(
        ActivationRequest {
            product,
            prefix,
            channel,
            pin: PinAction::Preserve,
            expected: None,
            provenance: Some(downloaded.provenance),
            verifier,
        },
        &artifact,
        checkpoint,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    #[test]
    fn no_release_does_not_create_an_installation() {
        let directory = TestDirectory::new();
        let prefix = directory.path.join("prefix");
        let mut calls = 0;
        let error = install_release_with(
            Product::Office,
            &prefix,
            "aarch64-apple-darwin",
            Channel::Alpha,
            Some(&|_: &Path, _: &semver::Version| Ok(())),
            || Ok(()),
            |url, _, _, _| {
                calls += 1;
                assert_eq!(
                    url,
                    "https://api.github.com/repos/pj-tmt/tmt/git/matching-refs/tags/tmt-office-v?per_page=100&page=1"
                );
                Ok(b"[]".to_vec().into())
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(calls, 1);
        assert!(!prefix.exists());
    }

    #[test]
    fn unpublished_remote_never_downloads_another_products_binary_or_creates_a_prefix() {
        for refs in [
            b"[]".as_slice(),
            br#"[{"ref":"refs/tags/tmt-remote-v0.1.0-alpha.1"}]"#.as_slice(),
        ] {
            let directory = TestDirectory::new();
            let prefix = directory.path.join("prefix");
            let mut calls = Vec::new();
            let error = install_release_with(
                Product::Remote,
                &prefix,
                "aarch64-apple-darwin",
                Channel::Alpha,
                None,
                || Ok(()),
                |url, _, _, _| {
                    calls.push(url.to_owned());
                    if url.ends_with("/git/matching-refs/tags/tmt-remote-v?per_page=100&page=1") {
                        return Ok(refs.to_vec().into());
                    }
                    assert!(url.ends_with("/releases/tags/tmt-remote-v0.1.0-alpha.1"));
                    Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "Remote has no published release",
                    ))
                },
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
            assert!(error.get_ref().unwrap().is::<release::ReleaseUnavailable>());
            assert_eq!(calls.len(), if refs == b"[]" { 1 } else { 2 });
            assert!(!prefix.exists());
        }
    }

    #[test]
    fn interruption_before_acquisition_has_no_network_or_filesystem_effect() {
        let directory = TestDirectory::new();
        let prefix = directory.path.join("prefix");
        let error = install_release_with(
            Product::Office,
            &prefix,
            "aarch64-apple-darwin",
            Channel::Alpha,
            Some(&|_: &Path, _: &semver::Version| Ok(())),
            || Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
            |_, _, _, _| panic!("cancelled installation must not download"),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!prefix.exists());
    }
}
