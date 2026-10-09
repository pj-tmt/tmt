//! Extension-owned skills: bytes an installed extension supplies through the
//! local API, materialized in core's content-addressed store and published
//! into the same provider roots as the optional Office skills, with the owner
//! recorded. Core's own names stay core's; another owner's names are refused.
//! The owner cannot be authenticated (same-user API), so ownership is a
//! bookkeeping boundary between cooperating installers, not a security one.

use super::catalog::{Catalog, Group};
use super::{
    ProviderEnvironment, assets::SkillAssets, files, managed_link, optional_roots, registry,
};
use crate::bounded_file;
use crate::drivers::{DriverDefinition, Registry};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tmt_core::content_digest::sha256;
use uuid::Uuid;

/// Names core itself installs; no extension can claim them.
pub const MAXIMUM_SKILLS: usize = 16;
pub const MAXIMUM_FILES: usize = 64;
pub const MAXIMUM_FILE_BYTES: usize = 1_048_576;
const REGISTRY_BYTES: usize = 1_048_576;

/// One skill directory as supplied: relative paths and their bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedSkill {
    pub name: String,
    pub files: Vec<(String, Vec<u8>)>,
}

/// Why nothing was published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The owner, a skill name or a file path is not acceptable.
    Invalid(String),
    /// Another owner, or core, holds this name.
    Claimed { name: String, owner: String },
    /// A path neither core nor an owner published; `force` backs it up.
    Unmanaged(PathBuf),
}

impl fmt::Display for Refusal {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(output, "{reason}"),
            Self::Claimed { name, owner } => {
                write!(
                    output,
                    "Skill {name} belongs to {owner}; nothing was published."
                )
            }
            Self::Unmanaged(path) => write!(
                output,
                "Refusing to replace existing unmanaged path: {}; force backs it up and replaces it.",
                path.display()
            ),
        }
    }
}

impl Error for Refusal {}

fn refused(refusal: Refusal) -> io::Error {
    io::Error::other(refusal)
}

/// The refusal behind an error, when there is one.
pub fn refusal(error: &io::Error) -> Option<&Refusal> {
    error.get_ref().and_then(|cause| cause.downcast_ref())
}

