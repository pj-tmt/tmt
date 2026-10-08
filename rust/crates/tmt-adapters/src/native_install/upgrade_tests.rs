use super::*;
use crate::native_install::inspect;
use crate::native_install::{
    receipt::Receipt,
    test_support::{artifact, publish, published_layout, state},
};
use std::fs;

#[path = "companion_release_tests.rs"]
mod companion_release_tests;

fn release_download()
-> impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response> {
    let (release, manifest, archive, _) =
        release::valid_fixture("1.2.4", "aarch64-apple-darwin", 42);
    move |url, _, limit, deadline| {
        assert!(deadline > Instant::now());
        assert!(url.starts_with("https://api.github.com/repos/pj-tmt/tmt/"));
        let bytes = if url.ends_with("/assets/421") {
            manifest.clone()
        } else if url.ends_with("/assets/422") {
            archive.clone()
        } else if url.ends_with("/tags/v1.2.4") {
            serde_json::to_vec(&release).unwrap()
        } else {
            assert!(url.ends_with("?per_page=100&page=1"));
            serde_json::to_vec(&serde_json::json!([{ "ref": format!("refs/tags/{}", release["tag_name"].as_str().unwrap()) }])).unwrap()
        };
        assert!(bytes.len() <= limit);
        Ok(bytes.into())
    }
}

fn assert_no_downloads(root: &Path) {
    assert!(!fs::read_dir(root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".download-")
    }));
}

#[test]
fn skill_refresh_holds_the_existing_install_lock_and_refuses_stale_releases() {
    let (_directory, layout, old) = published_layout();
    let executable = layout
        .root
        .join("releases")
        .join(old.id.to_string())
        .join("tmt");
    let mut ran = false;
    crate::native_install::with_active_release(&executable, || {
        ran = true;
        assert!(crate::file_lock::exclusive(&layout.root.join("install.lock")).is_err());
    })
    .unwrap();
    assert!(ran);
    drop(crate::file_lock::exclusive(&layout.root.join("install.lock")).unwrap());
    let next = artifact("1.2.4", b"next executable");
    publish(&layout, &next, &Receipt::new(&next, state("1.2.4", None)));
    let error = crate::native_install::with_active_release(&executable, || {
        panic!("stale skill must not be published")
    })
    .unwrap_err();
    assert!(error.to_string().contains("not the active managed release"));
}

