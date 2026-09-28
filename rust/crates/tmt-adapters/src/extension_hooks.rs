//! Consented extension hooks: best-effort lifecycle observations delivered to
//! enabled extensions after core commits.
//!
//! PATH discovery alone never runs a hook. `enable` records the resolved
//! executable, its digest and a metadata fingerprint after verifying the
//! current user owns it and nothing else can write it, then negotiates
//! capabilities. Before each delivery the fingerprint and ownership are
//! checked again; any change skips the extension until it is re-enabled.
//!
//! Capture is per connection and transactional: temporary triggers record
//! typed identity and room evidence in a temporary table, so a rolled-back
//! change is never observed and nothing is persisted. Storage drains the table
//! into this process's queue on close; the CLI delivers the queue after its
//! command, under one aggregate deadline and output budget. Observations never
//! change a command's result. With nothing enabled, the only cost is one read
//! attempt of the consent file, on the first storage open of a `tmt` process.

use crate::{
    config::ConfigPaths,
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Set on every hook invocation; a `tmt` process that sees it emits nothing,
/// so an extension calling `tmt` cannot cause nested delivery.
pub const DELIVERY_MARKER: &str = "TMT_HOOK_DELIVERY";
pub const CONSENT_FILE: &str = "extension-hooks.json";
pub const PROTOCOL_VERSION: &str = "1";
pub const LIFECYCLE_CAPABILITY: &str = "lifecycle_observations_v1";
pub const CONTEXT_CAPABILITY: &str = "context_v1";
const HOOK_COMMAND: &str = "__tmt-hooks";
const PROBE_DEADLINE: Duration = Duration::from_secs(1);
const PROBE_OUTPUT_LIMIT: usize = 1024;
/// One budget for all observers of one command.
pub const DELIVERY_DEADLINE: Duration = Duration::from_millis(500);
pub const DELIVERY_OUTPUT_LIMIT: usize = 4096;
const CONSENT_LIMIT: usize = 64 * 1024;
const ENV: &str = "/usr/bin/env";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Consent {
    pub name: String,
    pub path: PathBuf,
    pub digest: String,
    pub fingerprint: Fingerprint,
    pub protocol: String,
    pub capabilities: Vec<String>,
    pub consented_at_ms: u64,
}

/// Metadata that changes whenever the executable is replaced, rewritten or
/// re-permissioned (the change time moves on every such operation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Fingerprint {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub modified_ns: i64,
    pub changed_ns: i64,
    pub uid: u32,
    pub mode: u32,
}

impl Fingerprint {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified_ns: metadata.mtime() * 1_000_000_000 + metadata.mtime_nsec(),
            changed_ns: metadata.ctime() * 1_000_000_000 + metadata.ctime_nsec(),
            uid: metadata.uid(),
            mode: metadata.mode(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsentDocument {
    version: u8,
    extensions: Vec<Consent>,
}

#[derive(Debug)]
pub enum ExtensionHookError {
    NotFound(String),
    Unsafe(String),
    Incompatible(String),
    Invalid(String),
    Io(io::Error),
}

impl ExtensionHookError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "EXTENSION_NOT_FOUND",
            Self::Unsafe(_) => "EXTENSION_UNSAFE",
            Self::Incompatible(_) => "EXTENSION_INCOMPATIBLE",
            Self::Invalid(_) => "EXTENSION_HOOKS_INVALID",
            Self::Io(_) => "EXTENSION_HOOKS_UNAVAILABLE",
        }
    }
}

impl std::fmt::Display for ExtensionHookError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message)
            | Self::Unsafe(message)
            | Self::Incompatible(message)
            | Self::Invalid(message) => formatter.write_str(message),
            Self::Io(error) => write!(
                formatter,
                "Extension hook settings are unavailable: {error}"
            ),
        }
    }
}

impl std::error::Error for ExtensionHookError {}

impl From<io::Error> for ExtensionHookError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn consent_path(global_dir: &Path) -> PathBuf {
    global_dir.join(CONSENT_FILE)
}

