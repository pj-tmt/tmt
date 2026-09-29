//! Read-only planning and targeted publication for guided setup: which skill
//! links a set of skill roots still needs. Apply publishes exactly the planned
//! targets: core's skills through [`publish_core`], and extensions' skills
//! already recorded by their owners into roots that do not have them yet.

use super::{
    ProviderEnvironment, assets::SkillAssets, catalog, files, managed_link, owned, registry,
};
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
    /// or moved installation: setup backs it up and replaces it.
    Foreign {
        source: PathBuf,
    },
    /// Something that is not TMT's: setup leaves it and reports it.
    Occupied,
}

impl SkillState {
    /// Whether apply publishes this target.
    pub fn changes(&self) -> bool {
        matches!(self, Self::Missing | Self::Stale | Self::Foreign { .. })
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
    Ok(match managed_link(target, assets)? {
        Some(source) if source == assets.source_of(name) => SkillState::Current,
        Some(_) => SkillState::Stale,
        None => match foreign_source(target, name)? {
            Some(source) => SkillState::Foreign { source },
            None => SkillState::Occupied,
        },
    })
}

/// A symlink into `<a TMT home>/skill-assets/<bundle>/<name>` whose skill,
/// while still readable, declares the same name: TMT's own skill from another
/// installation rather than a user's.
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
    if !layout {
        return Ok(None);
    }
    let declared = match fs::read_to_string(source.join("SKILL.md")) {
        Ok(text) => declares(&text, name),
        // A dangling link into another home's assets is still TMT's.
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
    };
    Ok(declared.then_some(source))
}

/// Whether the skill's front matter names it `name`.
fn declares(text: &str, name: &str) -> bool {
    let mut lines = text.lines();
    lines.next() == Some("---")
        && lines
            .take_while(|line| *line != "---")
            .any(|line| line.strip_prefix("name:").map(str::trim) == Some(name))
}

/// What [`publish_core`] did with the planned targets.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CorePublication {
    pub linked: Vec<PathBuf>,
    /// Where each replaced foreign link was moved first.
    pub backups: Vec<PathBuf>,
    /// Targets whose state changed after planning; left untouched.
    pub skipped: Vec<PathBuf>,
}

/// Publishes exactly the planned core targets that change. Each one is
/// classified again under the installer lock, and one that no longer matches
/// its plan is skipped rather than overwritten: apply never touches a path
/// the plan did not show as a change.
pub fn publish_core(global: &Path, planned: &[SkillTarget]) -> io::Result<CorePublication> {
    let global = files::resolved(global)?;
    let assets = SkillAssets::new(&global);
    let planned: Vec<&SkillTarget> = planned.iter().filter(|item| item.state.changes()).collect();
    for item in &planned {
        files::safe_target(assets.root(), &item.target)?;
    }
    files::with_lock(&global, || {
        registry::read(&global)?;
        let sources = assets.materialize_bundle()?;
        registry::remember(&global, planned.iter().map(|item| item.target.clone()))?;
        let mut done = CorePublication::default();
        for item in planned {
            if core_state(&item.target, &item.name, &assets)? != item.state {
                done.skipped.push(item.target.clone());
                continue;
            }
            if matches!(item.state, SkillState::Foreign { .. }) {
                done.backups.push(files::backup(&item.target)?);
            }
            let source = sources.get(&item.name).expect("a bundled core source");
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
