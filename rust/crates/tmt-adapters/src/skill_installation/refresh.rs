//! Refresh existing managed links, never install new integrations from intent.

use super::{assets::SkillAssets, files, owned::owned_names, registry, retired};
use std::{
    collections::BTreeSet,
    error::Error,
    fmt, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct RefreshedSkill {
    pub target: PathBuf,
    pub changed: bool,
}

#[derive(Debug, Default)]
pub struct RefreshReport {
    pub refreshed: Vec<RefreshedSkill>,
    pub skipped: Vec<PathBuf>,
    pub conflicts: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct RefreshFailure {
    cause: io::Error,
    pub report: RefreshReport,
}

impl fmt::Display for RefreshFailure {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(output, "Skill refresh failed: {}", self.cause)
    }
}

impl Error for RefreshFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.cause)
    }
}

/// Invoke from the new active executable so materialization uses its embedded
/// skill, not the old updater process's bytes. No provider detection occurs.
pub fn refresh(global: &Path) -> Result<RefreshReport, RefreshFailure> {
    refresh_with_publisher(global, files::link)
}

pub(super) fn refresh_with_publisher(
    global: &Path,
    mut publish: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<RefreshReport, RefreshFailure> {
    let mut report = RefreshReport::default();
    let pending = (|| {
        let global = files::resolved(global)?;
        // A never-installed user must not gain application state just by updating
        // a binary. This read is observational; nonempty intent is re-read locked.
        if registry::read(&global)?.is_empty() {
            return Ok(());
        }
        files::with_lock(&global, || {
            let targets = registry::read(&global)?;
            let owned = owned_names(&global)?;
            let assets = SkillAssets::new(&global);
            let mut locations = BTreeSet::new();
            let mut sources = None;
            for target in targets {
                let location = files::entry_location(&target)?;
                if !locations.insert(location) {
                    continue;
                }
                let held = target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| owned.contains(name));
                if held || !files::exists(&target)? {
                    report.skipped.push(target);
                    continue;
                }
                if target.file_name().is_some_and(|name| name == retired::NAME) {
                    let current_sources = match &sources {
                        Some(sources) => sources,
                        None => sources.insert(assets.materialize_bundle()?),
                    };
                    match retired::replace(
                        &global,
                        target.parent().expect("registered target parent"),
                        &assets,
                        current_sources
                            .get(super::catalog::MAIN)
                            .expect("main source"),
                        &mut publish,
                    )? {
                        retired::Replacement::Missing => report.skipped.push(target),
                        retired::Replacement::Conflict(path) => report.conflicts.push(path),
                        retired::Replacement::Replaced { target, changed } => {
                            if !report.refreshed.iter().any(|item| item.target == target) {
                                report.refreshed.push(RefreshedSkill { target, changed });
                            }
                        }
                    }
                    continue;
                }
                files::safe_target(assets.root(), &target)?;
                let current_sources = match &sources {
                    Some(sources) => sources,
                    None => sources.insert(assets.materialize_bundle()?),
                };
                let current = match target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| current_sources.get(name))
                {
                    Some(current) => current,
                    None => {
                        report.conflicts.push(target);
                        continue;
                    }
                };
                let changed = !files::current_link(&target, current);
                if changed {
                    publish(&target, current)?;
                }
                report.refreshed.push(RefreshedSkill { target, changed });
            }
            if report.conflicts.is_empty() {
                Ok(())
            } else {
                Err(io::Error::other(
                    "Unmanaged or modified skill targets were preserved; inspect the conflicts before reinstalling.",
                ))
            }
        })
    })();
    match pending {
        Ok(()) => Ok(report),
        Err(cause) => Err(RefreshFailure { cause, report }),
    }
}
