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
        bundled_skill_named("tmt").unwrap()
    );
    assert_eq!(
        fs::read(source.parent().unwrap().join("tmt-inbox/SKILL.md")).unwrap(),
        bundled_skill_named("tmt-inbox").unwrap()
    );
    assert_eq!(fs::read_dir(source.parent().unwrap()).unwrap().count(), 2);
    assert!(bundled_skill_named("tmt-office").is_none());
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
        bundled_skill_named("tmt").unwrap(),
    )
    .unwrap();
    assert!(!assets.owns(&external));
    fs::remove_file(source.join("SKILL.md")).unwrap();
    symlink(external.join("SKILL.md"), source.join("SKILL.md")).unwrap();
    assert!(!assets.owns(&source));
    assert!(assets.materialize().is_err());
    assert_eq!(
        fs::read(external.join("SKILL.md")).unwrap(),
        bundled_skill_named("tmt").unwrap()
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
        fs::read(outside.join("tmt/SKILL.md")).unwrap(),
        bundled_skill_named("tmt").unwrap()
    );
    assert_eq!(fs::read_link(version).unwrap(), outside);
}

#[test]
fn missing_wrong_type_or_symlinked_bundle_sibling_never_establishes_ownership() {
    for corruption in ["missing", "file", "symlink"] {
        let root = TestDirectory::new();
        let assets = SkillAssets::new(&root.path);
        let source = assets.materialize().unwrap();
        let sibling = source.parent().unwrap().join("tmt-inbox");
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

#[test]
fn dangling_generations_require_known_names_inside_the_canonical_store() {
    let root = TestDirectory::new();
    let assets = SkillAssets::new(&root.path);
    let version = assets.root().join("a".repeat(64));
    let source = version.join("tmt");
    assert!(assets.owns(&source));
    assert!(!assets.owns(&version.join("unknown-user-skill")));
    let target = root.path.join("skills/tmt-inbox");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&source, &target).unwrap();
    assert!(super::managed_link(&target, &assets).unwrap().is_none());
    fs::create_dir(assets.root()).unwrap();
    let outside = root.path.join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, &version).unwrap();
    assert!(!assets.owns(&source));
}

#[test]
fn default_uninstall_preserves_retired_office_links_sources_and_owner_records() {
    use super::{OwnedSkill, ProviderEnvironment, install, install_owned, registry, uninstall};
    let temp = TestDirectory::new();
    let global = temp.path.join("global");
    let home = temp.path.join("home");
    let env = ProviderEnvironment::from_parts(&home, &temp.path, Vec::new(), []);
    install(&env, &global, None, None, false).unwrap();
    let skill = OwnedSkill {
        name: "tmt-office".into(),
        files: vec![("SKILL.md".into(), b"Retained installed guidance".to_vec())],
    };
    install_owned(&env, &global, "office", &[skill], true).unwrap();
    let target = home.join(".agents/skills/tmt-office");
    let source = fs::read_link(&target).unwrap();
    let record = fs::read(global.join("skill-owners.json")).unwrap();
    let plan = super::plan_uninstall(&env, &global).unwrap();
    assert!(!plan.removed.contains(&target));
    assert!(plan.kept.contains(&target));
    let report = uninstall(&env, &global).unwrap();
    assert!(report.kept.contains(&target));
    assert_eq!(fs::read_link(&target).unwrap(), source);
    assert_eq!(
        fs::read(source.join("SKILL.md")).unwrap(),
        b"Retained installed guidance"
    );
    assert_eq!(fs::read(global.join("skill-owners.json")).unwrap(), record);
    assert!(!home.join(".agents/skills/tmt").exists());
    assert!(
        registry::read(&global)
            .unwrap()
            .iter()
            .all(|target| target.file_name().unwrap() != "tmt")
    );
}

#[test]
fn default_uninstall_preserves_historical_bundle_sources_even_without_intents() {
    use super::{ProviderEnvironment, install, uninstall};
    let temp = TestDirectory::new();
    let global = temp.path.join("global");
    let home = temp.path.join("home");
    let env = ProviderEnvironment::from_parts(&home, &temp.path, Vec::new(), []);
    install(&env, &global, None, None, false).unwrap();
    let historical = global
        .join("skill-assets")
        .join("a".repeat(64))
        .join("tmt-office");
    fs::create_dir_all(&historical).unwrap();
    fs::write(historical.join("SKILL.md"), b"Edited retained source").unwrap();
    let target = home.join(".agents/skills/tmt-office");
    std::os::unix::fs::symlink(&historical, &target).unwrap();
    let report = uninstall(&env, &global).unwrap();
    assert!(report.kept.contains(&target));
    assert_eq!(fs::read_link(&target).unwrap(), historical);
    assert_eq!(
        fs::read(target.join("SKILL.md")).unwrap(),
        b"Edited retained source"
    );
    assert!(!home.join(".agents/skills/tmt").exists());
}
