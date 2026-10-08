//! Private, expiring suppression hints. A cache hit authorizes no effect.

use super::lifecycle::CallerSession;
use std::{
    fs::File,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use tmt_core::binding::{Binding, session::HarnessId};

pub const TTL_MS: u64 = 10 * 60 * 1_000;
const ENTRIES: usize = 32;
const BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    harness: String,
    session: String,
    runtime_pid: Option<u32>,
    binding: String,
    pane: String,
    pane_pid: u64,
    pane_incarnation: Option<String>,
    host: String,
    server: String,
    socket: String,
    server_pid: u64,
    server_incarnation: String,
}

impl Key {
    pub fn new(harness: &HarnessId, coordinates: &CallerSession, binding: &Binding) -> Self {
        Self {
            harness: harness.as_str().into(),
            session: coordinates.session.as_str().into(),
            runtime_pid: coordinates.runtime_pid,
            binding: binding.id.clone(),
            pane: binding.pane_id.clone(),
            pane_pid: binding.pane_pid,
            pane_incarnation: binding.pane_incarnation.clone(),
            host: binding.server.host.as_str().into(),
            server: binding.server.server_id.clone(),
            socket: binding.server.socket_path.clone(),
            server_pid: binding.server.server_pid,
            server_incarnation: binding.server.server_start_time.clone(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RefusalHint {
    key: Key,
    refused_at_ms: u64,
    layer: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    entries: Vec<RefusalHint>,
}

pub struct Refusals {
    path: PathBuf,
}

impl Refusals {
    pub fn in_directory(directory: &Path) -> Self {
        Self {
            path: directory.join("caller-session-refusals.json"),
        }
    }

    pub fn lookup(&self, key: &Key, now_ms: u64) -> Option<String> {
        self.read()?
            .entries
            .into_iter()
            .find(|entry| &entry.key == key && fresh(entry.refused_at_ms, now_ms))
            .map(|entry| entry.layer)
    }

    /// Best effort, bounded and atomic. Concurrent writers may lose a hint;
    /// that causes another verified attempt, never unverified admission.
    pub fn remember(&self, key: Key, layer: &str, now_ms: u64) {
        if layer.len() > 64 {
            return;
        }
        let mut entries = self
            .read()
            .map_or_else(Vec::new, |document| document.entries);
        entries.retain(|entry| fresh(entry.refused_at_ms, now_ms) && entry.key != key);
        entries.push(RefusalHint {
            key,
            refused_at_ms: now_ms,
            layer: layer.into(),
        });
        entries.sort_by_key(|entry| entry.refused_at_ms);
        if entries.len() > ENTRIES {
            entries.drain(..entries.len() - ENTRIES);
        }
        if let Ok(bytes) = serde_json::to_vec(&Document {
            version: 1,
            entries,
        }) && bytes.len() as u64 <= BYTES
        {
            let _ = crate::private_file::replace(&self.path, &bytes);
        }
    }

    fn read(&self) -> Option<Document> {
        let flags = nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK;
        let file = File::options()
            .read(true)
            .custom_flags(flags.bits())
            .open(&self.path)
            .ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > BYTES {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > BYTES {
            return None;
        }
        let document: Document = serde_json::from_slice(&bytes).ok()?;
        (document.version == 1 && document.entries.len() <= ENTRIES).then_some(document)
    }
}

fn fresh(refused_at_ms: u64, now_ms: u64) -> bool {
    now_ms
        .checked_sub(refused_at_ms)
        .is_some_and(|age| age < TTL_MS)
}

#[cfg(test)]
mod tests;
