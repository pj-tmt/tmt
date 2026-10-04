//! Optional cross-extension uses (#1575) through the real archive, receipt and
//! inspection path: a release may carry `TMT-USES.json`; nothing installs the
//! extension it names.

use super::*;
use crate::native_install::{
    Affected, CheckError, Product, Unavailable, affected, check_use, declared_uses,
    inspect_product, uninstall_extension, use_statuses,
};
use std::path::Path;
use tmt_core::native_install::PinAction;

const COLAB_USES: &str = r#"{"version":1,"uses":[{"feature":"browser-access","label":"Browser access","extension":"remote","requires":">=0.1.0-alpha.1"}]}"#;

/// An extension release of `product` at `version`. `uses` is carried (and
/// declared in the manifest) when given; `declared` overrides the manifest side.
fn extension(
    product: Product,
    version: &str,
    uses: Option<(&str, u32)>,
    declared: Option<bool>,
) -> Fixture {
    let root = format!("tmux-team-{version}-aarch64-apple-darwin");
    let mut entries = valid_entries(&root);
    entries[0] = Entry::File {
        path: format!("{root}/{}", product.executable()),
        bytes: format!("{} {version}\n", product.as_str()).into_bytes(),
        mode: 0o755,
    };
    let mut files = product.files().to_vec();
    if let Some((text, mode)) = uses {
        entries.push(Entry::File {
            path: format!("{root}/TMT-USES.json"),
            bytes: text.as_bytes().to_vec(),
            mode,
        });
    }
    if declared.unwrap_or(uses.is_some()) {
        files.push("TMT-USES.json");
    }
    product_fixture_at(entries, product.package(), &files, version)
}

fn install(
    product: Product,
    fixture: &Fixture,
    prefix: &Path,
) -> io::Result<super::super::InstallReport> {
    super::super::install_product(
        product,
        super::super::InstallRequest {
            archive: &fixture.archive,
            manifest: &fixture.manifest,
            prefix,
            target: TARGET,
            channel: tmt_core::native_install::Channel::Alpha,
            pin: PinAction::Preserve,
        },
        None,
        || Ok(()),
    )
}

fn colab(prefix: &Path) -> super::super::InstallReport {
    let fixture = extension(
        Product::Colab,
        "0.1.0-alpha.1",
        Some((COLAB_USES, 0o644)),
        None,
    );
    install(Product::Colab, &fixture, prefix).unwrap()
}

fn remote(prefix: &Path, version: &str) {
    let fixture = extension(Product::Remote, version, None, None);
    install(Product::Remote, &fixture, prefix).unwrap();
}

fn prefix() -> (crate::test_support::TestDirectory, std::path::PathBuf) {
    let directory = crate::test_support::TestDirectory::new();
    let prefix = directory.path.join("prefix");
    (directory, prefix)
}

