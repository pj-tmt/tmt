use super::{ProviderEnvironment, SkillState, SkillTarget, plan_core, publish_core};
use crate::{drivers::Registry, test_support::TestDirectory};
use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

fn fixture() -> (TestDirectory, ProviderEnvironment, PathBuf, PathBuf) {
    let directory = TestDirectory::new();
    let home = directory.path.join("home");
    let cwd = directory.path.join("cwd");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&cwd).unwrap();
    let global = directory.path.join("global");
    let env = ProviderEnvironment::from_parts(home.clone(), cwd, Vec::new(), []);
    let root = home.join(".claude/skills");
    fs::create_dir_all(&root).unwrap();
    (directory, env, global, root)
}

fn old_skill(directory: &TestDirectory, name: &str, declared: &str) -> PathBuf {
    let source = directory
        .path
        .join("old-home/skill-assets/abc123")
        .join(name);
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {declared}\ndescription: x\n---\nold\n"),
    )
    .unwrap();
    source
}

fn state(env: &ProviderEnvironment, global: &Path, name: &str) -> SkillState {
    let claude = Registry::builtin().find("claude").unwrap();
    plan_core(env, global, claude)
        .unwrap()
        .into_iter()
        .find(|skill| skill.name == name)
        .unwrap()
        .state
}

#[test]
fn a_link_into_another_tmt_home_is_foreign_and_anything_else_is_occupied() {
    let (directory, env, global, root) = fixture();
    let source = old_skill(&directory, "tmt", "tmt");
    symlink(&source, root.join("tmt")).unwrap();
    assert_eq!(state(&env, &global, "tmt"), SkillState::Foreign { source });
    // Same layout, but the skill says it is something else: a user's.
    let renamed = old_skill(&directory, "tmt-inbox", "my-inbox");
    symlink(&renamed, root.join("tmt-inbox")).unwrap();
    assert_eq!(state(&env, &global, "tmt-inbox"), SkillState::Occupied);
    fs::remove_file(root.join("tmt-inbox")).unwrap();
    // A plain directory and a link elsewhere are the user's too.
    fs::create_dir(root.join("tmt-inbox")).unwrap();
    assert_eq!(state(&env, &global, "tmt-inbox"), SkillState::Occupied);
    fs::remove_dir(root.join("tmt-inbox")).unwrap();
    let elsewhere = directory.path.join("mine/tmt-inbox");
    fs::create_dir_all(&elsewhere).unwrap();
    symlink(&elsewhere, root.join("tmt-inbox")).unwrap();
    assert_eq!(state(&env, &global, "tmt-inbox"), SkillState::Occupied);
}

#[test]
fn a_dangling_link_into_another_tmt_home_is_still_foreign() {
    let (directory, env, global, root) = fixture();
    let gone = directory.path.join("old-home/skill-assets/abc123/tmt");
    symlink(&gone, root.join("tmt")).unwrap();
    assert_eq!(
        state(&env, &global, "tmt"),
        SkillState::Foreign { source: gone }
    );
}

#[test]
fn publication_touches_only_planned_changes_and_skips_what_changed_since() {
    let (directory, env, global, root) = fixture();
    let source = old_skill(&directory, "tmt", "tmt");
    symlink(&source, root.join("tmt")).unwrap();
    let claude = Registry::builtin().find("claude").unwrap();
    let planned: Vec<SkillTarget> = plan_core(&env, &global, claude).unwrap();
    // After planning, someone puts their own skill where one was missing.
    fs::create_dir(root.join("tmt-inbox")).unwrap();
    fs::write(root.join("tmt-inbox/SKILL.md"), "mine").unwrap();
    let published = publish_core(&global, &planned).unwrap();
    assert_eq!(published.linked, [root.join("tmt")]);
    assert_eq!(published.skipped, [root.join("tmt-inbox")]);
    assert_eq!(published.backups.len(), 1);
    assert_eq!(fs::read_link(&published.backups[0]).unwrap(), source);
    assert_eq!(
        fs::read_to_string(root.join("tmt-inbox/SKILL.md")).unwrap(),
        "mine"
    );
    assert_eq!(state(&env, &global, "tmt"), SkillState::Current);
    // An occupied plan entry is never published.
    let kept = SkillTarget {
        name: "tmt-inbox".into(),
        target: root.join("tmt-inbox"),
        state: SkillState::Occupied,
    };
    assert_eq!(
        publish_core(&global, &[kept]).unwrap().linked,
        Vec::<PathBuf>::new()
    );
}
