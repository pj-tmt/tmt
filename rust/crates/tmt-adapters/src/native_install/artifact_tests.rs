use super::artifact::{self, FILES};
use crate::test_support::TestDirectory;
use flate2::{Compression, write::GzEncoder};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Read},
    path::PathBuf,
};
use tar::{Builder, EntryType, Header};

const TARGET: &str = "aarch64-apple-darwin";
const OPS_PAYLOAD: &[u8] = b"#!/bin/sh\nexit 0\n";

#[path = "companion_archive_tests.rs"]
mod companion_archive_tests;
#[path = "skills_release_tests.rs"]
mod skills_release_tests;
#[path = "squad_product_tests.rs"]
mod squad_product_tests;
#[path = "uses_tests.rs"]
mod uses_tests;

enum Entry {
    File {
        path: String,
        bytes: Vec<u8>,
        mode: u32,
    },
    AdversarialFile {
        path: String,
        bytes: Vec<u8>,
        mode: u32,
    },
    Symlink {
        path: String,
        target: String,
    },
    /// A directory record as archivers write it, with its trailing slash.
    Directory {
        path: String,
    },
}

struct Fixture {
    directory: TestDirectory,
    manifest: PathBuf,
    archive: PathBuf,
    name: String,
}

fn valid_entries(root: &str) -> Vec<Entry> {
    vec![
        Entry::File {
            path: format!("{root}/tmt"),
            bytes: b"native executable\n".to_vec(),
            mode: 0o755,
        },
        Entry::File {
            path: format!("{root}/LICENSE"),
            bytes: b"MIT\n".to_vec(),
            mode: 0o644,
        },
        Entry::File {
            path: format!("{root}/NATIVE-INSTALL.md"),
            bytes: "Native install\n".as_bytes().to_vec(),
            mode: 0o644,
        },
        Entry::File {
            path: format!("{root}/THIRD-PARTY-NOTICES.txt"),
            bytes: b"Third-party notices\n".to_vec(),
            mode: 0o644,
        },
    ]
}

