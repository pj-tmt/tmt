//! Bounded provider-owned source locators, never transcript content or argv.
use super::{hook_protocol::HOOK_INPUT_LIMIT, lifecycle::TurnEnd, transcript};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::Instant,
};
use tmt_core::binding::session::ProviderSessionId;

pub const CADENCE_MS: u64 = 5_000;
const DISCOVERY_ENTRIES: usize = 4096;

/// Private supervised-worker request. It contains correlation evidence only.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SamplingRequest {
    pub identity_id: String,
    pub binding_id: String,
    pub owner_pid: u64,
    pub owner_start: String,
}

pub fn payload_path(payload: &[u8]) -> Option<PathBuf> {
    if payload.len() > HOOK_INPUT_LIMIT {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    value["transcript_path"].as_str().map(PathBuf::from)
}

/// Lexical admission also permits a provider file not yet materialized at start.
/// Actual reads always pass through transcript::open's descriptor trust boundary.
pub fn locator(root: &Path, path: &Path) -> Option<String> {
    if !path.is_absolute() || path.extension()? != "jsonl" {
        return None;
    }
    let relative = path.strip_prefix(root).ok()?;
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    let text = relative.to_str()?;
    (!text.is_empty() && text.len() <= 4096 && !text.chars().any(char::is_control))
        .then(|| text.to_owned())
}

pub fn located(root: &Path, value: &str, session: &ProviderSessionId) -> Option<TurnEnd> {
    let path = root.join(value);
    (locator(root, &path).as_deref() == Some(value)).then(|| TurnEnd {
        session: session.clone(),
        transcript: Some(path),
    })
}

/// Codex can report a null path. Resolve only an exact ID-bearing JSONL name,
/// under its own sessions tree. Symlink directories and ambiguous matches refuse.
/// No provider database, transcript prefix or unrelated conversation is read.
pub fn codex_turn(root: &Path, session: &ProviderSessionId, deadline: Instant) -> Option<TurnEnd> {
    uuid::Uuid::parse_str(session.as_str()).ok()?;
    let suffix = format!("-{}.jsonl", session.as_str());
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut visited = 0;
    let mut found = None;
    while let Some((directory, depth)) = pending.pop() {
        if Instant::now() >= deadline {
            return None;
        }
        for entry in fs::read_dir(directory).ok()? {
            visited += 1;
            if visited > DISCOVERY_ENTRIES || Instant::now() >= deadline {
                return None;
            }
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if kind.is_dir() && depth < 3 {
                pending.push((entry.path(), depth + 1));
            }
            if kind.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(&suffix))
            {
                if found.is_some() {
                    return None;
                }
                let path = entry.path();
                // The final file must satisfy the same regular-file boundary.
                transcript::open(root, &path)?;
                found = Some(path);
            }
        }
    }
    Some(TurnEnd {
        session: session.clone(),
        transcript: Some(found?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use std::time::Duration;
    fn session() -> ProviderSessionId {
        ProviderSessionId::new("44444444-4444-4444-8444-444444444444").unwrap()
    }
    #[test]
    fn source_locator_is_private_relative_and_rejects_escapes() {
        let root = TestDirectory::new();
        let outside = TestDirectory::new();
        let path = root.path.join("project/session.jsonl");
        assert_eq!(
            locator(&root.path, &path).as_deref(),
            Some("project/session.jsonl")
        );
        assert!(locator(&root.path, &outside.path.join("session.jsonl")).is_none());
        for value in [
            "../other.jsonl",
            "/foreign.jsonl",
            "project/../other.jsonl",
            "bad.txt",
            "./session.jsonl",
        ] {
            assert!(located(&root.path, value, &session()).is_none(), "{value}");
        }
        assert!(located(&root.path, "project/session.jsonl", &session()).is_some());
    }
    #[test]
    fn codex_discovery_requires_exact_session_and_refuses_ambiguity_and_symlinks() {
        let root = TestDirectory::new();
        let outside = TestDirectory::new();
        let session = session();
        let name = format!("rollout-2026-10-04-{}.jsonl", session.as_str());
        fs::write(root.path.join("unrelated.jsonl"), "not read").unwrap();
        fs::write(outside.path.join(&name), "not read").unwrap();
        std::os::unix::fs::symlink(&outside.path, root.path.join("linked")).unwrap();
        let deadline = || Instant::now() + Duration::from_secs(1);
        assert!(codex_turn(&root.path, &session, deadline()).is_none());
        let date = root.path.join("2026/10/04");
        fs::create_dir_all(&date).unwrap();
        let path = date.join(&name);
        fs::write(&path, "").unwrap();
        assert_eq!(
            codex_turn(&root.path, &session, deadline())
                .unwrap()
                .transcript,
            Some(path)
        );
        fs::write(root.path.join(&name), "").unwrap();
        assert!(codex_turn(&root.path, &session, deadline()).is_none());
        assert!(codex_turn(&root.path, &session, Instant::now()).is_none());
    }
}
