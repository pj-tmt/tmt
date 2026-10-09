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
    assert!(fs::symlink_metadata(root.join("tmt")).unwrap().is_symlink());
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
fn only_office_adopts_core_office_links_without_force() {
    let (_directory, env, global, root) = fixture();
    install(&env, &global, None, None, false).unwrap();
    install_office(&env, &global, false).unwrap();
    let office = root.join("tmt-office");
    let core_link = fs::read_link(&office).unwrap();

    let refused = install_owned(
        &env,
        &global,
        "squad",
        &[skill("tmt-office", "squad's office")],
        false,
    )
    .unwrap_err();
    assert_eq!(
        claimed(&refused.cause),
        Some(("tmt-office".into(), "core".into()))
    );
    assert!(refused.report.published.is_empty());
    assert_eq!(fs::read_link(&office).unwrap(), core_link);
    assert!(!global.join("skill-owners.json").exists());

    let forced = install_owned(
        &env,
        &global,
        "squad",
        &[skill("tmt-office", "squad's office")],
        true,
    )
    .unwrap();
    assert!(forced.published[0].changed);
    assert!(
        forced.published[0].backup.is_none(),
        "a managed link is relinked"
    );
    assert_eq!(read_skill(&office), "squad's office");
    assert_eq!(owners(&global).unwrap()["tmt-office"], "squad");
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

    let removed = remove_owned(None, &global, "squad", None).unwrap();
    assert_eq!(removed.removed, [root.join("tmt-ops")]);
    assert_eq!(removed.kept, std::slice::from_ref(&replaced));
    assert!(!root.join("tmt-ops").exists());
    assert_eq!(read_skill(&replaced), "user copy");
    assert_eq!(read_skill(&root.join("tmt-office")), "o");
    let remaining = owners(&global).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining["tmt-office"], "office");
    assert!(
        remove_owned(None, &global, "squad", None)
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
        let error = remove_owned(None, &global, "squad", Some(&bad)).unwrap_err();
        assert!(matches!(refusal(&error.cause), Some(Refusal::Invalid(_))));
        assert_eq!(owners(&global).unwrap(), before);
        assert!(root.join("tmux-squad").exists());
    }

    // Names the owner does not hold, or another owner holds, are nothing to remove.
    let none = remove_owned(
        None,
        &global,
        "squad",
        Some(&["tmt-office".to_string(), "missing".to_string()]),
    )
    .unwrap();
    assert!(none.removed.is_empty() && none.kept.is_empty());
    assert_eq!(owners(&global).unwrap(), before);

    let removed = remove_owned(None, &global, "squad", Some(&["tmux-squad".to_string()])).unwrap();
    assert_eq!(removed.removed, [root.join("tmux-squad")]);
    assert!(removed.kept.is_empty());
    assert!(!root.join("tmux-squad").exists());
    assert_eq!(read_skill(&root.join("tmt-ops")), "s");
    assert_eq!(read_skill(&root.join("tmt-office")), "o");
    let remaining = owners(&global).unwrap();
    assert_eq!(remaining.len(), 2);
    assert_eq!(remaining["tmt-ops"], "squad");
    assert!(
        remove_owned(None, &global, "squad", Some(&["tmux-squad".to_string()]))
            .unwrap()
            .removed
            .is_empty(),
        "repeat is a no-op"
    );
    // Without a selection the owner's remaining skill goes as before.
    let rest = remove_owned(None, &global, "squad", None).unwrap();
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
    let removed = remove_owned(None, &global, "squad", None).unwrap();
    assert!(removed.removed.contains(&new_root.join("tmt-ops")));
}

#[test]
fn recorded_owner_retires_dangling_generations_without_removing_a_changed_target() {
    let (_directory, env, global, root) = fixture();
    let report = install_owned(&env, &global, "squad", &[skill("tmt-ops", "old")], false).unwrap();
    let target = &report.published[0].target;
    let source = fs::read_link(target).unwrap();
    fs::remove_dir_all(source.parent().unwrap()).unwrap();
    let removed = remove_owned(None, &global, "squad", None).unwrap();
    assert_eq!(removed.removed, vec![target.clone()]);
    assert!(removed.kept.is_empty());
    assert!(!fs::exists(target).unwrap());
    assert!(owners(&global).unwrap().is_empty());

    install_owned(&env, &global, "squad", &[skill("tmt-ops", "new")], false).unwrap();
    fs::remove_file(target).unwrap();
    fs::create_dir(target).unwrap();
    fs::write(target.join("SKILL.md"), b"user directory").unwrap();
    let kept = remove_owned(None, &global, "squad", None).unwrap();
    assert_eq!(kept.kept, vec![root.join("tmt-ops")]);
    assert_eq!(
        fs::read(target.join("SKILL.md")).unwrap(),
        b"user directory"
    );
}

