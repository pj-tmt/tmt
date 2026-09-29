//! Extension-owned skills: bytes an installed extension supplies through the
//! local API, materialized in core's content-addressed store and published
//! into the same provider roots as the optional Office skills, with the owner
//! recorded. Core's own names stay core's; another owner's names are refused.
//! The owner cannot be authenticated (same-user API), so ownership is a
//! bookkeeping boundary between cooperating installers, not a security one.

use super::{ProviderEnvironment, assets::SkillAssets, files, managed_link, optional_roots};
use crate::bounded_file;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tmt_core::{content_digest::sha256, skill_provider::Provider};
use uuid::Uuid;

/// Names core itself installs; no extension can claim them.
pub const CORE_NAMES: &[&str] = &["tmux-team", "tmt-inbox"];
/// Office names core's bundle published before owners existed; owner
/// `office` adopts them without force.
pub(super) const OFFICE_NAMES: [&str; 3] = ["tmt-office", "tmt-prop-create", "tmt-avatar-create"];
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

fn valid_name(name: &str) -> bool {
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
fn valid_file(path: &str) -> bool {
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
        if CORE_NAMES.contains(&skill.name.as_str()) {
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
    if owner_directory.parent()? != assets.root().join("owners") || digest.len() != 64 {
        return None;
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
        if let Some((owner, _)) = owned_source(assets, &source) {
            return Ok(Prior::Owned { owner, source });
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

fn registry_path(global: &Path) -> PathBuf {
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
    pub agent: Option<Provider>,
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
                                    && OFFICE_NAMES.contains(&skill.name.as_str())) =>
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

/// Removes an owner's links: only targets that still point into that owner's
/// store. Anything else at a recorded target is kept and reported.
pub fn remove_owned(global: &Path, owner: &str) -> Result<OwnedReport, OwnedFailure> {
    let mut report = OwnedReport::default();
    let pending = (|| {
        if !valid_owner(owner) {
            return Err(refused(Refusal::Invalid(format!(
                "Owner '{owner}' must be a lowercase extension name other than core."
            ))));
        }
        let global = files::resolved(global)?;
        let assets = SkillAssets::new(&global);
        files::with_lock(&global, || {
            let mut owners = read_owners(&global)?;
            let names: Vec<String> = owners
                .skills
                .iter()
                .filter(|(_, entry)| entry.owner == owner)
                .map(|(name, _)| name.clone())
                .collect();
            for name in &names {
                for target in owners.skills[name].targets.clone() {
                    match prior(&target, &assets)? {
                        Prior::Owned { owner: holder, .. } if holder == owner => {
                            fs::remove_file(&target)?;
                            report.removed.push(target);
                        }
                        Prior::Absent => {}
                        _ => report.kept.push(target),
                    }
                }
                owners.skills.remove(name);
            }
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
        for provider in Provider::ALL {
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

/// Owner of each recorded skill name, for status output.
pub fn owners(global: &Path) -> io::Result<BTreeMap<String, String>> {
    Ok(read_owners(&files::resolved(global)?)?
        .skills
        .into_iter()
        .map(|(name, entry)| (name, entry.owner))
        .collect())
}