fn valid_owner(owner: &str) -> bool {
    let bytes = owner.as_bytes();
    owner != "core"
        && (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// A skill directory name: the one rule for owned skills and release archives.
pub(crate) fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=64).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// Canonical relative paths only, bounded: `/`-separated non-empty segments,
/// none dot-prefixed (so never `.`, `..` or hidden staging names). The digest
/// hashes the supplied string, so a form the filesystem would normalize
/// (`a//b`, `a/`) could never verify after materialization.
pub(crate) fn valid_file(path: &str) -> bool {
    path.len() <= 512
        && path
            .split('/')
            .all(|part| (1..=255).contains(&part.len()) && !part.starts_with('.'))
}

/// Checks everything before any effect.
pub fn validate(owner: &str, skills: &[OwnedSkill]) -> Result<(), Refusal> {
    let invalid = |reason: String| Err(Refusal::Invalid(reason));
    if !valid_owner(owner) {
        return invalid(format!(
            "Owner '{owner}' must be a lowercase extension name other than core."
        ));
    }
    if skills.is_empty() || skills.len() > MAXIMUM_SKILLS {
        return invalid(format!("Supply 1 to {MAXIMUM_SKILLS} skills."));
    }
    let mut names = std::collections::BTreeSet::new();
    for skill in skills {
        if !valid_name(&skill.name) || !names.insert(&skill.name) {
            return invalid(format!(
                "Skill name '{}' is invalid or repeated.",
                skill.name
            ));
        }
        if Catalog::bundled().is_core(&skill.name) {
            return Err(Refusal::Claimed {
                name: skill.name.clone(),
                owner: "core".into(),
            });
        }
        if skill.files.is_empty() || skill.files.len() > MAXIMUM_FILES {
            return invalid(format!(
                "Skill {} needs 1 to {MAXIMUM_FILES} files.",
                skill.name
            ));
        }
        let mut paths = std::collections::BTreeSet::new();
        for (path, bytes) in &skill.files {
            if !valid_file(path) || !paths.insert(path) || bytes.len() > MAXIMUM_FILE_BYTES {
                return invalid(format!(
                    "File '{path}' in skill {} is not acceptable.",
                    skill.name
                ));
            }
        }
        if !paths.contains(&"SKILL.md".to_owned()) {
            return invalid(format!("Skill {} has no top-level SKILL.md.", skill.name));
        }
        // A file and a directory cannot share a path.
        if paths.iter().any(|path| {
            paths
                .iter()
                .any(|other| other.starts_with(&format!("{path}/")))
        }) {
            return invalid(format!("Skill {} uses a file as a directory.", skill.name));
        }
    }
    Ok(())
}

/// Order-independent digest of one skill's name and files.
fn skill_digest(name: &str, files: &[(String, Vec<u8>)]) -> String {
    let mut sorted: Vec<_> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut framed = Vec::new();
    for part in std::iter::once(name.as_bytes()).chain(
        sorted
            .iter()
            .flat_map(|(path, bytes)| [path.as_bytes(), bytes.as_slice()]),
    ) {
        framed.extend_from_slice(&(part.len() as u64).to_be_bytes());
        framed.extend_from_slice(part);
    }
    sha256(&framed)
}

fn store(assets: &SkillAssets, owner: &str) -> PathBuf {
    assets.root().join("owners").join(owner)
}

fn source(assets: &SkillAssets, owner: &str, digest: &str, name: &str) -> PathBuf {
    store(assets, owner).join(digest).join(name)
}

/// Reads a materialized skill back: regular files and directories only.
fn read_tree(root: &Path) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        if !fs::symlink_metadata(&directory)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a directory",
            ));
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() && files.len() < MAXIMUM_FILES {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .map_err(io::Error::other)?
                    .to_string_lossy()
                    .into_owned();
                let bytes = bounded_file::read(&entry.path(), MAXIMUM_FILE_BYTES)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                files.push((relative, bytes));
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected entry in an owned skill",
                ));
            }
        }
    }
    Ok(files)
}

/// The owner and name of a link source inside the owned store, verified by
/// recomputing its digest: local evidence, not authentication.
fn owned_source(assets: &SkillAssets, source: &Path) -> Option<(String, String)> {
    let name = source.file_name()?.to_str()?;
    let version = source.parent()?;
    let digest = version.file_name()?.to_str()?;
    let owner_directory = version.parent()?;
    let owner = owner_directory.file_name()?.to_str()?;
    if files::resolved(owner_directory.parent()?).ok()?
        != files::resolved(&assets.root().join("owners")).ok()?
        || digest.len() != 64
    {
        return None;
    }
    if !valid_owner(owner) || !valid_name(name) || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let expected = files::resolved(assets.root())
        .ok()?
        .join("owners")
        .join(owner)
        .join(digest)
        .join(name);
    if files::resolved(source).ok()? != expected {
        return None;
    }
    if !files::exists(version).ok()? {
        // A recorded name may outlive its old generation. The record supplies
        // the claim; the canonical store path supplies link ownership.
        let recorded = read_owners(assets.root().parent()?).ok()?;
        return recorded
            .skills
            .get(name)
            .is_some_and(|entry| entry.owner == owner)
            .then(|| (owner.to_owned(), name.to_owned()));
    }
    let files = read_tree(source).ok()?;
    (skill_digest(name, &files) == digest).then(|| (owner.to_owned(), name.to_owned()))
}

/// Where a target's link points and who manages it.
enum Prior {
    Absent,
    /// Core's bundle (an Office skill published before owners existed).
    Core,
    Owned {
        owner: String,
        source: PathBuf,
    },
    Unmanaged,
}