#[test]
fn ops_migration_preserves_recorded_custom_targets_without_discovering_providers() {
    let (directory, env, global, root) = fixture();
    install_owned(
        &env,
        &global,
        "squad",
        &[
            skill("tmt-squad", "former lead"),
            skill("tmux-squad", "playbook"),
        ],
        false,
    )
    .unwrap();
    let custom = directory.path.join("custom/tmt-squad");
    super::owned::link_recorded(&global, [("tmt-squad", custom.as_path())]).unwrap();
    let deleted = root.join("tmux-squad");
    fs::remove_file(&deleted).unwrap();
    let replacement = skill("tmt-ops", "Ops lead");
    super::migrate_former_owned(
        &global,
        tmt_core::native_install::Product::Ops,
        std::slice::from_ref(&replacement),
    )
    .unwrap();
    assert!(!root.join("tmt-squad").exists());
    assert!(!custom.exists());
    assert_eq!(read_skill(&root.join("tmt-ops")), "Ops lead");
    assert_eq!(read_skill(&custom.with_file_name("tmt-ops")), "Ops lead");
    assert!(!deleted.exists(), "removed links do not renew consent");
    let recorded = super::owned_by(None, &global, "ops").unwrap();
    assert_eq!(recorded["tmt-ops"].len(), 2);
    assert!(
        !owners(&global)
            .unwrap()
            .values()
            .any(|owner| owner == "squad")
    );
    super::migrate_former_owned(
        &global,
        tmt_core::native_install::Product::Ops,
        &[replacement],
    )
    .unwrap();
    fs::remove_file(root.join("tmt-ops")).unwrap();
    super::refresh_owned(&global, "ops", &[skill("tmt-ops", "updated")]).unwrap();
    assert!(!root.join("tmt-ops").exists());
    assert_eq!(read_skill(&custom.with_file_name("tmt-ops")), "updated");
    assert!(!env.home().join(".codex/skills/tmt-ops").exists());
}

#[test]
fn ops_migration_validates_sources_and_replaces_existing_new_names() {
    for modified_source in [false, true] {
        let (_directory, env, global, root) = fixture();
        install_owned(&env, &global, "squad", &[skill("tmt-squad", "old")], false).unwrap();
        let old = root.join("tmt-squad");
        if modified_source {
            fs::write(old.join("SKILL.md"), "modified").unwrap();
        } else {
            fs::create_dir(root.join("tmt-ops")).unwrap();
            fs::write(root.join("tmt-ops/SKILL.md"), "old copy").unwrap();
        }
        let result = super::migrate_former_owned(
            &global,
            tmt_core::native_install::Product::Ops,
            &[skill("tmt-ops", "new")],
        );
        if modified_source {
            assert!(result.is_err());
            assert_eq!(owners(&global).unwrap()["tmt-squad"], "squad");
            assert_eq!(read_skill(&old), "modified");
        } else {
            result.unwrap();
            assert_eq!(read_skill(&root.join("tmt-ops")), "new");
            assert!(!old.exists());
        }
    }
}

#[test]
fn ops_migration_recovers_after_the_renamed_link_was_published_and_old_link_removed() {
    let (_directory, env, global, root) = fixture();
    install_owned(&env, &global, "squad", &[skill("tmt-squad", "old")], false).unwrap();
    // Simulate an interruption after link publication but before retiring the
    // old owner record. The immutable destination itself remains verifiable.
    let replacement = skill("tmt-ops", "new");
    let before = fs::read(global.join("skill-owners.json")).unwrap();
    install_owned(
        &env,
        &global,
        "ops",
        std::slice::from_ref(&replacement),
        false,
    )
    .unwrap();
    fs::write(global.join("skill-owners.json"), before).unwrap();
    fs::remove_file(root.join("tmt-squad")).unwrap();
    super::migrate_former_owned(
        &global,
        tmt_core::native_install::Product::Ops,
        &[replacement],
    )
    .unwrap();
    assert_eq!(
        super::owned_by(None, &global, "ops").unwrap()["tmt-ops"],
        [root.join("tmt-ops")]
    );
    assert!(!owners(&global).unwrap().contains_key("tmt-squad"));
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

#[test]
fn ops_migration_replaces_a_foreign_new_name_after_the_former_link_was_removed() {
    let (_directory, env, global, root) = fixture();
    install_owned(&env, &global, "squad", &[skill("tmt-squad", "old")], false).unwrap();
    fs::remove_file(root.join("tmt-squad")).unwrap();
    let personal = root.join("personal");
    fs::create_dir(&personal).unwrap();
    fs::write(personal.join("SKILL.md"), "personal").unwrap();
    std::os::unix::fs::symlink(&personal, root.join("tmt-ops")).unwrap();
    super::migrate_former_owned(
        &global,
        tmt_core::native_install::Product::Ops,
        &[skill("tmt-ops", "new")],
    )
    .unwrap();
    assert_eq!(read_skill(&root.join("tmt-ops")), "new");
    assert_eq!(read_skill(&personal), "personal");
    assert!(!owners(&global).unwrap().contains_key("tmt-squad"));
}
