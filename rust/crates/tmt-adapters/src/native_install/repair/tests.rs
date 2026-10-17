use super::super::{receipt::GitHubProvenance, test_support::state};
use super::*;
use crate::test_support::TestDirectory;
use std::{
    collections::BTreeMap,
    os::unix::fs::{PermissionsExt, symlink},
};
use tmt_core::native_install::Channel;

const TARGET: &str = "aarch64-apple-darwin";
struct Fixture {
    _directory: TestDirectory,
    layout: Layout,
    old: PathBuf,
    metadata: Vec<u8>,
    manifest: Vec<u8>,
    archive: Vec<u8>,
    version: String,
    network: release::PublishedNetwork,
}
impl Fixture {
    fn new(pinned: bool) -> Self {
        Self::in_channel(pinned, Channel::Stable)
    }
    fn in_channel(pinned: bool, channel: Channel) -> Self {
        let version = if channel == Channel::Stable {
            "1.2.3"
        } else {
            "1.2.3-alpha.1"
        };
        let directory = TestDirectory::new();
        let layout = Layout::open_product(&directory.path.join("prefix"), Product::Ops).unwrap();
        let (release, manifest, archive, name) =
            release::product_fixture(Product::Ops, version, TARGET, 42);
        let artifact =
            artifact::acquire_bytes(Product::Ops, &manifest, &name, &archive, TARGET).unwrap();
        let mut installed = state(version, pinned.then_some(version));
        installed.channel = channel;
        let mut receipt = Receipt::new(&artifact, installed);
        receipt.provenance = Some(
            GitHubProvenance {
                release_id: 42,
                manifest_sha256: artifact::digest(&manifest),
            }
            .into(),
        );
        let _lock = crate::file_lock::exclusive(&layout.root.join("install.lock")).unwrap();
        layout
            .publish(&artifact, &receipt, None, None, &mut || Ok(()))
            .unwrap();
        drop(_lock);
        let old = layout.root.join("releases").join(receipt.id.to_string());
        Self {
            _directory: directory,
            layout,
            old,
            metadata: release::indexed_routes(&release, &manifest, &archive)
                .into_iter()
                .find(|(url, _)| url.ends_with("tmt-release-record.json"))
                .unwrap()
                .1,
            manifest,
            archive,
            version: version.to_owned(),
            network: release::PublishedNetwork::default(),
        }
    }
    fn download(&self, url: &str) -> io::Result<Vec<u8>> {
        self.network.record(url);
        if url.ends_with("/dist-manifest.json") {
            Ok(self.manifest.clone())
        } else if url.ends_with("/tmt-ops-aarch64-apple-darwin.tar.gz") {
            Ok(self.archive.clone())
        } else {
            assert!(
                url.ends_with(&format!(
                    "/tmt-ops-v{}/tmt-release-record.json",
                    self.version
                )),
                "repair selects exact version: {url}"
            );
            Ok(self.metadata.clone())
        }
    }
    fn repair(&self) -> io::Result<RepairReport> {
        repair_with(
            Product::Ops,
            &self.layout.prefix,
            None,
            || Ok(()),
            |url, _, _, _| self.download(url).map(Into::into),
        )
    }
}