fn prior(target: &Path, assets: &SkillAssets) -> io::Result<Prior> {
    if !files::exists(target)? {
        return Ok(Prior::Absent);
    }
    if managed_link(target, assets)?.is_some() {
        return Ok(Prior::Core);
    }
    if fs::symlink_metadata(target)?.is_symlink() {
        let source = crate::config::normalize(
            &target
                .parent()
                .expect("skill target parent")
                .join(fs::read_link(target)?),
        );
        // The one-release default-directory cutover can leave an absolute
        // managed link dangling. Only a present, fully verified moved tree may
        // prove the prior link; an owner claim alone is not relocation evidence.
        let holder = owned_source(assets, &source).or_else(|| {
            let moved = crate::config::relocated_skill_source(assets.root().parent()?, &source)?;
            moved
                .is_dir()
                .then(|| owned_source(assets, &moved))
                .flatten()
        });
        if source.file_name() == target.file_name()
            && let Some((owner, _)) = holder
        {
            return Ok(Prior::Owned {
                owner,
                source: files::resolved(&source)?,
            });
        }
    }
    Ok(Prior::Unmanaged)
}

/// `skill-owners.json`: which owner holds each name, its current digest and
/// the targets published for it. Separate from the target intents so older
/// binaries and core refresh never see extension names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Owners {
    skills: BTreeMap<String, OwnerRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerRecord {
    owner: String,
    digest: String,
    targets: Vec<PathBuf>,
}

pub(super) fn registry_path(global: &Path) -> PathBuf {
    global.join("skill-owners.json")
}

fn corrupt() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid skill owner registry; preserve it for inspection.",
    )
}

fn read_owners(global: &Path) -> io::Result<Owners> {
    let path = registry_path(global);
    if !files::exists(&path)? {
        return Ok(Owners::default());
    }
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(corrupt());
    }
    let bytes = bounded_file::read(&path, REGISTRY_BYTES).map_err(|_| corrupt())?;
    let document: Value = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
    if document["version"] != 1 || document.as_object().is_none_or(|object| object.len() != 2) {
        return Err(corrupt());
    }
    let mut owners = Owners::default();
    for (name, entry) in document["skills"].as_object().ok_or_else(corrupt)? {
        let owner = entry["owner"].as_str().filter(|owner| valid_owner(owner));
        let digest = entry["digest"].as_str().filter(|digest| digest.len() == 64);
        let targets = entry["targets"].as_array().ok_or_else(corrupt)?;
        let (Some(owner), Some(digest), true) = (owner, digest, valid_name(name)) else {
            return Err(corrupt());
        };
        owners.skills.insert(
            name.clone(),
            OwnerRecord {
                owner: owner.to_owned(),
                digest: digest.to_owned(),
                targets: targets
                    .iter()
                    .map(|target| {
                        target
                            .as_str()
                            .map(PathBuf::from)
                            .filter(|path| path.is_absolute())
                            .ok_or_else(corrupt)
                    })
                    .collect::<io::Result<_>>()?,
            },
        );
    }
    Ok(owners)
}

fn write_owners(global: &Path, owners: &Owners) -> io::Result<()> {
    let skills: Map<String, Value> = owners
        .skills
        .iter()
        .map(|(name, entry)| {
            (
                name.clone(),
                json!({
                    "owner": entry.owner,
                    "digest": entry.digest,
                    "targets": entry.targets.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>(),
                }),
            )
        })
        .collect();
    let bytes =
        serde_json::to_vec(&json!({"version": 1, "skills": skills})).map_err(|_| corrupt())?;
    if bytes.len() > REGISTRY_BYTES {
        return Err(corrupt());
    }
    files::atomic_write(&registry_path(global), &bytes)
}

