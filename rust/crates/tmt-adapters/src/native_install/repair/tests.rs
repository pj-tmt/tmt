use super::super::{receipt::GitHubProvenance, test_support::state};
use super::*;
use crate::test_support::TestDirectory;
use std::{
    collections::BTreeMap,
    os::unix::fs::{PermissionsExt, symlink},
};

const TARGET: &str = "aarch64-apple-darwin";
struct Fixture {
    _directory: TestDirectory,
    layout: Layout,
    old: PathBuf,
    metadata: Vec<u8>,
    manifest: Vec<u8>,
    archive: Vec<u8>,
}
impl Fixture {
    fn new(pinned: bool) -> Self {
        let directory = TestDirectory::new();
        let layout = Layout::open_product(&directory.path.join("prefix"), Product::Squad).unwrap();
        let (release, manifest, archive, name) =
            release::product_fixture(Product::Squad, "1.2.3", TARGET, 42);
        let artifact =
            artifact::acquire_bytes(Product::Squad, &manifest, &name, &archive, TARGET).unwrap();
        let mut receipt = Receipt::new(&artifact, state("1.2.3", pinned.then_some("1.2.3")));
        receipt.provenance = Some(GitHubProvenance {
            release_id: 42,
            manifest_sha256: artifact::digest(&manifest),
        });
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
            metadata: serde_json::to_vec(&release).unwrap(),
            manifest,
            archive,
        }
    }
    fn download(&self, url: &str) -> io::Result<Vec<u8>> {
        if url.ends_with("/assets/421") {
            Ok(self.manifest.clone())
        } else if url.ends_with("/assets/422") {
            Ok(self.archive.clone())
        } else {
            assert!(
                url.ends_with("/tags/tmt-squad-v1.2.3"),
                "repair selects exact version: {url}"
            );
            Ok(self.metadata.clone())
        }
    }
    fn repair(&self) -> io::Result<RepairReport> {
        repair_with(
            Product::Squad,
            &self.layout.prefix,
            None,
            || Ok(()),
            |url, _, _, _| self.download(url),
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
        let original = fs::read(fixture.old.join("tmt-squad")).unwrap();
        match defect {
            "tampered" => fs::write(fixture.old.join("tmt-squad"), b"tampered").unwrap(),
            "extra" => fs::write(fixture.old.join("extra.txt"), b"extra").unwrap(),
            "missing" => fs::remove_file(fixture.old.join("tmt-squad")).unwrap(),
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
        Product::Squad,
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
        fs::write(fixture.old.join("tmt-squad"), b"damaged").unwrap();
        let before = tree(&fixture.old);
        let result = repair_with(
            Product::Squad,
            &fixture.layout.prefix,
            None,
            || Ok(()),
            |url, _, _, _| {
                let bytes = fixture.download(url)?;
                if url.ends_with("/assets/422") {
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
                Ok(bytes)
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
        assert_eq!(fs::read(fixture.old.join("tmt-squad")).unwrap(), b"damaged");
    }
}

#[test]
fn unavailable_or_different_original_artifact_leaves_everything_unchanged() {
    for different in [false, true] {
        let fixture = Fixture::new(false);
        fs::write(fixture.old.join("tmt-squad"), b"damaged").unwrap();
        let before = tree(&fixture.layout.prefix);
        let error = repair_with(
            Product::Squad,
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
                if url.ends_with("/tags/tmt-squad-v1.2.3") {
                    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    value["id"] = 43.into();
                    return Ok(serde_json::to_vec(&value).unwrap());
                }
                Ok(bytes)
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(if different {
                "does not match"
            } else {
                "unavailable"
            }),
            "{error}"
        );
        assert_eq!(tree(&fixture.layout.prefix), before);
    }
}
