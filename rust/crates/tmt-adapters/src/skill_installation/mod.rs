//! Managed local agent guidance; independent of identities, SQLite and tmux.

mod assets;
pub mod catalog;
mod drift;
pub(crate) mod files;
mod owned;
mod providers;
mod refresh;
mod registry;
mod uninstall;
pub use refresh::{RefreshFailure, RefreshReport, RefreshedSkill, refresh};
pub use uninstall::{SkillsRemoval, plan_uninstall, uninstall};
#[cfg(test)]
mod refresh_tests;
pub use drift::inspect_local_drift;
/// The skill name and file path rules, shared with native release archives.
pub(crate) use owned::{
    MAXIMUM_FILE_BYTES, MAXIMUM_FILES, MAXIMUM_SKILLS, valid_file as valid_skill_file,
    valid_name as valid_skill_name,
};
pub use owned::{
    OwnedFailure, OwnedReport, OwnedSkill, OwnedTarget, Refusal, install_owned, owned_by,
    owned_roots, owners, refusal, remove_owned,
};
#[cfg(test)]
mod owned_tests;
pub use providers::{ProviderEnvironment, SKILL_NAME};
#[cfg(test)]
mod assets_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod install_tests;
#[cfg(test)]
mod publication_tests;

pub fn bundled_skill() -> &'static [u8] {
    catalog::bundled(catalog::MAIN).expect("main skill").bytes
}

pub fn bundled_skill_named(name: &str) -> Option<&'static [u8]> {
    catalog::bundled(name).map(|skill| skill.bytes)
}

use crate::drivers::{DriverDefinition, Registry};
use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct InstalledSkill {
    pub name: &'static str,
    pub agent: Option<&'static DriverDefinition>,
    pub target: PathBuf,
    pub changed: bool,
    pub backup: Option<PathBuf>,
    pub legacy_backups: Vec<PathBuf>,
}

#[derive(Debug, Default)]
pub struct InstallReport {
    pub installed: Vec<InstalledSkill>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct InstallFailure {
    cause: io::Error,
    pub report: InstallReport,
    pub pending_backup: Option<PathBuf>,
}

impl fmt::Display for InstallFailure {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(output, "Skill installation failed: {}", self.cause)?;
        for item in &self.report.installed {
            write!(output, "; installed target: {}", item.target.display())?;
            for backup in item.backup.iter().chain(&item.legacy_backups) {
                write!(output, "; recoverable backup: {}", backup.display())?;
            }
        }
        if let Some(backup) = &self.pending_backup {
            write!(
                output,
                "; failed target's recoverable backup: {}",
                backup.display()
            )?;
        }
        Ok(())
    }
}
impl Error for InstallFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.cause)
    }
}

fn selected(
    env: &ProviderEnvironment,
    provider: Option<&str>,
    directory: Option<&Path>,
) -> io::Result<Vec<(Option<&'static DriverDefinition>, PathBuf)>> {
    if let Some(directory) = directory {
        if provider.is_some() || directory.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Use a non-empty --dir without a provider or all.",
            ));
        }
        return Ok(vec![(None, env.custom_target(directory))]);
    }
    let providers = match provider {
        None => env.detect(),
        Some(value) if value.eq_ignore_ascii_case("all") => Registry::builtin().iter().collect(),
        Some(value) => vec![Registry::builtin().find(value).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Unknown agent: {value}"),
            )
        })?],
    };
    if providers.is_empty() {
        return Ok(vec![(None, env.universal_target())]);
    }
    Ok(providers
        .into_iter()
        .map(|provider| (Some(provider), env.target(provider)))
        .collect())
}

fn managed_link(target: &Path, assets: &assets::SkillAssets) -> io::Result<Option<PathBuf>> {
    if !files::exists(target)? || !fs::symlink_metadata(target)?.is_symlink() {
        return Ok(None);
    }
    let source = crate::config::normalize(
        &target
            .parent()
            .expect("selected target parent")
            .join(fs::read_link(target)?),
    );
    Ok(assets.owns(&source).then_some(source))
}

struct PublicationContext<'a> {
    assets: &'a assets::SkillAssets,
    force: bool,
    report: &'a mut InstallReport,
    pending_backup: &'a mut Option<PathBuf>,
}

fn publish_managed_target(
    context: &mut PublicationContext<'_>,
    target: &Path,
    source: &Path,
    name: &'static str,
    agent: Option<&'static DriverDefinition>,
    publish: &mut impl FnMut(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let prior = managed_link(target, context.assets)?;
    let changed = prior.as_deref() != Some(source);
    if changed {
        if prior.is_none() && files::exists(target)? {
            if !context.force {
                return Err(io::Error::other(format!(
                    "Refusing to replace existing unmanaged path: {} (use --force)",
                    target.display()
                )));
            }
            *context.pending_backup = Some(files::backup(target)?);
        }
        publish(target, source)?;
    }
    context.report.installed.push(InstalledSkill {
        name,
        agent,
        target: target.to_path_buf(),
        changed,
        backup: context.pending_backup.take(),
        legacy_backups: Vec::new(),
    });
    Ok(())
}

/// Materialize and publish only requested integrations. The lock covers all
/// cooperating native installers; this is not a hostile-filesystem sandbox.
pub fn install(
    env: &ProviderEnvironment,
    global: &Path,
    provider: Option<&str>,
    directory: Option<&Path>,
    force: bool,
) -> Result<InstallReport, InstallFailure> {
    install_with_publisher(env, global, provider, directory, force, files::link)
}

/// Install the optional Office and art-creation guidance into detected provider roots and any
/// custom root that still contains an owned core skill. Explicit Office setup
/// may add this sibling; ordinary core installation and binary refresh do not.
pub fn install_office(
    env: &ProviderEnvironment,
    global: &Path,
    force: bool,
) -> Result<InstallReport, InstallFailure> {
    install_office_with_publisher(env, global, force, files::link)
}

/// Skill directories that receive optional (Office and extension-owned)
/// skills: every detected provider root, plus custom roots that still hold a
/// managed core skill. Call with the installer lock held.
fn optional_roots(
    env: &ProviderEnvironment,
    global: &Path,
    assets: &assets::SkillAssets,
) -> io::Result<BTreeMap<PathBuf, Option<&'static DriverDefinition>>> {
    let mut roots = BTreeMap::<PathBuf, Option<&'static DriverDefinition>>::new();
    for (agent, main) in selected(env, None, None)? {
        roots.insert(
            main.parent().expect("skill target parent").to_path_buf(),
            agent,
        );
    }
    for registered in registry::read(global)? {
        let Some(name) = registered.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !matches!(
            name,
            "tmux-team" | "tmt-inbox" | "tmt-office" | "tmt-prop-create" | "tmt-avatar-create"
        ) || managed_link(&registered, assets)?.is_none()
        {
            continue;
        }
        let parent = registered.parent().expect("registered skill target parent");
        roots.entry(parent.to_path_buf()).or_insert(None);
    }
    Ok(roots)
}

