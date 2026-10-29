//! The skills an agent can invoke, for a composer's skill menu (#5531).
//!
//! A driver names the directories its harness reads skills from; this module
//! walks them. The stamp and the full read walk the same directories, so a new,
//! removed or edited skill changes the stamp, and the stamp reads no file.
//! Every name and description read here is untrusted display data.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// At most this many skills are reported; more sets `truncated`.
pub const SKILL_LIMIT: usize = 256;
/// Directory entries examined per root, so a huge directory stays bounded.
const SCAN_LIMIT: usize = 1024;
/// The front matter must close within this prefix of `SKILL.md`.
const FRONT_MATTER_LIMIT: u64 = 64 * 1024;
/// Description characters kept after flattening to one line.
const DESCRIPTION_LIMIT: usize = 200;
const SKILL_FILE: &str = "SKILL.md";

/// How a composer inserts a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum InvocationSyntax {
    /// The skill name follows this character: Claude Code `/`, Codex `$`.
    Prefix { sigil: char },
    /// The harness has no skill invocation TMT can describe.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "group", rename_all = "lowercase")]
pub enum SkillSource {
    /// Published by TMT: a link into TMT's own skill assets.
    Tmt,
    User,
    Plugin {
        plugin: String,
    },
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillEntry {
    /// The exact token after the sigil.
    pub name: String,
    pub description: String,
    pub source: SkillSource,
}

/// One directory whose child directories are skills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRoot {
    pub path: PathBuf,
    /// `User`, `Plugin` or `Project`; a child linking into TMT's assets is `Tmt`.
    pub source: SkillSource,
}

/// What a driver reads skills from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRoots {
    pub syntax: InvocationSyntax,
    pub roots: Vec<SkillRoot>,
}

impl SkillRoots {
    pub fn unsupported() -> Self {
        Self {
            syntax: InvocationSyntax::Unsupported,
            roots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillList {
    pub skills: Vec<SkillEntry>,
    pub truncated: bool,
}

/// A skill directory found by the walk, before its file is read.
struct Candidate {
    directory: PathBuf,
    directory_name: String,
    source: SkillSource,
}

/// Walk every root without reading a file, calling `visit` per candidate.
/// Returns whether a root held more entries than the scan bound.
fn walk(roots: &SkillRoots, assets: &Path, mut visit: impl FnMut(&Candidate)) -> bool {
    let assets = fs::canonicalize(assets).ok();
    let mut clipped = false;
    for root in &roots.roots {
        // A root must be a real directory; a link there is not followed.
        if !fs::symlink_metadata(&root.path).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&root.path) else {
            continue;
        };
        let mut names: Vec<_> = entries
            .filter_map(Result::ok)
            .take(SCAN_LIMIT + 1)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        if names.len() > SCAN_LIMIT {
            names.truncate(SCAN_LIMIT);
            clipped = true;
        }
        names.sort();
        for name in names {
            let path = root.path.join(&name);
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            let source = if metadata.is_dir() {
                root.source.clone()
            } else if metadata.file_type().is_symlink()
                && let Some(assets) = &assets
                && fs::canonicalize(&path)
                    .is_ok_and(|target| target.starts_with(assets) && target.is_dir())
            {
                // The one followed link: TMT's own published skill.
                SkillSource::Tmt
            } else {
                continue;
            };
            visit(&Candidate {
                directory: path,
                directory_name: name,
                source,
            });
        }
    }
    clipped
}

/// An opaque digest of the walked directories and their skill files' stat
/// data: names, kinds, sizes and modification times. It reads no file.
pub fn stamp(roots: &SkillRoots, assets: &Path) -> String {
    let mut hash = Sha256::new();
    hash.update(format!("{:?}", roots.syntax));
    for root in &roots.roots {
        hash.update(root.path.as_os_str().as_encoded_bytes());
        hash.update([0]);
        hash.update(stat_line(&root.path));
    }
    walk(roots, assets, |candidate| {
        hash.update(candidate.directory.as_os_str().as_encoded_bytes());
        hash.update(format!("{:?}", candidate.source));
        hash.update(stat_line(&candidate.directory));
        hash.update(stat_line(&candidate.directory.join(SKILL_FILE)));
    });
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn stat_line(path: &Path) -> String {
    fs::metadata(path).map_or_else(
        |_| "absent;".to_owned(),
        |m| format!("{}:{}:{}:{};", m.len(), m.mtime(), m.mtime_nsec(), m.ino()),
    )
}

/// Every invokable skill, sorted by source group then name, at most
/// [`SKILL_LIMIT`]. A skill whose file is missing, unreadable, without closed
/// front matter, or whose name is not one composer token is dropped.
pub fn list(roots: &SkillRoots, assets: &Path) -> SkillList {
    if roots.syntax == InvocationSyntax::Unsupported {
        return SkillList {
            skills: Vec::new(),
            truncated: false,
        };
    }
    let mut skills = Vec::new();
    let clipped = walk(roots, assets, |candidate| {
        if let Some(skill) = read(candidate) {
            skills.push(skill);
        }
    });
    skills.sort_by(|a, b| (&a.source, &a.name).cmp(&(&b.source, &b.name)));
    skills.dedup_by(|a, b| a.name == b.name && a.source == b.source);
    let truncated = clipped || skills.len() > SKILL_LIMIT;
    skills.truncate(SKILL_LIMIT);
    SkillList { skills, truncated }
}

fn read(candidate: &Candidate) -> Option<SkillEntry> {
    let text = front_matter(&candidate.directory.join(SKILL_FILE))?;
    let field = |key: &str| {
        text.lines().find_map(|line| {
            let value = line.strip_prefix(key)?.strip_prefix(':')?.trim();
            Some(value.trim_matches(|c| c == '"' || c == '\'').to_owned())
        })
    };
    let base = field("name")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| candidate.directory_name.clone());
    if !token(&base) {
        return None;
    }
    let name = match &candidate.source {
        SkillSource::Plugin { plugin } => format!("{plugin}:{base}"),
        _ => base,
    };
    Some(SkillEntry {
        name,
        description: one_line(&field("description").unwrap_or_default()),
        source: candidate.source.clone(),
    })
}

/// The text between a leading `---` line and the next one, read from a
/// bounded prefix of a regular file opened without following a link.
fn front_matter(path: &Path) -> Option<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(no_follow_flags())
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(FRONT_MATTER_LIMIT).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let end = rest
        .match_indices("\n---")
        .find(|(index, _)| {
            let after = &rest[index + 4..];
            after.is_empty() || after.starts_with('\n') || after.starts_with("\r\n")
        })?
        .0;
    Some(rest[..end].to_owned())
}

fn no_follow_flags() -> i32 {
    (nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK).bits()
}

/// One composer token: the skill-name rule TMT installs by.
fn token(name: &str) -> bool {
    crate::skill_installation::valid_skill_name(name)
}

fn one_line(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    let mut words = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((index, _)) = words.char_indices().nth(DESCRIPTION_LIMIT) {
        words.truncate(index);
    }
    words
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