#[test]
fn a_release_carrying_the_file_publishes_records_and_inspects_it() {
    let (_directory, prefix) = prefix();
    let report = colab(&prefix);
    let release = report.active_executable.parent().unwrap().to_path_buf();
    assert_eq!(
        fs::read_to_string(release.join("TMT-USES.json")).unwrap(),
        COLAB_USES
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(release.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(
        receipt["file_sha256"]["TMT-USES.json"],
        artifact::digest(COLAB_USES.as_bytes())
    );
    inspect_product(Product::Colab, &report.executable).unwrap();
    let uses = declared_uses(Product::Colab, &prefix).unwrap();
    assert_eq!(uses.len(), 1);
    assert_eq!(uses[0].feature, "browser-access");
}

#[test]
fn a_release_without_the_file_declares_nothing_and_still_verifies() {
    let (_directory, prefix) = prefix();
    remote(&prefix, "0.1.0-alpha.1");
    assert!(declared_uses(Product::Remote, &prefix).unwrap().is_empty());
    assert!(use_statuses(Product::Remote, &prefix).unwrap().is_empty());
}

#[test]
fn a_malformed_or_inconsistent_file_rejects_the_release_before_publication() {
    type Case<'a> = (&'a str, Option<(&'a str, u32)>, Option<bool>);
    let cases: [Case; 5] = [
        ("malformed JSON", Some(("nope", 0o644)), None),
        ("executable mode", Some((COLAB_USES, 0o755)), None),
        (
            "carried but not declared",
            Some((COLAB_USES, 0o644)),
            Some(false),
        ),
        ("declared but not carried", None, Some(true)),
        (
            "names itself",
            Some((&COLAB_USES.replace("remote", "colab"), 0o644)),
            None,
        ),
    ];
    for (name, uses, declared) in cases {
        let (_directory, prefix) = prefix();
        let fixture = extension(Product::Colab, "0.1.0-alpha.1", uses, declared);
        assert!(
            install(Product::Colab, &fixture, &prefix).is_err(),
            "{name} installed"
        );
        assert!(
            !prefix.join("lib/tmt-colab").exists(),
            "{name} left a release"
        );
    }
}

#[test]
fn only_extensions_may_carry_the_file() {
    let (_directory, prefix) = prefix();
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut entries = valid_entries(root);
    entries.push(Entry::File {
        path: format!("{root}/TMT-USES.json"),
        bytes: COLAB_USES.as_bytes().to_vec(),
        mode: 0o644,
    });
    let mut files = FILES.to_vec();
    files.push("TMT-USES.json");
    let fixture = product_fixture(entries, "tmt-cli", &files);
    assert!(install_fixture(&fixture, &prefix, Product::Cli).is_err());
}

#[test]
fn a_changed_file_fails_inspection_closed() {
    let (_directory, prefix) = prefix();
    let report = colab(&prefix);
    let release = report.active_executable.parent().unwrap();
    fs::write(
        release.join("TMT-USES.json"),
        COLAB_USES.replace("alpha.1", "alpha.9"),
    )
    .unwrap();
    assert!(inspect_product(Product::Colab, &report.executable).is_err());
    assert!(declared_uses(Product::Colab, &prefix).is_err());
}

#[test]
fn availability_follows_the_named_extension_and_never_installs_it() {
    let (_directory, prefix) = prefix();
    colab(&prefix);
    let status = || {
        let found = check_use(&prefix, Product::Colab, "browser-access").unwrap();
        (
            found.unavailable,
            found.installed.as_ref().map(|v| v.to_string()),
            found.hint(None),
        )
    };
    let (reason, installed, hint) = status();
    assert_eq!((reason, installed), (Some(Unavailable::Missing), None));
    assert_eq!(
        hint,
        "Browser access needs the Remote extension: tmt extension install remote --yes"
    );
    assert!(
        !prefix.join("lib/tmt-remote").exists(),
        "nothing was installed"
    );

    remote(&prefix, "0.1.0-alpha.0");
    let (reason, installed, hint) = status();
    assert_eq!(
        (reason, installed.as_deref()),
        (Some(Unavailable::TooOld), Some("0.1.0-alpha.0"))
    );
    assert!(hint.ends_with("tmt extension upgrade remote --yes"));

    remote(&prefix, "0.1.0-alpha.2");
    let (reason, installed, hint) = status();
    assert_eq!(
        (reason, installed.as_deref(), hint.as_str()),
        (None, Some("0.1.0-alpha.2"), "")
    );
    let listed = use_statuses(Product::Colab, &prefix).unwrap();
    assert!(listed.len() == 1 && listed[0].available());

    // Changed content in the named extension's release is damage, not absence.
    let release = fs::canonicalize(prefix.join("lib/tmt-remote/current")).unwrap();
    fs::write(release.join("LICENSE"), "edited").unwrap();
    let (reason, installed, hint) = status();
    assert_eq!((reason, installed), (Some(Unavailable::Damaged), None));
    assert!(hint.contains("tmt extension ls"));
}

#[test]
fn a_removed_extension_reads_as_missing_again() {
    let (_directory, prefix) = prefix();
    colab(&prefix);
    remote(&prefix, "0.1.0-alpha.1");
    assert!(
        check_use(&prefix, Product::Colab, "browser-access")
            .unwrap()
            .available()
    );
    uninstall_extension(&prefix, Product::Remote).unwrap();
    let found = check_use(&prefix, Product::Colab, "browser-access").unwrap();
    assert_eq!(found.unavailable, Some(Unavailable::Missing));
}

#[test]
fn an_unmanaged_command_is_not_an_installation() {
    let (_directory, prefix) = prefix();
    colab(&prefix);
    fs::write(prefix.join("bin/tmt-remote"), "#!/bin/sh\n").unwrap();
    let found = check_use(&prefix, Product::Colab, "browser-access").unwrap();
    assert_eq!(found.unavailable, Some(Unavailable::Missing));
}

#[test]
fn undeclared_callers_and_features_are_errors_not_answers() {
    let (_directory, prefix) = prefix();
    assert!(matches!(
        check_use(&prefix, Product::Colab, "browser-access"),
        Err(CheckError::Undeclared(_))
    ));
    colab(&prefix);
    assert!(matches!(
        check_use(&prefix, Product::Colab, "other-feature"),
        Err(CheckError::Undeclared(_))
    ));
    remote(&prefix, "0.1.0-alpha.1");
    assert!(matches!(
        check_use(&prefix, Product::Remote, "browser-access"),
        Err(CheckError::Undeclared(_))
    ));
}

#[test]
fn removal_and_exact_versions_name_only_features_that_stop_working() {
    let (_directory, prefix) = prefix();
    colab(&prefix);
    // Nothing works through Remote yet, so nothing is affected.
    assert!(affected(&prefix, Product::Remote, None).is_empty());
    remote(&prefix, "0.1.0-alpha.2");
    let expected = vec![Affected {
        extension: Product::Colab,
        feature: "browser-access".into(),
        label: "Browser access".into(),
    }];
    assert_eq!(affected(&prefix, Product::Remote, None), expected);
    let lower = semver::Version::parse("0.1.0-alpha.0").unwrap();
    let same = semver::Version::parse("0.1.0-alpha.1").unwrap();
    assert_eq!(affected(&prefix, Product::Remote, Some(&lower)), expected);
    assert!(affected(&prefix, Product::Remote, Some(&same)).is_empty());
    // Squad is not used by anything.
    assert!(affected(&prefix, Product::Squad, None).is_empty());
}