fn append_entry(builder: &mut Builder<GzEncoder<Vec<u8>>>, entry: Entry) {
    match entry {
        Entry::File { path, bytes, mode } => {
            let mut header = Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_entry_type(EntryType::Regular);
            header.set_mode(mode);
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            builder.append(&header, bytes.as_slice()).unwrap();
        }
        Entry::AdversarialFile { path, bytes, mode } => {
            let mut header = Header::new_gnu();
            header.set_path("safe-placeholder").unwrap();
            header.set_entry_type(EntryType::Regular);
            header.set_mode(mode);
            header.set_size(bytes.len() as u64);
            assert!(path.is_ascii() && path.len() <= 100);
            // Bypass tar::Header::set_path only to create a valid checksum-bearing archive with
            // a traversal name; production decode must reject it before any extraction.
            let raw = header.as_mut_bytes();
            raw[..100].fill(0);
            raw[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            builder.append(&header, bytes.as_slice()).unwrap();
        }
        Entry::Symlink { path, target } => {
            let mut header = Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_entry_type(EntryType::Symlink);
            header.set_mode(0o777);
            header.set_size(0);
            header.set_link_name(target).unwrap();
            header.set_cksum();
            builder.append(&header, &[][..]).unwrap();
        }
        Entry::Directory { path } => {
            let mut header = Header::new_gnu();
            header.set_path("placeholder").unwrap();
            header.set_entry_type(EntryType::Directory);
            header.set_mode(0o755);
            header.set_size(0);
            assert!(path.is_ascii() && path.len() <= 100);
            let raw = header.as_mut_bytes();
            raw[..100].fill(0);
            raw[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            builder.append(&header, &[][..]).unwrap();
        }
    }
}

fn gzip_tar(entries: Vec<Entry>) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);
    for entry in entries {
        append_entry(&mut builder, entry);
    }
    builder.finish().unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

fn fixture(entries: Vec<Entry>) -> Fixture {
    product_fixture(entries, "tmt-cli", &FILES)
}

fn product_fixture(entries: Vec<Entry>, package: &str, files: &[&str]) -> Fixture {
    product_fixture_at(entries, package, files, "1.2.3")
}

fn product_fixture_at(
    entries: Vec<Entry>,
    package: &str,
    files: &[&str],
    version: &str,
) -> Fixture {
    let directory = TestDirectory::new();
    let name = format!("tmux-team-{version}-aarch64-apple-darwin.tar.gz");
    let archive = directory.path.join(&name);
    let manifest = directory.path.join("manifest.json");
    let compressed = gzip_tar(entries);
    let checksum = artifact::digest(&compressed);
    fs::write(&archive, compressed).unwrap();
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({
            "artifacts": {
                name.clone(): {
                    "kind": "executable-zip",
                    "name": name.clone(),
                    "target_triples": [TARGET],
                    "checksums": {"sha256": checksum},
                    "assets": files.iter().map(|path| json!({"path": path})).collect::<Vec<_>>(),
                }
            },
            "releases": [{
                "app_name": package,
                "app_version": version,
                "artifacts": [name.clone()]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    Fixture {
        directory,
        manifest,
        archive,
        name,
    }
}

fn ops_fixture() -> Fixture {
    ops_fixture_with_payload(OPS_PAYLOAD)
}

fn ops_fixture_with_payload(payload: &[u8]) -> Fixture {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut entries = valid_entries(root);
    entries[0] = Entry::File {
        path: format!("{root}/tmt-ops"),
        bytes: payload.to_vec(),
        mode: 0o755,
    };
    product_fixture(
        entries,
        "tmt-ops",
        &[
            "tmt-ops",
            "LICENSE",
            "NATIVE-INSTALL.md",
            "THIRD-PARTY-NOTICES.txt",
        ],
    )
}

fn install_fixture(
    fixture: &Fixture,
    prefix: &std::path::Path,
    product: super::Product,
) -> io::Result<super::InstallReport> {
    super::install_product(
        product,
        super::InstallRequest {
            archive: &fixture.archive,
            manifest: &fixture.manifest,
            prefix,
            target: TARGET,
            channel: tmt_core::native_install::Channel::Stable,
            pin: tmt_core::native_install::PinAction::Preserve,
        },
        None,
        || Ok(()),
    )
}

fn extension_fixture(product: super::Product, version: &str, payload: &[u8]) -> Fixture {
    let root = format!("tmux-team-{version}-aarch64-apple-darwin");
    let mut entries = valid_entries(&root);
    entries[0] = Entry::File {
        path: format!("{root}/{}", product.executable()),
        bytes: payload.to_vec(),
        mode: 0o755,
    };
    product_fixture_at(entries, product.package(), &product.files(), version)
}

#[test]
fn extension_receipt_round_trip_repeat_upgrade_and_removal_leave_cli_and_state_unchanged() {
    use super::{Product, inspect_product, uninstall_extension};
    for product in [Product::Remote, Product::Colab, Product::Digest] {
        let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
        let prefix = cli.directory.path.join("prefix");
        let cli_report = install_fixture(&cli, &prefix, Product::Cli).unwrap();
        let state = cli.directory.path.join("data").join(product.as_str());
        fs::create_dir_all(&state).unwrap();
        fs::write(state.join("machine.key"), b"retained machine key").unwrap();
        fs::write(state.join("state.db"), b"retained grants").unwrap();
        let first = extension_fixture(product, "1.2.3", b"extension 1.2.3\n");
        let report = install_fixture(&first, &prefix, product).unwrap();
        assert!(report.changed);
        assert_eq!(
            fs::read_link(&report.executable).unwrap(),
            PathBuf::from(product.link_target())
        );
        let installed = inspect_product(product, &report.executable).unwrap();
        assert_eq!(installed.state.version.to_string(), "1.2.3");
        assert_eq!(
            installed.state.channel,
            tmt_core::native_install::Channel::Stable
        );
        assert!(installed.state.pinned_version.is_none());
        for other in Product::ALL.into_iter().filter(|other| *other != product) {
            assert!(inspect_product(other, &report.executable).is_err());
        }
        let repeat = install_fixture(&first, &prefix, product).unwrap();
        assert!(!repeat.changed);
        assert_eq!(repeat.active_executable, report.active_executable);
        let newer = extension_fixture(product, "1.2.4", b"extension 1.2.4\n");
        assert!(install_fixture(&newer, &prefix, product).unwrap().changed);
        assert_eq!(
            fs::read(&report.active_executable).unwrap(),
            b"extension 1.2.3\n"
        );
        assert_eq!(fs::read(&report.executable).unwrap(), b"extension 1.2.4\n");
        assert!(install_fixture(&first, &prefix, product).is_err());
        assert!(uninstall_extension(&prefix, product).unwrap());
        assert!(!prefix.join("bin").join(product.executable()).exists());
        assert_eq!(
            fs::read_dir(prefix.join(product.namespace()).join("releases"))
                .unwrap()
                .count(),
            2
        );
        assert_eq!(
            inspect_product(Product::Cli, &cli_report.executable)
                .unwrap()
                .active_executable,
            cli_report.active_executable
        );
        assert_eq!(
            fs::read(state.join("machine.key")).unwrap(),
            b"retained machine key"
        );
        assert_eq!(
            fs::read(state.join("state.db")).unwrap(),
            b"retained grants"
        );
    }
}

#[test]
fn extensions_reject_wrong_packages_tampering_and_foreign_command_links() {
    use super::{Product, inspect_product, uninstall_extension};
    for product in [Product::Remote, Product::Colab, Product::Digest] {
        let extension = extension_fixture(product, "1.2.3", b"extension bytes");
        let prefix = extension.directory.path.join("prefix");
        for other in Product::ALL.into_iter().filter(|other| *other != product) {
            assert!(install_fixture(&extension, &prefix, other).is_err());
            assert!(!prefix.exists());
        }
        let report = install_fixture(&extension, &prefix, product).unwrap();
        fs::write(&report.active_executable, b"modified bytes").unwrap();
        assert!(inspect_product(product, &report.executable).is_err());
        assert!(install_fixture(&extension, &prefix, product).is_err());
        fs::remove_file(&report.executable).unwrap();
        fs::write(&report.executable, b"user-owned command").unwrap();
        assert!(uninstall_extension(&prefix, product).is_err());
        assert_eq!(fs::read(&report.executable).unwrap(), b"user-owned command");
    }
}

#[test]
fn interrupted_extension_activation_retains_the_previous_receipt_and_cleans_staging() {
    use super::Product;
    for product in [Product::Remote, Product::Colab, Product::Digest] {
        let extension = extension_fixture(product, "1.2.3", b"extension old");
        let prefix = extension.directory.path.join("prefix");
        let report = install_fixture(&extension, &prefix, product).unwrap();
        let root = prefix.join(product.namespace());
        let current = fs::read_link(root.join("current")).unwrap();
        let receipt = fs::read(
            report
                .active_executable
                .parent()
                .unwrap()
                .join("receipt.json"),
        )
        .unwrap();
        let newer = extension_fixture(product, "1.2.4", b"extension new");
        let mut calls = 0;
        let error = super::install_product(
            product,
            super::InstallRequest {
                archive: &newer.archive,
                manifest: &newer.manifest,
                prefix: &prefix,
                target: TARGET,
                channel: tmt_core::native_install::Channel::Stable,
                pin: tmt_core::native_install::PinAction::Preserve,
            },
            None,
            || {
                calls += 1;
                if calls == 4 {
                    Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read_link(root.join("current")).unwrap(), current);
        assert_eq!(fs::read(&report.executable).unwrap(), b"extension old");
        assert_eq!(
            fs::read(
                report
                    .active_executable
                    .parent()
                    .unwrap()
                    .join("receipt.json")
            )
            .unwrap(),
            receipt
        );
        assert_eq!(fs::read_dir(root.join("releases")).unwrap().count(), 1);
        assert!(!fs::read_dir(&root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".staging-")
        }));
    }
}

#[test]
fn ops_installation_and_exact_retry_preserve_cli_ownership_and_bytes() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let ops = ops_fixture();
    let prefix = cli.directory.path.join("prefix");
    let cli_report = install_fixture(&cli, &prefix, super::Product::Cli).unwrap();
    let cli_receipt = fs::read(
        cli_report
            .active_executable
            .parent()
            .unwrap()
            .join("receipt.json"),
    )
    .unwrap();
    let cli_pointer = fs::read_link(prefix.join("lib/tmt/current")).unwrap();
    let ops_report = install_fixture(&ops, &prefix, super::Product::Ops).unwrap();
    assert!(ops_report.changed);
    assert_eq!(
        ops_report.executable,
        fs::canonicalize(&prefix).unwrap().join("bin/tmt-ops")
    );
    assert_eq!(fs::read(&ops_report.executable).unwrap(), OPS_PAYLOAD);
    assert_eq!(
        fs::read_link(&ops_report.executable).unwrap(),
        PathBuf::from("../lib/tmt-ops/current/tmt-ops")
    );
    let retry = install_fixture(&ops, &prefix, super::Product::Ops).unwrap();
    assert!(!retry.changed);
    assert_eq!(retry.active_executable, ops_report.active_executable);
    assert_eq!(
        fs::read_link(prefix.join("lib/tmt/current")).unwrap(),
        cli_pointer
    );
    assert_eq!(
        fs::read(&cli_report.executable).unwrap(),
        b"native executable\n"
    );
    assert_eq!(
        fs::read(
            cli_report
                .active_executable
                .parent()
                .unwrap()
                .join("receipt.json")
        )
        .unwrap(),
        cli_receipt
    );
    super::inspect(&cli_report.executable).unwrap();
    super::inspect_product(super::Product::Ops, &ops_report.executable).unwrap();
    assert!(super::inspect(&ops_report.executable).is_err());
    assert!(super::inspect_product(super::Product::Ops, &cli_report.executable).is_err());
}

#[test]
fn cross_product_archives_are_rejected_before_creating_installation_prefix() {
    for (fixture, product) in [
        (ops_fixture(), super::Product::Cli),
        (
            fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin")),
            super::Product::Ops,
        ),
    ] {
        let prefix = fixture.directory.path.join("prefix");
        assert!(install_fixture(&fixture, &prefix, product).is_err());
        assert!(!prefix.exists());
        assert_only_inputs_remain(&fixture);
    }
}

#[test]
fn interrupted_ops_pin_preserves_both_active_releases() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let ops = ops_fixture();
    let prefix = cli.directory.path.join("prefix");
    let cli_report = install_fixture(&cli, &prefix, super::Product::Cli).unwrap();
    let ops_report = install_fixture(&ops, &prefix, super::Product::Ops).unwrap();
    let ops_root = prefix.join("lib/tmt-ops");
    let current = fs::read_link(ops_root.join("current")).unwrap();
    let mut calls = 0;
    let error = super::install_product(
        super::Product::Ops,
        super::InstallRequest {
            archive: &ops.archive,
            manifest: &ops.manifest,
            prefix: &prefix,
            target: TARGET,
            channel: tmt_core::native_install::Channel::Stable,
            pin: tmt_core::native_install::PinAction::PinCandidate,
        },
        Some(&|_: &std::path::Path, _: &semver::Version| Ok(())),
        || {
            calls += 1;
            if calls == 4 {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "test cancellation",
                ))
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(calls, 4);
    assert_eq!(fs::read_link(ops_root.join("current")).unwrap(), current);
    assert_eq!(fs::read_dir(ops_root.join("releases")).unwrap().count(), 1);
    assert!(
        super::inspect_product(super::Product::Ops, &ops_report.executable)
            .unwrap()
            .state
            .pinned_version
            .is_none()
    );
    assert_eq!(
        super::inspect(&cli_report.executable)
            .unwrap()
            .active_executable,
        cli_report.active_executable
    );
    assert_eq!(fs::read(&ops_report.executable).unwrap(), OPS_PAYLOAD);
}

#[test]
fn ops_command_collision_does_not_overwrite_user_file_or_cli() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let ops = ops_fixture();
    let prefix = cli.directory.path.join("prefix");
    let cli_report = install_fixture(&cli, &prefix, super::Product::Cli).unwrap();
    fs::write(prefix.join("bin/tmt-ops"), b"user managed command").unwrap();
    assert!(install_fixture(&ops, &prefix, super::Product::Ops).is_err());
    assert_eq!(
        fs::read(prefix.join("bin/tmt-ops")).unwrap(),
        b"user managed command"
    );
    assert!(!prefix.join("lib/tmt-ops/current").exists());
    assert_eq!(
        super::inspect(&cli_report.executable)
            .unwrap()
            .active_executable,
        cli_report.active_executable
    );
}

fn replace_archive(fixture: &Fixture, compressed: &[u8]) {
    fs::write(&fixture.archive, compressed).unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.manifest).unwrap()).unwrap();
    manifest["artifacts"][fixture.name.clone()]["checksums"]["sha256"] =
        json!(artifact::digest(compressed));
    fs::write(&fixture.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

fn file_map(entries: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    entries
        .iter()
        .map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec()))
        .collect()
}

fn multi_product_manifest() -> serde_json::Value {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let ops = ops_fixture();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&cli.manifest).unwrap()).unwrap();
    let ops_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&ops.manifest).unwrap()).unwrap();
    let ops_name = "tmt-ops-aarch64-apple-darwin.tar.gz";
    let mut ops_asset = ops_manifest["artifacts"][&ops.name].clone();
    ops_asset["name"] = json!(ops_name);
    manifest["artifacts"][ops_name] = ops_asset;
    manifest["releases"].as_array_mut().unwrap().push(json!({
        "app_name": "tmt-ops", "app_version": "0.1.0", "artifacts": [ops_name]
    }));
    manifest
}

