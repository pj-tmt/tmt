//! A driver's managed mod: files setup keeps beside the provider's settings once the user
//! consented (for Claude, the allowance mod, #5187). TMT owns only a directory it marked; whatever
//! else sits at that path, or behind a link, is left alone and reported.

use crate::{
    bounded_file,
    runtime::RuntimeRegistry,
    skill_installation::files::{atomic_write, exists},
};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tmt_core::{binding::session::HarnessId, content_digest::sha256};
use uuid::Uuid;

/// The files a driver's mod holds, embedded in this build so that the copy is locked to the
/// installed TMT.
pub struct ProviderMod {
    /// The directory beside the settings file, e.g. `skills/tmt-allowance`.
    pub directory: &'static str,
    /// Relative path and bytes of each file.
    pub files: &'static [(&'static str, &'static [u8])],
    /// One line for setup's preview.
    pub purpose: &'static str,
}

const MARKER: &str = ".tmt-managed.json";
const MARKER_LIMIT: usize = 4096;
const OWNER: &str = "tmt";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModMarker {
    owner: String,
    digest: String,
}

/// The mod `provider` keeps, when it has one.
pub fn for_driver(provider: &str) -> Option<&'static ProviderMod> {
    let harness = HarnessId::new(provider).ok()?;
    RuntimeRegistry::first_party()
        .lifecycle(&harness)
        .and_then(|lifecycle| lifecycle.provider_mod())
}

fn digest(provider_mod: &ProviderMod) -> String {
    let mut framed = Vec::new();
    for (path, bytes) in provider_mod.files {
        framed.extend_from_slice(format!("{path}\0{}\0", bytes.len()).as_bytes());
        framed.extend_from_slice(bytes);
    }
    sha256(&framed)
}

/// What is at the mod's path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModState {
    Absent,
    /// TMT's copy, byte for byte what this build ships.
    Current,
    /// TMT's copy from another build, or with a file changed.
    Stale,
    /// Not marked as TMT's: never touched.
    Foreign,
}

fn marked(directory: &Path) -> bool {
    bounded_file::read_no_follow(&directory.join(MARKER), MARKER_LIMIT)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ModMarker>(&bytes).ok())
        .is_some_and(|marker| marker.owner == OWNER)
}

fn state_at(target: &Path, provider_mod: &ProviderMod) -> io::Result<ModState> {
    let Some(parent) = target.parent() else {
        return Ok(ModState::Foreign);
    };
    match fs::symlink_metadata(parent) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(ModState::Absent),
        Err(error) => return Err(error),
        Ok(metadata) if !metadata.is_dir() => return Ok(ModState::Foreign),
        Ok(_) => {}
    }
    match fs::symlink_metadata(target) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(ModState::Absent),
        Err(error) => return Err(error),
        Ok(metadata) if !metadata.is_dir() => return Ok(ModState::Foreign),
        Ok(_) => {}
    }
    if !marked(target) {
        return Ok(ModState::Foreign);
    }
    let recorded = bounded_file::read_no_follow(&target.join(MARKER), MARKER_LIMIT)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ModMarker>(&bytes).ok());
    let identical = recorded.is_some_and(|marker| marker.digest == digest(provider_mod))
        && provider_mod.files.iter().all(|(path, bytes)| {
            bounded_file::read_no_follow(&target.join(path), bytes.len() + 1)
                .is_ok_and(|current| current == *bytes)
        });
    Ok(if identical {
        ModState::Current
    } else {
        ModState::Stale
    })
}

/// Where the mod goes for a provider whose settings file is `settings`.
pub fn target(settings: &Path, provider_mod: &ProviderMod) -> Option<PathBuf> {
    Some(settings.parent()?.join(provider_mod.directory))
}

/// What setup would do about a mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModStep {
    Install,
    Refresh,
    Remove,
    /// Wanted, but something that is not TMT's is in the way: nothing changes.
    Blocked,
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModOutcome {
    Installed,
    Refreshed,
    Removed,
    Blocked,
}

/// A planned step for one provider's mod, rechecked when applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMod {
    provider: String,
    provider_mod: &'static ProviderMod,
    target: PathBuf,
    pub step: ModStep,
}

impl PartialEq for ProviderMod {
    fn eq(&self, other: &Self) -> bool {
        self.directory == other.directory
    }
}
impl Eq for ProviderMod {}
impl std::fmt::Debug for ProviderMod {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.directory)
    }
}

impl ModOutcome {
    /// The word JSON output and messages use.
    pub fn label(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Refreshed => "refreshed",
            Self::Removed => "removed",
            Self::Blocked => "blocked",
        }
    }
}

