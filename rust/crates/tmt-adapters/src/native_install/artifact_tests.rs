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
const OFFICE_PAYLOAD: &[u8] = b"#!/bin/sh\nprintf 'TMT-OFFICE/1\\n1.2.3\\n'\n";

#[path = "companion_archive_tests.rs"]
mod companion_archive_tests;
#[path = "skills_release_tests.rs"]
mod skills_release_tests;
#[path = "squad_product_tests.rs"]
mod squad_product_tests;

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

fn office_fixture() -> Fixture {
    office_fixture_with_payload(OFFICE_PAYLOAD)
}

fn office_fixture_with_payload(payload: &[u8]) -> Fixture {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut entries = valid_entries(root);
    entries[0] = Entry::File {
        path: format!("{root}/tmt-office"),
        bytes: payload.to_vec(),
        mode: 0o755,
    };
    product_fixture(
        entries,
        "tmt-office",
        &[
            "tmt-office",
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
        product
            .requires_release_verifier()
            .then_some(&accept_release as super::ReleaseVerifier<'_>),
        || Ok(()),
    )
}

/// Stands in for the product owner's check; the Office handshake itself is
/// tested with the Office commands that own it.
fn accept_release(_: &std::path::Path, _: &semver::Version) -> io::Result<()> {
    Ok(())
}

#[test]
fn office_installation_and_exact_retry_preserve_cli_ownership_and_bytes() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let office = office_fixture();
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
    let cli_pointer = fs::read_link(prefix.join("lib/tmux-team/current")).unwrap();
    let office_report = install_fixture(&office, &prefix, super::Product::Office).unwrap();
    assert!(office_report.changed);
    assert_eq!(
        office_report.executable,
        fs::canonicalize(&prefix).unwrap().join("bin/tmt-office")
    );
    assert_eq!(fs::read(&office_report.executable).unwrap(), OFFICE_PAYLOAD);
    assert_eq!(
        fs::read_link(&office_report.executable).unwrap(),
        PathBuf::from("../lib/tmt-office/current/tmt-office")
    );
    let retry = install_fixture(&office, &prefix, super::Product::Office).unwrap();
    assert!(!retry.changed);
    assert_eq!(retry.active_executable, office_report.active_executable);
    assert_eq!(
        fs::read_link(prefix.join("lib/tmux-team/current")).unwrap(),
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
    super::inspect_product(super::Product::Office, &office_report.executable).unwrap();
    assert!(super::inspect(&office_report.executable).is_err());
    assert!(super::inspect_product(super::Product::Office, &cli_report.executable).is_err());
}

#[test]
fn cross_product_archives_are_rejected_before_creating_installation_prefix() {
    for (fixture, product) in [
        (office_fixture(), super::Product::Cli),
        (
            fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin")),
            super::Product::Office,
        ),
    ] {
        let prefix = fixture.directory.path.join("prefix");
        assert!(install_fixture(&fixture, &prefix, product).is_err());
        assert!(!prefix.exists());
        assert_only_inputs_remain(&fixture);
    }
}

#[test]
fn interrupted_office_pin_preserves_both_active_releases() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let office = office_fixture();
    let prefix = cli.directory.path.join("prefix");
    let cli_report = install_fixture(&cli, &prefix, super::Product::Cli).unwrap();
    let office_report = install_fixture(&office, &prefix, super::Product::Office).unwrap();
    let office_root = prefix.join("lib/tmt-office");
    let current = fs::read_link(office_root.join("current")).unwrap();
    let mut calls = 0;
    let error = super::install_product(
        super::Product::Office,
        super::InstallRequest {
            archive: &office.archive,
            manifest: &office.manifest,
            prefix: &prefix,
            target: TARGET,
            channel: tmt_core::native_install::Channel::Stable,
            pin: tmt_core::native_install::PinAction::PinCandidate,
        },
        Some(&accept_release),
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
    assert_eq!(fs::read_link(office_root.join("current")).unwrap(), current);
    assert_eq!(
        fs::read_dir(office_root.join("releases")).unwrap().count(),
        1
    );
    assert!(
        super::inspect_product(super::Product::Office, &office_report.executable)
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
    assert_eq!(fs::read(&office_report.executable).unwrap(), OFFICE_PAYLOAD);
}

#[test]
fn office_command_collision_does_not_overwrite_user_file_or_cli() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let office = office_fixture();
    let prefix = cli.directory.path.join("prefix");
    let cli_report = install_fixture(&cli, &prefix, super::Product::Cli).unwrap();
    fs::write(prefix.join("bin/tmt-office"), b"user managed command").unwrap();
    assert!(install_fixture(&office, &prefix, super::Product::Office).is_err());
    assert_eq!(
        fs::read(prefix.join("bin/tmt-office")).unwrap(),
        b"user managed command"
    );
    assert!(!prefix.join("lib/tmt-office/current").exists());
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
    let office = office_fixture();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&cli.manifest).unwrap()).unwrap();
    let office_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&office.manifest).unwrap()).unwrap();
    let office_name = "tmt-office-aarch64-apple-darwin.tar.gz";
    let mut office_asset = office_manifest["artifacts"][&office.name].clone();
    office_asset["name"] = json!(office_name);
    manifest["artifacts"][office_name] = office_asset;
    manifest["releases"].as_array_mut().unwrap().push(json!({
        "app_name": "tmt-office", "app_version": "0.1.0", "artifacts": [office_name]
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
        artifact::select(super::Product::Office, &manifest, TARGET).unwrap(),
        (
            "tmt-office-aarch64-apple-darwin.tar.gz".into(),
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
        .push(json!("tmt-office-aarch64-apple-darwin.tar.gz"));
    let error = artifact::select(
        super::Product::Office,
        &serde_json::to_vec(&shared).unwrap(),
        TARGET,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native archive must belong to exactly one TMT release."
    );

    let mut duplicate = multi_product_manifest();
    let mut asset = duplicate["artifacts"]["tmt-office-aarch64-apple-darwin.tar.gz"].clone();
    asset["name"] = json!("second-office.tar.gz");
    duplicate["artifacts"]["second-office.tar.gz"] = asset;
    duplicate["releases"][1]["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(json!("second-office.tar.gz"));
    let error = artifact::select(
        super::Product::Office,
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
    let fixture = office_fixture();
    let prefix = fixture.directory.path.join("prefix");
    let report = install_fixture(&fixture, &prefix, super::Product::Office).unwrap();
    let original_link = fs::read_link(&report.executable).unwrap();
    let pointer = fs::read_link(prefix.join("lib/tmt-office/current")).unwrap();
    let user_file = fixture.directory.path.join("user-command");
    fs::write(&user_file, b"user content").unwrap();
    fs::remove_file(&report.executable).unwrap();
    symlink(&user_file, &report.executable).unwrap();
    assert!(super::uninstall_extension(&prefix, super::Product::Office).is_err());
    assert_eq!(fs::read_link(&report.executable).unwrap(), user_file);
    assert_eq!(fs::read(&user_file).unwrap(), b"user content");
    assert_eq!(
        fs::read_link(prefix.join("lib/tmt-office/current")).unwrap(),
        pointer
    );
    fs::remove_file(&report.executable).unwrap();
    symlink(&original_link, &report.executable).unwrap();
    fs::write(&report.active_executable, b"user edited executable").unwrap();
    let error = super::uninstall_extension(&prefix, super::Product::Office).unwrap_err();
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

fn office_request<'a>(
    fixture: &'a Fixture,
    prefix: &'a std::path::Path,
) -> super::InstallRequest<'a> {
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
fn a_product_that_requires_a_verifier_is_refused_without_one_before_any_write() {
    let office = office_fixture();
    let prefix = office.directory.path.join("prefix");
    let error = super::install_product(
        super::Product::Office,
        office_request(&office, &prefix),
        None,
        || Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(!prefix.exists());
}

#[test]
fn verifier_sees_the_written_candidate_and_its_rejection_keeps_the_previous_release() {
    let office = office_fixture();
    let prefix = office.directory.path.join("prefix");
    let previous = install_fixture(&office, &prefix, super::Product::Office).unwrap();
    let pointer = fs::read_link(prefix.join("lib/tmt-office/current")).unwrap();

    let update = office_fixture_with_payload(b"candidate payload\n");
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
                == fs::canonicalize(prefix.join("lib/tmt-office/releases")).unwrap(),
        ));
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "candidate rejected",
        ))
    };
    let error = super::install_product(
        super::Product::Office,
        office_request(&update, &prefix),
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
        fs::read_link(prefix.join("lib/tmt-office/current")).unwrap(),
        pointer
    );
    assert_eq!(
        fs::read(&previous.active_executable).unwrap(),
        OFFICE_PAYLOAD
    );
    assert_eq!(
        fs::read_dir(prefix.join("lib/tmt-office/releases"))
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
fn every_product_reads_receipts_recorded_under_the_pre_rename_repository() {
    let cli = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let office = office_fixture();
    let prefix = cli.directory.path.join("prefix");
    let installed = [
        (
            super::Product::Cli,
            install_fixture(&cli, &prefix, super::Product::Cli).unwrap(),
        ),
        (
            super::Product::Office,
            install_fixture(&office, &prefix, super::Product::Office).unwrap(),
        ),
    ];
    for (product, report) in &installed {
        // Receipts from v5.0.0-alpha.2..alpha.6 and Office 0.1.0-alpha.1..alpha.3.
        record_release_source(&report.executable, "wkh237/tmux-team");
        super::inspect_product(*product, &report.executable).unwrap();
        for foreign in [
            "attacker/tmux-team",
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
fn installing_over_an_alpha_6_receipt_under_the_pre_rename_repository_succeeds() {
    // The one-line installer runs `__native-install` over the existing prefix.
    let old = fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"));
    let prefix = old.directory.path.join("prefix");
    let installed = install_fixture(&old, &prefix, super::Product::Cli).unwrap();
    record_release_source(&installed.executable, "wkh237/tmux-team");
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
