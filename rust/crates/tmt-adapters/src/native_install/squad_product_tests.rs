//! Squad is a non-Office extension product: the fixed table alone gives it an
//! installation with two command links, and nothing Office-specific runs.

use super::*;
use crate::native_install::{Product, inspect_product, uninstall_extension};
use tmt_core::native_install::PinAction;

fn squad_fixture(version: &str, payload: &[u8]) -> Fixture {
    let root = format!("tmux-team-{version}-aarch64-apple-darwin");
    let mut entries = valid_entries(&root);
    entries[0] = Entry::File {
        path: format!("{root}/tmt-squad"),
        bytes: payload.to_vec(),
        mode: 0o755,
    };
    product_fixture_at(entries, "tmt-squad", &Product::Squad.files(), version)
}

fn install(
    fixture: &Fixture,
    prefix: &std::path::Path,
    pin: PinAction,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<super::super::InstallReport> {
    super::super::install_product(
        Product::Squad,
        super::super::InstallRequest {
            archive: &fixture.archive,
            manifest: &fixture.manifest,
            prefix,
            target: TARGET,
            channel: tmt_core::native_install::Channel::Stable,
            pin,
        },
        &mut checkpoint,
    )
}

fn links(prefix: &std::path::Path) -> Vec<PathBuf> {
    Product::Squad
        .links()
        .iter()
        .map(|name| fs::read_link(prefix.join("bin").join(name)).unwrap())
        .collect()
}

#[test]
fn squad_installs_both_links_repeats_as_a_no_op_and_leaves_the_cli_alone() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let squad = squad_fixture("1.2.3", b"squad 1.2.3\n");
    let prefix = cli.directory.path.join("prefix");
    let cli_report = install_fixture(&cli, &prefix, Product::Cli).unwrap();
    let report = install(&squad, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    assert!(report.changed);
    assert_eq!(
        report.executable,
        fs::canonicalize(&prefix).unwrap().join("bin/tmt-squad")
    );
    assert_eq!(
        links(&prefix),
        vec![PathBuf::from("../lib/tmt-squad/current/tmt-squad"); 2]
    );
    assert_eq!(
        fs::read(prefix.join("bin/tmt-sq")).unwrap(),
        b"squad 1.2.3\n"
    );
    let again = install(&squad, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    assert!(!again.changed);
    assert_eq!(again.active_executable, report.active_executable);
    inspect_product(Product::Squad, &report.executable).unwrap();
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
    let executable = prefix.join("bin/tmt-squad");
    let pinned = inspect_product(Product::Squad, &executable).unwrap();
    assert_eq!(pinned.state.pinned_version.unwrap().to_string(), "1.2.3");

    let newer = squad_fixture("1.2.4", b"squad 1.2.4\n");
    assert!(install(&newer, &prefix, PinAction::Preserve, || Ok(())).is_err());
    assert_eq!(fs::read(&executable).unwrap(), b"squad 1.2.3\n");
    let upgraded = install(&newer, &prefix, PinAction::Clear, || Ok(())).unwrap();
    assert!(upgraded.changed);
    assert_eq!(
        fs::read(prefix.join("bin/tmt-sq")).unwrap(),
        b"squad 1.2.4\n"
    );
    let state = inspect_product(Product::Squad, &executable).unwrap().state;
    assert_eq!(
        (state.version.to_string(), state.pinned_version),
        ("1.2.4".into(), None)
    );
    // Both releases are retained; only the current pointer moved.
    assert_eq!(
        fs::read_dir(prefix.join("lib/tmt-squad/releases"))
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
    let root = prefix.join("lib/tmt-squad");
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
        fs::read(prefix.join("bin/tmt-sq")).unwrap(),
        b"squad 1.2.3\n"
    );
}

#[test]
fn uninstall_removes_both_links_and_keeps_releases() {
    let squad = squad_fixture("1.2.3", b"squad 1.2.3\n");
    let prefix = squad.directory.path.join("prefix");
    install(&squad, &prefix, PinAction::Preserve, || Ok(())).unwrap();
    assert!(uninstall_extension(&prefix, Product::Squad).unwrap());
    for name in Product::Squad.links() {
        assert!(fs::symlink_metadata(prefix.join("bin").join(name)).is_err());
    }
    assert!(!prefix.join("lib/tmt-squad/current").exists());
    assert_eq!(
        fs::read_dir(prefix.join("lib/tmt-squad/releases"))
            .unwrap()
            .count(),
        1
    );
    assert!(
        !uninstall_extension(&prefix, Product::Squad).unwrap(),
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
    fs::remove_file(prefix.join("bin/tmt-sq")).unwrap();
    fs::write(prefix.join("bin/tmt-sq"), b"user command").unwrap();
    assert!(uninstall_extension(&prefix, Product::Squad).is_err());
    assert_eq!(
        fs::read(prefix.join("bin/tmt-sq")).unwrap(),
        b"user command"
    );
    assert!(prefix.join("bin/tmt-squad").exists());
    assert!(prefix.join("lib/tmt-squad/current").exists());

    // With no installation, a same-named user command is still refused.
    let bare = TestDirectory::new();
    fs::create_dir_all(bare.path.join("bin")).unwrap();
    fs::write(bare.path.join("bin/tmt-sq"), b"user command").unwrap();
    assert!(uninstall_extension(&bare.path, Product::Squad).is_err());
    assert!(
        uninstall_extension(&bare.path, Product::Cli).is_err(),
        "not an extension"
    );
}