/// Writes a skill into the store unless the same content is already there.
fn materialize(assets: &SkillAssets, owner: &str, skill: &OwnedSkill) -> io::Result<PathBuf> {
    let digest = skill_digest(&skill.name, &skill.files);
    let destination = source(assets, owner, &digest, &skill.name);
    if files::exists(&destination)? {
        return match owned_source(assets, &destination) {
            Some(_) => Ok(destination),
            None => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Owned skill source was modified: {}", destination.display()),
            )),
        };
    }
    let version = destination.parent().expect("digest directory");
    fs::create_dir_all(store(assets, owner))?;
    let stage = store(assets, owner).join(format!(".stage-{}", Uuid::new_v4()));
    let pending = (|| {
        for (path, bytes) in &skill.files {
            let file = stage.join(&skill.name).join(path);
            fs::create_dir_all(file.parent().expect("staged file parent"))?;
            let mut handle = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)?;
            handle.write_all(bytes)?;
            handle.sync_all()?;
        }
        fs::create_dir_all(version)?;
        fs::rename(stage.join(&skill.name), &destination)
    })();
    let _ = fs::remove_dir_all(&stage);
    pending.map(|()| destination)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedTarget {
    pub name: String,
    pub agent: Option<&'static DriverDefinition>,
    pub target: PathBuf,
    pub changed: bool,
    pub backup: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct OwnedReport {
    pub published: Vec<OwnedTarget>,
    pub removed: Vec<PathBuf>,
    /// Recorded targets left alone because they no longer point at the owner.
    pub kept: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct OwnedFailure {
    pub cause: io::Error,
    pub report: OwnedReport,
}

impl fmt::Display for OwnedFailure {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(output, "{}", self.cause)?;
        for item in &self.report.published {
            write!(output, "; published: {}", item.target.display())?;
        }
        Ok(())
    }
}

impl Error for OwnedFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.cause)
    }
}

/// Preserve consented targets during the fixed product rename. Sources remain
/// immutable, and all conflicts are admitted before any link is changed.
pub fn migrate_former_owned(
    global: &Path,
    product: tmt_core::native_install::Product,
    replacements: &[OwnedSkill],
) -> io::Result<()> {
    let Some(former) = product.former() else {
        return Ok(());
    };
    let global = files::resolved(global)?;
    if !files::exists(&registry_path(&global))? {
        return Ok(());
    }
    files::with_lock(&global, || {
        let assets = SkillAssets::new(&global);
        let mut owners = read_owners(&global)?;
        let old = owners
            .skills
            .iter()
            .filter(|(_, entry)| entry.owner == former.name)
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect::<Vec<_>>();
        let mut plan = Vec::new();
        for (name, entry) in &old {
            let old_source = source(&assets, former.name, &entry.digest, name);
            let old_files = read_tree(&old_source)?;
            if skill_digest(name, &old_files) != entry.digest {
                return Err(refused(Refusal::Unmanaged(old_source)));
            }
            let new_name = former
                .renamed_skills
                .iter()
                .find_map(|(old, new)| (*old == name).then_some(*new))
                .unwrap_or(name);
            let skill = replacements
                .iter()
                .find(|skill| skill.name == new_name)
                .cloned()
                .unwrap_or_else(|| OwnedSkill {
                    name: new_name.into(),
                    files: old_files,
                });
            validate(product.as_str(), std::slice::from_ref(&skill)).map_err(refused)?;
            if new_name != name
                && let Some(record) = owners.skills.get(new_name)
                && record.owner != product.as_str()
            {
                return Err(refused(Refusal::Claimed {
                    name: new_name.into(),
                    owner: record.owner.clone(),
                }));
            }
            let expected = source(
                &assets,
                product.as_str(),
                &skill_digest(&skill.name, &skill.files),
                new_name,
            );
            let mut targets = Vec::new();
            for old_target in &entry.targets {
                let target = old_target.parent().ok_or_else(corrupt)?.join(new_name);
                files::safe_target(assets.root(), old_target)?;
                files::safe_target(assets.root(), &target)?;
                match prior(old_target, &assets)? {
                    Prior::Absent
                        if target != *old_target
                            && matches!(prior(&target, &assets)?,
                        Prior::Owned { owner, source } if owner == product.as_str() && source == expected) =>
                        {}
                    Prior::Absent => continue,
                    Prior::Owned { owner, source }
                        if (owner == former.name && source == old_source)
                            || (new_name == name
                                && owner == product.as_str()
                                && source == expected) => {}
                    _ => return Err(refused(Refusal::Unmanaged(old_target.clone()))),
                }
                if target != *old_target {
                    match prior(&target, &assets)? {
                        Prior::Absent => {}
                        Prior::Owned { owner, source }
                            if owner == product.as_str() && source == expected => {}
                        _ => return Err(refused(Refusal::Unmanaged(target))),
                    }
                }
                targets.push((old_target.clone(), target));
            }
            plan.push((name.clone(), skill, old_source, targets));
        }
        for (old_name, skill, old_source, targets) in plan {
            let destination = materialize(&assets, product.as_str(), &skill)?;
            // Keep the old intent until each link transition completes. A retry
            // can recognize either immutable source without expanding consent.
            for (old_target, target) in &targets {
                files::link(target, &destination)?;
                if old_target != target
                    && matches!(prior(old_target, &assets)?,
                    Prior::Owned { owner, source } if owner == former.name && source == old_source)
                {
                    fs::remove_file(old_target)?;
                }
            }
            let mut published = owners
                .skills
                .get(&skill.name)
                .filter(|record| record.owner == product.as_str())
                .map(|record| record.targets.clone())
                .unwrap_or_default();
            for (_, target) in targets {
                if !published.contains(&target) {
                    published.push(target);
                }
            }
            owners.skills.remove(&old_name);
            owners.skills.insert(
                skill.name.clone(),
                OwnerRecord {
                    owner: product.as_str().into(),
                    digest: skill_digest(&skill.name, &skill.files),
                    targets: published,
                },
            );
            write_owners(&global, &owners)?;
        }
        Ok(())
    })
}

