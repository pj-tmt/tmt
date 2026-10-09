//! One embedded authored source, materialized without a checkout dependency.

use super::catalog::{self, BUNDLED, MAIN, PRIOR_LAYOUTS};
use crate::bounded_file;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tmt_core::content_digest::sha256 as digest;
use uuid::Uuid;

fn framed_digest<B: AsRef<[u8]>>(parts: &[B]) -> String {
    let parts: Vec<&[u8]> = parts.iter().map(AsRef::as_ref).collect();
    let mut bytes =
        Vec::with_capacity(parts.iter().map(|part| part.len()).sum::<usize>() + parts.len() * 8);
    for part in parts {
        bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
        bytes.extend_from_slice(part);
    }
    digest(&bytes)
}

fn bundle_digest() -> String {
    framed_digest(&BUNDLED.iter().map(|skill| skill.bytes).collect::<Vec<_>>())
}

fn invalid(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "Managed skill source has been modified or is invalid: {}",
            path.display()
        ),
    )
}

/// Exact source inventory matters: an unexpected provider-readable file is not
/// made trusted merely because SKILL.md still matches its original digest.
fn source_bytes(source: &Path) -> io::Result<Vec<u8>> {
    let parent = source.parent().ok_or_else(|| invalid(source))?;
    if !fs::symlink_metadata(parent)?.is_dir() || !fs::symlink_metadata(source)?.is_dir() {
        return Err(invalid(source));
    }
    let mut entries = fs::read_dir(source)?;
    let entry = entries.next().ok_or_else(|| invalid(source))??;
    if entry.file_name() != "SKILL.md" || !entry.file_type()?.is_file() || entries.next().is_some()
    {
        return Err(invalid(source));
    }
    bounded_file::read(&entry.path(), 1_048_576)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn inventory(version: &Path) -> io::Result<Vec<String>> {
    if !fs::symlink_metadata(version)?.is_dir() {
        return Err(invalid(version));
    }
    let mut names = fs::read_dir(version)?
        .map(|entry| {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                return Err(invalid(version));
            }
            entry
                .file_name()
                .into_string()
                .map_err(|_| invalid(version))
        })
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

/// The source bytes of the first `count` bundled skills, in digest order,
/// when the version directory holds exactly those skills.
fn layout_bytes(version: &Path, count: usize) -> io::Result<Vec<Vec<u8>>> {
    named_layout_bytes(version, count, MAIN)
}

fn named_layout_bytes(version: &Path, count: usize, main: &str) -> io::Result<Vec<Vec<u8>>> {
    let layout = &BUNDLED[..count];
    let name = |skill: &catalog::BundledSkill| if skill.name == MAIN { main } else { skill.name };
    let mut expected: Vec<&str> = layout.iter().map(name).collect();
    expected.sort_unstable();
    if inventory(version)? != expected {
        return Err(invalid(version));
    }
    layout
        .iter()
        .map(|skill| source_bytes(&version.join(name(skill))))
        .collect()
}

/// The current bundle's sources, one per bundled skill.
pub(super) struct BundleSources {
    paths: BTreeMap<&'static str, PathBuf>,
}

impl BundleSources {
    pub(super) fn get(&self, name: &str) -> Option<&PathBuf> {
        self.paths.get(name)
    }
}

pub(super) struct SkillAssets {
    root: PathBuf,
}

impl SkillAssets {
    pub(super) fn root(&self) -> &Path {
        &self.root
    }
    pub(super) fn new(global: &Path) -> Self {
        Self {
            root: global.join("skill-assets"),
        }
    }

    /// Core's main skill source in the current bundle.
    pub(super) fn source(&self) -> PathBuf {
        self.source_of(MAIN)
    }

    pub(super) fn source_of(&self, name: &str) -> PathBuf {
        self.root.join(bundle_digest()).join(name)
    }

    /// This is local managed-file evidence, not authentication of remote code.
    pub(super) fn owns(&self, source: &Path) -> bool {
        let Some(version) = source.parent() else {
            return false;
        };
        if version.parent() != Some(self.root.as_path()) {
            return false;
        }
        let Some(expected) = version.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        let Some(source_name) = source
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| catalog::bundled(name).is_some())
        else {
            return false;
        };
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
        // A removed generation leaves dangling links, not user content. Only
        // canonical paths inside this home's store qualify; a symlinked store
        // ancestor escaping the store cannot acquire ownership.
        if !super::files::exists(version).unwrap_or(true) {
            return super::files::resolved(self.root()).is_ok_and(|root| {
                super::files::resolved(source)
                    .is_ok_and(|resolved| resolved == root.join(expected).join(source_name))
            });
        }
        // The current layout, then each earlier one that held this skill.
        let contains = |count: usize| {
            BUNDLED[..count]
                .iter()
                .any(|skill| skill.name == source_name)
        };
        if [BUNDLED.len()]
            .into_iter()
            .chain(PRIOR_LAYOUTS)
            .filter(|count| contains(*count))
            .any(|count| {
                layout_bytes(version, count).is_ok_and(|bytes| framed_digest(&bytes) == expected)
                    // The one-release main-name cutover keeps sibling names.
                    // Their old generations still require the full framed proof.
                    || (source_name != MAIN
                        && named_layout_bytes(version, count, super::retired::NAME)
                            .is_ok_and(|bytes| framed_digest(&bytes) == expected))
            })
        {
            return true;
        }
        source_name == MAIN
            && inventory(version).is_ok_and(|names| names == [MAIN])
            && source_bytes(&version.join(MAIN)).is_ok_and(|bytes| digest(&bytes) == expected)
    }

    /// One-shot retirement proof for the former core name. A real generation
    /// must retain its exact inventory and framed digest; dangling generations
    /// do not authorize deletion of the old target.
    pub(super) fn owns_retired(&self, source: &Path) -> bool {
        let Some(version) = source.parent() else {
            return false;
        };
        if source
            .file_name()
            .is_none_or(|name| name != super::retired::NAME)
            || version.parent() != Some(self.root.as_path())
        {
            return false;
        }
        let Some(expected) = version.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
        [BUNDLED.len()]
            .into_iter()
            .chain(PRIOR_LAYOUTS)
            .any(|count| {
                named_layout_bytes(version, count, super::retired::NAME)
                    .is_ok_and(|bytes| framed_digest(&bytes) == expected)
            })
            || (inventory(version).is_ok_and(|names| names == [super::retired::NAME])
                && source_bytes(source).is_ok_and(|bytes| digest(&bytes) == expected))
    }

    /// Called while the installer lock is held. Existing sources are never
    /// overwritten, even with force; that flag authorizes target backups only.
    #[cfg(test)]
    pub(super) fn materialize(&self) -> io::Result<PathBuf> {
        self.materialize_bundle()
            .map(|sources| sources.get(MAIN).expect("main source").clone())
    }

    pub(super) fn materialize_bundle(&self) -> io::Result<BundleSources> {
        let sources = BundleSources {
            paths: BUNDLED
                .iter()
                .map(|skill| (skill.name, self.source_of(skill.name)))
                .collect(),
        };
        let parent = self.root.join(bundle_digest());
        if sources
            .paths
            .values()
            .any(|path| fs::symlink_metadata(path).is_ok())
        {
            let bytes = layout_bytes(&parent, BUNDLED.len())?;
            if bytes
                .iter()
                .zip(&BUNDLED)
                .any(|(found, skill)| found.as_slice() != skill.bytes)
            {
                return Err(invalid(&self.source()));
            }
            return Ok(sources);
        }
        fs::create_dir_all(&self.root)?;
        let stage = self.root.join(format!(".stage-{}", Uuid::new_v4()));
        fs::create_dir(&stage)?;
        let pending = (|| {
            for skill in &BUNDLED {
                let directory = stage.join(skill.name);
                fs::create_dir(&directory)?;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(directory.join("SKILL.md"))?;
                file.write_all(skill.bytes)?;
                file.sync_all()?;
            }
            // An invalid digest directory is not disposable user data.
            if fs::symlink_metadata(&parent).is_ok() {
                return Err(invalid(&parent));
            }
            fs::rename(&stage, &parent)?;
            Ok(sources)
        })();
        if stage.exists()
            && let Err(cleanup) = fs::remove_dir_all(&stage)
        {
            return Err(io::Error::other(format!(
                "{}; could not remove owned staging directory {}: {cleanup}",
                pending
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "Skill publication failed".into()),
                stage.display()
            )));
        }
        pending
    }
}
