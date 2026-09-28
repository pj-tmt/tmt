//! One embedded authored source, materialized without a checkout dependency.

use crate::bounded_file;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tmt_core::content_digest::sha256 as digest;
use uuid::Uuid;

pub(super) const SKILL: &[u8] = include_bytes!("../../../../../skills/tmux-team/SKILL.md");
pub(super) const INBOX_SKILL: &[u8] = include_bytes!("../../../../../skills/tmt-inbox/SKILL.md");
pub(super) const OFFICE_SKILL: &[u8] =
    include_bytes!("../../../../../extensions/tmt-office/skills/tmt-office/SKILL.md");
pub(super) const PROP_CREATE_SKILL: &[u8] =
    include_bytes!("../../../../../extensions/tmt-office/skills/tmt-prop-create/SKILL.md");
pub(super) const AVATAR_CREATE_SKILL: &[u8] =
    include_bytes!("../../../../../extensions/tmt-office/skills/tmt-avatar-create/SKILL.md");

fn framed_digest(parts: &[&[u8]]) -> String {
    let mut bytes =
        Vec::with_capacity(parts.iter().map(|part| part.len()).sum::<usize>() + parts.len() * 8);
    for part in parts {
        bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
        bytes.extend_from_slice(part);
    }
    digest(&bytes)
}

fn bundle_digest() -> String {
    framed_digest(&[
        SKILL,
        INBOX_SKILL,
        OFFICE_SKILL,
        PROP_CREATE_SKILL,
        AVATAR_CREATE_SKILL,
    ])
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

type BundleSourceBytes = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type PriorPropBundleSourceBytes = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

fn bundle_source_bytes(version: &Path) -> io::Result<BundleSourceBytes> {
    if inventory(version)?
        != [
            "tmt-avatar-create",
            "tmt-inbox",
            "tmt-office",
            "tmt-prop-create",
            "tmux-team",
        ]
    {
        return Err(invalid(version));
    }
    Ok((
        source_bytes(&version.join("tmux-team"))?,
        source_bytes(&version.join("tmt-inbox"))?,
        source_bytes(&version.join("tmt-office"))?,
        source_bytes(&version.join("tmt-prop-create"))?,
        source_bytes(&version.join("tmt-avatar-create"))?,
    ))
}

fn prior_prop_bundle_source_bytes(version: &Path) -> io::Result<PriorPropBundleSourceBytes> {
    if inventory(version)? != ["tmt-inbox", "tmt-office", "tmt-prop-create", "tmux-team"] {
        return Err(invalid(version));
    }
    Ok((
        source_bytes(&version.join("tmux-team"))?,
        source_bytes(&version.join("tmt-inbox"))?,
        source_bytes(&version.join("tmt-office"))?,
        source_bytes(&version.join("tmt-prop-create"))?,
    ))
}

fn prior_office_bundle_source_bytes(version: &Path) -> io::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    if inventory(version)? != ["tmt-inbox", "tmt-office", "tmux-team"] {
        return Err(invalid(version));
    }
    Ok((
        source_bytes(&version.join("tmux-team"))?,
        source_bytes(&version.join("tmt-inbox"))?,
        source_bytes(&version.join("tmt-office"))?,
    ))
}

fn prior_bundle_source_bytes(version: &Path) -> io::Result<(Vec<u8>, Vec<u8>)> {
    if inventory(version)? != ["tmt-inbox", "tmux-team"] {
        return Err(invalid(version));
    }
    Ok((
        source_bytes(&version.join("tmux-team"))?,
        source_bytes(&version.join("tmt-inbox"))?,
    ))
}

