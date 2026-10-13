//! The CLI's move from `lib/tmux-team` to `lib/tmt`: the former prefix stays the
//! rollback source, links move atomically, and each updater generation is served.

use super::{
    ActivationRequest, Caller, InstallReport, Product, active_companion,
    former_prefix::{blockers, in_moved_prefix, is_move, retire},
    inspect, inspect_former_product,
    publication::Layout,
    receipt::Receipt,
    remove::{plan_product_removal, remove_product},
    retained_former_hint,
    test_support::{artifact, publish, state},
};
use crate::test_support::TestDirectory;
use std::{
    fs, io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
};
use tmt_core::native_install::{Channel, PinAction};
use uuid::Uuid;

const OLD: &[u8] = b"#!/bin/sh\necho old\n";
const NEW: &[u8] = b"#!/bin/sh\necho new\n";
const NEWER: &[u8] = b"#!/bin/sh\necho newer\n";
const LINKS: [&str; 2] = ["tmt", "tmux-team"];

/// An installation as the previous generation left it: `lib/tmux-team` active.
fn legacy(directory: &TestDirectory, version: &str) -> (PathBuf, Uuid) {
    let prefix = directory.path.join("prefix");
    let layout = Layout::open_former(&prefix).unwrap();
    let release = artifact(version, OLD);
    let receipt = Receipt::new(&release, state(version, None));
    publish(&layout, &release, &receipt);
    (layout.prefix.clone(), receipt.id)
}

fn activate(
    prefix: &Path,
    caller: Caller,
    version: &str,
    payload: &[u8],
    expected: Option<Uuid>,
) -> io::Result<InstallReport> {
    super::activate(
        ActivationRequest {
            caller,
            product: Product::Cli,
            prefix,
            channel: Channel::Stable,
            pin: PinAction::Preserve,
            expected,
            provenance: None,
            verifier: None,
            explicit_channel: true,
            schema: None,
        },
        &artifact(version, payload),
        || Ok(()),
    )
}

fn release_id(report: &InstallReport) -> Uuid {
    let name = report.active_executable.parent().unwrap().file_name();
    Uuid::parse_str(name.unwrap().to_str().unwrap()).unwrap()
}

fn link(prefix: &Path, name: &str) -> PathBuf {
    fs::read_link(prefix.join("bin").join(name)).unwrap()
}

fn assert_links(prefix: &Path, namespace: &str) {
    for name in LINKS {
        assert_eq!(
            link(prefix, name),
            PathBuf::from(format!("../{namespace}/current/tmt")),
            "{name}"
        );
    }
    let mut entries: Vec<_> = fs::read_dir(prefix.join("bin"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    entries.sort();
    assert_eq!(entries, LINKS, "no temporary link survives");
}

fn run(executable: &Path) -> String {
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn a_current_updater_moves_the_installation_and_keeps_the_former_prefix_runnable() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    assert_links(&prefix, "lib/tmux-team");
    let old_executable = prefix.join(format!("lib/tmux-team/releases/{old}/tmt"));

    let report = activate(&prefix, Caller::Current, "1.3.0", NEW, Some(old)).unwrap();

    assert!(report.changed);
    assert_eq!(report.version, "1.3.0");
    let moved = prefix.join("lib/tmt/releases");
    assert!(report.active_executable.starts_with(&moved));
    assert_links(&prefix, "lib/tmt");
    assert_eq!(
        inspect(&prefix.join("bin/tmt"))
            .unwrap()
            .state
            .version
            .to_string(),
        "1.3.0"
    );
    // Rollback source: the previous release is still current in its prefix,
    // verified through the former-layout read path, and still runs.
    assert_eq!(
        fs::read_link(prefix.join("lib/tmux-team/current")).unwrap(),
        PathBuf::from(format!("releases/{old}"))
    );
    let former = inspect_former_product(Product::Cli, &prefix)
        .unwrap()
        .unwrap();
    assert_eq!(former.release_id(), old);
    assert_eq!(former.state.version.to_string(), "1.2.3");
    assert_eq!(run(&old_executable), "old\n");
    assert_eq!(run(&prefix.join("bin/tmt")), "new\n");
    assert!(in_moved_prefix(&report.active_executable));
    assert!(!in_moved_prefix(&old_executable));
}

#[test]
fn a_legacy_updater_keeps_getting_its_release_under_the_former_prefix() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");

    let report = activate(&prefix, Caller::Legacy, "1.3.0", NEW, Some(old)).unwrap();

    assert!(report.changed);
    assert!(
        report
            .active_executable
            .starts_with(prefix.join("lib/tmux-team/releases"))
    );
    assert!(!prefix.join("lib/tmt").exists(), "no new prefix is created");
    assert_links(&prefix, "lib/tmux-team");
    assert_eq!(run(&prefix.join("bin/tmt")), "new\n");
}

#[test]
fn a_legacy_updater_after_the_move_is_refused_and_changes_nothing() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    let moved = release_id(&activate(&prefix, Caller::Current, "1.3.0", NEW, Some(old)).unwrap());

    // A stale updater still observed the release that is now the rollback source.
    let error = activate(&prefix, Caller::Legacy, "1.4.0", NEWER, Some(old)).unwrap_err();

    assert!(
        error.to_string().contains("changed while downloading"),
        "{error}"
    );
    assert_links(&prefix, "lib/tmt");
    assert_eq!(
        fs::read_link(prefix.join("lib/tmt/current")).unwrap(),
        PathBuf::from(format!("releases/{moved}"))
    );
    assert_eq!(
        fs::read_link(prefix.join("lib/tmux-team/current")).unwrap(),
        PathBuf::from(format!("releases/{old}"))
    );
}