fn read_consents(path: &Path) -> Result<Vec<Consent>, ExtensionHookError> {
    let bytes = match crate::bounded_file::read_no_follow(path, CONSENT_LIMIT) {
        Ok(bytes) => bytes,
        Err(crate::bounded_file::FileReadError::Io(error))
            if error.kind() == io::ErrorKind::NotFound =>
        {
            return Ok(Vec::new());
        }
        Err(error) => return Err(ExtensionHookError::Io(io::Error::other(error.to_string()))),
    };
    let document: ConsentDocument = serde_json::from_slice(&bytes).map_err(|_| {
        ExtensionHookError::Invalid("The extension hook settings are not valid.".into())
    })?;
    if document.version != 1 {
        return Err(ExtensionHookError::Invalid(
            "The extension hook settings use an unsupported version.".into(),
        ));
    }
    Ok(document.extensions)
}

/// Private, atomic replacement: a crash leaves the previous file or the new one.
fn write_consents(path: &Path, extensions: &[Consent]) -> Result<(), ExtensionHookError> {
    let directory = path
        .parent()
        .ok_or_else(|| ExtensionHookError::Invalid("No settings directory.".into()))?;
    fs::create_dir_all(directory)?;
    let staging = directory.join(format!(".{CONSENT_FILE}.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&ConsentDocument {
        version: 1,
        extensions: extensions.to_vec(),
    })
    .map_err(|error| ExtensionHookError::Io(io::Error::other(error)))?;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&staging)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&staging, path)?;
        File::open(directory)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result.map_err(ExtensionHookError::Io)
}

pub fn list_consents(paths: &ConfigPaths) -> Result<Vec<Consent>, ExtensionHookError> {
    read_consents(&consent_path(&paths.global_dir))
}

pub fn disable(paths: &ConfigPaths, name: &str) -> Result<bool, ExtensionHookError> {
    let path = consent_path(&paths.global_dir);
    let mut extensions = read_consents(&path)?;
    let before = extensions.len();
    extensions.retain(|consent| consent.name != name);
    if extensions.len() == before {
        return Ok(false);
    }
    write_consents(&path, &extensions)?;
    Ok(true)
}

/// Resolves, verifies and probes `tmt-<name>`, then records consent.
pub fn enable(
    paths: &ConfigPaths,
    name: &str,
    search_path: &OsStr,
    tmt: &Path,
) -> Result<Consent, ExtensionHookError> {
    let resolved = crate::extension_command::resolve(name, search_path)?.ok_or_else(|| {
        ExtensionHookError::NotFound(format!("No executable tmt-{name} was found on PATH."))
    })?;
    let executable = fs::canonicalize(&resolved)?;
    let metadata = verify_ownership(&executable)?;
    let bytes = fs::read(&executable)?;
    let digest = tmt_core::content_digest::sha256(&bytes);
    let capabilities = probe(&executable, tmt)?;
    let consent = Consent {
        name: name.to_owned(),
        path: executable,
        digest,
        fingerprint: Fingerprint::of(&metadata),
        protocol: PROTOCOL_VERSION.to_owned(),
        capabilities,
        consented_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64),
    };
    let path = consent_path(&paths.global_dir);
    let mut extensions = read_consents(&path)?;
    extensions.retain(|existing| existing.name != name);
    extensions.push(consent.clone());
    extensions.sort_by(|left, right| left.name.cmp(&right.name));
    write_consents(&path, &extensions)?;
    Ok(consent)
}

/// A hook executable must be a regular executable file owned by the current
/// user, and neither it nor its directory may be writable by anyone else.
fn verify_ownership(executable: &Path) -> Result<fs::Metadata, ExtensionHookError> {
    let metadata = fs::metadata(executable)?;
    let uid = nix::unistd::getuid().as_raw();
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err(ExtensionHookError::Unsafe(format!(
            "{} is not an executable file.",
            executable.display()
        )));
    }
    if metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
        return Err(ExtensionHookError::Unsafe(format!(
            "{} must be owned by you and not writable by others.",
            executable.display()
        )));
    }
    let directory = executable.parent().unwrap_or(Path::new("/"));
    let parent = fs::metadata(directory)?;
    if parent.mode() & 0o022 != 0 {
        return Err(ExtensionHookError::Unsafe(format!(
            "{} must not be writable by others.",
            directory.display()
        )));
    }
    Ok(metadata)
}

