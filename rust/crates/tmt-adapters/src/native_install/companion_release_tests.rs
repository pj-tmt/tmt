//! A CLI release may carry a companion executable (the first-party Herdr
//! driver, #479). The running, older tmt verifies and publishes an upgrade,
//! so this reader must accept both shapes: a release from before companions
//! existed and one that carries them. Either fails closed when its files and
//! its receipt disagree.

use super::*;
use crate::native_install::release::companion_fixture;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

const TARGET: &str = "aarch64-apple-darwin";
const DRIVER: &str = "tmt-driver-herdr";
const DRIVER_BYTES: &[u8] = b"#!/bin/sh\necho driver\n";

/// Serves exactly one release: `version`, with `companions` in it.
fn serving(
    version: &'static str,
    companions: &'static [(&'static str, &'static [u8])],
) -> impl FnMut(&str, &str, usize, Instant) -> io::Result<Vec<u8>> {
    let (release, manifest, archive, _) = companion_fixture(
        crate::native_install::Product::Cli,
        version,
        TARGET,
        42,
        companions,
    );
    move |url, _, limit, _| {
        let bytes = if url.ends_with("/assets/421") {
            manifest.clone()
        } else if url.ends_with("/assets/422") {
            archive.clone()
        } else if url.ends_with(&format!("/tags/v{version}")) {
            serde_json::to_vec(&release).unwrap()
        } else {
            serde_json::to_vec(&vec![release.clone()]).unwrap()
        };
        assert!(bytes.len() <= limit);
        Ok(bytes)
    }
}

fn upgrade_to(
    executable: &Path,
    exact: Option<&str>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<Vec<u8>>,
) -> UpgradeReport {
    upgrade_with(
        UpgradeRequest {
            executable,
            channel: None,
            exact,
            unpin: false,
        },
        || Ok(()),
        get,
    )
    .unwrap()
}

/// A 4-file release, upgraded to one that carries the driver.
fn with_driver() -> (crate::test_support::TestDirectory, PathBuf, PathBuf) {
    let (directory, layout, old) = published_layout();
    let old_executable = layout
        .root
        .join("releases")
        .join(old.id.to_string())
        .join("tmt");
    let report = upgrade_to(
        &old_executable,
        None,
        serving("1.2.4", &[(DRIVER, DRIVER_BYTES)]),
    );
    let release = report
        .installation
        .active_executable
        .parent()
        .unwrap()
        .to_path_buf();
    (directory, report.installation.active_executable, release)
}

#[test]
fn an_upgrade_from_a_release_without_the_driver_publishes_and_records_it() {
    let (_directory, active, release) = with_driver();
    let driver = release.join(DRIVER);
    assert_eq!(fs::read(&driver).unwrap(), DRIVER_BYTES);
    assert_eq!(
        fs::metadata(&driver).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(release.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(
        receipt["file_sha256"][DRIVER],
        crate::native_install::artifact::digest(DRIVER_BYTES)
    );
    inspect(&active).unwrap();
}

#[test]
fn pinning_from_a_release_with_the_driver_to_one_without_it_verifies() {
    let (_directory, active, _) = with_driver();
    let report = upgrade_to(&active, Some("1.2.5"), serving("1.2.5", &[]));
    assert!(report.installation.changed);
    let release = report.installation.active_executable.parent().unwrap();
    assert!(!release.join(DRIVER).exists());
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(release.join("receipt.json")).unwrap()).unwrap();
    assert!(receipt["file_sha256"].get(DRIVER).is_none());
    assert_eq!(receipt["pinned_version"], "1.2.5");
    inspect(&report.installation.active_executable).unwrap();
}

#[test]
fn an_interrupted_upgrade_to_the_driver_keeps_the_previous_release_active() {
    let (_directory, layout, old) = published_layout();
    let old_executable = layout
        .root
        .join("releases")
        .join(old.id.to_string())
        .join("tmt");
    let mut checkpoints = 0;
    let error = upgrade_with(
        UpgradeRequest {
            executable: &old_executable,
            channel: None,
            exact: None,
            unpin: false,
        },
        || {
            checkpoints += 1;
            // Past the download, inside publication: the driver may already
            // be written into the candidate release.
            if checkpoints == 6 {
                return Err(io::Error::other("interrupted"));
            }
            Ok(())
        },
        serving("1.2.4", &[(DRIVER, DRIVER_BYTES)]),
    )
    .unwrap_err();
    assert!(error.activated.is_none(), "{error:?}");
    let current = inspect(&old_executable).unwrap();
    assert_eq!(current.state.version.to_string(), "1.2.3");
}

/// Repair is an extension operation; a CLI release from before companions
/// is re-verified by installing it again, which must stay a verified no-op.
#[test]
fn a_release_without_the_driver_reinstalls_as_a_verified_no_op() {
    let directory = crate::test_support::TestDirectory::new();
    let prefix = directory.path.join("prefix");
    let (_, manifest, archive, name) = companion_fixture(
        crate::native_install::Product::Cli,
        "1.2.3",
        TARGET,
        42,
        &[],
    );
    let archive_path = directory.path.join(&name);
    let manifest_path = directory.path.join("manifest.json");
    fs::write(&archive_path, &archive).unwrap();
    fs::write(&manifest_path, &manifest).unwrap();
    let install = || {
        crate::native_install::install_product(
            crate::native_install::Product::Cli,
            crate::native_install::InstallRequest {
                archive: &archive_path,
                manifest: &manifest_path,
                prefix: &prefix,
                target: TARGET,
                channel: tmt_core::native_install::Channel::Stable,
                pin: tmt_core::native_install::PinAction::Preserve,
            },
            None,
            || Ok(()),
        )
        .unwrap()
    };
    let first = install();
    assert!(first.changed);
    assert!(
        !first
            .active_executable
            .parent()
            .unwrap()
            .join(DRIVER)
            .exists()
    );
    assert!(!install().changed);
    inspect(&first.active_executable).unwrap();
}

#[test]
fn a_driver_file_and_its_receipt_disagreeing_fails_closed() {
    type Change = fn(&Path);
    let changes: [(Change, &str); 3] = [
        (
            |release| fs::remove_file(release.join(DRIVER)).unwrap(),
            "inventory has changed",
        ),
        (
            |release| fs::write(release.join(DRIVER), b"tampered").unwrap(),
            "has changed",
        ),
        (
            |release| {
                fs::set_permissions(release.join(DRIVER), fs::Permissions::from_mode(0o644))
                    .unwrap()
            },
            "permissions have changed",
        ),
    ];
    for (change, message) in changes {
        let (_directory, active, release) = with_driver();
        change(&release);
        let error = inspect(&active).unwrap_err();
        assert!(error.to_string().contains(message), "{message}: {error}");
    }
    // A driver file in a release whose receipt doesn't record one.
    let (_directory, layout, old) = published_layout();
    let release = layout.root.join("releases").join(old.id.to_string());
    fs::write(release.join(DRIVER), DRIVER_BYTES).unwrap();
    fs::set_permissions(release.join(DRIVER), fs::Permissions::from_mode(0o755)).unwrap();
    let error = inspect(&release.join("tmt")).unwrap_err();
    assert!(
        error.to_string().contains("inventory has changed"),
        "{error}"
    );
}
