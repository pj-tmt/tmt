use super::files;
use crate::test_support::TestDirectory;
use std::{fs, path::PathBuf};

#[test]
fn safe_target_rejects_exact_ancestor_descendant_and_symlink_aliases() {
    let root = TestDirectory::new();
    let source = root.path.join("source");
    fs::create_dir_all(source.join("..nested")).unwrap();
    fs::write(source.join("SKILL.md"), b"canonical").unwrap();

    for target in [
        source.clone(),
        source.join("child/target"),
        source.parent().unwrap().to_path_buf(),
        source.join("..nested/target"),
    ] {
        let error = files::safe_target(&source, &target).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("Skill target overlaps bundled source: {}", target.display())
        );
    }

    let alias = root.path.join("source-alias");
    std::os::unix::fs::symlink(source.parent().unwrap(), &alias).unwrap();
    let target = alias.join("source/target");
    let error = files::safe_target(&source, &target).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Skill target overlaps bundled source: {}", target.display())
    );
}

#[test]
fn safe_target_allows_dot_dot_lookalike_sibling_and_unrelated_paths() {
    let root = TestDirectory::new();
    let source = root.path.join("source");
    fs::create_dir_all(&source).unwrap();
    let lookalike = root.path.join("source..nested/target");
    let unrelated = root.path.join("other/target");
    assert!(files::safe_target(&source, &lookalike).is_ok());
    assert!(files::safe_target(&source, &unrelated).is_ok());
}

#[test]
fn lock_is_exclusive_and_releases_after_guard_drop() {
    let root = TestDirectory::new();
    let first = files::lock(&root.path).unwrap();
    let second = match files::lock(&root.path) {
        Ok(_) => panic!("a second installer acquired the lock"),
        Err(error) => error,
    };
    assert_eq!(second.kind(), std::io::ErrorKind::WouldBlock);
    drop(first);
    let third = files::lock(&root.path).unwrap();
    drop(third);
}

#[test]
fn backup_preserves_regular_directory_and_broken_symlink_entries() {
    let root = TestDirectory::new();
    let target = root.path.join("skills/tmt");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("user.md"), b"user").unwrap();
    let backup = files::backup(&target).unwrap();
    assert!(!target.exists());
    assert_eq!(fs::read(backup.join("user.md")).unwrap(), b"user");
    assert!(backup.starts_with(root.path.join(".tmt-skill-backups")));

    let broken = root.path.join("skills/broken");
    std::os::unix::fs::symlink(root.path.join("missing"), &broken).unwrap();
    let broken_backup = files::backup(&broken).unwrap();
    assert_eq!(
        fs::read_link(broken_backup).unwrap(),
        root.path.join("missing")
    );
}

#[test]
fn atomic_write_replaces_manifest_without_leaving_staging_files() {
    let root = TestDirectory::new();
    let destination = root.path.join("nested/manifest.json");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, br#"{"version":0}"#).unwrap();
    files::atomic_write(&destination, br#"{"version":1}"#).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), br#"{"version":1}"#);
    let entries = fs::read_dir(destination.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec![PathBuf::from("manifest.json")]);
}