fn invocation(executable: &Path, tmt: &Path, operation: &str) -> Vec<OsString> {
    let mut marker = OsString::from(DELIVERY_MARKER);
    marker.push("=1");
    let mut selector = OsString::from("TMT_EXECUTABLE=");
    selector.push(tmt);
    vec![
        marker,
        selector,
        executable.as_os_str().to_owned(),
        HOOK_COMMAND.into(),
        PROTOCOL_VERSION.into(),
        operation.into(),
    ]
}

fn probe(executable: &Path, tmt: &Path) -> Result<Vec<String>, ExtensionHookError> {
    let args = invocation(executable, tmt, "capabilities");
    let output = UnixCommandRunner
        .execute(CommandRequest {
            program: OsStr::new(ENV),
            args: &args,
            input: b"",
            deadline: Instant::now() + PROBE_DEADLINE,
            max_output_bytes: PROBE_OUTPUT_LIMIT,
        })
        .map_err(|_| {
            ExtensionHookError::Incompatible(
                "The extension did not answer the hook capability probe.".into(),
            )
        })?;
    let capabilities = decode_capabilities(&output.stdout).ok_or_else(|| {
        ExtensionHookError::Incompatible("The extension does not speak hook protocol 1.".into())
    })?;
    if !capabilities
        .iter()
        .any(|capability| capability == LIFECYCLE_CAPABILITY || capability == CONTEXT_CAPABILITY)
    {
        return Err(ExtensionHookError::Incompatible(
            "The extension offers no hook capability this tmt supports.".into(),
        ));
    }
    Ok(capabilities)
}

/// `TMT-HOOKS/1`, then distinct lowercase tokens, one per line.
pub fn decode_capabilities(bytes: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let body = text
        .strip_prefix(&format!("TMT-HOOKS/{PROTOCOL_VERSION}\n"))?
        .strip_suffix('\n')?;
    let mut seen = BTreeSet::new();
    for line in body.split('\n') {
        let token = !line.is_empty()
            && line.len() <= 64
            && line
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !token || !seen.insert(line.to_owned()) {
            return None;
        }
    }
    Some(seen.into_iter().collect())
}

/// Consents still valid now: same file, same owner, nothing else can write it.
fn verified(consents: Vec<Consent>, capability: &str) -> Vec<Consent> {
    consents
        .into_iter()
        .filter(|consent| {
            consent.capabilities.iter().any(|value| value == capability)
                && verify_ownership(&consent.path)
                    .is_ok_and(|metadata| Fingerprint::of(&metadata) == consent.fingerprint)
        })
        .collect()
}

// ---- Capture and delivery -------------------------------------------------

static CAPTURE_ALLOWED: AtomicBool = AtomicBool::new(false);
static OBSERVERS: OnceLock<Vec<Consent>> = OnceLock::new();
static PENDING: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());

/// Only the `tmt` CLI process emits observations; libraries and extensions
/// that open core storage never do.
pub fn allow_capture() {
    CAPTURE_ALLOWED.store(true, Ordering::Relaxed);
}

/// Whether a storage connection on `database` should capture lifecycle
/// evidence. The consent file is read at most once per process.
pub(crate) fn should_capture(database: &Path) -> bool {
    if !CAPTURE_ALLOWED.load(Ordering::Relaxed) || std::env::var_os(DELIVERY_MARKER).is_some() {
        return false;
    }
    let observers = OBSERVERS.get_or_init(|| {
        let global = database.parent().unwrap_or(Path::new("."));
        read_consents(&consent_path(global))
            .unwrap_or_default()
            .into_iter()
            .filter(|consent| {
                consent
                    .capabilities
                    .iter()
                    .any(|value| value == LIFECYCLE_CAPABILITY)
            })
            .collect()
    });
    !observers.is_empty()
}

/// Temporary, per-connection capture. Temporary tables take part in the
/// transaction, so rolled-back changes leave no evidence.
pub(crate) const CAPTURE_SQL: &str = "
CREATE TEMP TABLE IF NOT EXISTS tmt_lifecycle_events (
  seq INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  subject TEXT NOT NULL,
  lifetime TEXT,
  revision INTEGER,
  retired INTEGER NOT NULL
);
CREATE TEMP TRIGGER IF NOT EXISTS tmt_identity_created AFTER INSERT ON main.identities BEGIN
  INSERT INTO tmt_lifecycle_events (kind, subject, lifetime, retired)
  VALUES ('identity.created', NEW.id, NEW.lifetime, NEW.retired_at_ms IS NOT NULL);
