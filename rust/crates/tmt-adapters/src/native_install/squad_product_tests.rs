//! Squad is a non-Office extension product: the fixed table alone gives it an
//! installation with two command links, and nothing Office-specific runs.

use super::*;
use crate::native_install::{Product, inspect_product, uninstall_extension};
use tmt_core::native_install::PinAction;

fn squad_fixture(version: &str, payload: &[u8]) -> Fixture {
    let root = format!("tmux-team-{version}-aarch64-apple-darwin");
    let mut entries = valid_entries(&root);
    entries[0] = Entry::File {
        path: format!("{root}/tmt-ops"),
        bytes: payload.to_vec(),
        mode: 0o755,
    };
    product_fixture_at(entries, "tmt-ops", &Product::Ops.files(), version)
}

fn install(
    fixture: &Fixture,
    prefix: &std::path::Path,
    pin: PinAction,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<super::super::InstallReport> {
    super::super::install_product(
        Product::Ops,
        super::super::InstallRequest {
            archive: &fixture.archive,
            manifest: &fixture.manifest,
            prefix,
            target: TARGET,
            channel: tmt_core::native_install::Channel::Stable,
            pin,
        },
        None,
        &mut checkpoint,
    )
}

fn links(prefix: &std::path::Path) -> Vec<PathBuf> {
    Product::Ops
        .links()
        .iter()
        .map(|name| fs::read_link(prefix.join("bin").join(name)).unwrap())
        .collect()
}

#[test]
fn ops_installs_one_link_repeats_as_a_no_op_and_leaves_the_cli_alone() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let squad = squad_fixture("1.2.3", b"squad 1.2.3\n");
    let prefix = cli.directory.path.join("prefix");
    let cli_report = install_fixture(&cli, &prefix, Product::Cli).unwrap();
    let report = install(&squad, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    assert!(report.changed);
    assert_eq!(
        report.executable,
        fs::canonicalize(&prefix).unwrap().join("bin/tmt-ops")
    );
    assert_eq!(
        links(&prefix),
        vec![PathBuf::from("../lib/tmt-ops/current/tmt-ops")]
    );
    assert_eq!(
        fs::read(prefix.join("bin/tmt-ops")).unwrap(),
        b"squad 1.2.3\n"
    );
    let again = install(&squad, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    assert!(!again.changed);
    assert_eq!(again.active_executable, report.active_executable);
    inspect_product(Product::Ops, &report.executable).unwrap();
    assert!(inspect_product(Product::Office, &report.executable).is_err());
    assert_eq!(
        crate::native_install::inspect(&cli_report.executable)
            .unwrap()
            .active_executable,
        cli_report.active_executable
    );
    assert!(!prefix.join("lib/tmt-office").exists());
}

#[test]
fn squad_upgrades_and_a_pin_holds_until_cleared() {
    let first = squad_fixture("1.2.3", b"squad 1.2.3\n");
    let prefix = first.directory.path.join("prefix");
    install(&first, &prefix, PinAction::PinCandidate, || Ok(())).unwrap();
    let executable = prefix.join("bin/tmt-ops");
    let pinned = inspect_product(Product::Ops, &executable).unwrap();
    assert_eq!(pinned.state.pinned_version.unwrap().to_string(), "1.2.3");

    let newer = squad_fixture("1.2.4", b"squad 1.2.4\n");
    assert!(install(&newer, &prefix, PinAction::Preserve, || Ok(())).is_err());
    assert_eq!(fs::read(&executable).unwrap(), b"squad 1.2.3\n");
    let upgraded = install(&newer, &prefix, PinAction::Clear, || Ok(())).unwrap();
    assert!(upgraded.changed);
    assert_eq!(
        fs::read(prefix.join("bin/tmt-ops")).unwrap(),
        b"squad 1.2.4\n"
    );
    let state = inspect_product(Product::Ops, &executable).unwrap().state;
    assert_eq!(
        (state.version.to_string(), state.pinned_version),
        ("1.2.4".into(), None)
    );
    // Both releases are retained; only the current pointer moved.
    assert_eq!(
        fs::read_dir(prefix.join("lib/tmt-ops/releases"))
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn interrupted_squad_publication_keeps_the_previous_release_active() {
    let first = squad_fixture("1.2.3", b"squad 1.2.3\n");
    let prefix = first.directory.path.join("prefix");
    install(&first, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    let root = prefix.join("lib/tmt-ops");
    let current = fs::read_link(root.join("current")).unwrap();
    let newer = squad_fixture("1.2.4", b"squad 1.2.4\n");
    let mut calls = 0;
    let error = install(&newer, &prefix, PinAction::Preserve, || {
        calls += 1;
        if calls == 4 {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "test cancellation",
            ))
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(fs::read_link(root.join("current")).unwrap(), current);
    assert_eq!(fs::read_dir(root.join("releases")).unwrap().count(), 1);
    assert_eq!(
        fs::read(prefix.join("bin/tmt-ops")).unwrap(),
        b"squad 1.2.3\n"
    );
}

#[test]
fn uninstall_removes_the_link_and_keeps_releases() {
    let squad = squad_fixture("1.2.3", b"squad 1.2.3\n");
    let prefix = squad.directory.path.join("prefix");
    install(&squad, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    assert!(uninstall_extension(&prefix, Product::Ops).unwrap());
    for name in Product::Ops.links() {
        assert!(fs::symlink_metadata(prefix.join("bin").join(name)).is_err());
    }
    assert!(!prefix.join("lib/tmt-ops/current").exists());
    assert_eq!(
        fs::read_dir(prefix.join("lib/tmt-ops/releases"))
            .unwrap()
            .count(),
        1
    );
    assert!(
        !uninstall_extension(&prefix, Product::Ops).unwrap(),
        "repeat"
    );
    // Reinstalling the retained release reactivates it.
    assert!(
        install(&squad, &prefix, PinAction::Preserve, || Ok(()))
            .unwrap()
            .changed
    );
}

#[test]
fn uninstall_refuses_an_unmanaged_link_and_changes_nothing() {
    let squad = squad_fixture("1.2.3", b"squad 1.2.3\n");
    let prefix = squad.directory.path.join("prefix");
    install(&squad, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    fs::remove_file(prefix.join("bin/tmt-ops")).unwrap();
    fs::write(prefix.join("bin/tmt-ops"), b"user command").unwrap();
    assert!(uninstall_extension(&prefix, Product::Ops).is_err());
    assert_eq!(
        fs::read(prefix.join("bin/tmt-ops")).unwrap(),
        b"user command"
    );
    assert!(prefix.join("bin/tmt-ops").exists());
    assert!(prefix.join("lib/tmt-ops/current").exists());

    // With no installation, a same-named user command is still refused.
    let bare = TestDirectory::new();
    fs::create_dir_all(bare.path.join("bin")).unwrap();
    fs::write(bare.path.join("bin/tmt-ops"), b"user command").unwrap();
    assert!(uninstall_extension(&bare.path, Product::Ops).is_err());
    assert!(
        uninstall_extension(&bare.path, Product::Cli).is_err(),
        "not an extension"
    );
}

#[test]
fn squad_reads_a_receipt_recorded_under_the_pre_rename_repository() {
    let squad = squad_fixture("1.2.3", b"squad 1.2.3\n");
    let prefix = squad.directory.path.join("prefix");
    let report = install(&squad, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    record_release_source(&report.executable, "wkh237/tmt");
    inspect_product(Product::Ops, &report.executable).unwrap();
    record_release_source(&report.executable, "attacker/other");
    assert!(inspect_product(Product::Ops, &report.executable).is_err());
}

/// Build a synthetic historical receipt by projecting an independently
/// verified fixture into the former executable and layout. Receipt schema,
/// bytes and hashes match the shipped historical format; no user state is used.
fn former_install(prefix: &std::path::Path) {
    let fixture = squad_fixture("1.2.3", b"old squad\n");
    let report = install(&fixture, prefix, PinAction::Preserve, || Ok(())).unwrap();
    let release = report.active_executable.parent().unwrap();
    let receipt_path = release.join("receipt.json");
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
    let digest = receipt["file_sha256"]
        .as_object_mut()
        .unwrap()
        .remove("tmt-ops")
        .unwrap();
    receipt["file_sha256"]["tmt-squad"] = digest;
    receipt["archive"] = receipt["archive"]
        .as_str()
        .unwrap()
        .replace("tmt-ops", "tmt-squad")
        .into();
    fs::rename(release.join("tmt-ops"), release.join("tmt-squad")).unwrap();
    fs::write(receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    fs::remove_file(prefix.join("bin/tmt-ops")).unwrap();
    fs::rename(prefix.join("lib/tmt-ops"), prefix.join("lib/tmt-squad")).unwrap();
    for name in ["tmt-squad", "tmt-sq"] {
        std::os::unix::fs::symlink(
            "../lib/tmt-squad/current/tmt-squad",
            prefix.join("bin").join(name),
        )
        .unwrap();
    }
}

#[test]
fn ops_replacement_verifies_both_receipts_and_removes_only_links_into_the_former_namespace() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");
    former_install(&prefix);
    let prefix = fs::canonicalize(prefix).unwrap();
    let application = directory.path.join("application/ops/cron.json");
    fs::create_dir_all(application.parent().unwrap()).unwrap();
    fs::write(&application, "retained application state").unwrap();
    let old = crate::native_install::inspect_product_prefix(Product::Ops, &prefix).unwrap();
    assert!(
        old.active_executable
            .starts_with(prefix.join("lib/tmt-squad"))
    );
    assert_eq!(old.state.version.to_string(), "1.2.3");
    assert!(super::super::finish_product_replacement(&prefix, Product::Ops).is_err());
    assert!(prefix.join("bin/tmt-squad").exists());
    let next = squad_fixture("1.2.4", b"ops\n");
    let installed = install(&next, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    assert!(installed.changed);
    assert!(
        prefix.join("bin/tmt-sq").exists(),
        "settlement follows publication"
    );
    // A same-named foreign link is retained even though its name is historical.
    fs::remove_file(prefix.join("bin/tmt-sq")).unwrap();
    std::os::unix::fs::symlink(&application, prefix.join("bin/tmt-sq")).unwrap();
    let removed = super::super::finish_product_replacement(&prefix, Product::Ops).unwrap();
    assert_eq!(
        removed.removed,
        [prefix.join("bin/tmt-squad"), prefix.join("lib/tmt-squad")]
    );
    assert_eq!(removed.kept, [prefix.join("bin/tmt-sq")]);
    assert_eq!(
        fs::read_to_string(application).unwrap(),
        "retained application state"
    );
    assert!(!prefix.join("lib/tmt-squad").exists());
    assert!(prefix.join("bin/tmt-ops").exists());
    assert_eq!(
        super::super::finish_product_replacement(&prefix, Product::Ops).unwrap(),
        Default::default()
    );
}

#[test]
fn modified_former_receipt_or_binary_refuses_replacement_without_removing_links() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");
    former_install(&prefix);
    let prefix = fs::canonicalize(prefix).unwrap();
    fs::write(prefix.join("bin/tmt-squad"), "modified").unwrap();
    let next = squad_fixture("1.2.4", b"ops\n");
    assert!(install(&next, &prefix, PinAction::Preserve, || Ok(())).is_err());
    assert!(prefix.join("lib/tmt-squad").exists());
    assert_eq!(
        fs::read_to_string(prefix.join("bin/tmt-sq")).unwrap(),
        "modified"
    );
    assert!(!prefix.join("bin/tmt-ops").exists());
}

#[test]
fn full_uninstall_removes_verified_former_install_without_a_successor_and_retains_foreign_links() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");
    former_install(&prefix);
    let prefix = fs::canonicalize(prefix).unwrap();
    let application = directory.path.join("application/ops/cron.json");
    fs::create_dir_all(application.parent().unwrap()).unwrap();
    fs::write(&application, "retained application state").unwrap();
    fs::remove_file(prefix.join("bin/tmt-sq")).unwrap();
    std::os::unix::fs::symlink(&application, prefix.join("bin/tmt-sq")).unwrap();
    let expected = super::super::ProductRemoval {
        removed: vec![prefix.join("bin/tmt-squad"), prefix.join("lib/tmt-squad")],
        kept: vec![prefix.join("bin/tmt-sq")],
    };
    assert_eq!(
        super::super::plan_product_removal(&prefix, Product::Ops).unwrap(),
        expected
    );
    assert!(
        prefix.join("lib/tmt-squad").exists(),
        "planning is read-only"
    );
    assert_eq!(
        super::super::remove_product(&prefix, Product::Ops).unwrap(),
        expected
    );
    assert!(!prefix.join("lib/tmt-squad").exists());
    assert!(!prefix.join("bin/tmt-squad").exists());
    assert_eq!(
        fs::read_to_string(&application).unwrap(),
        "retained application state"
    );
    assert_eq!(
        fs::read_link(prefix.join("bin/tmt-sq")).unwrap(),
        application
    );
    assert_eq!(
        super::super::remove_product(&prefix, Product::Ops).unwrap(),
        Default::default()
    );
}

#[test]
fn full_uninstall_refuses_a_damaged_former_install_before_removing_current_or_former_files() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");
    former_install(&prefix);
    let prefix = fs::canonicalize(prefix).unwrap();
    let next = squad_fixture("1.2.4", b"ops\n");
    install(&next, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    fs::write(prefix.join("bin/tmt-squad"), "modified").unwrap();
    assert!(super::super::plan_product_removal(&prefix, Product::Ops).is_err());
    assert!(super::super::remove_product(&prefix, Product::Ops).is_err());
    for path in [
        "bin/tmt-ops",
        "bin/tmt-squad",
        "bin/tmt-sq",
        "lib/tmt-ops",
        "lib/tmt-squad",
    ] {
        assert!(prefix.join(path).exists(), "{path} remains on refusal");
    }
}
