//! Removes current managed guidance while retaining retired installations.
//! Historical skill links, source generations and owner records stay in place. A path that no longer links into TMT's store is
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

/// Historical targets are preservation evidence only, never removal authority.
fn retained_targets(env: &ProviderEnvironment, global: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let names = &tmt_core::skill_catalog::HISTORICAL_BUNDLE_NAMES[2..];
    let mut targets = registry::read(global)?;
    targets.retain(|target| {
        target
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| names.contains(&name))
    });
    let mut roots: Vec<_> = Registry::builtin()
        .iter()
        .map(|driver| env.locations(driver).skills)
        .collect();
    roots.push(env.universal_skills());
    for root in roots {
        for name in names {
            let target = root.join(name);
            if files::exists(&target)? {
                targets.insert(target);
            }
        }
    }
    targets.extend(
        owned::owned_by(None, global, "office")?
            .into_values()
            .flatten(),
    );
    Ok(targets)
}

/// A retained historical generation must not lose its backing tree even when
/// targets were removed or their intent record was lost. This grants no ownership.
fn retained_sources(assets: &SkillAssets) -> io::Result<bool> {
    if !files::exists(assets.root())? {
        return Ok(false);
    }
    if files::exists(&assets.root().join("owners/office"))? {
        return Ok(true);
    }
    for generation in fs::read_dir(assets.root())? {
        let generation = generation?;
        if !generation.file_type()?.is_dir() {
            continue;
        }
        for name in &tmt_core::skill_catalog::HISTORICAL_BUNDLE_NAMES[2..] {
            if files::exists(&generation.path().join(name))? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Read-only: what [`uninstall`] would remove and keep.
pub fn plan_uninstall(env: &ProviderEnvironment, global: &Path) -> io::Result<SkillsRemoval> {
    let mut plan = SkillsRemoval::default();
    let global = files::resolved(global)?;
    if !files::exists(&global)? {
        return Ok(plan);
    }
    let retained = retained_targets(env, &global)?;
    plan.kept.extend(retained.iter().cloned());
    for (_, targets) in owned::owned_targets(&global)? {
        plan.removed.extend(
            targets
                .into_iter()
                .filter(|target| !retained.contains(target)),
        );
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
    let retained = retained_targets(env, &global)?;
    removal.kept.extend(retained.iter().cloned());
    for owner in owned::owners(&global)?
        .into_values()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|owner| owner != "office")
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
    // Removed current intents are forgotten; historical intents and sources remain.
    // Retention requires no digest admission because it authorizes no deletion.
    if !retained.is_empty() || retained_sources(&assets)? {
        files::with_lock(&global, || registry::forget(&global, &removal.removed))?;
        return Ok(removal);
    }
    // No retired installation remains; dispose of the owned store and records.
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