/// Refresh only recorded, still-owned targets. A removed target does not grant
/// consent to recreate it, and provider discovery never expands this operation.
pub fn refresh_owned(
    global: &Path,
    owner: &str,
    skills: &[OwnedSkill],
) -> Result<OwnedReport, OwnedFailure> {
    let mut report = OwnedReport::default();
    let pending = (|| {
        validate(owner, skills).map_err(refused)?;
        let global = files::resolved(global)?;
        let assets = SkillAssets::new(&global);
        files::with_lock(&global, || {
            let mut owners = read_owners(&global)?;
            let mut plan = Vec::new();
            for skill in skills {
                let entry = owners.skills.get(&skill.name).ok_or_else(corrupt)?;
                if entry.owner != owner {
                    return Err(refused(Refusal::Claimed {
                        name: skill.name.clone(),
                        owner: entry.owner.clone(),
                    }));
                }
                for target in &entry.targets {
                    files::safe_target(assets.root(), target)?;
                    match prior(target, &assets)? {
                        Prior::Absent => {}
                        Prior::Owned {
                            owner: holder,
                            source,
                        } if holder == owner => {
                            plan.push((skill, target.clone(), source));
                        }
                        _ => return Err(refused(Refusal::Unmanaged(target.clone()))),
                    }
                }
            }
            let mut sources = BTreeMap::new();
            for skill in skills {
                let destination = materialize(&assets, owner, skill)?;
                owners
                    .skills
                    .get_mut(&skill.name)
                    .ok_or_else(corrupt)?
                    .digest = skill_digest(&skill.name, &skill.files);
                sources.insert(skill.name.clone(), destination);
            }
            write_owners(&global, &owners)?;
            for (skill, target, previous) in plan {
                let destination = &sources[&skill.name];
                let changed = &previous != destination;
                if changed {
                    files::link(&target, destination)?;
                }
                report.published.push(OwnedTarget {
                    name: skill.name.clone(),
                    agent: None,
                    target,
                    changed,
                    backup: None,
                });
            }
            Ok(())
        })
    })();
    match pending {
        Ok(()) => Ok(report),
        Err(cause) => Err(OwnedFailure { cause, report }),
    }
}

