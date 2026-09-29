//! An extension release's skills tree is installed and re-verified with the
//! same integrity as its binaries, and any violation rejects the whole release.

use super::*;
use crate::native_install::{Product, inspect_product, uninstall_extension};
use std::path::Path;
use tmt_core::native_install::PinAction;

const SKILL: &[u8] = b"---\nname: tmt-squad\n---\nLead a squad.\n";

fn skill_entries(root: &str, files: &[(&str, &[u8])]) -> Vec<Entry> {
    files
        .iter()
        .map(|(path, bytes)| Entry::File {
            path: format!("{root}/{path}"),
            bytes: bytes.to_vec(),
            mode: 0o644,
        })
        .collect()
}

/// A Squad release whose archive holds `archived` and whose manifest lists
/// the required files plus `listed`. cargo-dist declares a skills tree as the
/// single asset `skills`.
fn squad_release(archived: Vec<Entry>, listed: &[&str]) -> Fixture {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let mut entries = valid_entries(root);
    entries[0] = Entry::File {
        path: format!("{root}/tmt-squad"),
        bytes: b"squad\n".to_vec(),
        mode: 0o755,
    };
    entries.extend(archived);
    let files = Product::Squad
        .files()
        .into_iter()
        .chain(listed.iter().copied())
        .collect::<Vec<_>>();
    product_fixture_at(entries, "tmt-squad", &files, "1.2.3")
}

fn with_skills() -> Fixture {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let files: [(&str, &[u8]); 2] = [
        ("skills/tmt-squad/SKILL.md", SKILL),
        ("skills/tmt-squad/references/usage.md", b"usage\n"),
    ];
    squad_release(skill_entries(root, &files), &["skills"])
}

fn install(fixture: &Fixture, prefix: &Path) -> io::Result<super::super::InstallReport> {
    install_on(fixture, prefix, tmt_core::native_install::Channel::Stable)
}

fn install_on(
    fixture: &Fixture,
    prefix: &Path,
    channel: tmt_core::native_install::Channel,
) -> io::Result<super::super::InstallReport> {
    super::super::install_product(
        Product::Squad,
        super::super::InstallRequest {
            archive: &fixture.archive,
            manifest: &fixture.manifest,
            prefix,
            target: TARGET,
            channel,
            pin: PinAction::Preserve,
        },
        None,
        || Ok(()),
    )
}

fn release_dir(report: &super::super::InstallReport) -> PathBuf {
    report.active_executable.parent().unwrap().to_path_buf()
}