#[test]
fn manifest_selection_keeps_products_and_versions_independent_on_the_same_target() {
    let manifest = serde_json::to_vec(&multi_product_manifest()).unwrap();
    assert_eq!(
        artifact::select(super::Product::Cli, &manifest, TARGET).unwrap(),
        (
            "tmux-team-1.2.3-aarch64-apple-darwin.tar.gz".into(),
            semver::Version::new(1, 2, 3)
        )
    );
    assert_eq!(
        artifact::select(super::Product::Ops, &manifest, TARGET).unwrap(),
        (
            "tmt-ops-aarch64-apple-darwin.tar.gz".into(),
            semver::Version::new(0, 1, 0)
        )
    );
}

#[test]
fn manifest_selection_rejects_cross_product_claims_and_duplicate_candidates() {
    let mut shared = multi_product_manifest();
    shared["releases"][0]["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(json!("tmt-ops-aarch64-apple-darwin.tar.gz"));
    let error = artifact::select(
        super::Product::Ops,
        &serde_json::to_vec(&shared).unwrap(),
        TARGET,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native archive must belong to exactly one TMT release."
    );

    let mut duplicate = multi_product_manifest();
    let mut asset = duplicate["artifacts"]["tmt-ops-aarch64-apple-darwin.tar.gz"].clone();
    asset["name"] = json!("second-ops.tar.gz");
    duplicate["artifacts"]["second-ops.tar.gz"] = asset;
    duplicate["releases"][1]["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(json!("second-ops.tar.gz"));
    let error = artifact::select(
        super::Product::Ops,
        &serde_json::to_vec(&duplicate).unwrap(),
        TARGET,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native manifest must select exactly one target archive."
    );
}

fn root_entries(directory: &TestDirectory) -> Vec<String> {
    let mut entries = fs::read_dir(&directory.path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn assert_only_inputs_remain(fixture: &Fixture) {
    let mut expected = vec![fixture.name.clone(), "manifest.json".to_owned()];
    expected.sort();
    assert_eq!(root_entries(&fixture.directory), expected);
}

#[test]
fn acquire_returns_exact_metadata_and_payload_without_extracting_files() {
    let fixture = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let artifact = artifact::acquire(&fixture.manifest, &fixture.archive, TARGET).unwrap();
    assert_eq!(artifact.name, fixture.name);
    assert_eq!(artifact.version, semver::Version::new(1, 2, 3));
    assert_eq!(artifact.target, TARGET);
    assert_eq!(artifact.sha256.len(), 64);
    assert_eq!(
        artifact.files,
        file_map(&[
            ("tmt", b"native executable\n"),
            ("LICENSE", b"MIT\n"),
            ("NATIVE-INSTALL.md", b"Native install\n"),
            ("THIRD-PARTY-NOTICES.txt", b"Third-party notices\n"),
        ])
    );
    assert_only_inputs_remain(&fixture);
}

#[test]
fn corrupt_checksum_and_wrong_target_are_rejected_before_archive_processing() {
    let checksum_fixture = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&checksum_fixture.manifest).unwrap()).unwrap();
    let name = checksum_fixture.name.clone();
    manifest["artifacts"][name]["checksums"]["sha256"] = json!("0".repeat(64));
    fs::write(
        &checksum_fixture.manifest,
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let error = artifact::acquire(
        &checksum_fixture.manifest,
        &checksum_fixture.archive,
        TARGET,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Native archive checksum mismatch.");
    assert_only_inputs_remain(&checksum_fixture);

    let target_fixture = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let error = artifact::acquire(
        &target_fixture.manifest,
        &target_fixture.archive,
        "x86_64-unknown-linux-musl",
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native archive metadata or target does not match."
    );
    assert_only_inputs_remain(&target_fixture);
}

#[test]
fn duplicate_and_missing_required_entries_are_rejected() {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut duplicate = valid_entries(root);
    duplicate.push(Entry::File {
        path: format!("{root}/tmt"),
        bytes: b"duplicate\n".to_vec(),
        mode: 0o755,
    });
    let duplicate_fixture = fixture(duplicate);
    let error = artifact::acquire(
        &duplicate_fixture.manifest,
        &duplicate_fixture.archive,
        TARGET,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Duplicate native archive file.");
    assert_only_inputs_remain(&duplicate_fixture);

    let mut missing = valid_entries(root);
    missing
        .retain(|entry| !matches!(entry, Entry::File { path, .. } if path.ends_with("/LICENSE")));
    let missing_fixture = fixture(missing);
    let error =
        artifact::acquire(&missing_fixture.manifest, &missing_fixture.archive, TARGET).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native archive is missing required files."
    );
    assert_only_inputs_remain(&missing_fixture);
}

#[test]
fn symlink_path_traversal_and_special_permissions_are_rejected() {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut symlink = valid_entries(root);
    symlink[0] = Entry::Symlink {
        path: format!("{root}/tmt"),
        target: "outside".to_owned(),
    };
    let symlink_fixture = fixture(symlink);
    let error =
        artifact::acquire(&symlink_fixture.manifest, &symlink_fixture.archive, TARGET).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native archive requires nonempty regular files with safe permissions."
    );
    assert_only_inputs_remain(&symlink_fixture);

    let traversal = vec![Entry::AdversarialFile {
        path: format!("{root}/../escape"),
        bytes: b"escape\n".to_vec(),
        mode: 0o644,
    }];
    let traversal_fixture = fixture(traversal);
    let error = artifact::acquire(
        &traversal_fixture.manifest,
        &traversal_fixture.archive,
        TARGET,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Unexpected native archive path.");
    assert_only_inputs_remain(&traversal_fixture);

    let mut special = valid_entries(root);
    special[0] = Entry::File {
        path: format!("{root}/tmt"),
        bytes: b"native executable\n".to_vec(),
        mode: 0o4755,
    };
    let special_fixture = fixture(special);
    let error =
        artifact::acquire(&special_fixture.manifest, &special_fixture.archive, TARGET).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native archive requires nonempty regular files with safe permissions."
    );
    assert_only_inputs_remain(&special_fixture);
}

#[test]
fn rejects_empty_and_non_executable_required_files() {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut empty = valid_entries(root);
    empty[0] = Entry::File {
        path: format!("{root}/tmt"),
        bytes: Vec::new(),
        mode: 0o755,
    };
    let empty_fixture = fixture(empty);
    let error =
        artifact::acquire(&empty_fixture.manifest, &empty_fixture.archive, TARGET).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native archive requires nonempty regular files with safe permissions."
    );
    assert_only_inputs_remain(&empty_fixture);

    let mut non_executable = valid_entries(root);
    non_executable[0] = Entry::File {
        path: format!("{root}/tmt"),
        bytes: b"native executable\n".to_vec(),
        mode: 0o644,
    };
    let non_executable_fixture = fixture(non_executable);
    let error = artifact::acquire(
        &non_executable_fixture.manifest,
        &non_executable_fixture.archive,
        TARGET,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native archive requires nonempty regular files with safe permissions."
    );
    assert_only_inputs_remain(&non_executable_fixture);
}

#[test]
fn rejects_truncated_archive_after_checksum_verification() {
    let fixture = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let mut truncated = fs::read(&fixture.archive).unwrap();
    truncated.truncate(truncated.len() - 8);
    replace_archive(&fixture, &truncated);
    let error = artifact::acquire(&fixture.manifest, &fixture.archive, TARGET).unwrap_err();
    assert_eq!(error.to_string(), "unexpected end of file");
    assert_only_inputs_remain(&fixture);
}

#[test]
fn rejects_compressed_archive_above_the_bound_before_checksum() {
    let fixture = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    OpenOptions::new()
        .write(true)
        .open(&fixture.archive)
        .unwrap()
        .set_len((64 * 1024 * 1024 + 1) as u64)
        .unwrap();
    let error = artifact::acquire(&fixture.manifest, &fixture.archive, TARGET).unwrap_err();
    assert_eq!(error.to_string(), "File exceeds the input bound.");
    assert_only_inputs_remain(&fixture);
}

#[test]
fn rejects_expanded_archive_above_the_bound_before_tar_parsing() {
    let fixture = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut source = io::repeat(0).take((128 * 1024 * 1024 + 1) as u64);
    io::copy(&mut source, &mut encoder).unwrap();
    let compressed = encoder.finish().unwrap();
    assert!(compressed.len() < 64 * 1024 * 1024);
    replace_archive(&fixture, &compressed);
    let error = artifact::acquire(&fixture.manifest, &fixture.archive, TARGET).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native archive expansion exceeds its bound."
    );
    assert_only_inputs_remain(&fixture);
}

#[test]
fn malformed_manifest_is_rejected_without_touching_archive_or_extracting() {
    let fixture = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let original_archive = fs::read(&fixture.archive).unwrap();
    fs::write(&fixture.manifest, b"{ malformed").unwrap();
    assert!(artifact::acquire(&fixture.manifest, &fixture.archive, TARGET).is_err());
    assert_eq!(fs::read(&fixture.archive).unwrap(), original_archive);
    assert_only_inputs_remain(&fixture);
}

#[test]
fn uninstall_preserves_unmanaged_links_and_tampered_releases() {
    use std::os::unix::fs::symlink;
    let fixture = ops_fixture();
    let prefix = fixture.directory.path.join("prefix");
    let report = install_fixture(&fixture, &prefix, super::Product::Ops).unwrap();
    let original_link = fs::read_link(&report.executable).unwrap();
    let pointer = fs::read_link(prefix.join("lib/tmt-ops/current")).unwrap();
    let user_file = fixture.directory.path.join("user-command");
    fs::write(&user_file, b"user content").unwrap();
    fs::remove_file(&report.executable).unwrap();
    symlink(&user_file, &report.executable).unwrap();
    assert!(super::uninstall_extension(&prefix, super::Product::Ops).is_err());
    assert_eq!(fs::read_link(&report.executable).unwrap(), user_file);
    assert_eq!(fs::read(&user_file).unwrap(), b"user content");
    assert_eq!(
        fs::read_link(prefix.join("lib/tmt-ops/current")).unwrap(),
        pointer
    );
    fs::remove_file(&report.executable).unwrap();
    symlink(&original_link, &report.executable).unwrap();
    fs::write(&report.active_executable, b"user edited executable").unwrap();
    let error = super::uninstall_extension(&prefix, super::Product::Ops).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Installed release file has changed; refusing replacement."
    );
    assert_eq!(fs::read_link(&report.executable).unwrap(), original_link);
    assert_eq!(
        fs::read(&report.active_executable).unwrap(),
        b"user edited executable"
    );
}

