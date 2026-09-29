use super::{
    artifact::{self, FILES},
    publication::Layout,
    receipt::Receipt,
    test_support::{
        VERSION, artifact as synthetic_artifact, checkpoint_count, publish, published_layout, state,
    },
};
use crate::test_support::TestDirectory;
use semver::Version;
use std::{
    collections::BTreeMap,
    fs, io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

fn current_target(layout: &Layout) -> PathBuf {
    fs::read_link(layout.root.join("current")).unwrap()
}

fn assert_managed_links(layout: &Layout) {
    for name in ["tmt", "tmux-team"] {
        assert_eq!(
            fs::read_link(layout.prefix.join("bin").join(name)).unwrap(),
            Path::new("../lib/tmux-team/current/tmt")
        );
    }
}

#[test]
fn first_publish_writes_receipt_hashes_and_repeat_links_are_stable() {
    let (_directory, layout, receipt) = published_layout();
    let target = current_target(&layout);
    let current = layout.current().unwrap().unwrap();

    assert_eq!(current.id, receipt.id);
    assert_eq!(current.file_hashes, receipt.file_hashes);
    assert_eq!(
        current.file_hashes["tmt"],
        artifact::digest(b"synthetic tmt payload\n")
    );
    assert_eq!(target, PathBuf::from(format!("releases/{}", receipt.id)));
    assert_managed_links(&layout);

    layout.ensure_links().unwrap();
    assert_eq!(current_target(&layout), target);
    assert_managed_links(&layout);
}

#[test]
fn corrupted_installed_payload_is_rejected_without_changing_current() {
    let (_directory, layout, receipt) = published_layout();
    let target = current_target(&layout);
    let release = layout.root.join(&target);
    fs::write(release.join("tmt"), b"modified executable\n").unwrap();

    let error = match layout.current() {
        Ok(_) => panic!("corrupted payload must be rejected"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "Installed release file has changed; refusing replacement."
    );
    assert_eq!(current_target(&layout), target);
    assert_eq!(
        fs::read_link(layout.prefix.join("bin/tmt")).unwrap(),
        Path::new("../lib/tmux-team/current/tmt")
    );
    assert_eq!(target, PathBuf::from(format!("releases/{}", receipt.id)));
}

#[test]
fn invalid_current_pointer_is_rejected_without_mutating_releases() {
    let (_directory, layout, receipt) = published_layout();
    let pointer = layout.root.join("current");
    fs::remove_file(&pointer).unwrap();
    symlink("releases/not-a-uuid", &pointer).unwrap();

    let error = match layout.current() {
        Ok(_) => panic!("invalid current pointer must be rejected"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "Native current pointer is not an owned release."
    );
    assert_eq!(
        fs::read_link(&pointer).unwrap(),
        Path::new("releases/not-a-uuid")
    );
    assert!(
        layout
            .root
            .join(format!("releases/{}", receipt.id))
            .is_dir()
    );
    assert_eq!(
        fs::read(layout.root.join(format!("releases/{}/tmt", receipt.id))).unwrap(),
        b"synthetic tmt payload\n"
    );
}

#[test]
fn failed_receipt_validation_keeps_old_current_and_release_bytes() {
    let (_directory, layout, receipt) = published_layout();
    let target = current_target(&layout);
    let release = layout.root.join(&target);
    fs::write(release.join("receipt.json"), b"{}\n").unwrap();

    assert!(layout.current().is_err());
    assert_eq!(current_target(&layout), target);
    assert_eq!(
        fs::read(release.join("tmt")).unwrap(),
        b"synthetic tmt payload\n"
    );
    assert!(
        layout
            .root
            .join(format!("releases/{}", receipt.id))
            .is_dir()
    );
}

#[test]
fn unmanaged_files_and_package_manager_links_are_preserved() {
    for symlinked in [false, true] {
        let directory = TestDirectory::new();
        let layout = Layout::open(&directory.path.join("prefix")).unwrap();
        let target = layout.prefix.join("bin/tmt");
        if symlinked {
            symlink("/opt/package-manager/bin/tmt", &target).unwrap();
        } else {
            fs::write(&target, b"package manager executable\n").unwrap();
        }

        assert!(layout.check_links(false).is_err());
        if symlinked {
            assert_eq!(
                fs::read_link(&target).unwrap(),
                Path::new("/opt/package-manager/bin/tmt")
            );
        } else {
            assert_eq!(fs::read(&target).unwrap(), b"package manager executable\n");
        }
    }
}

#[test]
fn pin_metadata_publication_changes_receipt_and_pointer_retaining_previous_release() {
    let (_directory, layout, first) = published_layout();
    let first_target = current_target(&layout);
    let artifact = synthetic_artifact(VERSION, b"synthetic tmt payload\n");
    let pinned = Receipt::new(&artifact, state(VERSION, Some(VERSION)));

    publish(&layout, &artifact, &pinned);
    let second_target = current_target(&layout);
    let current = layout.current().unwrap().unwrap();

    assert_ne!(second_target, first_target);
    assert_eq!(current.id, pinned.id);
    assert_eq!(
        current.state.pinned_version,
        Some(Version::parse(VERSION).unwrap())
    );
    assert!(layout.root.join(&first_target).is_dir());
    assert!(layout.root.join(&second_target).is_dir());
    assert_ne!(
        fs::read(layout.root.join(&first_target).join("receipt.json")).unwrap(),
        fs::read(layout.root.join(&second_target).join("receipt.json")).unwrap()
    );
    assert_eq!(
        Receipt::read(&layout.root.join(&first_target), &layout.prefix, first.id)
            .unwrap()
            .state
            .pinned_version,
        None
    );
}

#[test]
fn preactivation_checkpoint_failures_cleanup_owned_state_and_preserve_current() {
    // Publication checkpoints occur before staging, once per required file,
    // after the receipt and release sync, and after staging the activation link.
    for failure_at in 0..checkpoint_count() {
        let (_directory, layout, first) = published_layout();
        let old_target = current_target(&layout);
        let old_release = layout.root.join(&old_target);
        let old_files = FILES
            .iter()
            .map(|name| {
                (
                    (*name).to_owned(),
                    fs::read(old_release.join(name)).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let old_receipt = fs::read(old_release.join("receipt.json")).unwrap();

        let next_artifact = synthetic_artifact("1.2.4", b"next synthetic tmt payload\n");
        let next_receipt = Receipt::new(&next_artifact, state("1.2.4", None));
        let next_release = layout
            .root
            .join("releases")
            .join(next_receipt.id.to_string());
        let next_pointer = layout.root.join(format!(".current-{}", next_receipt.id));
        let mut calls = 0;
        let mut checkpoint = || {
            let call = calls;
            calls += 1;
            if call == failure_at {
                Err(io::Error::other("injected checkpoint"))
            } else {
                Ok(())
            }
        };

        let error = layout
            .publish(
                &next_artifact,
                &next_receipt,
                Some(first.id),
                None,
                &mut checkpoint,
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "injected checkpoint",
            "checkpoint {failure_at}"
        );
        assert_eq!(
            current_target(&layout),
            old_target,
            "checkpoint {failure_at}"
        );
        let current = layout.current().unwrap().unwrap();
        assert_eq!(current.id, first.id, "checkpoint {failure_at}");
        assert_eq!(
            fs::read(old_release.join("receipt.json")).unwrap(),
            old_receipt,
            "checkpoint {failure_at}"
        );
        for (name, bytes) in &old_files {
            assert_eq!(
                fs::read(old_release.join(name)).unwrap(),
                *bytes,
                "checkpoint {failure_at}"
            );
        }
        assert!(
            fs::symlink_metadata(next_release).is_err(),
            "checkpoint {failure_at}"
        );
        assert!(
            fs::symlink_metadata(next_pointer).is_err(),
            "checkpoint {failure_at}"
        );
    }
}

#[test]
fn current_change_at_activation_checkpoint_preserves_unexpected_release() {
    let (_directory, layout, first) = published_layout();
    let old_target = current_target(&layout);
    let foreign_artifact = synthetic_artifact("1.2.2", b"unexpected tmt payload\n");
    let foreign_receipt = Receipt::new(&foreign_artifact, state("1.2.2", None));
    publish(&layout, &foreign_artifact, &foreign_receipt);
    let foreign_target = current_target(&layout);
    assert_ne!(foreign_target, old_target);
    fs::remove_file(layout.root.join("current")).unwrap();
    symlink(&old_target, layout.root.join("current")).unwrap();

    let next_artifact = synthetic_artifact("1.2.4", b"next synthetic tmt payload\n");
    let next_receipt = Receipt::new(&next_artifact, state("1.2.4", None));
    let next_release = layout
        .root
        .join("releases")
        .join(next_receipt.id.to_string());
    let next_pointer = layout.root.join(format!(".current-{}", next_receipt.id));
    let last_checkpoint = checkpoint_count() - 1;
    let mut calls = 0;
    let mut checkpoint = || {
        if calls == last_checkpoint {
            fs::remove_file(layout.root.join("current")).unwrap();
            symlink(&foreign_target, layout.root.join("current")).unwrap();
        }
        calls += 1;
        Ok(())
    };

    let error = layout
        .publish(
            &next_artifact,
            &next_receipt,
            Some(first.id),
            None,
            &mut checkpoint,
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Native current release changed during installation."
    );
    assert_eq!(current_target(&layout), foreign_target);
    assert_eq!(layout.current().unwrap().unwrap().id, foreign_receipt.id);
    assert!(fs::symlink_metadata(next_release).is_err());
    assert!(fs::symlink_metadata(next_pointer).is_err());
    assert_managed_links(&layout);
}

#[test]
fn command_link_change_at_activation_checkpoint_is_preserved() {
    let (_directory, layout, first) = published_layout();
    let old_target = current_target(&layout);
    let command = layout.prefix.join("bin/tmt");
    let next_artifact = synthetic_artifact("1.2.4", b"next synthetic tmt payload\n");
    let next_receipt = Receipt::new(&next_artifact, state("1.2.4", None));
    let next_release = layout
        .root
        .join("releases")
        .join(next_receipt.id.to_string());
    let next_pointer = layout.root.join(format!(".current-{}", next_receipt.id));
    let last_checkpoint = checkpoint_count() - 1;
    let mut calls = 0;
    let mut checkpoint = || {
        if calls == last_checkpoint {
            fs::remove_file(&command).unwrap();
            fs::write(&command, b"unexpected manager command\n").unwrap();
        }
        calls += 1;
        Ok(())
    };

    let error = layout
        .publish(
            &next_artifact,
            &next_receipt,
            Some(first.id),
            None,
            &mut checkpoint,
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(current_target(&layout), old_target);
    assert_eq!(fs::read(&command).unwrap(), b"unexpected manager command\n");
    assert!(fs::symlink_metadata(next_release).is_err());
    assert!(fs::symlink_metadata(next_pointer).is_err());
}

#[test]
fn prefix_leaf_symlink_with_trailing_separator_is_rejected_without_following_it() {
    let directory = TestDirectory::new();
    let destination = directory.path.join("destination");
    let prefix = directory.path.join("prefix");
    fs::create_dir(&destination).unwrap();
    symlink(&destination, &prefix).unwrap();
    let requested = PathBuf::from(format!("{}/", prefix.display()));

    let error = match Layout::open(&requested) {
        Ok(_) => panic!("prefix leaf symlink must be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read_link(&prefix).unwrap(), destination);
    assert!(fs::symlink_metadata(destination.join("lib")).is_err());
    assert!(fs::symlink_metadata(destination.join("bin")).is_err());
    assert!(fs::symlink_metadata(prefix.join("lib")).is_err());
    assert!(fs::symlink_metadata(prefix.join("bin")).is_err());
}

#[test]
fn ancestor_symlink_is_canonicalized_while_leaf_is_created() {
    let directory = TestDirectory::new();
    let destination = directory.path.join("destination");
    let alias = directory.path.join("alias");
    fs::create_dir(&destination).unwrap();
    symlink(&destination, &alias).unwrap();
    let requested = alias.join("nested");

    let layout = Layout::open(&requested).unwrap();
    assert_eq!(
        layout.prefix,
        fs::canonicalize(destination.join("nested")).unwrap()
    );
    assert!(layout.prefix.is_dir());
    assert!(layout.prefix.join("lib").is_dir());
    assert!(layout.prefix.join("bin").is_dir());
    assert_eq!(fs::read_link(&alias).unwrap(), destination);
}

#[test]
fn preexisting_activation_pointer_file_or_directory_is_preserved() {
    for directory_collision in [false, true] {
        let (_directory, layout, first) = published_layout();
        let old_target = current_target(&layout);
        let old_release = layout.root.join(&old_target);
        let old_receipt = fs::read(old_release.join("receipt.json")).unwrap();
        let next_artifact = synthetic_artifact("1.2.4", b"next synthetic tmt payload\n");
        let next_receipt = Receipt::new(&next_artifact, state("1.2.4", None));
        let pointer = layout.root.join(format!(".current-{}", next_receipt.id));
        let next_release = layout
            .root
            .join("releases")
            .join(next_receipt.id.to_string());

        if directory_collision {
            fs::create_dir(&pointer).unwrap();
            fs::write(pointer.join("sentinel"), b"preserve directory").unwrap();
        } else {
            fs::write(&pointer, b"preserve regular file").unwrap();
        }

        let mut checkpoint = || Ok(());
        let error = layout
            .publish(
                &next_artifact,
                &next_receipt,
                Some(first.id),
                None,
                &mut checkpoint,
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(current_target(&layout), old_target);
        assert_eq!(layout.current().unwrap().unwrap().id, first.id);
        assert_eq!(
            fs::read(old_release.join("receipt.json")).unwrap(),
            old_receipt
        );
        assert!(fs::symlink_metadata(next_release).is_err());
        if directory_collision {
            assert!(pointer.is_dir());
            assert_eq!(
                fs::read(pointer.join("sentinel")).unwrap(),
                b"preserve directory"
            );
        } else {
            assert_eq!(fs::read(&pointer).unwrap(), b"preserve regular file");
        }
        assert_managed_links(&layout);
    }
}

#[test]
fn publication_lock_contention_preserves_one_stable_lock_file() {
    let directory = TestDirectory::new();
    let lock = directory.path.join("install.lock");
    let first = crate::file_lock::exclusive(&lock).unwrap();
    assert!(lock.is_file());
    assert!(crate::file_lock::exclusive(&lock).is_err());
    drop(first);

    let second = crate::file_lock::exclusive(&lock).unwrap();
    assert!(lock.is_file());
    drop(second);
    assert!(lock.is_file());
}

#[test]
fn missing_command_links_are_repaired_without_replacing_the_release() {
    let (_directory, layout, receipt) = published_layout();
    let pointer = current_target(&layout);
    fs::remove_file(layout.prefix.join("bin/tmt")).unwrap();
    fs::remove_file(layout.prefix.join("bin/tmux-team")).unwrap();
    layout.check_links(true).unwrap();
    layout.ensure_links().unwrap();
    assert_managed_links(&layout);
    assert_eq!(current_target(&layout), pointer);
    assert_eq!(layout.current().unwrap().unwrap().id, receipt.id);
}

#[test]
fn finalization_failure_reports_activation_and_retry_repairs_first_install() {
    let directory = TestDirectory::new();
    let layout = Layout::open(&directory.path.join("prefix")).unwrap();
    let artifact = synthetic_artifact(VERSION, b"complete staged payload");
    let receipt = Receipt::new(&artifact, state(VERSION, None));
    let error = layout
        .publish_with_finalization(&artifact, &receipt, None, None, &mut || Ok(()), || {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected command-link failure",
            ))
        })
        .unwrap_err();
    assert!(error.to_string().contains("Release activated"));
    assert!(error.to_string().contains("injected command-link failure"));
    assert_eq!(layout.current().unwrap().unwrap().id, receipt.id);
    assert!(!layout.prefix.join("bin/tmt").exists());
    assert_eq!(
        fs::read(layout.root.join("current/tmt")).unwrap(),
        b"complete staged payload"
    );
    layout.ensure_links().unwrap();
    assert_managed_links(&layout);
    assert_eq!(layout.current().unwrap().unwrap().id, receipt.id);
}

#[test]
fn receipt_symlinks_extra_files_and_nonexecutable_payloads_are_not_ownership() {
    use std::os::unix::fs::PermissionsExt;
    for defect in [
        "receipt-link",
        "extra-file",
        "not-executable",
        "relocated-prefix",
    ] {
        let (_directory, layout, receipt) = published_layout();
        let pointer = current_target(&layout);
        let release = layout.root.join(&pointer);
        let original = fs::read(release.join("tmt")).unwrap();
        match defect {
            "receipt-link" => {
                let external = layout.prefix.join("external-receipt.json");
                fs::rename(release.join("receipt.json"), &external).unwrap();
                symlink(&external, release.join("receipt.json")).unwrap();
            }
            "extra-file" => fs::write(release.join("unexpected"), b"preserve me").unwrap(),
            "not-executable" => {
                fs::set_permissions(release.join("tmt"), fs::Permissions::from_mode(0o644)).unwrap()
            }
            "relocated-prefix" => {
                fs::write(
                    release.join("receipt.json"),
                    receipt.encode(Path::new("/another-prefix")).unwrap(),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(layout.current().is_err(), "{defect}");
        assert_eq!(current_target(&layout), pointer, "{defect}");
        assert_eq!(fs::read(release.join("tmt")).unwrap(), original, "{defect}");
        assert!(release.exists());
    }
}
