//! One-shot replacement of the former managed core skill, not a catalog alias.

use super::{assets::SkillAssets, catalog::MAIN, files, managed_link, registry};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(super) const NAME: &str = "tmux-team";

pub(super) enum Replacement {
    Missing,
    Replaced { target: PathBuf, changed: bool },
    Conflict(PathBuf),
}

/// Called under the installer lock. Publish and record the replacement before
/// removing the exact verified old link, so publication failure keeps guidance.
/// Neither force nor a matching directory name makes user content disposable.
pub(super) fn replace(
    global: &Path,
    root: &Path,
    assets: &SkillAssets,
    current: &Path,
    publish: &mut impl FnMut(&Path, &Path) -> io::Result<()>,
) -> io::Result<Replacement> {
    let old = root.join(NAME);
    if !files::exists(&old)? {
        return Ok(Replacement::Missing);
    }
    if !fs::symlink_metadata(&old)?.is_symlink() {
        return Ok(Replacement::Conflict(old));
    }
    let link = fs::read_link(&old)?;
    let source = files::resolved(&crate::config::normalize(&root.join(&link)))?;
    let source = if assets.owns_retired(&source) {
        source
    } else {
        crate::config::relocated_skill_source(global, &source).unwrap_or(source)
    };
    if !assets.owns_retired(&source) {
        return Ok(Replacement::Conflict(old));
    }
    let target = root.join(MAIN);
    files::safe_target(assets.root(), &target)?;
    let prior = managed_link(&target, assets)?;
    if prior.is_none() && files::exists(&target)? {
        return Ok(Replacement::Conflict(target));
    }
    registry::remember(global, [target.clone()])?;
    let changed = prior.as_deref() != Some(current);
    if changed {
        publish(&target, current)?;
    }
    // A cooperating installer holds this lock. Recheck the symlink and digest
    // as well before deletion; an external edit leaves both entries for review.
    if fs::read_link(&old)? != link || !assets.owns_retired(&source) {
        return Ok(Replacement::Conflict(old));
    }
    fs::remove_file(&old)?;
    registry::forget(global, &[old])?;
    Ok(Replacement::Replaced { target, changed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        skill_installation::{ProviderEnvironment, install, refresh},
        test_support::TestDirectory,
    };

    fn old_link(global: &Path, root: &Path) -> PathBuf {
        let bytes = b"---\nname: tmux-team\n---\nEarlier managed guidance.\n";
        let source = global
            .join("skill-assets")
            .join(tmt_core::content_digest::sha256(bytes))
            .join(NAME);
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), bytes).unwrap();
        fs::create_dir_all(root).unwrap();
        let target = root.join(NAME);
        std::os::unix::fs::symlink(&source, &target).unwrap();
        registry::remember(global, [target]).unwrap();
        source
    }

    #[test]
    fn install_replaces_verified_old_names_in_selected_and_registered_custom_roots() {
        let temp = TestDirectory::new();
        let global = temp.path.join("global");
        let home = temp.path.join("home");
        let shared = home.join(".agents/skills");
        let custom = temp.path.join("custom");
        old_link(&global, &shared);
        old_link(&global, &custom);
        let env = ProviderEnvironment::from_parts(home, &temp.path, Vec::new(), []);
        let report = install(&env, &global, None, None, false).unwrap();
        assert_eq!(report.installed.len(), 3); // selected main/inbox plus custom main
        assert!(report.installed.iter().all(|item| item.changed));
        for root in [&shared, &custom] {
            assert!(!files::exists(&root.join(NAME)).unwrap());
            assert_eq!(
                fs::read(root.join(MAIN).join("SKILL.md")).unwrap(),
                super::super::bundled_skill()
            );
        }
        let targets = registry::read(&global).unwrap();
        assert!(
            targets
                .iter()
                .all(|target| target.file_name().unwrap() != NAME)
        );
        assert!(targets.contains(&custom.join(MAIN)));
    }

    #[test]
    fn replacing_a_shared_target_preserves_each_selected_provider_report() {
        let temp = TestDirectory::new();
        let global = temp.path.join("global");
        let home = temp.path.join("home");
        old_link(&global, &home.join(".agents/skills"));
        let env = ProviderEnvironment::from_parts(home, &temp.path, Vec::new(), []);
        let report = install(&env, &global, Some("all"), None, false).unwrap();
        assert_eq!(
            report
                .installed
                .iter()
                .map(|item| item.agent.unwrap().name())
                .collect::<Vec<_>>(),
            vec![
                "claude", "claude", "codex", "codex", "gemini", "gemini", "agy", "agy", "pi", "pi",
                "opencode", "opencode"
            ]
        );
        assert_eq!(
            report.installed.iter().filter(|item| item.changed).count(),
            8
        );
    }

    #[test]
    fn refresh_replaces_old_managed_guidance_but_never_installs_missing_integrations() {
        let temp = TestDirectory::new();
        let root = temp.path.join("skills");
        let global = temp.path.join("global");
        old_link(&global, &root);
        let report = refresh(&global).unwrap();
        assert_eq!(report.refreshed.len(), 1);
        assert_eq!(report.refreshed[0].target, root.join(MAIN));
        assert!(!files::exists(&root.join(NAME)).unwrap());
        assert_eq!(
            fs::read(root.join(MAIN).join("SKILL.md")).unwrap(),
            super::super::bundled_skill()
        );
        let missing = temp.path.join("missing").join(NAME);
        registry::remember(&global, [missing.clone()]).unwrap();
        assert!(refresh(&global).unwrap().skipped.contains(&missing));
        assert!(!missing.parent().unwrap().join(MAIN).exists());
    }

    #[test]
    fn edited_sources_and_unmanaged_directories_remain_conflicts_even_with_force() {
        for edit_source in [false, true] {
            let temp = TestDirectory::new();
            let global = temp.path.join("global");
            let home = temp.path.join("home");
            let root = home.join(".agents/skills");
            if edit_source {
                let source = old_link(&global, &root);
                fs::write(source.join("SKILL.md"), b"user changes").unwrap();
            } else {
                fs::create_dir_all(root.join(NAME)).unwrap();
                fs::write(root.join(NAME).join("SKILL.md"), b"user changes").unwrap();
            }
            let env = ProviderEnvironment::from_parts(home, &temp.path, Vec::new(), []);
            let error = install(&env, &global, None, None, true).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("former skill targets were preserved")
            );
            assert_eq!(
                fs::read(root.join(NAME).join("SKILL.md")).unwrap(),
                b"user changes"
            );
            assert!(!root.join(MAIN).exists());
        }
    }

    #[test]
    fn publication_failure_keeps_the_old_link_and_intent() {
        let temp = TestDirectory::new();
        let root = temp.path.join("skills");
        let global = temp.path.join("global");
        let source = old_link(&global, &root);
        let error = super::super::refresh::refresh_with_publisher(&global, |_, _| {
            Err(io::Error::other("fixture refused publication"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("fixture refused publication"));
        assert_eq!(fs::read_link(root.join(NAME)).unwrap(), source);
        assert!(registry::read(&global).unwrap().contains(&root.join(NAME)));
        assert!(!root.join(MAIN).exists());
    }

    #[test]
    fn prior_framed_bundles_require_the_complete_unchanged_inventory() {
        for count in [2, 3, 4, 5] {
            let temp = TestDirectory::new();
            let global = temp.path.join("global");
            let root = temp.path.join("skills");
            let layout = &super::super::catalog::BUNDLED[..count];
            let prior_main = b"---\nname: tmux-team\n---\nPrior bundled core guidance.\n";
            let bytes = |skill: &super::super::catalog::BundledSkill| {
                if skill.name == MAIN {
                    prior_main.as_slice()
                } else {
                    skill.bytes
                }
            };
            let mut framed = Vec::new();
            for skill in layout {
                framed.extend_from_slice(&(bytes(skill).len() as u64).to_be_bytes());
                framed.extend_from_slice(bytes(skill));
            }
            let version = global
                .join("skill-assets")
                .join(tmt_core::content_digest::sha256(&framed));
            for skill in layout {
                let name = if skill.name == MAIN { NAME } else { skill.name };
                let source = version.join(name);
                fs::create_dir_all(&source).unwrap();
                fs::write(source.join("SKILL.md"), bytes(skill)).unwrap();
            }
            let assets = SkillAssets::new(&global);
            assert!(assets.owns_retired(&version.join(NAME)));
            fs::write(version.join("tmt-inbox/SKILL.md"), b"edited peer").unwrap();
            assert!(!assets.owns_retired(&version.join(NAME)));
            fs::write(version.join("tmt-inbox/SKILL.md"), layout[1].bytes).unwrap();
            fs::create_dir_all(&root).unwrap();
            for skill in layout {
                let name = if skill.name == MAIN { NAME } else { skill.name };
                std::os::unix::fs::symlink(version.join(name), root.join(name)).unwrap();
                registry::remember(&global, [root.join(name)]).unwrap();
            }
            let report = refresh(&global).unwrap();
            assert_eq!(report.refreshed.len(), count);
            assert!(!files::exists(&root.join(NAME)).unwrap());
            for skill in layout {
                assert_eq!(
                    fs::read(root.join(skill.name).join("SKILL.md")).unwrap(),
                    skill.bytes
                );
                assert_ne!(
                    fs::read_link(root.join(skill.name)).unwrap(),
                    version.join(skill.name)
                );
            }
        }
    }

    #[test]
    fn occupied_replacement_keeps_both_the_old_link_and_user_content() {
        let temp = TestDirectory::new();
        let global = temp.path.join("global");
        let root = temp.path.join("skills");
        let source = old_link(&global, &root);
        fs::create_dir(root.join(MAIN)).unwrap();
        fs::write(root.join(MAIN).join("SKILL.md"), b"user content").unwrap();
        let error = refresh(&global).unwrap_err();
        assert_eq!(error.report.conflicts, [root.join(MAIN)]);
        assert_eq!(fs::read_link(root.join(NAME)).unwrap(), source);
        assert_eq!(
            fs::read(root.join(MAIN).join("SKILL.md")).unwrap(),
            b"user content"
        );
        assert!(registry::read(&global).unwrap().contains(&root.join(NAME)));
    }

    #[test]
    fn an_edit_during_publication_keeps_the_old_target_registered() {
        let temp = TestDirectory::new();
        let global = temp.path.join("global");
        let root = temp.path.join("skills");
        let source = old_link(&global, &root);
        let error = super::super::refresh::refresh_with_publisher(&global, |target, current| {
            files::link(target, current)?;
            fs::write(source.join("SKILL.md"), b"concurrent edit")
        })
        .unwrap_err();
        assert_eq!(error.report.conflicts, [root.join(NAME)]);
        assert_eq!(fs::read_link(root.join(NAME)).unwrap(), source);
        assert_eq!(
            fs::read(root.join(NAME).join("SKILL.md")).unwrap(),
            b"concurrent edit"
        );
        assert!(root.join(MAIN).join("SKILL.md").is_file());
        let targets = registry::read(&global).unwrap();
        assert!(targets.contains(&root.join(NAME)));
        assert!(targets.contains(&root.join(MAIN)));
    }

    #[test]
    fn directory_cutover_republishes_old_absolute_links_using_current_digest_evidence() {
        let temp = TestDirectory::new();
        let old_global = temp.path.join(".config/tmux-team");
        let global = temp.path.join(".config/tmt");
        let root = temp.path.join("skills");
        let source = old_link(&old_global, &root);
        let old_assets = SkillAssets::new(&old_global);
        let inbox = old_assets
            .materialize_bundle()
            .unwrap()
            .get("tmt-inbox")
            .unwrap()
            .clone();
        std::os::unix::fs::symlink(&inbox, root.join("tmt-inbox")).unwrap();
        registry::remember(&old_global, [root.join("tmt-inbox")]).unwrap();
        fs::rename(&old_global, &global).unwrap();
        assert!(!source.exists());
        assert!(!inbox.exists());
        let report = refresh(&global).unwrap();
        assert_eq!(report.refreshed.len(), 2);
        assert!(report.refreshed.iter().all(|item| item.changed));
        assert!(!files::exists(&root.join(NAME)).unwrap());
        for name in [MAIN, "tmt-inbox"] {
            let target = root.join(name);
            assert!(target.join("SKILL.md").is_file());
            assert!(
                fs::read_link(target)
                    .unwrap()
                    .starts_with(fs::canonicalize(global.join("skill-assets")).unwrap())
            );
        }
    }
}