fn ops_request<'a>(fixture: &'a Fixture, prefix: &'a std::path::Path) -> super::InstallRequest<'a> {
    super::InstallRequest {
        archive: &fixture.archive,
        manifest: &fixture.manifest,
        prefix,
        target: TARGET,
        channel: tmt_core::native_install::Channel::Stable,
        pin: tmt_core::native_install::PinAction::Preserve,
    }
}

#[test]
fn verifier_sees_the_written_candidate_and_its_rejection_keeps_the_previous_release() {
    let ops = ops_fixture();
    let prefix = ops.directory.path.join("prefix");
    let previous = install_fixture(&ops, &prefix, super::Product::Ops).unwrap();
    let pointer = fs::read_link(prefix.join("lib/tmt-ops/current")).unwrap();

    let update = ops_fixture_with_payload(b"candidate payload\n");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&update.manifest).unwrap()).unwrap();
    manifest["releases"][0]["app_version"] = json!("1.2.4");
    fs::write(&update.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let seen = std::cell::RefCell::new(None);
    let verifier = |executable: &std::path::Path, version: &semver::Version| {
        let release = executable.parent().unwrap();
        *seen.borrow_mut() = Some((
            fs::read(executable).unwrap(),
            version.to_string(),
            release.join("receipt.json").exists(),
            fs::canonicalize(release.parent().unwrap()).unwrap()
                == fs::canonicalize(prefix.join("lib/tmt-ops/releases")).unwrap(),
        ));
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "candidate rejected",
        ))
    };
    let error = super::install_product(
        super::Product::Ops,
        ops_request(&update, &prefix),
        Some(&verifier),
        || Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "candidate rejected");
    assert_eq!(
        seen.into_inner(),
        Some((b"candidate payload\n".to_vec(), "1.2.4".into(), false, true))
    );
    assert_eq!(
        fs::read_link(prefix.join("lib/tmt-ops/current")).unwrap(),
        pointer
    );
    assert_eq!(fs::read(&previous.active_executable).unwrap(), OPS_PAYLOAD);
    assert_eq!(
        fs::read_dir(prefix.join("lib/tmt-ops/releases"))
            .unwrap()
            .count(),
        1
    );
}

