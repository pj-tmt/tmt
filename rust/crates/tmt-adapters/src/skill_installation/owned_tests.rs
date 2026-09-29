use super::{
    OwnedSkill, ProviderEnvironment, Refusal, inspect_local_drift, install, install_office,
    install_owned, owned::validate, owners, refresh, refusal, remove_owned,
};
use crate::test_support::TestDirectory;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn fixture() -> (TestDirectory, ProviderEnvironment, PathBuf, PathBuf) {
    let directory = TestDirectory::new();
    let home = directory.path.join("home");
    let cwd = directory.path.join("cwd");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&cwd).unwrap();
    let global = directory.path.join("global");
    let env =
        ProviderEnvironment::from_parts(home.clone(), cwd, Vec::new(), None, None, None, None);
    // With no provider detected, optional skills go to the universal root.
    let root = home.join(".agents/skills");
    (directory, env, global, root)
}

fn skill(name: &str, body: &str) -> OwnedSkill {
    OwnedSkill {
        name: name.into(),
        files: vec![
            ("SKILL.md".into(), body.as_bytes().to_vec()),
            ("references/usage.md".into(), b"reference".to_vec()),
        ],
    }
}

fn read_skill(target: &Path) -> String {
    fs::read_to_string(target.join("SKILL.md")).unwrap()
}

fn claimed(error: &std::io::Error) -> Option<(String, String)> {
    match refusal(error)? {
        Refusal::Claimed { name, owner } => Some((name.clone(), owner.clone())),
        _ => None,
    }
}

#[test]
fn an_owner_publishes_repeats_as_a_no_op_and_updates_to_new_content() {
    let (_directory, env, global, root) = fixture();
    let first = install_owned(&env, &global, "squad", &[skill("tmt-squad", "v1")], false).unwrap();
    assert_eq!(first.published.len(), 1);
    assert!(first.published[0].changed);
    let target = root.join("tmt-squad");
    assert!(fs::symlink_metadata(&target).unwrap().is_symlink());
    assert_eq!(read_skill(&target), "v1");
    assert_eq!(
        fs::read_to_string(target.join("references/usage.md")).unwrap(),
        "reference"
    );
    assert_eq!(owners(&global).unwrap()["tmt-squad"], "squad");

    let again = install_owned(&env, &global, "squad", &[skill("tmt-squad", "v1")], false).unwrap();
    assert!(!again.published[0].changed, "same content is a no-op");
    let updated =
        install_owned(&env, &global, "squad", &[skill("tmt-squad", "v2")], false).unwrap();
    assert!(updated.published[0].changed);
    assert_eq!(read_skill(&target), "v2");
    assert!(inspect_local_drift(&env, &global).unwrap().is_empty());
}

#[test]
fn names_held_by_core_or_another_owner_are_refused_before_any_effect() {
    let (_directory, env, global, root) = fixture();
    let core =
        install_owned(&env, &global, "squad", &[skill("tmux-team", "x")], false).unwrap_err();
    assert_eq!(
        claimed(&core.cause),
        Some(("tmux-team".into(), "core".into()))
    );
    assert!(!global.join("skill-owners.json").exists());

    install_owned(&env, &global, "squad", &[skill("tmt-squad", "mine")], false).unwrap();
    let taken = install_owned(
        &env,
        &global,
        "other",
        &[skill("tmt-other", "b"), skill("tmt-squad", "theirs")],
        false,
    )
    .unwrap_err();
    assert_eq!(
        claimed(&taken.cause),
        Some(("tmt-squad".into(), "squad".into()))
    );
    assert!(taken.report.published.is_empty());
    assert!(
        !root.join("tmt-other").exists(),
        "nothing published on refusal"
    );
    assert_eq!(read_skill(&root.join("tmt-squad")), "mine");

    // An explicit force transfers the claim.
    install_owned(
        &env,
        &global,
        "other",
        &[skill("tmt-squad", "theirs")],
        true,
    )
    .unwrap();
    assert_eq!(owners(&global).unwrap()["tmt-squad"], "other");
    assert_eq!(read_skill(&root.join("tmt-squad")), "theirs");
}

#[test]
fn an_unmanaged_path_is_refused_and_force_backs_it_up() {
    let (_directory, env, global, root) = fixture();
    let target = root.join("tmt-squad");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("SKILL.md"), b"hand written").unwrap();
    let refused =
        install_owned(&env, &global, "squad", &[skill("tmt-squad", "v1")], false).unwrap_err();
    assert!(matches!(refusal(&refused.cause), Some(Refusal::Unmanaged(path)) if *path == target));
    assert_eq!(read_skill(&target), "hand written");

    let forced = install_owned(&env, &global, "squad", &[skill("tmt-squad", "v1")], true).unwrap();
    let backup = forced.published[0].backup.clone().expect("backup");
    assert_eq!(fs::read(backup.join("SKILL.md")).unwrap(), b"hand written");
    assert_eq!(read_skill(&target), "v1");
}