/// Publishes an owner's skills into every provider root that gets optional
/// skills. Claims and unmanaged paths are checked for every target before any
/// effect; `force` transfers a claim and backs up unmanaged paths.
pub fn install_owned(
    env: &ProviderEnvironment,
    global: &Path,
    owner: &str,
    skills: &[OwnedSkill],
    force: bool,
) -> Result<OwnedReport, OwnedFailure> {
    let mut report = OwnedReport::default();
    let pending = (|| {
        validate(owner, skills).map_err(refused)?;
        let global = files::resolved(global)?;
        let assets = SkillAssets::new(&global);
        files::with_lock(&global, || {
            let roots = optional_roots(env, &global, &assets)?;
            let mut owners = read_owners(&global)?;
            let mut plan = Vec::new();
            for skill in skills {
                if let Some(entry) = owners.skills.get(&skill.name)
                    && entry.owner != owner
                    && !force
                {
                    return Err(refused(Refusal::Claimed {
                        name: skill.name.clone(),
                        owner: entry.owner.clone(),
                    }));
                }
                for (root, agent) in &roots {
                    let target = root.join(&skill.name);
                    files::safe_target(assets.root(), &target)?;
                    let prior = prior(&target, &assets)?;
                    match &prior {
                        Prior::Owned { owner: other, .. } if other != owner && !force => {
                            return Err(refused(Refusal::Claimed {
                                name: skill.name.clone(),
                                owner: other.clone(),
                            }));
                        }
                        // Only Office adopts the Office links core published
                        // before owners existed; anyone else needs force.
                        Prior::Core
                            if !force
                                && !(owner == "office"
                                    && Catalog::bundled()
                                        .names(Group::Office)
                                        .contains(skill.name.as_str())) =>
                        {
                            return Err(refused(Refusal::Claimed {
                                name: skill.name.clone(),
                                owner: "core".into(),
                            }));
                        }
                        Prior::Unmanaged if !force => {
                            return Err(refused(Refusal::Unmanaged(target)));
                        }
                        _ => {}
                    }
                    plan.push((skill, target, *agent, prior));
                }
            }
            let mut sources = BTreeMap::new();
            for skill in skills {
                let source = materialize(&assets, owner, skill)?;
                let digest = skill_digest(&skill.name, &skill.files);
                let entry = owners
                    .skills
                    .entry(skill.name.clone())
                    .or_insert(OwnerRecord {
                        owner: owner.to_owned(),
                        digest: digest.clone(),
                        targets: Vec::new(),
                    });
                entry.owner = owner.to_owned();
                entry.digest = digest;
                for (planned, target, _, _) in &plan {
                    if planned.name == skill.name && !entry.targets.contains(target) {
                        entry.targets.push(target.clone());
                    }
                }
                sources.insert(skill.name.clone(), source);
            }
            // Record intent before links, so a crash leaves nothing unowned.
            write_owners(&global, &owners)?;
            for (skill, target, agent, prior) in plan {
                let source = &sources[&skill.name];
                let changed = match &prior {
                    Prior::Owned {
                        source: current, ..
                    } => current != source,
                    _ => true,
                };
                let backup = match prior {
                    Prior::Unmanaged => Some(files::backup(&target)?),
                    _ => None,
                };
                if changed {
                    files::link(&target, source)?;
                }
                report.published.push(OwnedTarget {
                    name: skill.name.clone(),
                    agent,
                    target,
                    changed,
                    backup,
                });
            }
            Ok(())
        })
    })();
    match pending {
        Ok(()) => Ok(report),
        Err(cause) => Err(OwnedFailure { cause, report }),
    }
}

/// A selection of an owner's skills by name: nonempty, bounded, canonical
/// names, no repeats.
fn validate_selection(only: &[String]) -> Result<(), Refusal> {
    let mut names = std::collections::BTreeSet::new();
    if only.is_empty()
        || only.len() > MAXIMUM_SKILLS
        || only
            .iter()
            .any(|name| !valid_name(name) || !names.insert(name))
    {
        return Err(Refusal::Invalid(format!(
            "Select 1 to {MAXIMUM_SKILLS} distinct skill names."
        )));
    }
    Ok(())
}