// Byte/mode/path preservation of the old tree, including entries TMT never owned.
fn tree(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
    let mut result = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).unwrap();
        let bytes = if metadata.is_dir() {
            pending.extend(
                fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
            Vec::new()
        } else if metadata.is_symlink() {
            fs::read_link(&path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else {
            fs::read(&path).unwrap()
        };
        result.insert(
            path.strip_prefix(root).unwrap().into(),
            (metadata.permissions().mode(), bytes),
        );
    }
    result
}

#[test]
fn tampered_extra_missing_and_foreign_files_are_restored_without_changing_the_old_tree() {
    for defect in ["tampered", "extra", "missing", "foreign"] {
        let fixture = Fixture::new(true);
        let original = fs::read(fixture.old.join("tmt-ops")).unwrap();
        match defect {
            "tampered" => fs::write(fixture.old.join("tmt-ops"), b"tampered").unwrap(),
            "extra" => fs::write(fixture.old.join("extra.txt"), b"extra").unwrap(),
            "missing" => fs::remove_file(fixture.old.join("tmt-ops")).unwrap(),
            "foreign" => {
                fs::create_dir(fixture.old.join("user-files")).unwrap();
                fs::write(fixture.old.join("user-files/keep.txt"), b"foreign bytes").unwrap();
            }
            _ => unreachable!(),
        }
        let before = tree(&fixture.old);
        let error = fixture.layout.current().unwrap_err();
        assert!(
            error.get_ref().unwrap().is::<RepairRequired>(),
            "{defect}: {error}"
        );
        let result = fixture.repair().unwrap();
        assert!(result.installation.changed);
        assert_eq!(result.retained_release.as_ref(), Some(&fixture.old));
        assert_ne!(
            result.installation.active_executable.parent().unwrap(),
            fixture.old
        );
        assert_eq!(
            fs::read(&result.installation.active_executable).unwrap(),
            original
        );
        assert_eq!(
            tree(&fixture.old),
            before,
            "old release is the untouched backup: {defect}"
        );
        let current = fixture.layout.current().unwrap().unwrap();
        assert_eq!(current.state, state("1.2.3", Some("1.2.3")));
        assert!(!fixture.repair().unwrap().installation.changed);
    }
}

#[test]
fn healthy_repair_is_a_noop_without_network() {
    let fixture = Fixture::new(false);
    let before = tree(&fixture.layout.prefix);
    let result = repair_with(
        Product::Ops,
        &fixture.layout.prefix,
        None,
        || Ok(()),
        |_, _, _, _| panic!("healthy repair must not download"),
    )
    .unwrap();
    assert!(!result.installation.changed);
    assert!(result.retained_release.is_none());
    assert_eq!(tree(&fixture.layout.prefix), before);
}

#[test]
fn a_damaged_repair_requiring_a_newer_cli_preserves_the_entire_prefix() {
    let directory = TestDirectory::new();
    let layout = Layout::open_product(&directory.path.join("prefix"), Product::Ops).unwrap();
    let declaration = br#"{"version":1,"requiresCli":">=999.0.0","uses":[]}"#;
    let (_, manifest, archive, name) = release::companion_fixture(
        Product::Ops,
        "1.2.3",
        TARGET,
        42,
        &[("TMT-USES.json", declaration)],
    );
    let artifact =
        artifact::acquire_bytes(Product::Ops, &manifest, &name, &archive, TARGET).unwrap();
    let receipt = Receipt::new(&artifact, state("1.2.3", Some("1.2.3")));
    // Model an installation published by a newer CLI, not an admitted install
    // through this older build's candidate gate.
    let lock = crate::file_lock::exclusive(&layout.root.join("install.lock")).unwrap();
    layout
        .publish(&artifact, &receipt, None, None, &mut || Ok(()))
        .unwrap();
    drop(lock);
    let old = layout.root.join("releases").join(receipt.id.to_string());
    let healthy = tree(&layout.prefix);
    let unchanged = repair_using(
        Product::Ops,
        &layout.prefix,
        None,
        || Ok(()),
        |_, _| panic!("healthy repair stays a no-op"),
    )
    .unwrap();
    assert!(!unchanged.installation.changed);
    assert_eq!(tree(&layout.prefix), healthy);
    fs::write(old.join("tmt-ops"), b"damaged").unwrap();
    let before = tree(&layout.prefix);
    let error = repair_using(
        Product::Ops,
        &layout.prefix,
        None,
        || Ok(()),
        |_, _| {
            artifact::acquire_bytes(Product::Ops, &manifest, &name, &archive, TARGET)
                .map(|artifact| (artifact, None))
        },
    )
    .unwrap_err();
    let required = crate::native_install::cli_required(&error).expect("typed CLI refusal");
    assert_eq!(required.compatibility.minimum.to_string(), "999.0.0");
    assert_eq!(tree(&layout.prefix), before);
}

#[test]
fn a_symlink_in_the_old_release_is_refused_without_following_or_repair_hint() {
    let fixture = Fixture::new(false);
    let outside = fixture.layout.prefix.join("outside");
    fs::write(&outside, b"outside sentinel").unwrap();
    symlink(&outside, fixture.old.join("foreign-link")).unwrap();
    let before = tree(&fixture.layout.prefix);
    let error = fixture.layout.current().unwrap_err();
    assert!(
        !error
            .get_ref()
            .is_some_and(|cause| cause.is::<RepairRequired>())
    );
    assert!(fixture.repair().is_err());
    assert_eq!(tree(&fixture.layout.prefix), before);
    assert_eq!(fs::read(outside).unwrap(), b"outside sentinel");
}

#[test]
fn receipt_or_current_swap_during_acquisition_is_refused_before_publication() {
    for swap in ["receipt", "current"] {
        let fixture = Fixture::new(false);
        fs::write(fixture.old.join("tmt-ops"), b"damaged").unwrap();
        let before = tree(&fixture.old);
        let result = repair_with(
            Product::Ops,
            &fixture.layout.prefix,
            None,
            || Ok(()),
            |url, _, _, _| {
                let bytes = fixture.download(url)?;
                if url.ends_with("/tmt-ops-aarch64-apple-darwin.tar.gz") {
                    if swap == "receipt" {
                        let path = fixture.old.join("receipt.json");
                        let mut bytes = fs::read(&path).unwrap();
                        bytes.push(b'\n');
                        fs::write(path, bytes).unwrap();
                    } else {
                        fs::remove_file(fixture.layout.root.join("current")).unwrap();
                        symlink("releases/not-owned", fixture.layout.root.join("current")).unwrap();
                    }
                }
                Ok(bytes.into())
            },
        );
        assert!(result.is_err(), "{swap}");
        assert_eq!(
            fs::read_dir(fixture.layout.root.join("releases"))
                .unwrap()
                .count(),
            1
        );
        if swap == "current" {
            assert_eq!(tree(&fixture.old), before);
        }
        assert_eq!(fs::read(fixture.old.join("tmt-ops")).unwrap(), b"damaged");
    }
}

#[test]
fn unavailable_or_different_original_artifact_leaves_everything_unchanged() {
    for different in [false, true] {
        let fixture = Fixture::new(false);
        fs::write(fixture.old.join("tmt-ops"), b"damaged").unwrap();
        let before = tree(&fixture.layout.prefix);
        let error = repair_with(
            Product::Ops,
            &fixture.layout.prefix,
            None,
            || Ok(()),
            |url, _, _, _| {
                if !different {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "original release unavailable",
                    ));
                }
                let bytes = fixture.download(url)?;
                if url.ends_with("/tmt-ops-v1.2.3/tmt-release-record.json") {
                    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    value["releaseId"] = 43.into();
                    return Ok(serde_json::to_vec(&value).unwrap().into());
                }
                Ok(bytes.into())
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(if different {
                "does not match"
            } else {
                "cannot be installed directly"
            }),
            "{error}"
        );
        assert_eq!(tree(&fixture.layout.prefix), before);
    }
}

