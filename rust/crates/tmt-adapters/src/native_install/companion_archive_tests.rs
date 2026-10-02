//! A CLI archive may carry a declared companion executable; anything else the
//! manifest and the archive disagree on refuses the whole release before
//! anything is published.

use super::*;
use crate::native_install::Product;

const DRIVER: &str = "tmt-driver-herdr";

fn acquire(fixture: &Fixture, product: Product) -> io::Result<artifact::Artifact> {
    artifact::acquire_product(product, &fixture.manifest, &fixture.archive, TARGET)
}

fn cli_files_with<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    FILES.iter().copied().chain(extra.iter().copied()).collect()
}

fn driver_entry(root: &str, mode: u32) -> Entry {
    Entry::File {
        path: format!("{root}/{DRIVER}"),
        bytes: b"#!/bin/sh\n".to_vec(),
        mode,
    }
}

#[test]
fn a_declared_companion_is_acquired_with_the_release() {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut entries = valid_entries(root);
    entries.push(driver_entry(root, 0o755));
    let fixture = product_fixture(entries, "tmt-cli", &cli_files_with(&[DRIVER]));
    let artifact = acquire(&fixture, Product::Cli).unwrap();
    assert_eq!(artifact.files[DRIVER], b"#!/bin/sh\n");
}

#[test]
fn a_companion_the_manifest_and_archive_disagree_on_is_refused() {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let cases: Vec<(&str, Vec<Entry>, Vec<&str>)> = vec![
        (
            "archived, not declared",
            vec![driver_entry(root, 0o755)],
            cli_files_with(&[]),
        ),
        (
            "declared, not archived",
            Vec::new(),
            cli_files_with(&[DRIVER]),
        ),
        (
            "not executable",
            vec![driver_entry(root, 0o644)],
            cli_files_with(&[DRIVER]),
        ),
        (
            "declared twice",
            vec![driver_entry(root, 0o755)],
            cli_files_with(&[DRIVER, DRIVER]),
        ),
        (
            "an unknown extra file",
            vec![Entry::File {
                path: format!("{root}/tmt-driver-other"),
                bytes: b"x".to_vec(),
                mode: 0o755,
            }],
            cli_files_with(&["tmt-driver-other"]),
        ),
    ];
    for (why, extra, files) in cases {
        let mut entries = valid_entries(root);
        entries.extend(extra);
        let fixture = product_fixture(entries, "tmt-cli", &files);
        assert!(acquire(&fixture, Product::Cli).is_err(), "{why}");
    }
}

#[test]
fn only_the_cli_release_may_carry_the_driver() {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut entries = valid_entries(root);
    entries[0] = Entry::File {
        path: format!("{root}/tmt-squad"),
        bytes: b"squad\n".to_vec(),
        mode: 0o755,
    };
    entries.push(driver_entry(root, 0o755));
    let files: Vec<&str> = Product::Squad.files().into_iter().chain([DRIVER]).collect();
    let fixture = product_fixture(entries, "tmt-squad", &files);
    assert!(acquire(&fixture, Product::Squad).is_err());
}
