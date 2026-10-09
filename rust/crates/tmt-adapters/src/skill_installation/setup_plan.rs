//! Read-only planning and targeted publication for guided setup: which skill
//! links a set of skill roots still needs. Apply publishes exactly the planned
//! targets: core's skills through [`publish_core`], and extensions' skills
//! already recorded by their owners into roots that do not have them yet.

use super::{ProviderEnvironment, assets::SkillAssets, catalog, files, owned, registry};
use crate::drivers::DriverDefinition;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillState {
    /// Nothing there yet: setup publishes it.
    Missing,
    /// TMT's link to an earlier bundle: setup refreshes it.
    Stale,
    Current,
    /// A link to this skill in another TMT home's assets, left by an earlier
    /// or moved installation: setup replaces it.
    Foreign {
        source: PathBuf,
    },
    /// An existing entry at a published name: setup replaces it.
    Occupied,
}

impl SkillState {
    /// Whether apply publishes this target.
    pub fn changes(&self) -> bool {
        !matches!(self, Self::Current)
    }
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
            Ok(SkillTarget {
                name: name.to_owned(),
                state: core_state(&target, name, &assets)?,
                target,
            })
        })
        .collect()
}

fn core_state(target: &Path, name: &str, assets: &SkillAssets) -> io::Result<SkillState> {
    if !files::exists(target)? {
        return Ok(SkillState::Missing);
    }
    if files::current_link(target, &assets.source_of(name)) {
        return Ok(SkillState::Current);
    }
    Ok(match foreign_source(target, name)? {
        Some(source) => SkillState::Foreign { source },
        None => SkillState::Occupied,
    })
}

/// Read-only source coordinates for a link into another TMT home's assets.
/// Planning never reads the prior target's skill contents.
fn foreign_source(target: &Path, name: &str) -> io::Result<Option<PathBuf>> {
    if !fs::symlink_metadata(target)?.is_symlink() {
        return Ok(None);
    }
    let source = crate::config::normalize(
        &target
            .parent()
            .expect("skill target parent")
            .join(fs::read_link(target)?),
    );
    let layout = source.file_name().is_some_and(|file| file == name)
        && source
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|assets| assets == "skill-assets");
    Ok(layout.then_some(source))
}

/// What [`publish_core`] did with the planned targets.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CorePublication {
    pub linked: Vec<PathBuf>,
    /// Legacy report field; publication no longer creates backups.
    pub backups: Vec<PathBuf>,
    /// Planned targets already current when publication ran.
    pub skipped: Vec<PathBuf>,
}

/// Publishes the planned core names. Existing entries at those names are
/// replaced regardless of their contents or changes since planning.
pub fn publish_core(global: &Path, planned: &[SkillTarget]) -> io::Result<CorePublication> {
    let global = files::resolved(global)?;
    let assets = SkillAssets::new(&global);
    let planned: Vec<&SkillTarget> = planned.iter().collect();
    for item in &planned {
        if !catalog::Catalog::bundled()
            .names(catalog::Group::Core)
            .contains(&item.name.as_str())
            || item.target.file_name() != Some(std::ffi::OsStr::new(&item.name))
        {
            return Err(io::Error::other("Invalid planned core skill target."));
        }
        files::safe_target(assets.root(), &item.target)?;
    }
    files::with_lock(&global, || {
        registry::read(&global)?;
        let sources = assets.materialize_bundle()?;
        registry::remember(&global, planned.iter().map(|item| item.target.clone()))?;
        let mut done = CorePublication::default();
        for item in planned {
            let source = sources.get(&item.name).expect("a bundled core source");
            if files::current_link(&item.target, source) {
                done.skipped.push(item.target.clone());
                continue;
            }
            files::link(&item.target, source)?;
            done.linked.push(item.target.clone());
        }
        Ok(done)
    })
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
/// records the new targets; an entry that appeared meanwhile is replaced.
pub fn publish_owned(global: &Path, planned: &[SkillTarget]) -> io::Result<Vec<PathBuf>> {
    let global = files::resolved(global)?;
    owned::link_recorded(
        &global,
        planned
            .iter()
            .map(|item| (item.name.as_str(), item.target.as_path())),
    )
}
