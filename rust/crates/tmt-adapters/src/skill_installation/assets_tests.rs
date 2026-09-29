use super::{assets::SkillAssets, bundled_skill_named};
use crate::test_support::TestDirectory;
use std::fs;

#[test]
fn materialized_source_is_exact_and_repeated_install_preserves_it() {
    let root = TestDirectory::new();
    let assets = SkillAssets::new(&root.path);
    let source = assets.materialize().unwrap();
    assert_eq!(
        fs::read(source.join("SKILL.md")).unwrap(),
        bundled_skill_named("tmux-team").unwrap()
    );
    assert_eq!(
        fs::read(source.parent().unwrap().join("tmt-inbox/SKILL.md")).unwrap(),
        bundled_skill_named("tmt-inbox").unwrap()
    );
    assert_eq!(
        fs::read(source.parent().unwrap().join("tmt-office/SKILL.md")).unwrap(),
        bundled_skill_named("tmt-office").unwrap()
    );
    assert_eq!(
        fs::read(source.parent().unwrap().join("tmt-prop-create/SKILL.md")).unwrap(),
        bundled_skill_named("tmt-prop-create").unwrap()
    );
    assert_eq!(
        fs::read(source.parent().unwrap().join("tmt-avatar-create/SKILL.md")).unwrap(),
        bundled_skill_named("tmt-avatar-create").unwrap()
    );
    assert!(assets.owns(&source));
    assert_eq!(assets.materialize().unwrap(), source);
    assert_eq!(
        fs::read_dir(root.path.join("skill-assets"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn modified_or_extended_source_is_neither_overwritten_nor_trusted() {
    for extra in [false, true] {
        let root = TestDirectory::new();
        let assets = SkillAssets::new(&root.path);
        let source = assets.materialize().unwrap();
        let changed = source.join(if extra { "unexpected.md" } else { "SKILL.md" });
        fs::write(&changed, b"user-owned content").unwrap();
        assert!(!assets.owns(&source));
        assert!(assets.materialize().is_err());
        assert_eq!(fs::read(changed).unwrap(), b"user-owned content");
        assert_eq!(
            fs::read_dir(root.path.join("skill-assets"))
                .unwrap()
                .count(),
            1
        );
    }
}

#[test]
fn occupied_digest_path_is_preserved_and_failed_staging_is_removed() {
    let root = TestDirectory::new();
    let assets = SkillAssets::new(&root.path);
    let occupied = assets.source().parent().unwrap().to_path_buf();
    fs::create_dir_all(occupied.parent().unwrap()).unwrap();
    fs::write(&occupied, b"unrelated file").unwrap();
    assert!(assets.materialize().is_err());
    assert_eq!(fs::read(&occupied).unwrap(), b"unrelated file");
    assert_eq!(fs::read_dir(occupied.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn external_source_and_symlinked_skill_file_never_establish_managed_ownership() {
    use std::os::unix::fs::symlink;
    let root = TestDirectory::new();
    let assets = SkillAssets::new(&root.path);
    let source = assets.materialize().unwrap();
    let external = root.path.join("outside");
    fs::create_dir(&external).unwrap();
    fs::write(
        external.join("SKILL.md"),
        bundled_skill_named("tmux-team").unwrap(),
    )
    .unwrap();
    assert!(!assets.owns(&external));
    fs::remove_file(source.join("SKILL.md")).unwrap();
    symlink(external.join("SKILL.md"), source.join("SKILL.md")).unwrap();
    assert!(!assets.owns(&source));
    assert!(assets.materialize().is_err());
    assert_eq!(
        fs::read(external.join("SKILL.md")).unwrap(),
        bundled_skill_named("tmux-team").unwrap()
    );
}

#[test]
fn symlinked_digest_directory_is_not_a_managed_source() {
    let root = TestDirectory::new();
    let assets = SkillAssets::new(&root.path);
    let source = assets.materialize().unwrap();
    let version = source.parent().unwrap();
    let outside = root.path.join("external-version");
    fs::rename(version, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, version).unwrap();
    assert!(!assets.owns(&source));
    assert!(assets.materialize().is_err());
    assert_eq!(
        fs::read(outside.join("tmux-team/SKILL.md")).unwrap(),
        bundled_skill_named("tmux-team").unwrap()
    );
    assert_eq!(fs::read_link(version).unwrap(), outside);
}

#[test]
fn missing_wrong_type_or_symlinked_bundle_sibling_never_establishes_ownership() {
    for corruption in ["missing", "file", "symlink"] {
        let root = TestDirectory::new();
        let assets = SkillAssets::new(&root.path);
        let source = assets.materialize().unwrap();
        let sibling = source.parent().unwrap().join(if corruption == "missing" {
            "tmt-office"
        } else {
            "tmt-inbox"
        });
        fs::remove_dir_all(&sibling).unwrap();
        match corruption {
            "missing" => {}
            "file" => fs::write(&sibling, b"not a skill directory").unwrap(),
            "symlink" => {
                let outside = root.path.join("unsafe-inbox-sibling");
                fs::create_dir(&outside).unwrap();
                fs::write(outside.join("SKILL.md"), b"unsafe sibling").unwrap();
                std::os::unix::fs::symlink(outside, &sibling).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(!assets.owns(&source));
    }
}