#[test]
fn a_fresh_install_uses_the_new_prefix_with_both_commands() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");

    let report = activate(&prefix, Caller::Current, "1.2.3", NEW, None).unwrap();

    assert!(
        report
            .active_executable
            .starts_with(prefix.canonicalize().unwrap().join("lib/tmt"))
    );
    assert_links(&prefix.canonicalize().unwrap(), "lib/tmt");
    assert!(!prefix.join("lib/tmux-team").exists());
}

#[test]
fn a_new_prefix_that_is_not_an_installation_is_refused_before_any_write() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    fs::create_dir_all(prefix.join("lib/tmt")).unwrap();
    fs::write(prefix.join("lib/tmt/notes.txt"), "mine").unwrap();

    let error = activate(&prefix, Caller::Current, "1.3.0", NEW, Some(old)).unwrap_err();

    assert!(error.to_string().contains("did not write"), "{error}");
    let entries: Vec<_> = fs::read_dir(prefix.join("lib/tmt")).unwrap().collect();
    assert_eq!(entries.len(), 1);
    assert_links(&prefix, "lib/tmux-team");
    assert_eq!(run(&prefix.join("bin/tmt")), "old\n");
}

#[test]
fn a_foreign_command_is_never_replaced_by_the_move() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    fs::remove_file(prefix.join("bin/tmux-team")).unwrap();
    fs::write(prefix.join("bin/tmux-team"), "not ours").unwrap();

    let error = activate(&prefix, Caller::Current, "1.3.0", NEW, Some(old)).unwrap_err();

    assert!(error.to_string().contains("Refusing to replace"), "{error}");
    assert_eq!(
        fs::read_to_string(prefix.join("bin/tmux-team")).unwrap(),
        "not ours"
    );
    assert_eq!(
        link(&prefix, "tmt"),
        PathBuf::from("../lib/tmux-team/current/tmt")
    );
    assert!(!prefix.join("lib/tmt/current").exists());
}

#[test]
fn a_damaged_new_release_is_refused_and_the_rollback_source_stays_usable() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    let moved = release_id(&activate(&prefix, Caller::Current, "1.3.0", NEW, Some(old)).unwrap());
    fs::write(
        prefix.join(format!("lib/tmt/releases/{moved}/tmt")),
        b"tampered",
    )
    .unwrap();

    assert!(activate(&prefix, Caller::Current, "1.4.0", NEWER, Some(moved)).is_err());

    assert_links(&prefix, "lib/tmt");
    assert_eq!(
        inspect_former_product(Product::Cli, &prefix)
            .unwrap()
            .unwrap()
            .release_id(),
        old
    );
    assert!(
        prefix.join("lib/tmux-team").exists(),
        "damage never retires the former prefix"
    );
}

