//! Read-only evidence from the selected Codex home's local project settings.
//! This is not effective configuration resolution: other provider layers may
//! override this evidence. It supports only a hedged, informational advisory.

use std::{fs, io::ErrorKind, path::Path};

use crate::{bounded_file, skill_installation::ProviderEnvironment};
use toml_edit::DocumentMut;

const CONFIG_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalProjectTrust {
    Trusted,
    Untrusted,
    Unset,
    Unknown,
}

/// Inspect only the user config selected by the captured provider environment.
/// Never creates a home, config, or trust entry. Unsupported repository layouts,
/// config syntax, and failed reads produce unknown evidence, without diagnostics
/// containing configuration values.
pub(super) fn local_project_trust(
    environment: &ProviderEnvironment,
    cwd: &Path,
) -> LocalProjectTrust {
    inspect(&super::codex_home(environment), cwd).unwrap_or(LocalProjectTrust::Unknown)
}

fn inspect(home: &Path, cwd: &Path) -> Option<LocalProjectTrust> {
    if !cwd.is_absolute() || !cwd.is_dir() {
        return None;
    }
    fs::canonicalize(cwd).ok()?.to_str()?;
    cwd.to_str()?;
    let config = home.join("config.toml");
    let bytes = match bounded_file::read_no_follow(&config, CONFIG_LIMIT) {
        Ok(bytes) => bytes,
        Err(bounded_file::FileReadError::Io(error)) if error.kind() == ErrorKind::NotFound => {
            if fs::symlink_metadata(&config).is_ok() {
                return None;
            }
            Vec::new()
        }
        Err(_) => return None,
    };
    let document = std::str::from_utf8(&bytes)
        .ok()?
        .parse::<DocumentMut>()
        .ok()?;
    let projects = match document.get("projects") {
        Some(item) => Some(item.as_table_like()?),
        None => None,
    };
    let lookup = |path: &Path| -> Option<LocalProjectTrust> {
        let projects = projects?;
        let canonical = fs::canonicalize(path).ok()?;
        for candidate in [&canonical, path] {
            if let Some(project) = projects.get(candidate.to_str()?) {
                let Some(table) = project.as_table_like() else {
                    return Some(LocalProjectTrust::Unknown);
                };
                return Some(match table.get("trust_level") {
                    None => LocalProjectTrust::Unset,
                    Some(value) => match value.as_str() {
                        Some("trusted") => LocalProjectTrust::Trusted,
                        Some("untrusted") => LocalProjectTrust::Untrusted,
                        _ => LocalProjectTrust::Unknown,
                    },
                });
            }
        }
        None
    };
    // An exact cwd entry, even without trust_level, masks repository settings.
    if let Some(trust) = lookup(cwd) {
        return Some(trust);
    }
    if let Some(root) = repository_root(cwd)?
        && let Some(trust) = lookup(&root)
    {
        return Some(trust);
    }
    Some(LocalProjectTrust::Unset)
}

fn repository_root(cwd: &Path) -> Option<Option<std::path::PathBuf>> {
    for (depth, ancestor) in cwd.ancestors().enumerate() {
        if depth >= 128 {
            return None;
        }
        let git = ancestor.join(".git");
        match fs::symlink_metadata(&git) {
            Ok(metadata) if metadata.is_dir() => match fs::metadata(git.join("HEAD")) {
                Ok(_) => return Some(Some(ancestor.to_path_buf())),
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(_) => return None,
            },
            // Linked worktrees and separate git directories require Codex's
            // backlink validation; do not guess that this checkout is the root.
            Ok(_) => return None,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    Some(None)
}

#[cfg(test)]
mod tests;
