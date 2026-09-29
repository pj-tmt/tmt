//! Read-only planning and targeted publication for guided setup: which skill
//! links a set of skill roots still needs. Core's skills are published by the
//! ordinary install; extensions' skills already recorded by their owners are
//! linked into roots that do not have them yet.

use super::{ProviderEnvironment, assets::SkillAssets, catalog, files, managed_link, owned};
use crate::drivers::DriverDefinition;
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillState {
    /// Nothing there yet: setup publishes it.
    Missing,
    /// TMT's link to an earlier bundle: setup refreshes it.
    Stale,
    Current,
    /// Something that is not TMT's link: setup leaves it and reports it.
    Occupied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTarget {
    pub name: String,
    pub target: PathBuf,
    pub state: SkillState,
}

/// Core's skills in this driver's skill root.
pub fn plan_core(
    env: &ProviderEnvironment,
    global: &Path,
    driver: &DriverDefinition,
) -> io::Result<Vec<SkillTarget>> {
    let global = files::resolved(global)?;
    let assets = SkillAssets::new(&global);
    let root = env.locations(driver).skills;
    catalog::Catalog::bundled()
        .names(catalog::Group::Core)
        .into_iter()
        .map(|name| {
            let target = root.join(name);
            let state = if !files::exists(&target)? {
                SkillState::Missing
            } else {
                match managed_link(&target, &assets)? {
                    Some(source) if source == assets.source_of(name) => SkillState::Current,
                    Some(_) => SkillState::Stale,
                    None => SkillState::Occupied,
                }
            };
            Ok(SkillTarget {
                name: name.to_owned(),
                target,
                state,
            })
        })
        .collect()
}

/// Owned (extension) skills missing from these roots. Targets already
/// holding anything are left to their owner's own installer.
pub fn plan_owned(global: &Path, roots: &[PathBuf]) -> io::Result<Vec<SkillTarget>> {
    let global = files::resolved(global)?;
    if !files::exists(&global)? {
        return Ok(Vec::new());
    }
    let mut missing = Vec::new();
    for (name, _) in owned::owned_targets(&global)? {
        for root in roots {
            let target = root.join(&name);
            if !files::exists(&target)? {
                missing.push(SkillTarget {
                    name: name.clone(),
                    target,
                    state: SkillState::Missing,
                });
            }
        }
    }
    Ok(missing)
}

/// Links the planned owned skills to their owners' current sources and
/// records the new targets; a target that appeared meanwhile is skipped.
pub fn publish_owned(global: &Path, planned: &[SkillTarget]) -> io::Result<Vec<PathBuf>> {
    let global = files::resolved(global)?;
    owned::link_recorded(
        &global,
        planned
            .iter()
            .map(|item| (item.name.as_str(), item.target.as_path())),
    )
}
