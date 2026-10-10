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
    let deadline = Instant::now() + Duration::from_secs(60);
    let client = if matches!(channel, Channel::Pr(_)) {
        crate::release_http::Https::authenticated(deadline, &crate::process::UnixCommandRunner)?
    } else {
        crate::release_http::Https::new()
    };
    install_release_with(
        product,
        prefix,
        target,
        channel,
        verifier,
        checkpoint,
        |url, accept, limit, request_deadline| {
            let request_deadline = request_deadline.min(deadline);
            if matches!(channel, Channel::Pr(_)) && url.ends_with("/zip") {
                client.get_actions(url, limit, request_deadline)
            } else {
                client.get(url, accept, limit, request_deadline)
            }
        },
    )
}

/// The newest published version of `product` in `channel`, by bounded
/// metadata discovery only: nothing is downloaded or installed.
pub fn latest_release_version(product: Product, channel: Channel) -> io::Result<Version> {
    if matches!(channel, Channel::Pr(_)) {
        return Err(io::Error::other(super::pr_resolver::Unavailable(
            "PR channels require target-aware authenticated acquisition; published-release discovery cannot select a PR candidate.",
        )));
    }
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
    let deadline = Instant::now() + Duration::from_secs(60);
    let downloaded = if let Channel::Pr(pr) = channel {
        super::pr_resolver::download_current(
            super::pr_resolver::Request {
                product,
                pr,
                exact: None,
                target,
                opt_in: false,
                deadline,
            },
            get,
        )?
    } else {
        release::download_product(product, channel, None, target, deadline, get)?
    };
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
            explicit_channel: true,
            schema: None,
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
    fn pr_installation_never_falls_back_to_release_discovery_or_creates_a_prefix() {
        let directory = TestDirectory::new();
        let prefix = directory.path.join("prefix");
        let channel = Channel::parse("pr234").unwrap();
        let mut calls = Vec::new();
        let error = install_release_with(
            Product::Cli,
            &prefix,
            "aarch64-apple-darwin",
            channel,
            None,
            || Ok(()),
            |url, _, _, _| {
                calls.push(url.to_owned());
                Err(io::Error::other("PR metadata unavailable"))
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "PR metadata unavailable");
        assert_eq!(calls, ["https://api.github.com/repos/pj-tmt/tmt/pulls/234"]);
        assert!(!prefix.exists());
        assert!(
            latest_release_version(Product::Cli, channel)
                .unwrap_err()
                .get_ref()
                .unwrap()
                .is::<super::super::pr_resolver::Unavailable>()
        );
    }

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
    fn unpublished_extensions_never_download_another_products_binary_or_creates_a_prefix() {
        for product in [Product::Remote, Product::Colab, Product::Digest] {
            let tagged = format!(
                r#"[{{"ref":"refs/tags/{}0.1.0-alpha.1"}}]"#,
                product.tag_prefix()
            );
            for refs in [b"[]".as_slice(), tagged.as_bytes()] {
                let directory = TestDirectory::new();
                let prefix = directory.path.join("prefix");
                let mut calls = Vec::new();
                let error = install_release_with(
                    product,
                    &prefix,
                    "aarch64-apple-darwin",
                    Channel::Alpha,
                    None,
                    || Ok(()),
                    |url, _, _, _| {
                        calls.push(url.to_owned());
                        if url.ends_with(&format!(
                            "/git/matching-refs/tags/{}?per_page=100&page=1",
                            product.tag_prefix()
                        )) {
                            return Ok(refs.to_vec().into());
                        }
                        assert!(url.ends_with(&format!(
                            "/releases/tags/{}0.1.0-alpha.1",
                            product.tag_prefix()
                        )));
                        Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            "No published product release",
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
