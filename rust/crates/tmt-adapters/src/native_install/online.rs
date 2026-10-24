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
use tmt_core::native_install::{Channel, PinAction, VersionError};

pub struct ReleaseInstallRequest<'a> {
    pub prefix: &'a Path,
    pub target: &'a str,
    pub channel: Channel,
    pub exact: Option<&'a str>,
}

pub fn default_install_prefix() -> io::Result<PathBuf> {
    std::env::home_dir()
        .map(|home| home.join(".local"))
        .ok_or_else(|| io::Error::other("Cannot determine the native installation home directory."))
}

pub fn install_release(
    product: Product,
    request: ReleaseInstallRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<InstallReport> {
    install_release_observed(product, request, verifier, checkpoint, &|_| {})
}

/// Preview only fixed official release metadata. No archive, prefix or skill writes.
/// The CLI treats an unavailable preview as unknown version/size, not a refusal.
pub fn preview_release(product: Product, target: &str) -> io::Result<super::ReleasePreview> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let client = crate::release_http::Https::new();
    release::preview_product(product, target, deadline, &mut |url, accept, limit, end| {
        client.get(url, accept, limit, end)
    })
}

pub fn install_release_observed(
    product: Product,
    request: ReleaseInstallRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
    progress: super::ProgressObserver<'_>,
) -> io::Result<InstallReport> {
    let channel = request.channel;
    let deadline = Instant::now() + Duration::from_secs(60);
    let client = if matches!(channel, Channel::Pr(_)) {
        crate::release_http::Https::authenticated()?
    } else {
        crate::release_http::Https::new()
    };
    install_release_with_observer(
        product,
        request,
        verifier,
        checkpoint,
        |url, accept, limit, request_deadline| {
            let request_deadline = request_deadline.min(deadline);
            if matches!(channel, Channel::Pr(_)) && url.ends_with("/zip") {
                client.get_actions(url, limit, request_deadline)
            } else {
                client.get_observed(
                    url,
                    accept,
                    limit,
                    request_deadline,
                    &|received_bytes, total_bytes| {
                        if accept == "application/octet-stream" && url.ends_with(".tar.gz") {
                            progress(super::ProgressEvent::Download {
                                received_bytes,
                                total_bytes,
                            });
                        }
                    },
                )
            }
        },
        progress,
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

#[cfg(test)]
fn install_release_with(
    product: Product,
    request: ReleaseInstallRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<InstallReport> {
    install_release_with_observer(product, request, verifier, checkpoint, get, &|_| {})
}

fn install_release_with_observer(
    product: Product,
    request: ReleaseInstallRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    mut checkpoint: impl FnMut() -> io::Result<()>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
    progress: super::ProgressObserver<'_>,
) -> io::Result<InstallReport> {
    let ReleaseInstallRequest {
        prefix,
        target,
        channel,
        exact,
    } = request;
    let exact = exact
        .map(|version| {
            version
                .parse::<Version>()
                .map_err(|_| io::Error::other(VersionError::InvalidSelection))
        })
        .transpose()?;
    if let Some(version) = &exact
        && !channel.accepts(version)
    {
        return Err(io::Error::other(VersionError::WrongChannel));
    }
    checkpoint()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let downloaded = if let Channel::Pr(pr) = channel {
        super::pr_resolver::download_current(
            super::pr_resolver::Request {
                product,
                pr,
                exact: exact.as_ref(),
                target,
                opt_in: false,
                deadline,
            },
            get,
        )?
    } else {
        release::download_product_observed(
            product,
            channel,
            exact.as_ref(),
            target,
            deadline,
            get,
            progress,
        )?
    };
    checkpoint()?;
    progress(super::ProgressEvent::Verifying);
    let artifact = artifact::acquire_bytes(
        product,
        &downloaded.manifest,
        &downloaded.archive_name,
        &downloaded.archive,
        target,
    )?;
    progress(super::ProgressEvent::Verified);
    checkpoint()?;
    progress(super::ProgressEvent::Activating);
    let report = activate(
        ActivationRequest {
            caller: super::Caller::Current,
            product,
            prefix,
            channel,
            pin: if exact.is_some() {
                PinAction::PinCandidate
            } else {
                PinAction::Preserve
            },
            expected: None,
            provenance: Some(downloaded.provenance),
            verifier,
            explicit_channel: true,
            schema: None,
        },
        &artifact,
        checkpoint,
    )?;
    progress(super::ProgressEvent::Activated);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    #[test]
    fn observed_install_reports_only_verified_transitions_and_retains_a_verified_receipt() {
        use std::cell::RefCell;
        let directory = TestDirectory::new();
        let prefix = directory.path.join("prefix");
        let product = Product::Ops;
        let (release, manifest, archive, _) =
            release::product_fixture(product, "0.1.0-alpha.2", "aarch64-apple-darwin", 42);
        let routes = release::indexed_routes(&release, &manifest, &archive);
        let events = RefCell::new(Vec::new());
        let report = install_release_with_observer(
            product,
            ReleaseInstallRequest {
                prefix: &prefix,
                target: "aarch64-apple-darwin",
                channel: Channel::Alpha,
                exact: None,
            },
            None,
            || Ok(()),
            |url, _, _, _| Ok(routes.get(url).unwrap().clone().into()),
            &|event| {
                events.borrow_mut().push(match event {
                    super::super::ProgressEvent::Selected {
                        version,
                        archive_bytes,
                    } => {
                        assert_eq!(version, "0.1.0-alpha.2");
                        assert_eq!(archive_bytes, archive.len() as u64);
                        "selected"
                    }
                    super::super::ProgressEvent::Downloaded => "downloaded",
                    super::super::ProgressEvent::Verifying => "verifying",
                    super::super::ProgressEvent::Verified => "verified",
                    super::super::ProgressEvent::Activating => "activating",
                    super::super::ProgressEvent::Activated => "activated",
                    _ => panic!("injected complete responses do not report chunk bytes"),
                })
            },
        )
        .unwrap();
        assert_eq!(
            *events.borrow(),
            [
                "selected",
                "downloaded",
                "verifying",
                "verified",
                "activating",
                "activated"
            ]
        );
        let installed = super::super::inspect_product_prefix(product, &prefix).unwrap();
        assert_eq!(installed.state.version.to_string(), report.version);
        assert!(report.executable.exists());
    }

    #[test]
    fn corrupt_acquisition_never_reports_verified_or_activation_or_creates_a_prefix() {
        use std::cell::RefCell;
        let directory = TestDirectory::new();
        let prefix = directory.path.join("prefix");
        let product = Product::Ops;
        let (release, manifest, archive, _) =
            release::product_fixture(product, "0.1.0-alpha.2", "aarch64-apple-darwin", 42);
        let routes = release::indexed_routes(&release, &manifest, &archive);
        let events = RefCell::new(Vec::new());
        let result = install_release_with_observer(
            product,
            ReleaseInstallRequest {
                prefix: &prefix,
                target: "aarch64-apple-darwin",
                channel: Channel::Alpha,
                exact: None,
            },
            None,
            || Ok(()),
            |url, _, _, _| {
                let mut bytes = routes.get(url).unwrap().clone();
                if url.ends_with(".tar.gz") {
                    bytes[0] ^= 1;
                }
                Ok(bytes.into())
            },
            &|event| events.borrow_mut().push(format!("{event:?}")),
        );
        assert!(result.unwrap_err().to_string().contains("SHA-256"));
        assert_eq!(events.borrow().len(), 1);
        assert!(events.borrow()[0].starts_with("Selected"));
        assert!(!prefix.exists());
    }

    #[test]
    fn exact_first_install_ignores_missing_or_stale_pointers_and_pins_verified_bytes() {
        for product in [Product::Remote, Product::Digest] {
            for pointer in [None, Some("0.1.0-alpha.1"), Some("0.1.0-alpha.3")] {
                let directory = TestDirectory::new();
                let prefix = directory.path.join("prefix");
                let version = "0.1.0-alpha.2";
                let (release, manifest, archive, _) =
                    release::product_fixture(product, version, "aarch64-apple-darwin", 42);
                let mut routes = release::indexed_routes(&release, &manifest, &archive);
                let pointer_url = format!(
                    "https://raw.githubusercontent.com/pj-tmt/tmt/release-index/channels/{}/alpha.json",
                    product.as_str()
                );
                match pointer {
                    None => {
                        routes.remove(&pointer_url);
                    }
                    Some(version) => {
                        routes.insert(
                            pointer_url,
                            serde_json::to_vec(&serde_json::json!({"version": version})).unwrap(),
                        );
                    }
                }
                let mut calls = Vec::new();
                let mut acquire = || {
                    install_release_with(
                        product,
                        ReleaseInstallRequest {
                            prefix: &prefix,
                            target: "aarch64-apple-darwin",
                            channel: Channel::Alpha,
                            exact: Some(version),
                        },
                        None,
                        || Ok(()),
                        |url, _, _, _| {
                            assert!(
                                url.starts_with("https://github.com/pj-tmt/tmt/releases/download/"),
                                "{url}"
                            );
                            calls.push(url.to_owned());
                            Ok(routes.get(url).expect("exact release asset").clone().into())
                        },
                    )
                };
                let report = acquire().unwrap();
                assert!(report.changed);
                let installed = super::super::inspect_product_prefix(product, &prefix).unwrap();
                assert_eq!(installed.state.version.to_string(), version);
                assert_eq!(
                    installed.state.pinned_version,
                    Some(version.parse().unwrap())
                );
                let executable = std::fs::read(&installed.active_executable).unwrap();
                let repeated = acquire().unwrap();
                assert!(!repeated.changed);
                assert_eq!(
                    std::fs::read(&installed.active_executable).unwrap(),
                    executable
                );
                assert_eq!(calls.len(), 6);
                assert!(calls[0].ends_with("/tmt-release-record.json"));
                assert_eq!(&calls[..3], &calls[3..]);
                assert!(!prefix.join("bin/tmt").exists());
            }
        }
    }

    #[test]
    fn exact_first_install_failure_preserves_the_prefix_and_never_falls_back() {
        for defect in [
            "record missing",
            "product",
            "version",
            "tag",
            "manifest",
            "archive",
        ] {
            let directory = TestDirectory::new();
            let prefix = directory.path.join("prefix");
            std::fs::create_dir(&prefix).unwrap();
            let sentinel = prefix.join("user-file");
            std::fs::write(&sentinel, b"unchanged").unwrap();
            let (release, manifest, archive, _) = release::product_fixture(
                Product::Digest,
                "0.1.0-alpha.2",
                "aarch64-apple-darwin",
                42,
            );
            let mut routes = release::indexed_routes(&release, &manifest, &archive);
            let record_url = routes
                .keys()
                .find(|url| url.ends_with("/tmt-release-record.json"))
                .unwrap()
                .clone();
            if matches!(defect, "product" | "version" | "tag") {
                let mut record: serde_json::Value =
                    serde_json::from_slice(&routes[&record_url]).unwrap();
                record[defect] = serde_json::json!(match defect {
                    "product" => "remote",
                    "version" => "0.1.0-alpha.3",
                    _ => "digest-v0.1.0-alpha.3",
                });
                routes.insert(record_url.clone(), serde_json::to_vec(&record).unwrap());
            }
            let mut calls = Vec::new();
            let error = install_release_with(
                Product::Digest,
                ReleaseInstallRequest {
                    prefix: &prefix,
                    target: "aarch64-apple-darwin",
                    channel: Channel::Alpha,
                    exact: Some("0.1.0-alpha.2"),
                },
                None,
                || Ok(()),
                |url, _, _, _| {
                    assert!(
                        url.starts_with("https://github.com/pj-tmt/tmt/releases/download/"),
                        "{url}"
                    );
                    calls.push(url.to_owned());
                    if defect == "record missing" && url == record_url {
                        return Err(io::Error::new(io::ErrorKind::NotFound, "record absent"));
                    }
                    let mut bytes = routes.get(url).expect("exact release asset").clone();
                    if (defect == "manifest" && url.ends_with("/dist-manifest.json"))
                        || (defect == "archive" && url.ends_with(".tar.gz"))
                    {
                        bytes[0] ^= 1;
                    }
                    Ok(bytes.into())
                },
            )
            .unwrap_err();
            if defect == "record missing" {
                assert!(error.to_string().contains("cannot be installed directly"));
                assert_eq!(calls.len(), 1);
            }
            assert_eq!(std::fs::read(&sentinel).unwrap(), b"unchanged");
            assert_eq!(std::fs::read_dir(&prefix).unwrap().count(), 1);
        }
    }

    #[test]
    fn public_first_install_uses_only_published_hosts_in_both_channels() {
        for product in [Product::Cli, Product::Ops, Product::Remote, Product::Colab] {
            for (version, channel) in [
                ("1.2.4", Channel::Stable),
                ("1.2.4-alpha.2", Channel::Alpha),
            ] {
                for defect in [None, Some("pointer"), Some("archive")] {
                    let directory = TestDirectory::new();
                    let prefix = directory.path.join("prefix");
                    let sentinel = directory.path.join("application-state");
                    std::fs::write(&sentinel, b"untouched state").unwrap();
                    let (release, manifest, archive, _) =
                        release::product_fixture(product, version, "aarch64-apple-darwin", 42);
                    let routes = release::indexed_routes(&release, &manifest, &archive);
                    let network = release::PublishedNetwork::default();
                    let result = install_release_with(
                        product,
                        ReleaseInstallRequest {
                            prefix: &prefix,
                            target: "aarch64-apple-darwin",
                            channel,
                            exact: None,
                        },
                        None,
                        || Ok(()),
                        |url, _, _, _| {
                            network.record(url);
                            if (defect == Some("pointer") && url.contains("/channels/"))
                                || (defect == Some("archive") && url.ends_with(".tar.gz"))
                            {
                                return Err(io::Error::new(
                                    io::ErrorKind::NotFound,
                                    "fixture unavailable",
                                ));
                            }
                            Ok(routes
                                .get(url)
                                .expect("canonical install URL")
                                .clone()
                                .into())
                        },
                    );
                    if defect.is_some() {
                        assert!(result.is_err(), "install / extension install failure");
                        assert!(
                            !prefix.exists(),
                            "failed acquisition must not create a prefix"
                        );
                    } else {
                        let report = result.unwrap();
                        assert!(report.changed);
                        let installed =
                            super::super::inspect_product_prefix(product, &prefix).unwrap();
                        assert_eq!(installed.state.version.to_string(), version);
                        assert_eq!(installed.state.channel, channel);
                        assert_eq!(
                            network.hosts(),
                            [
                                "raw.githubusercontent.com",
                                "github.com",
                                "github.com",
                                "github.com"
                            ],
                            "install / extension install {product:?}"
                        );
                    }
                    assert_eq!(std::fs::read(&sentinel).unwrap(), b"untouched state");
                }
            }
        }
    }

    #[test]
    fn pr_installation_never_falls_back_to_release_discovery_or_creates_a_prefix() {
        let directory = TestDirectory::new();
        let prefix = directory.path.join("prefix");
        let channel = Channel::parse("pr234").unwrap();
        let mut calls = Vec::new();
        let error = install_release_with(
            Product::Cli,
            ReleaseInstallRequest {
                prefix: &prefix,
                target: "aarch64-apple-darwin",
                channel,
                exact: None,
            },
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
            Product::Digest,
            ReleaseInstallRequest { prefix: &prefix, target: "aarch64-apple-darwin", channel: Channel::Alpha, exact: None },
            Some(&|_: &Path, _: &semver::Version| Ok(())),
            || Ok(()),
            |url, _, _, _| {
                calls += 1;
                assert_eq!(
                    url,
                    "https://raw.githubusercontent.com/pj-tmt/tmt/release-index/channels/digest/alpha.json"
                );
                Err(io::Error::new(io::ErrorKind::NotFound, "pointer missing"))
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(calls, 1);
        assert!(!prefix.exists());
    }

    #[test]
    fn unavailable_extension_pointer_never_downloads_another_product_or_creates_a_prefix() {
        for product in [Product::Remote, Product::Colab, Product::Digest] {
            let directory = TestDirectory::new();
            let prefix = directory.path.join("prefix");
            let mut calls = Vec::new();
            let error = install_release_with(product,ReleaseInstallRequest { prefix: &prefix, target: "aarch64-apple-darwin", channel: Channel::Alpha, exact: None },None,||Ok(()),
                |url,_,_,_| {calls.push(url.to_owned()); assert_eq!(url,format!("https://raw.githubusercontent.com/pj-tmt/tmt/release-index/channels/{}/alpha.json",product.as_str()));
                    Err(io::Error::new(io::ErrorKind::NotFound,"pointer missing"))}).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
            assert_eq!(calls.len(), 1);
            assert!(!prefix.exists());
            assert_eq!(
                error.to_string(),
                format!(
                    "No published {} release in the alpha channel yet. Nothing was changed.",
                    product.as_str()
                )
            );
        }
    }

    #[test]
    fn missing_pointer_or_record_preserves_installation_and_application_state() {
        for (product, version, channel) in [
            (Product::Cli, "1.2.4", Channel::Stable),
            (Product::Digest, "1.2.4-alpha.2", Channel::Alpha),
        ] {
            for at_pointer in [true, false] {
                let directory = TestDirectory::new();
                let prefix = directory.path.join("prefix");
                let sentinel = directory.path.join("application-state");
                std::fs::write(&sentinel, b"untouched").unwrap();
                let (release, manifest, archive, _) =
                    release::product_fixture(product, version, "aarch64-apple-darwin", 42);
                let routes = release::indexed_routes(&release, &manifest, &archive);
                let mut calls = Vec::new();
                let error = install_release_with(
                    product,
                    ReleaseInstallRequest {
                        prefix: &prefix,
                        target: "aarch64-apple-darwin",
                        channel,
                        exact: None,
                    },
                    None,
                    || Ok(()),
                    |url, _, _, _| {
                        calls.push(url.to_owned());
                        if (at_pointer && url.contains("/channels/"))
                            || (!at_pointer && url.ends_with("tmt-release-record.json"))
                        {
                            return Err(io::Error::new(
                                io::ErrorKind::NotFound,
                                "HTTPS resource was not found",
                            ));
                        }
                        Ok(routes
                            .get(url)
                            .expect("canonical metadata URL")
                            .clone()
                            .into())
                    },
                )
                .unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::NotFound);
                assert_eq!(
                    error.to_string(),
                    if at_pointer {
                        format!(
                            "No published {} release in the {} channel yet. Nothing was changed.",
                            product.as_str(),
                            channel.as_str()
                        )
                    } else {
                        format!(
                            "Could not check for {} updates on raw.githubusercontent.com: Could not download release bytes from github.com: HTTPS resource was not found. Nothing was changed.",
                            product.as_str()
                        )
                    }
                );
                assert_eq!(calls.len(), if at_pointer { 1 } else { 2 });
                assert!(
                    calls.iter().all(|url| url.contains("/channels/")
                        || url.ends_with("tmt-release-record.json"))
                );
                assert!(!prefix.exists());
                assert_eq!(std::fs::read(sentinel).unwrap(), b"untouched");
            }
        }
    }

    #[test]
    fn interruption_before_acquisition_has_no_network_or_filesystem_effect() {
        let directory = TestDirectory::new();
        let prefix = directory.path.join("prefix");
        let error = install_release_with(
            Product::Digest,
            ReleaseInstallRequest {
                prefix: &prefix,
                target: "aarch64-apple-darwin",
                channel: Channel::Alpha,
                exact: None,
            },
            Some(&|_: &Path, _: &semver::Version| Ok(())),
            || Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
            |_, _, _, _| panic!("cancelled installation must not download"),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!prefix.exists());
    }
}