fn legacy_source_bytes(version: &Path) -> io::Result<Vec<u8>> {
    if inventory(version)? != ["tmux-team"] {
        return Err(invalid(version));
    }
    source_bytes(&version.join("tmux-team"))
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

    pub(super) fn source(&self) -> PathBuf {
        self.root.join(bundle_digest()).join("tmux-team")
    }

    pub(super) fn inbox_source(&self) -> PathBuf {
        self.root.join(bundle_digest()).join("tmt-inbox")
    }

    pub(super) fn office_source(&self) -> PathBuf {
        self.root.join(bundle_digest()).join("tmt-office")
    }

    pub(super) fn prop_create_source(&self) -> PathBuf {
        self.root.join(bundle_digest()).join("tmt-prop-create")
    }

    pub(super) fn avatar_create_source(&self) -> PathBuf {
        self.root.join(bundle_digest()).join("tmt-avatar-create")
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
        let source_name = match source.file_name().and_then(|name| name.to_str()) {
            Some(
                name @ ("tmux-team" | "tmt-inbox" | "tmt-office" | "tmt-prop-create"
                | "tmt-avatar-create"),
            ) => name,
            _ => return false,
        };
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
        if bundle_source_bytes(version).is_ok_and(|(core, inbox, office, prop, avatar)| {
            framed_digest(&[&core, &inbox, &office, &prop, &avatar]) == expected
        }) {
            return true;
        }
        if source_name != "tmt-avatar-create"
            && prior_prop_bundle_source_bytes(version).is_ok_and(|(core, inbox, office, prop)| {
                framed_digest(&[&core, &inbox, &office, &prop]) == expected
            })
        {
            return true;
        }
        if !matches!(source_name, "tmt-prop-create" | "tmt-avatar-create")
            && prior_office_bundle_source_bytes(version).is_ok_and(|(core, inbox, office)| {
                framed_digest(&[&core, &inbox, &office]) == expected
            })
        {
            return true;
        }
        if !matches!(
            source_name,
            "tmt-office" | "tmt-prop-create" | "tmt-avatar-create"
        ) && prior_bundle_source_bytes(version)
            .is_ok_and(|(core, inbox)| framed_digest(&[&core, &inbox]) == expected)
        {
            return true;
        }
        source_name == "tmux-team"
            && legacy_source_bytes(version).is_ok_and(|bytes| digest(&bytes) == expected)
    }

    /// Called while the installer lock is held. Existing sources are never
    /// overwritten, even with force; that flag authorizes target backups only.
    #[cfg(test)]
    pub(super) fn materialize(&self) -> io::Result<PathBuf> {
        self.materialize_bundle().map(|sources| sources.0)
    }

    pub(super) fn materialize_bundle(
        &self,
    ) -> io::Result<(PathBuf, PathBuf, PathBuf, PathBuf, PathBuf)> {
        let destination = self.source();
        let inbox_destination = self.inbox_source();
        let office_destination = self.office_source();
        let prop_create_destination = self.prop_create_source();
        let avatar_create_destination = self.avatar_create_source();
        if fs::symlink_metadata(&destination).is_ok()
            || fs::symlink_metadata(&inbox_destination).is_ok()
            || fs::symlink_metadata(&office_destination).is_ok()
            || fs::symlink_metadata(&prop_create_destination).is_ok()
            || fs::symlink_metadata(&avatar_create_destination).is_ok()
        {
            let (core, inbox, office, prop, avatar) =
                bundle_source_bytes(destination.parent().expect("digest source directory"))?;
            if core != SKILL
                || inbox != INBOX_SKILL
                || office != OFFICE_SKILL
                || prop != PROP_CREATE_SKILL
                || avatar != AVATAR_CREATE_SKILL
            {
                return Err(invalid(&destination));
            }
            return Ok((
                destination,
                inbox_destination,
                office_destination,
                prop_create_destination,
                avatar_create_destination,
            ));
        }
        fs::create_dir_all(&self.root)?;
        let stage = self.root.join(format!(".stage-{}", Uuid::new_v4()));
        fs::create_dir(&stage)?;
        let pending = (|| {
            let directory = stage.join("tmux-team");
            fs::create_dir(&directory)?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("SKILL.md"))?;
            file.write_all(SKILL)?;
            file.sync_all()?;
            drop(file);
            let inbox_directory = stage.join("tmt-inbox");
            fs::create_dir(&inbox_directory)?;
            let mut inbox_file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(inbox_directory.join("SKILL.md"))?;
            inbox_file.write_all(INBOX_SKILL)?;
            inbox_file.sync_all()?;
            drop(inbox_file);
            let office_directory = stage.join("tmt-office");
            fs::create_dir(&office_directory)?;
            let mut office_file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(office_directory.join("SKILL.md"))?;
            office_file.write_all(OFFICE_SKILL)?;
            office_file.sync_all()?;
            drop(office_file);
            let prop_directory = stage.join("tmt-prop-create");
            fs::create_dir(&prop_directory)?;
            let mut prop_file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(prop_directory.join("SKILL.md"))?;
            prop_file.write_all(PROP_CREATE_SKILL)?;
            prop_file.sync_all()?;
            drop(prop_file);
            let avatar_directory = stage.join("tmt-avatar-create");
            fs::create_dir(&avatar_directory)?;
            let mut avatar_file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(avatar_directory.join("SKILL.md"))?;
            avatar_file.write_all(AVATAR_CREATE_SKILL)?;
            avatar_file.sync_all()?;
            drop(avatar_file);
            let parent = destination.parent().expect("digest source directory");
            // An invalid digest directory is not disposable user data.
            if fs::symlink_metadata(parent).is_ok() {
                return Err(invalid(parent));
            }
            fs::rename(&stage, parent)?;
            Ok((
                destination,
                inbox_destination,
                office_destination,
                prop_create_destination,
                avatar_create_destination,
            ))
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