END;
CREATE TEMP TRIGGER IF NOT EXISTS tmt_identity_retired AFTER UPDATE OF retired_at_ms ON main.identities
WHEN OLD.retired_at_ms IS NULL AND NEW.retired_at_ms IS NOT NULL BEGIN
  INSERT INTO tmt_lifecycle_events (kind, subject, lifetime, retired)
  VALUES ('identity.retired', NEW.id, NEW.lifetime, 1);
END;
CREATE TEMP TRIGGER IF NOT EXISTS tmt_room_created AFTER INSERT ON main.office_meeting_rooms BEGIN
  INSERT INTO tmt_lifecycle_events (kind, subject, revision, retired)
  VALUES ('room.created', NEW.room_id, NEW.revision, NEW.retired);
END;
CREATE TEMP TRIGGER IF NOT EXISTS tmt_room_changed AFTER UPDATE ON main.office_meeting_rooms
WHEN NEW.revision IS NOT OLD.revision BEGIN
  INSERT INTO tmt_lifecycle_events (kind, subject, revision, retired)
  VALUES (CASE WHEN OLD.retired = 0 AND NEW.retired = 1 THEN 'room.retired' ELSE 'room.updated' END,
          NEW.room_id, NEW.revision, NEW.retired);
END;";

/// Moves committed evidence from a capturing connection into this process's
/// delivery queue. Called before the connection closes.
pub(crate) fn drain(connection: &Connection) {
    if let Ok(events) = collect(connection)
        && !events.is_empty()
        && let Ok(mut pending) = PENDING.lock()
    {
        pending.extend(events);
    }
}

/// Reads and clears the connection's committed lifecycle evidence.
fn collect(connection: &Connection) -> rusqlite::Result<Vec<serde_json::Value>> {
    {
        let rows = connection
            .prepare("SELECT kind, subject, lifetime, revision, retired FROM temp.tmt_lifecycle_events ORDER BY seq")?
            .query_map([], |row| {
                let kind: String = row.get(0)?;
                let subject: String = row.get(1)?;
                let retired: bool = row.get(4)?;
                Ok(if kind.starts_with("identity.") {
                    serde_json::json!({"kind": kind, "identityId": subject, "lifetime": row.get::<_, Option<String>>(2)?, "retired": retired})
                } else {
                    serde_json::json!({"kind": kind, "roomId": subject, "revision": row.get::<_, Option<i64>>(3)?, "retired": retired})
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        connection.execute("DELETE FROM temp.tmt_lifecycle_events", [])?;
        Ok(rows)
    }
}

/// Delivers queued observations to every still-verified observer, in order,
/// under one aggregate deadline and output budget. Output is discarded and no
/// result changes the caller's outcome. Returns how many observers ran.
pub fn deliver_pending() -> usize {
    let events = match PENDING.lock() {
        Ok(mut pending) if !pending.is_empty() => std::mem::take(&mut *pending),
        _ => return 0,
    };
    let Ok(tmt) = std::env::current_exe() else {
        return 0;
    };
    let tmt = tmt.as_path();
    let observers = verified(
        OBSERVERS.get().cloned().unwrap_or_default(),
        LIFECYCLE_CAPABILITY,
    );
    let input = serde_json::json!({"version": 1, "events": events}).to_string();
    let deadline = Instant::now() + DELIVERY_DEADLINE;
    let mut budget = DELIVERY_OUTPUT_LIMIT;
    let mut delivered = 0;
    for observer in observers {
        if Instant::now() >= deadline || budget == 0 {
            break;
        }
        let args = invocation(&observer.path, tmt, "observe");
        let result = UnixCommandRunner.execute(CommandRequest {
            program: OsStr::new(ENV),
            args: &args,
            input: input.as_bytes(),
            deadline,
            max_output_bytes: budget,
        });
        let used = match &result {
            Ok(output) => output.stdout.len() + output.stderr.len(),
            Err(error) => error
                .output
                .as_ref()
                .map_or(budget, |output| output.stdout.len() + output.stderr.len()),
        };
        budget = budget.saturating_sub(used.max(1));
        delivered += 1;
    }
    delivered
}

#[cfg(test)]
mod tests;