#[test]
fn a_skills_tree_is_installed_recorded_and_reverified_with_the_release() {
    let fixture = with_skills();
    let prefix = fixture.directory.path.join("prefix");
    let report = install(&fixture, &prefix).unwrap();
    let release = release_dir(&report);
    assert_eq!(
        fs::read(release.join("skills/tmt-squad/SKILL.md")).unwrap(),
        SKILL
    );
    assert_eq!(
        fs::read(release.join("skills/tmt-squad/references/usage.md")).unwrap(),
        b"usage\n"
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(release.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(
        receipt["file_sha256"]["skills/tmt-squad/SKILL.md"],
        artifact::digest(SKILL)
    );
    inspect_product(Product::Squad, &report.executable).unwrap();
    // A repeated install of the same release is a verified no-op.
    assert!(!install(&fixture, &prefix).unwrap().changed);
    assert!(uninstall_extension(&prefix, Product::Squad).unwrap());
}

#[test]
fn a_changed_extra_or_missing_skill_file_fails_closed_without_rewriting_anything() {
    type Change = fn(&Path);
    let changes: [(Change, &str); 4] = [
        (
            |release| fs::write(release.join("skills/tmt-squad/SKILL.md"), b"tampered").unwrap(),
            "has changed",
        ),
        (
            |release| fs::write(release.join("skills/tmt-squad/extra.md"), b"x").unwrap(),
            "inventory has changed",
        ),
        (
            |release| {
                fs::remove_file(release.join("skills/tmt-squad/references/usage.md")).unwrap()
            },
            "inventory has changed",
        ),
        (
            |release| {
                std::os::unix::fs::symlink("/etc/hosts", release.join("skills/tmt-squad/link.md"))
                    .unwrap()
            },
            "inventory has changed",
        ),
    ];
    for (change, message) in changes {
        let fixture = with_skills();
        let prefix = fixture.directory.path.join("prefix");
        let report = install(&fixture, &prefix).unwrap();
        let release = release_dir(&report);
        change(&release);
        let before = tree(&prefix);
        let error = inspect_product(Product::Squad, &report.executable).unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert!(install(&fixture, &prefix).is_err());
        assert!(uninstall_extension(&prefix, Product::Squad).is_err());
        assert_eq!(tree(&prefix), before, "nothing was rewritten or removed");
    }
}

#[test]
fn a_reader_meeting_an_unknown_release_entry_fails_closed() {
    // An older tmt meets a skills tree the way this reader meets any entry it
    // does not know: it refuses inspection, activation and removal alike.
    let fixture = squad_release(Vec::new(), &[]);
    let prefix = fixture.directory.path.join("prefix");
    let report = install(&fixture, &prefix).unwrap();
    fs::write(release_dir(&report).join("unknown.txt"), b"newer layout").unwrap();
    let before = tree(&prefix);
    let error = inspect_product(Product::Squad, &report.executable).unwrap_err();
    assert!(
        error.to_string().contains("inventory has changed"),
        "{error}"
    );
    assert!(install(&fixture, &prefix).is_err());
    assert!(uninstall_extension(&prefix, Product::Squad).is_err());
    assert_eq!(tree(&prefix), before);
}

#[test]
fn an_invalid_skills_tree_rejects_the_whole_release_before_publication() {
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let oversized = vec![b'x'; crate::skill_installation::MAXIMUM_FILE_BYTES + 1];
    let too_many = (0..=crate::skill_installation::MAXIMUM_SKILLS)
        .map(|index| format!("skills/s{index}/SKILL.md"))
        .collect::<Vec<_>>();
    let declared = vec!["skills".to_owned()];
    let cases: Vec<(&str, Vec<Entry>, Vec<String>)> = vec![
        (
            "declared but carries no skill",
            Vec::new(),
            declared.clone(),
        ),
        (
            "declared, only a directory",
            vec![Entry::Directory {
                path: format!("{root}/skills/"),
            }],
            declared.clone(),
        ),
        (
            "archived but not declared",
            skill_entries(root, &[("skills/tmt-squad/SKILL.md", SKILL)]),
            Vec::new(),
        ),
        (
            "listed file by file instead of declared",
            skill_entries(root, &[("skills/tmt-squad/SKILL.md", SKILL)]),
            vec!["skills/tmt-squad/SKILL.md".into()],
        ),
        (
            "declared twice",
            skill_entries(root, &[("skills/tmt-squad/SKILL.md", SKILL)]),
            vec!["skills".into(), "skills".into()],
        ),
        (
            "a link where a skill file should be",
            vec![Entry::Symlink {
                path: format!("{root}/skills/tmt-squad/SKILL.md"),
                target: "/etc/passwd".into(),
            }],
            declared.clone(),
        ),
        (
            "a non-canonical path",
            [
                skill_entries(root, &[("skills/tmt-squad/SKILL.md", SKILL)]),
                vec![Entry::AdversarialFile {
                    path: format!("{root}/skills/tmt-squad//x.md"),
                    bytes: b"x".to_vec(),
                    mode: 0o644,
                }],
            ]
            .into_iter()
            .flatten()
            .collect(),
            declared.clone(),
        ),
        (
            "an empty directory beside the tree",
            [
                skill_entries(root, &[("skills/tmt-squad/SKILL.md", SKILL)]),
                vec![Entry::Directory {
                    path: format!("{root}/skills/empty/"),
                }],
            ]
            .into_iter()
            .flatten()
            .collect(),
            declared.clone(),
        ),
        (
            "a regular file where the tree directory should be",
            [
                vec![Entry::File {
                    path: format!("{root}/skills"),
                    bytes: b"not a directory".to_vec(),
                    mode: 0o644,
                }],
                skill_entries(root, &[("skills/tmt-squad/SKILL.md", SKILL)]),
            ]
            .into_iter()
            .flatten()
            .collect(),
            declared.clone(),
        ),
        (
            "missing SKILL.md",
            skill_entries(root, &[("skills/tmt-squad/README.md", b"r")]),
            declared.clone(),
        ),
        (
            "an oversized file",
            skill_entries(root, &[("skills/tmt-squad/SKILL.md", &oversized)]),
            declared.clone(),
        ),
        (
            "too many skills",
            too_many
                .iter()
                .map(|path| Entry::File {
                    path: format!("{root}/{path}"),
                    bytes: SKILL.to_vec(),
                    mode: 0o644,
                })
                .collect(),
            declared.clone(),
        ),
    ];
    for (case, archived, listed) in cases {
        let listed_refs = listed.iter().map(String::as_str).collect::<Vec<_>>();
        let fixture = squad_release(archived, &listed_refs);
        let prefix = fixture.directory.path.join("prefix");
        assert!(install(&fixture, &prefix).is_err(), "{case}");
        assert!(!prefix.exists(), "nothing published for {case}");
    }
    // The CLI never carries skills, even when cargo-dist would declare them.
    let mut cli = valid_entries(root);
    cli.extend(skill_entries(root, &[("skills/tmux-team/SKILL.md", SKILL)]));
    let files = FILES.into_iter().chain(["skills"]).collect::<Vec<_>>();
    let fixture = product_fixture_at(cli, "tmt-cli", &files, "1.2.3");
    let prefix = fixture.directory.path.join("prefix");
    let error = install_fixture(&fixture, &prefix, Product::Cli).unwrap_err();
    assert!(error.to_string().contains("no agent skills"), "{error}");
    assert!(!prefix.exists());
}

#[test]
fn archived_skill_files_are_bounded_while_decoding() {
    // One more file than the tree may hold is refused before it is kept,
    // whatever the per-skill layout.
    let root = "tmux-team-1.2.3-aarch64-apple-darwin";
    let limit = super::super::skills_tree::MAXIMUM_TREE_FILES;
    let archived = (0..=limit)
        .map(|index| Entry::File {
            path: format!("{root}/skills/s{}/f{index}.md", index % 3),
            bytes: b"x".to_vec(),
            mode: 0o644,
        })
        .collect();
    let fixture = squad_release(archived, &["skills"]);
    let prefix = fixture.directory.path.join("prefix");
    let error = install(&fixture, &prefix).unwrap_err();
    assert!(
        error.to_string().contains("too many agent skill files"),
        "{error}"
    );
}

#[test]
fn receipts_are_bounded_per_product() {
    let fixture = with_skills();
    let prefix = fixture.directory.path.join("prefix");
    let report = install(&fixture, &prefix).unwrap();
    let receipt = release_dir(&report).join("receipt.json");
    let original = fs::read(&receipt).unwrap();
    // Whitespace keeps the document valid; only its size changes.
    let pad = |size: usize| {
        let mut padded = original.clone();
        padded.resize(size, b' ');
        fs::write(&receipt, &padded).unwrap();
    };
    pad(super::super::skills_tree::CLI_RECEIPT_BYTES + 1);
    inspect_product(Product::Squad, &report.executable).unwrap();
    pad(super::super::skills_tree::receipt_limit(Product::Squad) + 1);
    assert!(inspect_product(Product::Squad, &report.executable).is_err());

    let cli = fixture_cli();
    let cli_prefix = cli.directory.path.join("prefix");
    let cli_report = install_fixture(&cli, &cli_prefix, Product::Cli).unwrap();
    let cli_receipt = release_dir(&cli_report).join("receipt.json");
    let mut padded = fs::read(&cli_receipt).unwrap();
    padded.resize(super::super::skills_tree::CLI_RECEIPT_BYTES + 1, b' ');
    fs::write(&cli_receipt, &padded).unwrap();
    assert!(crate::native_install::inspect(&cli_report.executable).is_err());
}

fn fixture_cli() -> Fixture {
    fixture(valid_entries("tmux-team-1.2.3-aarch64-apple-darwin"))
}

/// Every path under `root` with its bytes (or link target), for "nothing
/// changed" assertions.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).unwrap();
        if metadata.file_type().is_symlink() {
            found.insert(
                path.clone(),
                fs::read_link(&path)
                    .unwrap()
                    .into_os_string()
                    .into_encoded_bytes(),
            );
        } else if metadata.is_dir() {
            found.insert(path.clone(), Vec::new());
            pending.extend(
                fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        } else {
            found.insert(path.clone(), fs::read(&path).unwrap());
        }
    }
    found
}

/// The manifest cargo-dist 0.32.0 wrote for a local Squad archive
/// (`dist build --artifacts local --target aarch64-apple-darwin --tag
/// tmt-squad-v0.1.0-alpha.1 --output-format=json --no-local-paths`, with the
/// skills directory included). It declares the whole tree as one `skills`
/// asset; only the archive checksum is replaced below.
const CARGO_DIST_MANIFEST: &str = include_str!("fixtures/squad-cargo-dist-0.32.0-manifest.json");

#[test]
fn a_real_cargo_dist_squad_manifest_installs_its_archived_skills_tree() {
    let name = "tmt-squad-aarch64-apple-darwin.tar.gz";
    let root = "tmt-squad-aarch64-apple-darwin";
    // The same records, in the same order and form, as that archive: the
    // root directory with a slash, tree directories without one.
    let archived = vec![
        Entry::Directory {
            path: format!("{root}/"),
        },
        Entry::File {
            path: format!("{root}/tmt-squad"),
            bytes: b"squad\n".to_vec(),
            mode: 0o755,
        },
        Entry::Directory {
            path: format!("{root}/skills"),
        },
        Entry::Directory {
            path: format!("{root}/skills/tmt-squad"),
        },
        Entry::File {
            path: format!("{root}/skills/tmt-squad/SKILL.md"),
            bytes: SKILL.to_vec(),
            mode: 0o644,
        },
        Entry::File {
            path: format!("{root}/THIRD-PARTY-NOTICES.txt"),
            bytes: b"notices\n".to_vec(),
            mode: 0o644,
        },
        Entry::File {
            path: format!("{root}/LICENSE"),
            bytes: b"license\n".to_vec(),
            mode: 0o644,
        },
        Entry::File {
            path: format!("{root}/NATIVE-INSTALL.md"),
            bytes: b"guide\n".to_vec(),
            mode: 0o644,
        },
    ];
    let directory = TestDirectory::new();
    let compressed = gzip_tar(archived);
    let mut manifest: serde_json::Value = serde_json::from_str(CARGO_DIST_MANIFEST).unwrap();
    let assets = manifest["artifacts"][name]["assets"].as_array().unwrap();
    assert!(
        assets.iter().any(|asset| asset["path"] == "skills")
            && !assets
                .iter()
                .any(|asset| asset["path"].as_str().unwrap().starts_with("skills/")),
        "cargo-dist declares the tree as one asset"
    );
    manifest["artifacts"][name]["checksums"]["sha256"] = artifact::digest(&compressed).into();
    let fixture = Fixture {
        archive: directory.path.join(name),
        manifest: directory.path.join("dist-manifest.json"),
        name: name.into(),
        directory,
    };
    fs::write(&fixture.archive, &compressed).unwrap();
    fs::write(&fixture.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let prefix = fixture.directory.path.join("prefix");
    // Squad's first releases are alpha versions.
    let alpha = tmt_core::native_install::Channel::Alpha;
    let report = install_on(&fixture, &prefix, alpha).unwrap();
    let release = release_dir(&report);
    assert_eq!(
        fs::read(release.join("skills/tmt-squad/SKILL.md")).unwrap(),
        SKILL
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(release.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(
        receipt["file_sha256"]["skills/tmt-squad/SKILL.md"],
        artifact::digest(SKILL)
    );
    inspect_product(Product::Squad, &report.executable).unwrap();
    assert!(!install_on(&fixture, &prefix, alpha).unwrap().changed);
}
