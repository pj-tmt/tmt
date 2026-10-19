//! Private, expiring suppression hints. A cache hit authorizes no effect.

use super::lifecycle::CallerSession;
use crate::{driver_protocol::registry::DriverRecord, host::CallerEnvironment};
use std::path::{Path, PathBuf};
use tmt_core::binding::{Binding, session::HarnessId};

pub const TTL_MS: u64 = 10 * 60 * 1_000;
const ENTRIES: usize = 32;
const BYTES: u64 = 64 * 1024;
/// Version 2 added external caller keys; an older document is a miss. A
/// nested caller key is a new target that a version 2 reader only fails to
/// parse, which is a miss too, so it keeps the version.
const VERSION: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    harness: String,
    session: String,
    runtime_pid: Option<u32>,
    target: Target,
}

/// What the refused attempt was scheduled for: a binding the caller's pane
/// locator selected, an external caller that has no such locator, or a
/// caller that names tmux and one external host.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Binding(Pane),
    Caller {
        host: String,
        /// SHA-256 of the caller's coordinates (`caller_digest`); no raw
        /// environment value is kept.
        coordinates: String,
    },
    /// Either pane can own a nested caller, and only the worker's nested
    /// proof tells which, so the key holds both sides: a change to either
    /// runs admission again. The tmux binding is in it when the tmux pane
    /// is bound; it is never the owner.
    Nested {
        tmux: Option<Pane>,
        host: String,
        /// `caller_digest` over the external host's coordinates and the raw
        /// `TMUX` and `TMUX_PANE` values.
        coordinates: String,
    },
}

/// A stored binding's pane and server, as one refusal key holds it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Pane {
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

impl Pane {
    fn of(binding: &Binding) -> Self {
        Self {
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

impl Key {
    pub fn new(harness: &HarnessId, coordinates: &CallerSession, binding: &Binding) -> Self {
        Self {
            harness: harness.as_str().into(),
            session: coordinates.session.as_str().into(),
            runtime_pid: coordinates.runtime_pid,
            target: Target::Binding(Pane::of(binding)),
        }
    }

    /// An external caller with no pane locator: its host's declared caller
    /// variables and the host's binding IDs stand in for the binding, so a
    /// changed variable or a changed set of bindings runs admission again.
    pub fn caller(
        harness: &HarnessId,
        coordinates: &CallerSession,
        host: &DriverRecord,
        environment: &CallerEnvironment,
        binding_ids: &[String],
    ) -> Self {
        Self {
            harness: harness.as_str().into(),
            session: coordinates.session.as_str().into(),
            runtime_pid: coordinates.runtime_pid,
            target: Target::Caller {
                host: host.name.clone(),
                coordinates: caller_digest(host, environment, binding_ids, false),
            },
        }
    }

    /// A caller that names tmux and one external host: the tmux pane's
    /// binding when it has one, and the external caller's coordinates with
    /// the tmux variables, so a corrected tmux environment runs admission
    /// again even when no tmux binding changed.
    pub fn nested(
        harness: &HarnessId,
        coordinates: &CallerSession,
        host: &DriverRecord,
        environment: &CallerEnvironment,
        tmux: Option<&Binding>,
        binding_ids: &[String],
    ) -> Self {
        Self {
            harness: harness.as_str().into(),
            session: coordinates.session.as_str().into(),
            runtime_pid: coordinates.runtime_pid,
            target: Target::Nested {
                tmux: tmux.map(Pane::of),
                host: host.name.clone(),
                coordinates: caller_digest(host, environment, binding_ids, true),
            },
        }
    }
}

/// A fixed-size digest over a length-prefixed encoding: the host's name,
/// each declared caller variable in sorted order with whether it is set and
/// its raw bytes (lossless when not text), then the sorted binding IDs. A
/// nested digest has its own tag and then `TMUX` and `TMUX_PANE` the same way.
fn caller_digest(
    host: &DriverRecord,
    environment: &CallerEnvironment,
    binding_ids: &[String],
    nested: bool,
) -> String {
    use sha2::{Digest, Sha256};
    use std::os::unix::ffi::OsStrExt;
    fn field(hash: &mut Sha256, bytes: &[u8]) {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    fn value(hash: &mut Sha256, value: Option<&std::ffi::OsStr>) {
        match value {
            Some(value) => {
                field(hash, &[1]);
                field(hash, value.as_bytes());
            }
            None => field(hash, &[0]),
        }
    }
    let mut hash = Sha256::new();
    field(
        &mut hash,
        if nested {
            b"tmt-nested-caller-coordinates-1"
        } else {
            b"tmt-caller-coordinates-1"
        },
    );
    field(&mut hash, host.name.as_bytes());
    let mut names: Vec<&String> = host
        .capabilities
        .host()
        .map(|host| host.caller_env.iter().collect())
        .unwrap_or_default();
    names.sort();
    names.dedup();
    field(&mut hash, &(names.len() as u64).to_be_bytes());
    for name in names {
        field(&mut hash, name.as_bytes());
        value(
            &mut hash,
            environment
                .driver_env
                .get(name)
                .map(|value| value.as_os_str()),
        );
    }
    let mut ids: Vec<&String> = binding_ids.iter().collect();
    ids.sort();
    field(&mut hash, &(ids.len() as u64).to_be_bytes());
    for id in ids {
        field(&mut hash, id.as_bytes());
    }
    if nested {
        value(&mut hash, environment.tmux.as_deref());
        value(&mut hash, environment.pane.as_deref());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
            version: VERSION,
            entries,
        }) && bytes.len() as u64 <= BYTES
        {
            let _ = crate::private_file::replace(&self.path, &bytes);
        }
    }

    fn read(&self) -> Option<Document> {
        let bytes = crate::bounded_file::read_no_follow(&self.path, BYTES as usize).ok()?;
        let document: Document = serde_json::from_slice(&bytes).ok()?;
        (document.version == VERSION && document.entries.len() <= ENTRIES).then_some(document)
    }
}

fn fresh(refused_at_ms: u64, now_ms: u64) -> bool {
    now_ms
        .checked_sub(refused_at_ms)
        .is_some_and(|age| age < TTL_MS)
}

#[cfg(test)]
mod tests;