#[test]
fn finalization_failure_reports_the_active_release_and_retry_repairs_links() {
    let (_directory, layout, old) = published_layout();
    let executable = layout
        .root
        .join("releases")
        .join(old.id.to_string())
        .join("tmt");
    let mut checkpoints = 0;
    // Three orchestration checkpoints precede the publisher; downloads stay in memory.
    let last_checkpoint = crate::native_install::test_support::checkpoint_count() + 3;
    let error = upgrade_with(
        UpgradeRequest {
            executable: &executable,
            channel: None,
            exact: None,
            unpin: false,
        },
        || {
            checkpoints += 1;
            if checkpoints == last_checkpoint {
                for name in ["tmt", "tmux-team"] {
                    fs::remove_file(layout.prefix.join("bin").join(name))?;
                }
                fs::remove_dir(layout.prefix.join("bin"))?;
            }
            Ok(())
        },
        release_download(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("Release activated"));
    let activated = error.activated.unwrap();
    assert!(activated.installation.changed);
    assert_eq!(activated.installation.version, "1.2.4");
    assert_eq!(
        fs::read(&activated.installation.active_executable).unwrap(),
        b"native executable\n"
    );
    assert_eq!(fs::read(&executable).unwrap(), b"synthetic tmt payload\n");
    assert_no_downloads(&layout.root);
    fs::create_dir(layout.prefix.join("bin")).unwrap();
    let retry = upgrade_with(
        UpgradeRequest {
            executable: &activated.installation.active_executable,
            channel: None,
            exact: None,
            unpin: false,
        },
        || Ok(()),
        release_download(),
    )
    .unwrap();
    assert!(!retry.installation.changed);
    assert_eq!(
        fs::read(&retry.installation.executable).unwrap(),
        b"native executable\n"
    );
    assert_no_downloads(&layout.root);
}

#[test]
fn forged_remote_provenance_is_not_accepted_as_owned_receipt_metadata() {
    let (_directory, layout, old) = published_layout();
    let release = layout.root.join("releases").join(old.id.to_string());
    let receipt_path = release.join("receipt.json");
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
    for source in [
        serde_json::json!({"kind":"github-release", "repository":"attacker/other", "release_id":42, "manifest_sha256":"a".repeat(64)}),
        serde_json::json!({"kind":"github-release", "repository":"wkh237/tmux-team-fork", "release_id":42, "manifest_sha256":"a".repeat(64)}),
        serde_json::json!({"kind":"github-release", "repository":"wkh237/tmt", "release_id":0, "manifest_sha256":"a".repeat(64)}),
        serde_json::json!({"kind":"github-release", "repository":"wkh237/tmt", "release_id":42, "manifest_sha256":"invalid"}),
    ] {
        let mut forged = original.clone();
        forged["source"] = source;
        fs::write(&receipt_path, serde_json::to_vec(&forged).unwrap()).unwrap();
        let error = inspect(&release.join("tmt")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Invalid native release provenance")
        );
        assert_eq!(
            fs::read(release.join("tmt")).unwrap(),
            b"synthetic tmt payload\n"
        );
    }
}

#[test]
fn legacy_receipts_upgrade_from_the_current_repository_without_rewriting_history() {
    for repository in ["wkh237/tmt", "wkh237/tmux-team"] {
        let (_directory, layout, old) = published_layout();
        let release = layout.root.join("releases").join(old.id.to_string());
        let receipt_path = release.join("receipt.json");
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        // Existing installs retain provenance from before the transfer or rename.
        legacy["source"] = serde_json::json!({
            "kind": "github-release", "repository": repository,
            "release_id": 41, "manifest_sha256": "b".repeat(64),
        });
        fs::write(&receipt_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let report = upgrade_with(
            UpgradeRequest {
                executable: &release.join("tmt"),
                channel: None,
                exact: None,
                unpin: false,
            },
            || Ok(()),
            release_download(),
        )
        .unwrap();
        assert!(report.installation.changed);
        assert_eq!(report.state.version.to_string(), "1.2.4");
        let current = layout.current().unwrap().unwrap();
        let written: serde_json::Value = serde_json::from_slice(
            &fs::read(
                layout
                    .root
                    .join("releases")
                    .join(current.id.to_string())
                    .join("receipt.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(written["source"]["repository"], "pj-tmt/tmt");
        assert_eq!(written["source"]["release_id"], 42);
        // The superseded receipt is left as it was, and still reads.
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&receipt_path).unwrap()).unwrap(),
            legacy
        );
    }
}

#[test]
fn verified_update_pins_noops_and_unpins_through_one_publisher() {
    let (_directory, layout, old) = published_layout();
    let old_executable = layout
        .root
        .join("releases")
        .join(old.id.to_string())
        .join("tmt");
    let old_bytes = fs::read(&old_executable).unwrap();
    let report = upgrade_with(
        UpgradeRequest {
            executable: &old_executable,
            channel: None,
            exact: Some("1.2.4"),
            unpin: false,
        },
        || Ok(()),
        release_download(),
    )
    .unwrap();
    assert!(report.installation.changed);
    assert!(!report.skipped_pinned);
    assert_eq!(
        report.state.pinned_version.as_ref().unwrap().to_string(),
        "1.2.4"
    );
    assert_eq!(
        fs::read(&report.installation.active_executable).unwrap(),
        b"native executable\n"
    );
    assert_eq!(fs::read(&old_executable).unwrap(), old_bytes);
    let receipt = layout.current().unwrap().unwrap();
    assert_eq!(
        receipt
            .provenance
            .as_ref()
            .unwrap()
            .release()
            .unwrap()
            .release_id,
        42
    );
    assert_eq!(
        receipt.provenance.as_ref().unwrap().manifest_sha256().len(),
        64
    );
    let active = report.installation.active_executable;
    let current = fs::read_link(layout.root.join("current")).unwrap();
    let noop = upgrade_with(
        UpgradeRequest {
            executable: &active,
            channel: None,
            exact: Some("1.2.4"),
            unpin: false,
        },
        || Ok(()),
        release_download(),
    )
    .unwrap();
    assert!(!noop.installation.changed);
    assert_eq!(fs::read_link(layout.root.join("current")).unwrap(), current);
    let unpinned = upgrade_with(
        UpgradeRequest {
            executable: &active,
            channel: None,
            exact: None,
            unpin: true,
        },
        || Ok(()),
        release_download(),
    )
    .unwrap();
    assert!(unpinned.installation.changed);
    assert!(unpinned.state.pinned_version.is_none());
    assert_ne!(fs::read_link(layout.root.join("current")).unwrap(), current);
    assert_eq!(
        fs::read(&unpinned.installation.active_executable).unwrap(),
        fs::read(&active).unwrap()
    );
    assert_no_downloads(&layout.root);
}

#[test]
fn a_pin_changed_during_download_cannot_be_overridden() {
    let (_directory, layout, old) = published_layout();
    let executable = layout
        .root
        .join("releases")
        .join(old.id.to_string())
        .join("tmt");
    let mut fetch = release_download();
    let mut switched = false;
    let mut pinned_id = None;
    let error = upgrade_with(
        UpgradeRequest {
            executable: &executable,
            channel: None,
            exact: None,
            unpin: false,
        },
        || Ok(()),
        |url, accept, limit, deadline| {
            if !switched {
                let candidate = artifact("1.2.3", b"synthetic tmt payload\n");
                let receipt = Receipt::new(&candidate, state("1.2.3", Some("1.2.3")));
                pinned_id = Some(receipt.id);
                publish(&layout, &candidate, &receipt);
                switched = true;
            }
            fetch(url, accept, limit, deadline)
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("pin changed while downloading"));
    assert!(error.activated.is_none());
    let current = layout.current().unwrap().unwrap();
    assert_eq!(Some(current.id), pinned_id);
    assert_eq!(current.state.pinned_version.unwrap().to_string(), "1.2.3");
    assert_no_downloads(&layout.root);
}

#[test]
fn cancellation_after_download_removes_only_owned_staging_and_preserves_old_release() {
    for stop_at in [2, 3, 5] {
        let (_directory, layout, old) = published_layout();
        let executable = layout
            .root
            .join("releases")
            .join(old.id.to_string())
            .join("tmt");
        let mut calls = 0;
        let error = upgrade_with(
            UpgradeRequest {
                executable: &executable,
                channel: None,
                exact: None,
                unpin: false,
            },
            || {
                calls += 1;
                if calls == stop_at {
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "fixture interrupted",
                    ))
                } else {
                    Ok(())
                }
            },
            release_download(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(error.activated.is_none());
        assert_eq!(layout.current().unwrap().unwrap().id, old.id);
        assert_eq!(fs::read(&executable).unwrap(), b"synthetic tmt payload\n");
        assert_no_downloads(&layout.root);
        assert_eq!(
            fs::read_dir(layout.root.join("releases")).unwrap().count(),
            1
        );
    }
}

#[test]
fn pinned_installation_is_verified_without_network_or_staging() {
    let (_directory, layout, _) = published_layout();
    let artifact = artifact("1.2.3", b"pinned executable");
    let receipt = Receipt::new(&artifact, state("1.2.3", Some("1.2.3")));
    publish(&layout, &artifact, &receipt);
    let original = fs::read_link(layout.root.join("current")).unwrap();
    let executable = layout.root.join(&original).join("tmt");
    let report = upgrade_with(
        UpgradeRequest {
            executable: &executable,
            channel: None,
            exact: None,
            unpin: false,
        },
        || Ok(()),
        |_, _, _, _| panic!("pinned update must not access the network"),
    )
    .unwrap();
    assert!(report.skipped_pinned);
    assert!(!report.installation.changed);
    assert_eq!(report.state.pinned_version.unwrap().to_string(), "1.2.3");
    assert_eq!(
        fs::read_link(layout.root.join("current")).unwrap(),
        original
    );
    assert!(!fs::read_dir(&layout.root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".download-")
    }));
}

#[test]
fn stale_executable_and_unowned_paths_fail_before_network() {
    let (directory, layout, old) = published_layout();
    let stale = layout
        .root
        .join("releases")
        .join(old.id.to_string())
        .join("tmt");
    let next = artifact("1.2.4", b"new executable");
    publish(&layout, &next, &Receipt::new(&next, state("1.2.4", None)));
    let unmanaged = directory.path.join("unmanaged");
    fs::write(&unmanaged, b"unmanaged executable").unwrap();
    for executable in [&stale, &unmanaged] {
        let error = upgrade_with(
            UpgradeRequest {
                executable,
                channel: None,
                exact: None,
                unpin: false,
            },
            || Ok(()),
            |_, _, _, _| panic!("unowned update must not access the network"),
        )
        .unwrap_err();
        assert!(error.activated.is_none());
        assert!(error.to_string().contains("package manager"));
    }
    assert_eq!(fs::read(&unmanaged).unwrap(), b"unmanaged executable");
    assert_eq!(fs::read(&stale).unwrap(), b"synthetic tmt payload\n");
}

#[test]
fn missing_release_preserves_active_files_without_staging() {
    let (_directory, layout, _) = published_layout();
    let original = fs::read_link(layout.root.join("current")).unwrap();
    let executable = layout.root.join(&original).join("tmt");
    let before = fs::read(&executable).unwrap();
    let mut calls = 0;
    let error = upgrade_with(
        UpgradeRequest {
            executable: &executable,
            channel: None,
            exact: None,
            unpin: false,
        },
        || Ok(()),
        |url, _, _, _| {
            calls += 1;
            assert_eq!(
                url,
                "https://api.github.com/repos/pj-tmt/tmt/git/matching-refs/tags/v?per_page=100&page=1"
            );
            Ok(b"[]".to_vec().into())
        },
    )
    .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(error.activated.is_none());
    assert_eq!(
        fs::read_link(layout.root.join("current")).unwrap(),
        original
    );
    assert_eq!(fs::read(&executable).unwrap(), before);
}

#[test]
fn consent_selected_versions_upgrade_products_without_creating_pins() {
    use crate::native_install::{Product, publication::Layout};
    let directory = crate::test_support::TestDirectory::new();
    let prefix = directory.path.join("prefix");
    let target = "aarch64-apple-darwin";
    for product in [Product::Cli, Product::Ops, Product::Remote, Product::Colab] {
        let (_, manifest, archive, name) = release::product_fixture(product, "1.2.3", target, 41);
        let old =
            super::artifact::acquire_bytes(product, &manifest, &name, &archive, target).unwrap();
        let layout = Layout::open_product(&prefix, product).unwrap();
        publish(&layout, &old, &Receipt::new(&old, state("1.2.3", None)));
        let executable = prefix.join("bin").join(product.executable());
        let selected = "1.2.4".parse().unwrap();
        let (release, manifest, archive, _) =
            release::product_fixture(product, "1.2.4", target, 42);
        let expected = format!("/tags/{}1.2.4", product.tag_prefix());
        let report = super::upgrade_product_with(
            product,
            UpgradeRequest {
                executable: &executable,
                channel: None,
                exact: None,
                unpin: false,
            },
            None,
            Some(&selected),
            || Ok(()),
            |url, _, _, _| {
                let bytes = if url.ends_with("/assets/421") {
                    manifest.clone()
                } else if url.ends_with("/assets/422") {
                    archive.clone()
                } else {
                    assert!(url.ends_with(&expected));
                    serde_json::to_vec(&release).unwrap()
                };
                Ok(bytes.into())
            },
            crate::native_install::test_support::install_downloaded,
        )
        .unwrap();
        assert!(report.installation.changed);
        assert_eq!(report.state.version, selected);
        assert!(report.state.pinned_version.is_none());
        let installed = super::super::inspect_product(product, &executable).unwrap();
        assert_eq!(installed.state.version, selected);
        assert_eq!(installed.state.channel, Channel::Stable);
        assert!(installed.state.pinned_version.is_none());
        assert_eq!(layout.current().unwrap().unwrap().state.version, selected);
    }
    // Independently owned active pointers remain valid after every upgrade.
    for product in [Product::Cli, Product::Ops, Product::Remote, Product::Colab] {
        assert_eq!(
            super::super::inspect_product(product, &prefix.join("bin").join(product.executable()))
                .unwrap()
                .state
                .version
                .to_string(),
            "1.2.4"
        );
    }
}

#[test]
fn rate_limit_failure_preserves_active_executable_receipt_and_has_no_staging() {
    let (_directory, layout, _) = published_layout();
    let current = fs::read_link(layout.root.join("current")).unwrap();
    let executable = layout.root.join(&current).join("tmt");
    let receipt = layout.root.join(&current).join("receipt.json");
    let old_executable = fs::read(&executable).unwrap();
    let old_receipt = fs::read(&receipt).unwrap();
    let cause = "GitHub API rate limit: reset/earliest retry time 2030-01-01 UTC; the required wait exceeds the remaining deadline. Retry later or optionally set GITHUB_TOKEN.";
    let mut calls = 0;
    let error = upgrade_with(
        UpgradeRequest {
            executable: &executable,
            channel: None,
            exact: None,
            unpin: false,
        },
        || Ok(()),
        |_, _, _, _| {
            calls += 1;
            Err(io::Error::other(cause))
        },
    )
    .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.to_string(), cause);
    assert!(error.activated.is_none());
    assert_eq!(fs::read_link(layout.root.join("current")).unwrap(), current);
    assert_eq!(fs::read(&executable).unwrap(), old_executable);
    assert_eq!(fs::read(&receipt).unwrap(), old_receipt);
    assert_no_downloads(&layout.root);
    assert_eq!(
        fs::read_dir(layout.root.join("releases")).unwrap().count(),
        1
    );
}
