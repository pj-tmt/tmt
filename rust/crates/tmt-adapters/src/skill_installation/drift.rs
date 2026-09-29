//! Read-only, fixed-provider inspection. Never scan custom installation intents
//! or executable search paths on ordinary commands.

use super::{
    ProviderEnvironment, assets::SkillAssets, catalog, files, managed_link, owned::owned_drift,
};
use crate::drivers::Registry;
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
};

pub fn inspect_local_drift(env: &ProviderEnvironment, global: &Path) -> io::Result<Vec<PathBuf>> {
    let global = files::resolved(global)?;
    let assets = SkillAssets::new(&global);
    let current = assets.source();
    let mut seen = BTreeSet::new();
    let mut drift = Vec::new();
    for provider in Registry::builtin().iter() {
        let target = env.target(provider);
        let root = target.parent().expect("skill target parent");
        let bundled = catalog::BUNDLED
            .iter()
            .map(|skill| (root.join(skill.name), false, assets.source_of(skill.name)));
        let legacy = env
            .legacy_targets(provider)
            .into_iter()
            .map(|path| (path, true, current.clone()));
        for (path, legacy, expected) in bundled.chain(legacy) {
            if !seen.insert(path.clone()) || !files::exists(&path)? {
                continue;
            }
            if legacy || managed_link(&path, &assets)?.as_ref() != Some(&expected) {
                drift.push(path);
            }
        }
    }
    for path in owned_drift(env, &global)? {
        if seen.insert(path.clone()) {
            drift.push(path);
        }
    }
    Ok(drift)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        skill_installation::{install, install_office},
        test_support::TestDirectory,
    };
    use std::fs;

    #[test]
    fn missing_and_current_targets_are_quiet_while_legacy_and_modified_links_are_reported_once() {
        let root = TestDirectory::new();
        let home = root.path.join("home");
        fs::create_dir(&home).unwrap();
        let global = root.path.join("global");
        let env = ProviderEnvironment::from_parts(home.clone(), root.path.clone(), Vec::new(), []);
        assert!(inspect_local_drift(&env, &global).unwrap().is_empty());
        assert!(!global.exists());
        install(&env, &global, Some("all"), None, false).unwrap();
        assert!(inspect_local_drift(&env, &global).unwrap().is_empty());
        // Deliberately corrupt intents: passive inspection must not read them.
        fs::write(global.join("skill-installations.json"), b"invalid").unwrap();
        let shared = home.join(".agents/skills/tmux-team");
        fs::remove_file(&shared).unwrap();
        std::os::unix::fs::symlink(home.join("missing"), &shared).unwrap();
        let legacy = home.join(".claude/commands/team.md");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, b"legacy user content").unwrap();
        assert_eq!(
            inspect_local_drift(&env, &global).unwrap(),
            vec![legacy.clone(), shared.clone()]
        );
        assert_eq!(fs::read(&legacy).unwrap(), b"legacy user content");
        assert_eq!(fs::read_link(&shared).unwrap(), home.join("missing"));
        assert_eq!(
            fs::read(global.join("skill-installations.json")).unwrap(),
            b"invalid"
        );
        assert!(!global.join("state.db").exists());
    }

    #[test]
    fn optional_office_art_drift_is_reported_only_after_its_target_exists() {
        let root = TestDirectory::new();
        let home = root.path.join("home");
        fs::create_dir(&home).unwrap();
        let global = root.path.join("global");
        let env = ProviderEnvironment::from_parts(home.clone(), root.path.clone(), Vec::new(), []);
        install(&env, &global, None, None, false).unwrap();
        assert!(inspect_local_drift(&env, &global).unwrap().is_empty());
        install_office(&env, &global, false).unwrap();
        assert!(inspect_local_drift(&env, &global).unwrap().is_empty());

        let target = home.join(".agents/skills/tmt-office");
        fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink(home.join("missing-office-skill"), &target).unwrap();
        let avatar_target = home.join(".agents/skills/tmt-avatar-create");
        fs::remove_file(&avatar_target).unwrap();
        std::os::unix::fs::symlink(home.join("missing-avatar-skill"), &avatar_target).unwrap();
        assert_eq!(
            inspect_local_drift(&env, &global).unwrap(),
            vec![target, avatar_target]
        );
    }
}