impl PendingMod {
    /// The driver whose mod this is.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The directory the step concerns.
    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn purpose(&self) -> &'static str {
        self.provider_mod.purpose
    }

    /// Whether applying changes anything.
    pub fn changes(&self) -> bool {
        self.step != ModStep::Blocked
    }

    /// Publishes or removes the copy, after checking again that it is still TMT's to change.
    pub fn apply(&self) -> io::Result<ModOutcome> {
        let state = state_at(&self.target, self.provider_mod)?;
        match (self.step, state) {
            (ModStep::Install | ModStep::Refresh, ModState::Foreign) | (ModStep::Blocked, _) => {
                Ok(ModOutcome::Blocked)
            }
            (ModStep::Install | ModStep::Refresh, ModState::Current) => Ok(ModOutcome::Installed),
            (ModStep::Install | ModStep::Refresh, ModState::Absent | ModState::Stale) => {
                publish(&self.target, self.provider_mod, state == ModState::Stale)?;
                Ok(if state == ModState::Stale {
                    ModOutcome::Refreshed
                } else {
                    ModOutcome::Installed
                })
            }
            (ModStep::Remove, ModState::Current | ModState::Stale) => {
                take(&self.target)?;
                Ok(ModOutcome::Removed)
            }
            (ModStep::Remove, ModState::Absent | ModState::Foreign) => Ok(ModOutcome::Removed),
        }
    }
}

/// The step that brings `provider`'s mod in line with `wanted` (consented usage), or
/// nothing when it already is, or when the provider has none.
pub fn pending(provider: &str, settings: &Path, wanted: bool) -> io::Result<Option<PendingMod>> {
    let Some(provider_mod) = for_driver(provider) else {
        return Ok(None);
    };
    let Some(target) = target(settings, provider_mod) else {
        return Ok(None);
    };
    let step = match (wanted, state_at(&target, provider_mod)?) {
        (true, ModState::Absent) => ModStep::Install,
        (true, ModState::Stale) => ModStep::Refresh,
        (true, ModState::Foreign) => ModStep::Blocked,
        (false, ModState::Current | ModState::Stale) => ModStep::Remove,
        (true, ModState::Current) | (false, ModState::Absent | ModState::Foreign) => {
            return Ok(None);
        }
    };
    Ok(Some(PendingMod {
        provider: provider.to_owned(),
        provider_mod,
        target,
        step,
    }))
}

/// [`pending`] for an upgrade: an installed copy follows the new build or the usage choice, and a
/// missing one is never created, because the user approved setup, not this build's files.
pub fn pending_refresh(
    provider: &str,
    settings: &Path,
    wanted: bool,
) -> io::Result<Option<PendingMod>> {
    Ok(pending(provider, settings, wanted)?.filter(|pending| pending.step != ModStep::Install))
}

/// Removes the copy for an uninstall; a directory that is not TMT's stays.
pub fn pending_removal(provider: &str, settings: &Path) -> io::Result<Option<PendingMod>> {
    pending(provider, settings, false)
}

fn publish(target: &Path, provider_mod: &ProviderMod, replacing: bool) -> io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("The mod path has no parent."))?;
    fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".tmt-mod-{}", Uuid::new_v4()));
    let built = (|| {
        fs::create_dir(&stage)?;
        for (path, bytes) in provider_mod.files {
            atomic_write(&stage.join(path), bytes)?;
        }
        let marker = ModMarker {
            owner: OWNER.to_owned(),
            digest: digest(provider_mod),
        };
        atomic_write(
            &stage.join(MARKER),
            &serde_json::to_vec(&marker).map_err(io::Error::other)?,
        )
    })();
    if let Err(error) = built {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    let kept = if replacing {
        Some(set_aside(target)?)
    } else {
        None
    };
    match fs::rename(&stage, target) {
        Ok(()) => {
            if let Some(old) = kept {
                let _ = fs::remove_dir_all(old);
            }
            Ok(())
        }
        Err(error) => {
            if let Some(old) = kept {
                let _ = fs::rename(old, target);
            }
            let _ = fs::remove_dir_all(&stage);
            Err(error)
        }
    }
}

/// Moves TMT's directory away, putting it back and failing when it turns out not to be TMT's
/// after all (it changed since it was checked).
fn set_aside(target: &Path) -> io::Result<PathBuf> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("The mod path has no parent."))?;
    let old = parent.join(format!(".tmt-mod-old-{}", Uuid::new_v4()));
    fs::rename(target, &old)?;
    if !marked(&old) {
        let _ = fs::rename(&old, target);
        return Err(io::Error::other(format!(
            "{} changed while it was being replaced; it was left as it is.",
            target.display()
        )));
    }
    Ok(old)
}

fn take(target: &Path) -> io::Result<()> {
    if !exists(target)? {
        return Ok(());
    }
    fs::remove_dir_all(set_aside(target)?)
}