#[test]
fn local_archive_repair_requires_matching_inputs_and_preserves_old_content_and_pin() {
    let fixture = Fixture::new(true);
    let receipt_path = fixture.old.join("receipt.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
    value["source"] = "local-archive".into();
    fs::write(&receipt_path, serde_json::to_vec(&value).unwrap()).unwrap();
    let archive = fixture
        .layout
        .prefix
        .join(value["archive"].as_str().unwrap());
    let manifest = fixture.layout.prefix.join("manifest.json");
    fs::write(&archive, &fixture.archive).unwrap();
    fs::write(&manifest, &fixture.manifest).unwrap();
    let original = fs::read(fixture.old.join("tmt-ops")).unwrap();
    fs::write(fixture.old.join("tmt-ops"), b"tampered").unwrap();
    fs::write(fixture.old.join("foreign.txt"), b"keep foreign bytes").unwrap();
    let before = tree(&fixture.layout.prefix);
    let required = fixture.layout.current().unwrap_err();
    assert!(
        required
            .get_ref()
            .unwrap()
            .downcast_ref::<RepairRequired>()
            .unwrap()
            .requires_archive
    );
    let required = fixture.repair().unwrap_err();
    assert!(required.get_ref().unwrap().is::<RepairRequired>());
    assert_eq!(tree(&fixture.layout.prefix), before);
    let missing = repair_product_from_archive(
        Product::Ops,
        &fixture.layout.prefix,
        &archive.with_extension("missing"),
        &manifest,
        None,
        || Ok(()),
    );
    assert!(missing.is_err());
    assert_eq!(tree(&fixture.layout.prefix), before);
    // Different valid gzip bytes for the same version and file inventory are still not the recorded artifact.
    use std::io::{Read, Write};
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(fixture.archive.as_slice())
        .read_to_end(&mut tar)
        .unwrap();
    let mut encoder = flate2::GzBuilder::new()
        .mtime(1)
        .write(Vec::new(), flate2::Compression::default());
    encoder.write_all(&tar).unwrap();
    let other_archive = encoder.finish().unwrap();
    assert_ne!(
        artifact::digest(&other_archive),
        artifact::digest(&fixture.archive)
    );
    let mut other_manifest: serde_json::Value = serde_json::from_slice(&fixture.manifest).unwrap();
    other_manifest["artifacts"][value["archive"].as_str().unwrap()]["checksums"]["sha256"] =
        artifact::digest(&other_archive).into();
    fs::write(&archive, other_archive).unwrap();
    fs::write(&manifest, serde_json::to_vec(&other_manifest).unwrap()).unwrap();
    let mismatched = tree(&fixture.layout.prefix);
    let error = repair_product_from_archive(
        Product::Ops,
        &fixture.layout.prefix,
        &archive,
        &manifest,
        None,
        || Ok(()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not match"), "{error}");
    assert_eq!(tree(&fixture.layout.prefix), mismatched);
    fs::write(&archive, &fixture.archive).unwrap();
    fs::write(&manifest, &fixture.manifest).unwrap();
    let old = tree(&fixture.old);
    let repaired = repair_product_from_archive(
        Product::Ops,
        &fixture.layout.prefix,
        &archive,
        &manifest,
        None,
        || Ok(()),
    )
    .unwrap();
    assert!(repaired.installation.changed);
    assert_eq!(repaired.retained_release.as_ref(), Some(&fixture.old));
    assert_eq!(
        fs::read(repaired.installation.active_executable).unwrap(),
        original
    );
    assert_eq!(tree(&fixture.old), old);
    let current = fixture.layout.current().unwrap().unwrap();
    assert_eq!(current.state, state("1.2.3", Some("1.2.3")));
    assert!(current.provenance.is_none());
    assert!(
        !repair_product_from_archive(
            Product::Ops,
            &fixture.layout.prefix,
            &archive,
            &manifest,
            None,
            || Ok(())
        )
        .unwrap()
        .installation
        .changed
    );
}

#[test]
fn concurrent_repair_installs_preserve_the_winner_and_clean_losing_staging() {
    use std::sync::mpsc;
    let fixture = Fixture::new(true);
    fs::write(fixture.old.join("tmt-ops"), b"tampered").unwrap();
    fs::write(fixture.old.join("foreign.txt"), b"keep me").unwrap();
    let old = tree(&fixture.old);
    let lock_inode = fs::metadata(fixture.layout.root.join("install.lock"))
        .unwrap()
        .ino();
    let results = std::thread::scope(|scope| {
        let (ready_a, observe_a) = mpsc::channel();
        let (start_a, acquire_a) = mpsc::channel();
        let fixture_ref = &fixture;
        let a = scope.spawn(move || {
            repair_with(
                Product::Ops,
                &fixture_ref.layout.prefix,
                None,
                || Ok(()),
                |url, _, _, _| {
                    let bytes = fixture_ref.download(url)?;
                    if url.ends_with("/tmt-ops-aarch64-apple-darwin.tar.gz") {
                        ready_a.send(()).unwrap();
                        acquire_a
                            .recv_timeout(Duration::from_secs(5))
                            .map_err(io::Error::other)?;
                    }
                    Ok(bytes.into())
                },
            )
        });
        // Start B only after A has released its observation lock for acquisition.
        observe_a.recv_timeout(Duration::from_secs(5)).unwrap();
        let (ready_b, observe_b) = mpsc::channel();
        let (start_b, acquire_b) = mpsc::channel();
        let b = scope.spawn(move || {
            repair_with(
                Product::Ops,
                &fixture_ref.layout.prefix,
                None,
                || Ok(()),
                |url, _, _, _| {
                    let bytes = fixture_ref.download(url)?;
                    if url.ends_with("/tmt-ops-aarch64-apple-darwin.tar.gz") {
                        ready_b.send(()).unwrap();
                        acquire_b
                            .recv_timeout(Duration::from_secs(5))
                            .map_err(io::Error::other)?;
                    }
                    Ok(bytes.into())
                },
            )
        });
        observe_b.recv_timeout(Duration::from_secs(5)).unwrap();
        start_a.send(()).unwrap();
        start_b.send(()).unwrap();
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let error = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap()
        .to_string();
    assert!(
        error.contains("changed") || error.contains("Cannot acquire publication lock"),
        "{error}"
    );
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    let active = fixture.layout.current().unwrap().unwrap();
    assert_eq!(
        winner
            .installation
            .active_executable
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap(),
        active.id.to_string()
    );
    assert_eq!(active.state, state("1.2.3", Some("1.2.3")));
    assert_eq!(tree(&fixture.old), old);
    assert_eq!(
        fs::metadata(fixture.layout.root.join("install.lock"))
            .unwrap()
            .ino(),
        lock_inode
    );
    assert_eq!(
        fs::read_dir(fixture.layout.root.join("releases"))
            .unwrap()
            .count(),
        2
    );
    assert!(!fs::read_dir(&fixture.layout.root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".current-")
    }));
}

#[test]
fn repair_uses_only_published_hosts_and_preserves_state_on_acquisition_failure() {
    for channel in [Channel::Stable, Channel::Alpha] {
        for fail in [false, true] {
            let fixture = Fixture::in_channel(false, channel);
            fs::write(fixture.old.join("tmt-ops"), b"damaged").unwrap();
            let sentinel = fixture.layout.prefix.join("application-state");
            fs::write(&sentinel, b"untouched").unwrap();
            let before = tree(&fixture.layout.prefix);
            let result = repair_with(
                Product::Ops,
                &fixture.layout.prefix,
                None,
                || Ok(()),
                |url, _, _, _| {
                    if fail && url.ends_with(".tar.gz") {
                        fixture.network.record(url);
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            "archive unavailable",
                        ));
                    }
                    fixture.download(url).map(Into::into)
                },
            );
            assert_eq!(
                fixture.network.hosts(),
                ["github.com", "github.com", "github.com"],
                "repair exact installed version never reads a channel pointer"
            );
            if fail {
                assert!(result.is_err());
                assert_eq!(
                    tree(&fixture.layout.prefix),
                    before,
                    "failed repair preserves receipt, damaged payload and state"
                );
            } else {
                assert_eq!(result.unwrap().installation.version, fixture.version);
            }
            assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
        }
    }
}
