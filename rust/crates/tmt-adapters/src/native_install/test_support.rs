use super::{
    artifact::{self, Artifact, FILES},
    publication::Layout,
    receipt::Receipt,
};
use crate::test_support::TestDirectory;
use semver::Version;
use std::collections::BTreeMap;
use tmt_core::native_install::{Channel, InstalledVersion};

const TARGET: &str = "aarch64-apple-darwin";
pub(super) const VERSION: &str = "1.2.3";

pub(super) fn artifact(version: &str, payload: &[u8]) -> Artifact {
    let mut files = BTreeMap::new();
    files.insert("tmt".to_owned(), payload.to_vec());
    files.insert("LICENSE".to_owned(), b"MIT\n".to_vec());
    files.insert("NATIVE-INSTALL.md".to_owned(), b"Native install\n".to_vec());
    files.insert(
        "THIRD-PARTY-NOTICES.txt".to_owned(),
        b"Third-party notices\n".to_vec(),
    );
    assert_eq!(files.len(), FILES.len());
    Artifact {
        name: format!("tmux-team-{version}-{TARGET}.tar.gz"),
        version: Version::parse(version).unwrap(),
        target: TARGET.to_owned(),
        sha256: artifact::digest(b"synthetic archive"),
        files,
        executable_files: ["tmt".to_owned()].into(),
    }
}

pub(super) fn state(version: &str, pinned: Option<&str>) -> InstalledVersion {
    InstalledVersion {
        version: Version::parse(version).unwrap(),
        channel: Channel::Stable,
        pinned_version: pinned.map(|value| Version::parse(value).unwrap()),
    }
}

pub(super) fn published_layout() -> (TestDirectory, Layout, Receipt) {
    let directory = TestDirectory::new();
    let layout = Layout::open(&directory.path.join("prefix")).unwrap();
    let artifact = artifact(VERSION, b"synthetic tmt payload\n");
    let receipt = Receipt::new(&artifact, state(VERSION, None));
    publish(&layout, &artifact, &receipt);
    (directory, layout, receipt)
}

pub(super) fn publish(layout: &Layout, artifact: &Artifact, receipt: &Receipt) {
    let mut checkpoint = || Ok(());
    let expected_current = layout.current().unwrap().map(|current| current.id);
    layout
        .publish(artifact, receipt, expected_current, None, &mut checkpoint)
        .unwrap();
}

pub(super) fn checkpoint_count() -> usize {
    let directory = TestDirectory::new();
    let layout = Layout::open(&directory.path.join("probe-prefix")).unwrap();
    let artifact = artifact(VERSION, b"checkpoint probe\n");
    let receipt = Receipt::new(&artifact, state(VERSION, None));
    let mut calls = 0;
    let mut checkpoint = || {
        calls += 1;
        Ok(())
    };
    layout
        .publish(&artifact, &receipt, None, None, &mut checkpoint)
        .unwrap();
    calls
}

/// Acquisition/activation unit tests inject publication without starting a
/// synthetic non-executable payload. Handoff tests own candidate execution.
pub(super) fn install_downloaded(
    current: &super::ManagedInstallation,
    downloaded: &super::release::DownloadedRelease,
    channel: Channel,
    pin: tmt_core::native_install::PinAction,
    checkpoint: &mut dyn FnMut() -> std::io::Result<()>,
) -> std::io::Result<super::InstallReport> {
    let artifact = artifact::acquire_bytes(
        super::Product::Cli,
        &downloaded.manifest,
        &downloaded.archive_name,
        &downloaded.archive,
        &current.target,
    )?;
    super::activate(
        super::ActivationRequest {
            product: super::Product::Cli,
            prefix: &current.prefix,
            channel,
            pin,
            expected: Some(current.id),
            provenance: Some(downloaded.provenance.clone()),
            verifier: None,
        },
        &artifact,
        checkpoint,
    )
}