#[test]
fn office_links_published_before_owners_existed_are_adopted_without_force() {
    let (_directory, env, global, root) = fixture();
    install(&env, &global, None, None, false).unwrap();
    install_office(&env, &global, false).unwrap();
    let office = root.join("tmt-office");
    let core_link = fs::read_link(&office).unwrap();

    let adopted = install_owned(
        &env,
        &global,
        "office",
        &[
            skill("tmt-office", "office from its release"),
            skill("tmt-prop-create", "props"),
            skill("tmt-avatar-create", "avatars"),
        ],
        false,
    )
    .unwrap();
    assert_eq!(
        adopted.published.len(),
        3,
        "one target per name, no duplicates"
    );
    assert!(
        adopted
            .published
            .iter()
            .all(|item| item.changed && item.backup.is_none())
    );
    assert_ne!(fs::read_link(&office).unwrap(), core_link);
    assert_eq!(read_skill(&office), "office from its release");
    assert_eq!(owners(&global).unwrap()["tmt-prop-create"], "office");
    // Core's own skills are untouched and core refresh sees no extension names.
    assert!(
        fs::symlink_metadata(root.join("tmux-team"))
            .unwrap()
            .is_symlink()
    );
    let refreshed = refresh(&global).unwrap();
    assert!(refreshed.conflicts.is_empty());
    assert_eq!(
        refreshed.skipped.len(),
        3,
        "held names are skipped, not refreshed"
    );
    // Core's Office facade leaves the adopted names to their owner.
    let facade = install_office(&env, &global, false).unwrap();
    assert!(facade.installed.is_empty());
    assert_eq!(read_skill(&office), "office from its release");
}

#[test]
fn removal_by_owner_keeps_other_owners_and_anything_the_user_replaced() {
    let (_directory, env, global, root) = fixture();
    install_owned(
        &env,
        &global,
        "squad",
        &[skill("tmt-squad", "s"), skill("tmt-sq-play", "p")],
        false,
    )
    .unwrap();
    install_owned(&env, &global, "office", &[skill("tmt-office", "o")], false).unwrap();
    let replaced = root.join("tmt-sq-play");
    fs::remove_file(&replaced).unwrap();
    fs::create_dir(&replaced).unwrap();
    fs::write(replaced.join("SKILL.md"), b"user copy").unwrap();

    let removed = remove_owned(&global, "squad").unwrap();
    assert_eq!(removed.removed, [root.join("tmt-squad")]);
    assert_eq!(removed.kept, std::slice::from_ref(&replaced));
    assert!(!root.join("tmt-squad").exists());
    assert_eq!(read_skill(&replaced), "user copy");
    assert_eq!(read_skill(&root.join("tmt-office")), "o");
    let remaining = owners(&global).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining["tmt-office"], "office");
    assert!(
        remove_owned(&global, "squad").unwrap().removed.is_empty(),
        "repeat is a no-op"
    );
}

#[test]
fn drift_reports_an_owned_target_that_no_longer_points_at_its_content() {
    let (_directory, env, global, root) = fixture();
    install_owned(&env, &global, "squad", &[skill("tmt-squad", "v1")], false).unwrap();
    assert!(inspect_local_drift(&env, &global).unwrap().is_empty());
    let target = root.join("tmt-squad");
    fs::remove_file(&target).unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere"), &target).unwrap();
    assert_eq!(inspect_local_drift(&env, &global).unwrap(), [target]);
}

#[test]
fn owners_names_and_files_are_validated_before_anything_else() {
    let good = skill("tmt-squad", "x");
    for owner in ["", "core", "Squad", "1squad", "sq uad", &"s".repeat(33)] {
        assert!(
            matches!(
                validate(owner, std::slice::from_ref(&good)),
                Err(Refusal::Invalid(_))
            ),
            "{owner}"
        );
    }
    assert!(validate("squad", &[]).is_err());
    let mut bad_files: Vec<Vec<(String, Vec<u8>)>> = ["../escape.md", "/abs.md", ".hidden", ""]
        .iter()
        .map(|path| {
            vec![
                ("SKILL.md".to_owned(), b"x".to_vec()),
                ((*path).to_owned(), b"y".to_vec()),
            ]
        })
        .collect();
    bad_files.push(vec![("README.md".into(), b"no skill file".to_vec())]);
    bad_files.push(vec![
        ("SKILL.md".into(), b"x".to_vec()),
        ("ref".into(), b"file".to_vec()),
        ("ref/inner.md".into(), b"and dir".to_vec()),
    ]);
    bad_files.push(vec![
        ("SKILL.md".into(), b"x".to_vec()),
        ("SKILL.md".into(), b"twice".to_vec()),
    ]);
    for files in bad_files {
        let invalid = OwnedSkill {
            name: "tmt-squad".into(),
            files: files.clone(),
        };
        assert!(validate("squad", &[invalid]).is_err(), "{files:?}");
    }
    for name in ["", "Tmt", "-lead", "a b", "tmt_squad"] {
        assert!(validate("squad", &[skill(name, "x")]).is_err(), "{name}");
    }
    assert!(
        validate("squad", &[good.clone(), good]).is_err(),
        "repeated names"
    );
}
