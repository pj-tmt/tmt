//! Removes everything skill installation published: owners' links through
//! their own records, then core's and Office's bundled links, then the stores
//! and records themselves. A path that no longer links into TMT's store is
//! kept and reported, never removed.

use super::{
    ProviderEnvironment, assets::SkillAssets, catalog, files, managed_link, owned, registry,
};
use crate::drivers::Registry;
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SkillsRemoval {
    pub removed: Vec<PathBuf>,
    pub kept: Vec<PathBuf>,
}

/// The bundled-skill targets TMT may have published: every driver's skill
/// root and every recorded custom target, for every bundled name.
fn candidates(env: &ProviderEnvironment, global: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let mut roots: BTreeSet<PathBuf> = Registry::builtin()
        .iter()
        .map(|driver| env.locations(driver).skills)
        .collect();
    roots.insert(env.universal_skills());
    for target in registry::read(global)? {
        if let Some(root) = target.parent() {
            roots.insert(root.to_path_buf());
        }
    }
    Ok(roots
        .iter()
        .flat_map(|root| {
            catalog::BUNDLED
                .iter()
                .map(move |skill| root.join(skill.name))
        })
        .collect())
}

/// Read-only: what [`uninstall`] would remove and keep.
pub fn plan_uninstall(env: &ProviderEnvironment, global: &Path) -> io::Result<SkillsRemoval> {
    let mut plan = SkillsRemoval::default();
    let global = files::resolved(global)?;
    if !files::exists(&global)? {
        return Ok(plan);
    }
    for (_, targets) in owned::owned_targets(&global)? {
        plan.removed.extend(targets);
    }
    let assets = SkillAssets::new(&global);
    for target in candidates(env, &global)? {
        if managed_link(&target, &assets)?.is_some() {
            plan.removed.push(target);
        } else if files::exists(&target)? && registry::read(&global)?.contains(&target) {
            plan.kept.push(target);
        }
    }
    Ok(plan)
}

pub fn uninstall(env: &ProviderEnvironment, global: &Path) -> io::Result<SkillsRemoval> {
    let mut removal = SkillsRemoval::default();
    let global = files::resolved(global)?;
    if !files::exists(&global)? {
        return Ok(removal);
    }
    for owner in owned::owners(&global)?
        .into_values()
        .collect::<BTreeSet<_>>()
    {
        let report =
            owned::remove_owned(None, &global, &owner, None).map_err(|failure| failure.cause)?;
        removal.removed.extend(report.removed);
        removal.kept.extend(report.kept);
    }
    let assets = SkillAssets::new(&global);
    files::with_lock(&global, || {
        let recorded = registry::read(&global)?;
        for target in candidates(env, &global)? {
            if managed_link(&target, &assets)?.is_some() {
                fs::remove_file(&target)?;
                removal.removed.push(target);
            } else if files::exists(&target)? && recorded.contains(&target) {
                removal.kept.push(target);
            }
        }
        Ok(())
    })?;
    // Every link into the store is gone; the store and records go with it.
    for record in [
        registry::path(&global),
        owned::registry_path(&global),
        global.join("skill-install.lock"),
    ] {
        match fs::remove_file(&record) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
    }
    match fs::remove_dir_all(assets.root()) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        result => result?,
    }
    Ok(removal)
}