/// The legacy core bundle published Office before extension owner records
/// existed. Its recorded targets are publication evidence, not overwrite
/// authority: removal reclassifies every entry under the same lock.
fn legacy_targets(
    environment: Option<&ProviderEnvironment>,
    global: &Path,
    owner: &str,
) -> io::Result<BTreeMap<String, Vec<PathBuf>>> {
    let mut targets = BTreeMap::<String, Vec<PathBuf>>::new();
    if owner == "office" {
        let catalog = Catalog::bundled();
        let names = catalog.names(Group::Office);
        let mut candidates = registry::read(global)?;
        if let Some(env) = environment {
            let assets = SkillAssets::new(global);
            let mut roots: Vec<PathBuf> = Registry::builtin()
                .iter()
                .map(|driver| env.locations(driver).skills)
                .collect();
            roots.push(env.universal_skills());
            for root in roots {
                for name in &names {
                    let target = root.join(name);
                    // Without a record, only the exact old-layout link into
                    // this store proves publication; a user directory or an
                    // outside link never enters the removal plan.
                    if fs::symlink_metadata(&target).is_ok_and(|entry| entry.is_symlink()) {
                        let source = files::resolved(&root.join(fs::read_link(&target)?))?;
                        if source.file_name() == target.file_name()
                            && source.parent().and_then(Path::parent) == Some(assets.root())
                        {
                            candidates.insert(target);
                        }
                    }
                }
            }
        }
        for target in candidates {
            if let Some(name) = target.file_name().and_then(|name| name.to_str())
                && names.contains(name)
            {
                targets.entry(name.to_owned()).or_default().push(target);
            }
        }
    }
    Ok(targets)
}

/// Removes an owner's links: only targets that still point into that owner's
/// store. Anything else at a recorded target is kept and reported. `only`
/// limits removal to those of the owner's skills; a name the owner does not
/// hold (including one held by another owner) is nothing to remove, like a
/// repeated removal. An explicit extension removal supplies its captured
/// provider environment to find legacy links whose intent record is missing.
pub fn remove_owned(
    environment: Option<&ProviderEnvironment>,
    global: &Path,
    owner: &str,
    only: Option<&[String]>,
) -> Result<OwnedReport, OwnedFailure> {
    let mut report = OwnedReport::default();
    let pending = (|| {
        if !valid_owner(owner) {
            return Err(refused(Refusal::Invalid(format!(
                "Owner '{owner}' must be a lowercase extension name other than core."
            ))));
        }
        if let Some(only) = only {
            validate_selection(only).map_err(refused)?;
        }
        let global = files::resolved(global)?;
        let assets = SkillAssets::new(&global);
        files::with_lock(&global, || {
            let mut owners = read_owners(&global)?;
            let legacy = legacy_targets(environment, &global, owner)?;
            let mut targets = legacy.clone();
            for (name, entry) in &owners.skills {
                if entry.owner == owner {
                    targets
                        .entry(name.clone())
                        .or_default()
                        .extend(entry.targets.clone());
                } else {
                    // A later owner claim wins over the old bundle's intent.
                    targets.remove(name);
                }
            }
            targets.retain(|name, _| only.is_none_or(|only| only.contains(name)));
            let mut retired = Vec::new();
            for (name, entries) in targets {
                let mut seen = std::collections::BTreeSet::new();
                for target in entries {
                    retired.push(target.clone());
                    if !seen.insert(files::entry_location(&target)?) {
                        continue;
                    }
                    files::safe_target(assets.root(), &target)?;
                    match prior(&target, &assets)? {
                        Prior::Owned { owner: holder, .. } if holder == owner => {
                            fs::remove_file(&target)?;
                            report.removed.push(target.clone());
                        }
                        Prior::Core
                            if legacy
                                .get(&name)
                                .is_some_and(|entries| entries.contains(&target)) =>
                        {
                            fs::remove_file(&target)?;
                            report.removed.push(target.clone());
                        }
                        Prior::Absent => {}
                        _ => report.kept.push(target.clone()),
                    }
                }
                owners.skills.remove(&name);
            }
            // Forget intent before dropping owner records. A retry after a
            // returned failure can still find every uncompleted owner target.
            registry::forget(&global, &retired)?;
            write_owners(&global, &owners)
        })
    })();
    match pending {
        Ok(()) => Ok(report),
        Err(cause) => Err(OwnedFailure { cause, report }),
    }
}