/// Rewrite an installed receipt's source to a GitHub release of `repository`.
fn record_release_source(executable: &std::path::Path, repository: &str) {
    let receipt = fs::canonicalize(executable)
        .unwrap()
        .parent()
        .unwrap()
        .join("receipt.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    value["source"] = serde_json::json!({
        "kind": "github-release", "repository": repository,
        "release_id": 42, "manifest_sha256": "a".repeat(64),
    });
    fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn every_product_reads_former_organization_receipts_and_refuses_the_retired_name() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let ops = ops_fixture();
    let prefix = cli.directory.path.join("prefix");
    let installed = [
        (
            super::Product::Cli,
            install_fixture(&cli, &prefix, super::Product::Cli).unwrap(),
        ),
        (
            super::Product::Ops,
            install_fixture(&ops, &prefix, super::Product::Ops).unwrap(),
        ),
    ];
    for (product, report) in &installed {
        // The organization transfer remains independent of the retired product name.
        record_release_source(&report.executable, "wkh237/tmt");
        super::inspect_product(*product, &report.executable).unwrap();
        for foreign in [
            "wkh237/tmux-team",
            "attacker/tmt",
            "wkh237/tmux-team-fork",
            "WKH237/TMUX-TEAM",
            "",
        ] {
            record_release_source(&report.executable, foreign);
            let error = super::inspect_product(*product, &report.executable).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Invalid native release provenance"),
                "{product:?} accepted {foreign:?}"
            );
        }
    }
}