#[test]
fn an_equal_version_install_by_a_current_caller_still_moves_the_installation() {
    // A bootstrap or offline install of the release the former prefix already
    // holds: nothing differs, yet the links must not point into an empty prefix.
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");

    let report = activate(&prefix, Caller::Current, "1.2.3", OLD, Some(old)).unwrap();

    assert!(report.changed, "the installation moved");
    assert!(
        report
            .active_executable
            .starts_with(prefix.join("lib/tmt/releases"))
    );
    assert_links(&prefix, "lib/tmt");
    assert_eq!(
        run(&prefix.join("bin/tmt")),
        "old\n",
        "the command link resolves"
    );
    assert_eq!(
        inspect(&prefix.join("bin/tmt"))
            .unwrap()
            .state
            .version
            .to_string(),
        "1.2.3"
    );
    assert!(
        prefix
            .join(format!("lib/tmux-team/releases/{old}/tmt"))
            .exists(),
        "the former prefix stays the rollback source"
    );

    let again = activate(&prefix, Caller::Current, "1.2.3", OLD, None).unwrap();
    assert!(!again.changed, "the move happens once");
    assert_eq!(again.active_executable, report.active_executable);
    assert_links(&prefix, "lib/tmt");
}

#[test]
fn an_interrupted_move_is_completed_by_the_next_installation() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    // Activation of the new prefix happened; the links were not moved yet.
    let layout = Layout::open_product(&prefix, Product::Cli).unwrap();
    let release = artifact("1.3.0", NEW);
    let receipt = Receipt::new(&release, state("1.3.0", None));
    let interrupted =
        layout.publish_with_finalization(&release, &receipt, None, None, &mut || Ok(()), || {
            Err(io::Error::other("interrupted"))
        });
    assert!(interrupted.is_err());
    assert_links(&prefix, "lib/tmux-team");

    let report = activate(&prefix, Caller::Current, "1.3.0", NEW, None).unwrap();

    assert!(!report.changed);
    assert_links(&prefix, "lib/tmt");
    assert!(
        prefix.join("lib/tmux-team").exists(),
        "completing the move keeps the rollback source"
    );
    assert_eq!(
        fs::read_link(prefix.join("lib/tmux-team/current")).unwrap(),
        PathBuf::from(format!("releases/{old}"))
    );
}

#[test]
fn the_next_upgrade_retires_the_former_prefix() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    let moved = release_id(&activate(&prefix, Caller::Current, "1.3.0", NEW, Some(old)).unwrap());
    assert!(prefix.join("lib/tmux-team").exists());

    let report = activate(&prefix, Caller::Current, "1.4.0", NEWER, Some(moved)).unwrap();

    assert!(report.changed);
    assert!(!prefix.join("lib/tmux-team").exists());
    assert_links(&prefix, "lib/tmt");
    assert_eq!(run(&prefix.join("bin/tmt")), "newer\n");
    assert!(retained_former_hint(&report.active_executable).is_none());
}

#[test]
fn a_legacy_updater_never_retires_anything() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    let report = activate(&prefix, Caller::Legacy, "1.3.0", NEW, Some(old)).unwrap();
    let second = activate(
        &prefix,
        Caller::Legacy,
        "1.4.0",
        NEWER,
        Some(release_id(&report)),
    );

    assert!(second.is_ok());
    assert!(prefix.join("lib/tmux-team").exists());
    assert!(!prefix.join("lib/tmt").exists());
}

#[test]
fn a_reference_into_the_former_prefix_keeps_it_and_says_why() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    let moved = release_id(&activate(&prefix, Caller::Current, "1.3.0", NEW, Some(old)).unwrap());
    symlink("../lib/tmux-team/current/tmt", prefix.join("bin/old-tmt")).unwrap();

    let report = activate(&prefix, Caller::Current, "1.4.0", NEWER, Some(moved)).unwrap();

    // The upgrade itself succeeded; only the cleanup was declined.
    assert!(report.changed);
    assert!(prefix.join("lib/tmux-team").exists());
    let hint = retained_former_hint(&report.active_executable).unwrap();
    assert!(hint.contains("old-tmt"), "{hint}");
    assert!(hint.contains("next upgrade"), "{hint}");
    fs::remove_file(prefix.join("bin/old-tmt")).unwrap();
    let again = activate(
        &prefix,
        Caller::Current,
        "1.5.0",
        NEW,
        Some(release_id(&report)),
    )
    .unwrap();
    assert!(again.changed);
    assert!(
        !prefix.join("lib/tmux-team").exists(),
        "removed once nothing points into it"
    );
}