/// Passive drift for owned skills in fixed provider roots: a target that
/// exists but no longer points at its owner's current content. An unreadable
/// owner registry is not reported here; install and remove surface it.
pub(super) fn owned_drift(env: &ProviderEnvironment, global: &Path) -> io::Result<Vec<PathBuf>> {
    let assets = SkillAssets::new(global);
    let Ok(owners) = read_owners(global) else {
        return Ok(Vec::new());
    };
    let mut drift = Vec::new();
    for (name, entry) in &owners.skills {
        let expected = source(&assets, &entry.owner, &entry.digest, name);
        for provider in Registry::builtin().iter() {
            let target = env
                .target(provider)
                .parent()
                .expect("skill target parent")
                .join(name);
            if drift.contains(&target) || !files::exists(&target)? {
                continue;
            }
            match prior(&target, &assets)? {
                Prior::Owned { source, .. } if source == expected => {}
                _ => drift.push(target),
            }
        }
    }
    Ok(drift)
}

/// Names an extension holds. Core's own publication and refresh leave these
/// alone, so an adopted Office skill is never pulled back to the bundle.
pub(super) fn owned_names(global: &Path) -> io::Result<std::collections::BTreeSet<String>> {
    Ok(read_owners(global)?.skills.into_keys().collect())
}

/// Every provider root an owner's skills are published into, the same roots
/// `install_owned` uses. Read-only: for consent before any effect.
pub fn owned_roots(env: &ProviderEnvironment, global: &Path) -> io::Result<Vec<PathBuf>> {
    let global = files::resolved(global)?;
    let assets = SkillAssets::new(&global);
    Ok(optional_roots(env, &global, &assets)?.into_keys().collect())
}

/// The skills an owner holds and each one's recorded targets.
pub fn owned_by(
    environment: Option<&ProviderEnvironment>,
    global: &Path,
    owner: &str,
) -> io::Result<BTreeMap<String, Vec<PathBuf>>> {
    let global = files::resolved(global)?;
    let mut targets = legacy_targets(environment, &global, owner)?;
    for (name, entry) in read_owners(&global)?.skills {
        if entry.owner == owner {
            targets.entry(name).or_default().extend(entry.targets);
        } else {
            targets.remove(&name);
        }
    }
    for entries in targets.values_mut() {
        entries.sort();
        entries.dedup();
    }
    Ok(targets)
}

/// Links recorded owned skills at new targets to each owner's current
/// source, under the installation lock, and records those targets. A target
/// that exists is skipped, never replaced.
pub(super) fn link_recorded<'a>(
    global: &Path,
    targets: impl IntoIterator<Item = (&'a str, &'a Path)>,
) -> io::Result<Vec<PathBuf>> {
    let assets = SkillAssets::new(global);
    let targets: Vec<(&str, &Path)> = targets.into_iter().collect();
    files::with_lock(global, || {
        let mut owners = read_owners(global)?;
        let mut linked = Vec::new();
        for (name, target) in &targets {
            let Some(entry) = owners.skills.get_mut(*name) else {
                continue;
            };
            if files::exists(target)? {
                continue;
            }
            files::safe_target(assets.root(), target)?;
            let from = source(&assets, &entry.owner, &entry.digest, name);
            files::link(target, &from)?;
            entry.targets.push(target.to_path_buf());
            linked.push(target.to_path_buf());
        }
        if !linked.is_empty() {
            write_owners(global, &owners)?;
        }
        Ok(linked)
    })
}

/// Each owned skill's recorded targets, by name.
pub(super) fn owned_targets(global: &Path) -> io::Result<BTreeMap<String, Vec<PathBuf>>> {
    Ok(read_owners(global)?
        .skills
        .into_iter()
        .map(|(name, entry)| (name, entry.targets))
        .collect())
}

/// Owner of each recorded skill name, for status output.
pub fn owners(global: &Path) -> io::Result<BTreeMap<String, String>> {
    Ok(read_owners(&files::resolved(global)?)?
        .skills
        .into_iter()
        .map(|(name, entry)| (name, entry.owner))
        .collect())
}
