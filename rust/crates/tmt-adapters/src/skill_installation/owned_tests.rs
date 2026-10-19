use super::{
    OwnedSkill, ProviderEnvironment, Refusal, inspect_local_drift, install_owned, owned::validate,
    owners, refusal, remove_owned,
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
    let env = ProviderEnvironment::from_parts(home.clone(), cwd, Vec::new(), []);
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
    let first = install_owned(&env, &global, "squad", &[skill("tmt-ops", "v1")], false).unwrap();
    assert_eq!(first.published.len(), 1);
    assert!(first.published[0].changed);
    let target = root.join("tmt-ops");
    assert!(fs::symlink_metadata(&target).unwrap().is_symlink());
    assert_eq!(read_skill(&target), "v1");
    assert_eq!(
        fs::read_to_string(target.join("references/usage.md")).unwrap(),
        "reference"
    );
    assert_eq!(owners(&global).unwrap()["tmt-ops"], "squad");

    let again = install_owned(&env, &global, "squad", &[skill("tmt-ops", "v1")], false).unwrap();
    assert!(!again.published[0].changed, "same content is a no-op");
    let updated = install_owned(&env, &global, "squad", &[skill("tmt-ops", "v2")], false).unwrap();
    assert!(updated.published[0].changed);
    assert_eq!(read_skill(&target), "v2");
    assert!(inspect_local_drift(&env, &global).unwrap().is_empty());
}

#[test]
fn names_held_by_core_or_another_owner_are_refused_before_any_effect() {
    let (_directory, env, global, root) = fixture();
    let core = install_owned(&env, &global, "squad", &[skill("tmt", "x")], false).unwrap_err();
    assert_eq!(claimed(&core.cause), Some(("tmt".into(), "core".into())));
    assert!(!global.join("skill-owners.json").exists());

    install_owned(&env, &global, "squad", &[skill("tmt-ops", "mine")], false).unwrap();
    let taken = install_owned(
        &env,
        &global,
        "other",
        &[skill("tmt-other", "b"), skill("tmt-ops", "theirs")],
        false,
    )
    .unwrap_err();
    assert_eq!(
        claimed(&taken.cause),
        Some(("tmt-ops".into(), "squad".into()))
    );
    assert!(taken.report.published.is_empty());
    assert!(
        !root.join("tmt-other").exists(),
        "nothing published on refusal"
    );
    assert_eq!(read_skill(&root.join("tmt-ops")), "mine");

    // An explicit force transfers the claim.
    install_owned(&env, &global, "other", &[skill("tmt-ops", "theirs")], true).unwrap();
    assert_eq!(owners(&global).unwrap()["tmt-ops"], "other");
    assert_eq!(read_skill(&root.join("tmt-ops")), "theirs");
}

#[test]
fn api_names_refuse_unmanaged_targets_and_force_preserves_a_backup() {
    for name in ["personal", "tmt-colab"] {
        let (_directory, env, global, root) = fixture();
        let target = root.join(name);
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), "user content").unwrap();
        let refused = install_owned(&env, &global, "colab", &[skill(name, "api content")], false)
            .unwrap_err();
        assert!(
            matches!(refusal(&refused.cause), Some(Refusal::Unmanaged(path)) if path == &target)
        );
        assert!(refused.report.published.is_empty());
        assert_eq!(read_skill(&target), "user content");
        assert!(!global.join("skill-owners.json").exists());
        let forced =
            install_owned(&env, &global, "colab", &[skill(name, "api content")], true).unwrap();
        let backup = forced.published[0].backup.as_ref().unwrap();
        assert_eq!(read_skill(backup), "user content");
        assert_eq!(read_skill(&target), "api content");
    }
}