#[test]
fn the_audit_names_command_links_and_provider_settings_that_point_into_the_former_prefix() {
    let directory = TestDirectory::new();
    let (prefix, old) = legacy(&directory, "1.2.3");
    let root = prefix.join("lib/tmux-team");
    let hooks = directory.path.join("settings.json");
    let clean = directory.path.join("clean.json");
    fs::write(
        &clean,
        format!(
            r#"{{"hooks":"'{}/bin/tmt' __hook claude"}}"#,
            prefix.display()
        ),
    )
    .unwrap();
    fs::write(
        &hooks,
        format!(
            r#"{{"hooks":"'{}/current/tmt' __hook claude"}}"#,
            root.display()
        ),
    )
    .unwrap();
    let missing = directory.path.join("absent.json");

    let before = blockers(&prefix, &root, &[clean.clone(), missing.clone()]).unwrap();
    assert_eq!(
        before.len(),
        2,
        "both commands still point into it: {before:?}"
    );
    activate(&prefix, Caller::Current, "1.3.0", NEW, Some(old)).unwrap();
    assert!(
        blockers(&prefix, &root, &[clean, missing])
            .unwrap()
            .is_empty()
    );
    let found = blockers(&prefix, &root, std::slice::from_ref(&hooks)).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].contains("settings.json"), "{found:?}");
    let layout = Layout::existing_product(&prefix, Product::Cli).unwrap();
    assert!(retire(&layout).is_ok(), "a clean audit retires the prefix");
    assert!(!root.exists());
}

#[test]
fn removing_the_product_removes_both_prefixes_and_reports_each_command_once() {
    for moved in [false, true] {
        let directory = TestDirectory::new();
        let (prefix, old) = legacy(&directory, "1.2.3");
        if moved {
            activate(&prefix, Caller::Current, "1.3.0", NEW, Some(old)).unwrap();
        }

        let plan = plan_product_removal(&prefix, Product::Cli).unwrap();
        assert!(plan.kept.is_empty(), "{plan:?}");
        for name in LINKS {
            assert_eq!(
                plan.removed
                    .iter()
                    .filter(|path| path.ends_with(format!("bin/{name}")))
                    .count(),
                1,
                "{name} {plan:?}"
            );
        }

        remove_product(&prefix, Product::Cli).unwrap();

        assert_eq!(fs::read_dir(prefix.join("bin")).unwrap().count(), 0);
        assert!(!prefix.join("lib/tmt").exists());
        assert!(!prefix.join("lib/tmux-team").exists());
    }
}

#[test]
fn only_the_cli_former_prefix_is_a_move() {
    assert!(is_move(Product::Cli));
    for product in [Product::Ops, Product::Remote, Product::Colab] {
        assert!(!is_move(product), "{product:?}");
    }
}

#[test]
fn a_companion_is_found_in_both_layouts() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");
    let layout = Layout::open_former(&prefix).unwrap();
    let mut release = artifact("1.2.3", OLD);
    release
        .files
        .insert("tmt-driver-herdr".into(), b"herdr".to_vec());
    release.executable_files.insert("tmt-driver-herdr".into());
    let receipt = Receipt::new(&release, state("1.2.3", None));
    publish(&layout, &release, &receipt);
    let prefix = layout.prefix.clone();
    let before = prefix.join(format!("lib/tmux-team/releases/{}/tmt", receipt.id));

    let companion = active_companion(&before, "tmt-driver-herdr")
        .unwrap()
        .unwrap();
    assert!(
        companion
            .path
            .starts_with(prefix.join("lib/tmux-team/releases"))
    );

    let mut moved = artifact("1.3.0", NEW);
    moved
        .files
        .insert("tmt-driver-herdr".into(), b"herdr".to_vec());
    moved.executable_files.insert("tmt-driver-herdr".into());
    let report = super::activate(
        ActivationRequest {
            caller: Caller::Current,
            product: Product::Cli,
            prefix: &prefix,
            channel: Channel::Stable,
            pin: PinAction::Preserve,
            expected: Some(receipt.id),
            provenance: None,
            verifier: None,
            explicit_channel: true,
            schema: None,
        },
        &moved,
        || Ok(()),
    )
    .unwrap();
    let companion = active_companion(&report.active_executable, "tmt-driver-herdr")
        .unwrap()
        .unwrap();
    assert!(companion.path.starts_with(prefix.join("lib/tmt/releases")));
}