fn install_office_with_publisher(
    env: &ProviderEnvironment,
    global: &Path,
    force: bool,
    mut publish: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<InstallReport, InstallFailure> {
    let mut report = InstallReport::default();
    let mut pending_backup = None;
    let pending = (|| {
        let global = files::resolved(global)?;
        let assets = assets::SkillAssets::new(&global);
        registry::read(&global)?;
        files::with_lock(&global, || {
            let mut targets = BTreeMap::<PathBuf, Option<&'static DriverDefinition>>::new();
            // Once an owner holds a name (Office adopted through the owner
            // door), core's bundle no longer publishes it.
            let owned = owned::owned_names(&global)?;
            for (root, agent) in optional_roots(env, &global, &assets)? {
                for name in catalog::Catalog::bundled().names(catalog::Group::Office) {
                    if !owned.contains(name) {
                        targets.entry(root.join(name)).or_insert(agent);
                    }
                }
            }
            for target in targets.keys() {
                files::safe_target(assets.root(), target)?;
            }
            let sources = assets.materialize_bundle()?;
            registry::remember(&global, targets.keys().cloned())?;
            let mut context = PublicationContext {
                assets: &assets,
                force,
                report: &mut report,
                pending_backup: &mut pending_backup,
            };
            for (target, agent) in targets {
                let skill = target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(catalog::bundled)
                    .expect("an Office target is named after a bundled skill");
                let name = skill.name;
                let source = sources.get(name).expect("a bundled source");
                publish_managed_target(&mut context, &target, source, name, agent, &mut publish)?;
            }
            Ok(())
        })
    })();
    match pending {
        Ok(()) => Ok(report),
        Err(cause) => Err(InstallFailure {
            cause,
            report,
            pending_backup,
        }),
    }
}

// Keep publication at the existing file boundary so failure tests exercise the
// real selection, backup, registry, report, and lock lifecycle around it.
fn install_with_publisher(
    env: &ProviderEnvironment,
    global: &Path,
    provider: Option<&str>,
    directory: Option<&Path>,
    force: bool,
    mut publish: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<InstallReport, InstallFailure> {
    let mut report = InstallReport::default();
    let mut pending_backup = None;
    let pending = (|| {
        let targets = selected(env, provider, directory)?
            .into_iter()
            .flat_map(|(agent, main)| {
                let inbox = main
                    .parent()
                    .expect("skill target parent")
                    .join("tmt-inbox");
                [(agent, main, false), (agent, inbox, true)]
            })
            .collect::<Vec<_>>();
        let global = files::resolved(global)?;
        let assets = assets::SkillAssets::new(&global);
        // These guards precede even lock/cache creation or forced backups.
        for (_, target, _) in &targets {
            files::safe_target(assets.root(), target)?;
        }
        files::with_lock(&global, || {
            registry::read(&global)?;
            let sources = assets.materialize_bundle()?;
            let main_source = sources.get(catalog::MAIN).expect("main source");
            let inbox_source = sources.get(catalog::INBOX).expect("inbox source");
            registry::remember(&global, targets.iter().map(|(_, target, _)| target.clone()))?;
            let mut context = PublicationContext {
                assets: &assets,
                force,
                report: &mut report,
                pending_backup: &mut pending_backup,
            };
            for (agent, target, inbox) in targets {
                let source = if inbox { inbox_source } else { main_source };
                publish_managed_target(
                    &mut context,
                    &target,
                    source,
                    if inbox { "tmt-inbox" } else { "tmux-team" },
                    agent,
                    &mut publish,
                )?;
                if !inbox && let Some(agent) = agent {
                    for legacy in env.legacy_targets(agent) {
                        if !files::exists(&legacy)?
                            || files::entry_location(&legacy)? == files::entry_location(&target)?
                        {
                            continue;
                        }
                        if force {
                            files::safe_target(assets.root(), &legacy)?;
                            let backup = files::backup(&legacy)?;
                            context
                                .report
                                .installed
                                .last_mut()
                                .expect("published skill")
                                .legacy_backups
                                .push(backup);
                        } else {
                            context.report.warnings.push(format!("Legacy {} guidance found at {}; keeping it. Inspect before running tmt install {} --force.", agent.name(), legacy.display(), agent.name()));
                        }
                    }
                }
            }
            Ok(())
        })
    })();
    match pending {
        Ok(()) => Ok(report),
        Err(cause) => Err(InstallFailure {
            cause,
            report,
            pending_backup,
        }),
    }
}