#[test]
fn removal_by_owner_keeps_other_owners_and_anything_the_user_replaced() {
    let (_directory, env, global, root) = fixture();
    install_owned(
        &env,
        &global,
        "squad",
        &[skill("tmt-ops", "s"), skill("tmt-sq-play", "p")],
        false,
    )
    .unwrap();
    install_owned(&env, &global, "office", &[skill("tmt-office", "o")], false).unwrap();
    let replaced = root.join("tmt-sq-play");
    fs::remove_file(&replaced).unwrap();
    fs::create_dir(&replaced).unwrap();
    fs::write(replaced.join("SKILL.md"), b"user copy").unwrap();

    let removed = remove_owned(&global, "squad", None).unwrap();
    assert_eq!(removed.removed, [root.join("tmt-ops")]);
    assert_eq!(removed.kept, std::slice::from_ref(&replaced));
    assert!(!root.join("tmt-ops").exists());
    assert_eq!(read_skill(&replaced), "user copy");
    assert_eq!(read_skill(&root.join("tmt-office")), "o");
    let remaining = owners(&global).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining["tmt-office"], "office");
    assert!(
        remove_owned(&global, "squad", None)
            .unwrap()
            .removed
            .is_empty(),
        "repeat is a no-op"
    );
}

#[test]
fn removal_by_name_leaves_the_owners_other_skills_and_other_owners_alone() {
    let (_directory, env, global, root) = fixture();
    install_owned(
        &env,
        &global,
        "squad",
        &[skill("tmt-ops", "s"), skill("tmux-squad", "p")],
        false,
    )
    .unwrap();
    install_owned(&env, &global, "office", &[skill("tmt-office", "o")], false).unwrap();
    let before = owners(&global).unwrap();

    // Invalid selections change nothing.
    for bad in [
        vec![],
        vec!["Tmux-Squad".to_string()],
        vec!["tmux-squad".to_string(), "tmux-squad".to_string()],
    ] {
        let error = remove_owned(&global, "squad", Some(&bad)).unwrap_err();
        assert!(matches!(refusal(&error.cause), Some(Refusal::Invalid(_))));
        assert_eq!(owners(&global).unwrap(), before);
        assert!(root.join("tmux-squad").exists());
    }

    // Names the owner does not hold, or another owner holds, are nothing to remove.
    let none = remove_owned(
        &global,
        "squad",
        Some(&["tmt-office".to_string(), "missing".to_string()]),
    )
    .unwrap();
    assert!(none.removed.is_empty() && none.kept.is_empty());
    assert_eq!(owners(&global).unwrap(), before);

    let removed = remove_owned(&global, "squad", Some(&["tmux-squad".to_string()])).unwrap();
    assert_eq!(removed.removed, [root.join("tmux-squad")]);
    assert!(removed.kept.is_empty());
    assert!(!root.join("tmux-squad").exists());
    assert_eq!(read_skill(&root.join("tmt-ops")), "s");
    assert_eq!(read_skill(&root.join("tmt-office")), "o");
    let remaining = owners(&global).unwrap();
    assert_eq!(remaining.len(), 2);
    assert_eq!(remaining["tmt-ops"], "squad");
    assert!(
        remove_owned(&global, "squad", Some(&["tmux-squad".to_string()]))
            .unwrap()
            .removed
            .is_empty(),
        "repeat is a no-op"
    );
    // Without a selection the owner's remaining skill goes as before.
    let rest = remove_owned(&global, "squad", None).unwrap();
    assert_eq!(rest.removed, [root.join("tmt-ops")]);
}

#[test]
fn drift_reports_an_owned_target_that_no_longer_points_at_its_content() {
    let (_directory, env, global, root) = fixture();
    install_owned(&env, &global, "squad", &[skill("tmt-ops", "v1")], false).unwrap();
    assert!(inspect_local_drift(&env, &global).unwrap().is_empty());
    let target = root.join("tmt-ops");
    fs::remove_file(&target).unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere"), &target).unwrap();
    assert_eq!(inspect_local_drift(&env, &global).unwrap(), [target]);
}

#[test]
fn owners_names_and_files_are_validated_before_anything_else() {
    let good = skill("tmt-ops", "x");
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
    // Non-canonical spellings the filesystem would normalize are refused too.
    let mut bad_files: Vec<Vec<(String, Vec<u8>)>> = [
        "../escape.md",
        "/abs.md",
        ".hidden",
        "",
        "ref//usage.md",
        "ref/",
        "ref/usage.md/",
        "./ref.md",
    ]
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
            name: "tmt-ops".into(),
            files: files.clone(),
        };
        assert!(validate("squad", &[invalid]).is_err(), "{files:?}");
    }
    for name in ["", "Tmt", "-lead", "a b", "tmt_ops"] {
        assert!(validate("squad", &[skill(name, "x")]).is_err(), "{name}");
    }
    assert!(
        validate("squad", &[good.clone(), good]).is_err(),
        "repeated names"
    );
}

