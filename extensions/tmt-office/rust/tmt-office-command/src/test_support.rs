//! Invocation-owned filesystem fixture for Office command tests.

mod text_file_busy;

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use text_file_busy::retry_on_text_file_busy;

pub(crate) struct TestDirectory {
    pub path: PathBuf,
}

impl TestDirectory {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-office-command-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // Refuse to reuse an existing directory; cleanup owns only this creation.
        fs::create_dir(&path).expect("create unique Office command test directory");
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            if std::thread::panicking() {
                eprintln!(
                    "Could not remove Office command fixture {}: {error}",
                    self.path.display()
                );
            } else {
                panic!(
                    "Could not remove Office command fixture {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

/// An Office release archive and manifest in one invocation-owned directory,
/// installed through the real native publication path.
pub(crate) struct OfficeRelease {
    pub directory: TestDirectory,
    pub manifest: std::path::PathBuf,
    archive: std::path::PathBuf,
}

const TARGET: &str = "aarch64-apple-darwin";
const HANDSHAKE: &[u8] = b"#!/bin/sh\nprintf 'TMT-OFFICE/1\\n1.2.3\\n'\n";

pub(crate) fn office_fixture() -> OfficeRelease {
    office_fixture_with_payload(HANDSHAKE)
}

/// Version 1.2.3 of the Office product whose executable is `payload`.
pub(crate) fn office_fixture_with_payload(payload: &[u8]) -> OfficeRelease {
    use flate2::{Compression, write::GzEncoder};
    use tmt_adapters::native_install::Product;

    let directory = TestDirectory::new();
    let name = "tmux-team-1.2.3-aarch64-apple-darwin.tar.gz";
    let root = name.trim_end_matches(".tar.gz");
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    for file in Product::Office.files() {
        let (bytes, mode) = if file == Product::Office.executable() {
            (payload, 0o755)
        } else {
            (file.as_bytes(), 0o644)
        };
        let mut header = tar::Header::new_gnu();
        header.set_path(format!("{root}/{file}")).unwrap();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(mode);
        header.set_size(bytes.len() as u64);
        header.set_cksum();
        builder.append(&header, bytes).unwrap();
    }
    let compressed = builder.into_inner().unwrap().finish().unwrap();
    let archive = directory.path.join(name);
    let manifest = directory.path.join("manifest.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&serde_json::json!({
            "artifacts": {
                name: {
                    "kind": "executable-zip",
                    "name": name,
                    "target_triples": [TARGET],
                    "checksums": {"sha256": tmt_core::content_digest::sha256(&compressed)},
                    "assets": Product::Office.files().map(|path| serde_json::json!({"path": path})),
                }
            },
            "releases": [{
                "app_name": Product::Office.package(),
                "app_version": "1.2.3",
                "artifacts": [name]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(&archive, compressed).unwrap();
    OfficeRelease {
        directory,
        manifest,
        archive,
    }
}

/// Offline Office installation with the production release verifier.
///
/// The verifier executes the payload the installer has just written, which can
/// be refused with ETXTBSY while another test thread's fork still holds a copy
/// of the write descriptor. A failed install publishes nothing, so the fixture
/// repeats it (see `text_file_busy`); production code never does.
pub(crate) fn install_office(
    release: &OfficeRelease,
    prefix: &std::path::Path,
) -> std::io::Result<tmt_adapters::native_install::InstallReport> {
    retry_on_text_file_busy(|| {
        tmt_adapters::native_install::install_product(
            tmt_adapters::native_install::Product::Office,
            tmt_adapters::native_install::InstallRequest {
                archive: &release.archive,
                manifest: &release.manifest,
                prefix,
                target: TARGET,
                channel: tmt_core::native_install::Channel::Stable,
                pin: tmt_core::native_install::PinAction::Preserve,
            },
            Some(&crate::verify_release),
            || Ok(()),
        )
    })
}