#[test]
fn installing_over_a_former_organization_receipt_succeeds() {
    // The one-line installer runs `__native-install` over the existing prefix.
    let old = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let prefix = old.directory.path.join("prefix");
    let installed = install_fixture(&old, &prefix, super::Product::Cli).unwrap();
    record_release_source(&installed.executable, "wkh237/tmt");
    let next = product_fixture_at(
        valid_entries("tmux-team-1.2.4-aarch64-apple-darwin"),
        "tmt-cli",
        &FILES,
        "1.2.4",
    );
    let report = install_fixture(&next, &prefix, super::Product::Cli).unwrap();
    assert!(report.changed);
    assert_eq!(report.version, "1.2.4");
    super::inspect(&report.executable).unwrap();
    // A foreign repository in the current receipt still stops the installer.
    record_release_source(&report.executable, "attacker/other");
    let again = product_fixture_at(
        valid_entries("tmux-team-1.2.5-aarch64-apple-darwin"),
        "tmt-cli",
        &FILES,
        "1.2.5",
    );
    assert!(install_fixture(&again, &prefix, super::Product::Cli).is_err());
}

#[test]
fn transport_does_not_apply_the_running_installers_companion_policy() {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut entries = valid_entries(root);
    entries.push(Entry::File {
        path: format!("{root}/tmt-driver-herdr"),
        bytes: b"candidate-owned non-executable asset".to_vec(),
        mode: 0o644,
    });
    let files = FILES
        .into_iter()
        .chain(["tmt-driver-herdr"])
        .collect::<Vec<_>>();
    let fixture = product_fixture(entries, "tmt-cli", &files);
    let manifest = fs::read(&fixture.manifest).unwrap();
    let archive = fs::read(&fixture.archive).unwrap();
    let candidate =
        artifact::acquire_candidate(&manifest, &fixture.name, &archive, TARGET).unwrap();
    assert!(candidate.files.contains_key("tmt-driver-herdr"));
    assert!(!candidate.executable_files.contains("tmt-driver-herdr"));
    // The current installer still requires its declared companion to execute.
    assert!(
        artifact::acquire(&fixture.manifest, &fixture.archive, TARGET)
            .unwrap_err()
            .to_string()
            .contains("safe permissions")
    );
}
