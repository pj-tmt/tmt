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
) -> impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response> {
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
            serde_json::to_vec(&serde_json::json!([{ "ref": format!("refs/tags/{}", release["tag_name"].as_str().unwrap()) }])).unwrap()
        };
        assert!(bytes.len() <= limit);
        Ok(bytes.into())
    }
}

fn upgrade_to(
    executable: &Path,
    exact: Option<&str>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
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

/// [`serving`] for one companion chosen at run time.
fn serving_owned(
    version: &'static str,
    name: &'static str,
    bytes: &'static [u8],
) -> impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response> {
    let companions: &'static [(&'static str, &'static [u8])] = Box::leak(Box::new([(name, bytes)]));
    serving(version, companions)
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

/// A driver the probe accepts: it answers `capabilities` as Herdr.
const HERDR_DRIVER: &[u8] = br#"#!/bin/sh
printf '%s' '{"ok":{"protocols":[1],"kind":"host","name":"herdr","version":"0.0.0-test","ops":["snapshot"],"paneId":{"prefix":"term_"},"target":"w{n}:p{n}","callerEnv":[]}}'
"#;

mod first_party {
    use super::*;
    use crate::driver_protocol::registry::{self, ApprovalState, DriverSource};
    use crate::native_install::{Companion, active_companion};

    /// A 4-file release upgraded to one that ships a working Herdr driver.
    fn shipped() -> (crate::test_support::TestDirectory, PathBuf, PathBuf) {
        shipping(HERDR_DRIVER)
    }

    fn shipping(driver: &'static [u8]) -> (crate::test_support::TestDirectory, PathBuf, PathBuf) {
        let (directory, layout, old) = published_layout();
        let old_executable = layout
            .root
            .join("releases")
            .join(old.id.to_string())
            .join("tmt");
        let report = upgrade_to(
            &old_executable,
            None,
            serving_owned("1.2.4", DRIVER, driver),
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
    fn the_active_release_names_its_driver_and_receipt_digest() {
        let (_directory, active, release) = shipped();
        assert_eq!(
            active_companion(&active, DRIVER).unwrap(),
            Some(Companion {
                path: fs::canonicalize(&release).unwrap().join(DRIVER),
                sha256: crate::native_install::artifact::digest(HERDR_DRIVER),
            })
        );
        // Not a companion name, or a release that carries none: nothing.
        assert_eq!(active_companion(&active, "tmt-driver-other").unwrap(), None);
        let (_directory, layout, old) = published_layout();
        let four_files = layout
            .root
            .join("releases")
            .join(old.id.to_string())
            .join("tmt");
        assert_eq!(active_companion(&four_files, DRIVER).unwrap(), None);
    }

    #[test]
    fn first_party_approval_follows_the_receipt() {
        let (directory, active, release) = shipped();
        let global = directory.path.join("global");
        fs::create_dir_all(&global).unwrap();
        let runner = crate::process::UnixCommandRunner;
        // The shipped driver, as its receipt records it, approved by name.
        let inspected = registry::inspect_first_party(&global, "herdr", &active, &runner).unwrap();
        assert_eq!(inspected.source, DriverSource::FirstParty);
        assert_eq!(
            inspected.path,
            fs::canonicalize(&release).unwrap().join(DRIVER)
        );
        assert_eq!(
            inspected.digest,
            crate::native_install::artifact::digest(HERDR_DRIVER)
        );
        let approved = registry::commit(&global, inspected).unwrap();
        assert_eq!(
            registry::read(&global).unwrap(),
            std::slice::from_ref(&approved)
        );
        assert_eq!(
            registry::state(&global, &approved, &active, &runner),
            (ApprovalState::Ok, None)
        );
        // A shipped file that doesn't match its receipt is never run.
        fs::write(release.join(DRIVER), b"#!/bin/sh\necho swapped\n").unwrap();
        let error = registry::inspect_first_party(
            &global,
            "herdr",
            &active,
            &crate::process::UnixCommandRunner,
        )
        .unwrap_err();
        assert_eq!(error.code(), "DRIVER_UNSAFE", "{error}");
        // A release that ships none says so.
        let (_directory, layout, old) = published_layout();
        let four_files = layout
            .root
            .join("releases")
            .join(old.id.to_string())
            .join("tmt");
        let error = registry::inspect_first_party(
            &global,
            "herdr",
            &four_files,
            &crate::process::UnixCommandRunner,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("ships no herdr driver"),
            "{error}"
        );
    }

    /// A first-party record as approval would have written it.
    fn record(path: &Path, digest: &str) -> registry::DriverRecord {
        registry::DriverRecord {
            name: "herdr".into(),
            path: path.to_owned(),
            digest: digest.into(),
            fingerprint: crate::executable_trust::Fingerprint::of(
                &fs::metadata(path).unwrap(),
            ),
            protocol: 1,
            locations: None,
            capabilities: serde_json::from_value(serde_json::json!({"protocols":[1],"kind":"host","name":"herdr","version":"0.0.0-test","ops":["snapshot"],"paneId":{"prefix":"term_"},"target":"w{n}:p{n}","callerEnv":[]})).unwrap(),
            approved_at_ms: 1,
            source: DriverSource::FirstParty,
        }
    }

    #[test]
    fn a_first_party_driver_is_ok_while_it_matches_its_receipt() {
        let (directory, active, release) = shipped();
        let path = fs::canonicalize(&release).unwrap().join(DRIVER);
        let approved = record(
            &path,
            &crate::native_install::artifact::digest(HERDR_DRIVER),
        );
        let runner = crate::process::UnixCommandRunner;
        let global = directory.path.join("global");
        fs::create_dir_all(&global).unwrap();
        assert_eq!(
            registry::state(&global, &approved, &active, &runner),
            (ApprovalState::Ok, None)
        );
        // The shipped driver is the approved one: used as recorded, nothing
        // written.
        assert_eq!(
            registry::current_first_party(
                &global,
                &approved,
                &active,
                &crate::process::UnixCommandRunner
            ),
            Some(approved.clone())
        );
        assert!(!global.join(registry::REGISTRY_FILE).exists());
        fs::write(&path, b"#!/bin/sh\necho swapped\n").unwrap();
        assert_eq!(
            registry::state(&global, &approved, &active, &runner).0,
            ApprovalState::Changed
        );
        // A release that ships none: missing, and unavailable at run time.
        let (_directory, layout, old) = published_layout();
        let four_files = layout
            .root
            .join("releases")
            .join(old.id.to_string())
            .join("tmt");
        let (state, reason) = registry::state(&global, &approved, &four_files, &runner);
        assert_eq!(state, ApprovalState::Missing);
        assert!(reason.unwrap().contains("ships no herdr driver"));
        assert_eq!(
            registry::current_first_party(
                &global,
                &approved,
                &four_files,
                &crate::process::UnixCommandRunner
            ),
            None
        );
    }

    /// A driver that asks for more than the approved one: another operation
    /// and another environment variable.
    const HERDR_DRIVER_MORE: &[u8] = br#"#!/bin/sh
printf '%s' '{"ok":{"protocols":[1],"kind":"host","name":"herdr","version":"0.0.1-test","ops":["snapshot","capture"],"paneId":{"prefix":"term_"},"target":"w{n}:p{n}","callerEnv":["HERDR_PANE_ID"]}}'
"#;

    /// A driver that only reads one more environment variable.
    const HERDR_DRIVER_ENV: &[u8] = br#"#!/bin/sh
printf '%s' '{"ok":{"protocols":[1],"kind":"host","name":"herdr","version":"0.0.1-test","ops":["snapshot"],"paneId":{"prefix":"term_"},"target":"w{n}:p{n}","callerEnv":["HERDR_PANE_ID"]}}'
"#;

    /// A registry holding exactly `record`, as an earlier approval left it.
    fn registry_with(global: &Path, record: &registry::DriverRecord) {
        fs::create_dir_all(global).unwrap();
        fs::write(
            global.join(registry::REGISTRY_FILE),
            serde_json::to_vec(&serde_json::json!({"version": 1, "drivers": [record]})).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn an_upgraded_driver_within_its_approval_is_adopted_without_asking() {
        let (directory, active, release) = shipped();
        let path = fs::canonicalize(&release).unwrap().join(DRIVER);
        // Approved in an earlier release, whose driver had other bytes.
        let approved = record(
            &path,
            &crate::native_install::artifact::digest(b"older driver"),
        );
        let global = directory.path.join("global");
        registry_with(&global, &approved);
        let runner = crate::process::UnixCommandRunner;
        let adopted = registry::current_first_party(&global, &approved, &active, &runner).unwrap();
        assert_eq!(
            adopted.digest,
            crate::native_install::artifact::digest(HERDR_DRIVER)
        );
        assert_eq!(adopted.approved_at_ms, approved.approved_at_ms);
        assert_eq!(
            registry::read(&global).unwrap(),
            std::slice::from_ref(&adopted)
        );
        assert_eq!(
            registry::state(&global, &adopted, &active, &runner),
            (ApprovalState::Ok, None)
        );
    }

    #[test]
    fn an_upgraded_driver_asking_for_more_needs_approval_again() {
        for (driver, extra) in [
            (HERDR_DRIVER_MORE, "now also runs capture"),
            (HERDR_DRIVER_ENV, "now also reads HERDR_PANE_ID"),
        ] {
            asks_for_more(driver, extra);
        }
    }

    fn asks_for_more(driver: &'static [u8], extra: &str) {
        let (directory, active, release) = shipping(driver);
        let path = fs::canonicalize(&release).unwrap().join(DRIVER);
        let approved = record(
            &path,
            &crate::native_install::artifact::digest(b"older driver"),
        );
        let global = directory.path.join("global");
        registry_with(&global, &approved);
        let before = fs::read(global.join(registry::REGISTRY_FILE)).unwrap();
        let runner = crate::process::UnixCommandRunner;
        assert_eq!(
            registry::current_first_party(&global, &approved, &active, &runner),
            None
        );
        let (state, reason) = registry::state(&global, &approved, &active, &runner);
        assert_eq!(state, ApprovalState::Changed);
        let reason = reason.unwrap();
        assert!(reason.contains(extra), "{reason}");
        // Nothing beyond the approval was recorded.
        assert_eq!(
            fs::read(global.join(registry::REGISTRY_FILE)).unwrap(),
            before
        );
        // `tmt driver install herdr` approves what it now asks for.
        let inspected = registry::inspect_first_party(&global, "herdr", &active, &runner).unwrap();
        let reapproved = registry::commit(&global, inspected).unwrap();
        assert_eq!(
            reapproved.digest,
            crate::native_install::artifact::digest(driver)
        );
        assert_eq!(
            registry::read(&global).unwrap(),
            std::slice::from_ref(&reapproved)
        );
        assert_eq!(
            registry::state(&global, &reapproved, &active, &runner),
            (ApprovalState::Ok, None)
        );
        assert_eq!(
            registry::current_first_party(&global, &reapproved, &active, &runner),
            Some(reapproved)
        );
    }
}