#[test]
fn guided_setup_links_recorded_extension_skills_into_new_roots_once() {
    let (_directory, env, global, root) = fixture();
    install_owned(&env, &global, "squad", &[skill("tmt-ops", "v1")], false).unwrap();
    let new_root = root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(".claude/skills");
    let roots = [root.clone(), new_root.clone()];
    let planned = super::plan_owned(&global, &roots).unwrap();
    assert_eq!(
        planned
            .iter()
            .map(|item| item.target.clone())
            .collect::<Vec<_>>(),
        [new_root.join("tmt-ops")]
    );
    // A recorded API name does not authorize replacing a later user directory.
    fs::create_dir_all(new_root.join("tmt-ops")).unwrap();
    fs::write(new_root.join("tmt-ops/SKILL.md"), "user content").unwrap();
    assert!(super::publish_owned(&global, &planned).unwrap().is_empty());
    assert_eq!(read_skill(&new_root.join("tmt-ops")), "user content");
    fs::remove_dir_all(new_root.join("tmt-ops")).unwrap();
    let planned = super::plan_owned(&global, &roots).unwrap();
    let linked = super::publish_owned(&global, &planned).unwrap();
    assert_eq!(linked, [new_root.join("tmt-ops")]);
    assert_eq!(read_skill(&new_root.join("tmt-ops")), "v1");
    assert!(super::plan_owned(&global, &roots).unwrap().is_empty());
    // Recorded, so removing the owner removes the new link too.
    let removed = remove_owned(&global, "squad", None).unwrap();
    assert!(removed.removed.contains(&new_root.join("tmt-ops")));
}

#[test]
fn recorded_owner_retires_dangling_generations_without_removing_a_changed_target() {
    let (_directory, env, global, root) = fixture();
    let report = install_owned(&env, &global, "squad", &[skill("tmt-ops", "old")], false).unwrap();
    let target = &report.published[0].target;
    let source = fs::read_link(target).unwrap();
    fs::remove_dir_all(source.parent().unwrap()).unwrap();
    let removed = remove_owned(&global, "squad", None).unwrap();
    assert_eq!(removed.removed, vec![target.clone()]);
    assert!(removed.kept.is_empty());
    assert!(!fs::exists(target).unwrap());
    assert!(owners(&global).unwrap().is_empty());

    install_owned(&env, &global, "squad", &[skill("tmt-ops", "new")], false).unwrap();
    fs::remove_file(target).unwrap();
    fs::create_dir(target).unwrap();
    fs::write(target.join("SKILL.md"), b"user directory").unwrap();
    let kept = remove_owned(&global, "squad", None).unwrap();
    assert_eq!(kept.kept, vec![root.join("tmt-ops")]);
    assert_eq!(
        fs::read(target.join("SKILL.md")).unwrap(),
        b"user directory"
    );
}

#[test]
fn default_directory_cutover_republishes_only_full_digest_verified_owned_sources() {
    for modified in [false, true] {
        let (directory, env, _, root) = fixture();
        let former = directory.path.join(".config/tmux-team");
        fs::create_dir_all(&former).unwrap();
        let former = fs::canonicalize(former).unwrap();
        let current = former.parent().unwrap().join("tmt");
        let offered = skill("fixture-owner", "owned source");
        install_owned(
            &env,
            &former,
            "fixture",
            std::slice::from_ref(&offered),
            false,
        )
        .unwrap();
        let target = root.join(&offered.name);
        let old = fs::read_link(&target).unwrap();
        fs::rename(&former, &current).unwrap();
        let moved = current.join(old.strip_prefix(&former).unwrap());
        if modified {
            fs::write(moved.join("SKILL.md"), b"user edit").unwrap();
        }
        let result = super::refresh_owned(&current, "fixture", &[offered]);
        if modified {
            assert!(result.is_err());
            assert_eq!(fs::read_link(&target).unwrap(), old);
            assert_eq!(fs::read(moved.join("SKILL.md")).unwrap(), b"user edit");
        } else {
            let result = result.unwrap();
            assert_eq!(result.published.len(), 1);
            assert!(result.published[0].changed);
            assert_eq!(fs::read_link(&target).unwrap(), moved);
            assert_eq!(read_skill(&target), "owned source");
        }
        assert!(!former.exists());
    }
}
