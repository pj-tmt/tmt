//! The consented host drivers, in `<global>/drivers.json` (0600, replaced
//! atomically). There is no PATH discovery: a driver runs only from a record
//! made when the user approved that exact executable. Approval checks that
//! the user owns it and nothing else can write it, asks for its
//! capabilities, and refuses a declaration that could be read as a built-in
//! host's or another driver's. `tmt driver install|ls|rm` (#570 slice 6) is
//! its command front end.

use super::process;
use crate::{
    executable_trust::{self, Fingerprint, TrustError},
    process::CommandRunner,
};
use serde::{Deserialize, Serialize};
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tmt_core::host::MAX_EXTERNAL_HOSTS;
use tmt_driver_protocol::{Capabilities, Grammar, PROTOCOL};

pub const REGISTRY_FILE: &str = "drivers.json";
const REGISTRY_LIMIT: usize = 64 * 1024;

/// One approved executable and what it declared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DriverRecord {
    pub name: String,
    pub path: PathBuf,
    pub digest: String,
    pub fingerprint: Fingerprint,
    pub protocol: u32,
    pub capabilities: Capabilities,
    pub approved_at_ms: u64,
    /// Where the approved executable comes from. A path approval is pinned
    /// to its digest; a first-party one follows the release that ships it.
    /// Written only when not `Path`, so a registry of path approvals reads
    /// the same in a tmt from before sources existed.
    #[serde(default, skip_serializing_if = "DriverSource::is_path")]
    pub source: DriverSource,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DriverSource {
    /// `tmt driver install <path>`: exactly that executable, pinned to its
    /// digest; any change needs approval again.
    #[default]
    Path,
    /// `tmt driver install <name>`: the driver shipped with tmt as a
    /// companion of the active release. It stays approved across upgrades
    /// while it matches the SHA-256 that release's receipt records.
    FirstParty,
}

impl DriverSource {
    fn is_path(&self) -> bool {
        *self == Self::Path
    }
}

/// Whether `name` names a first-party driver some tmt release may ship.
pub fn is_first_party(name: &str) -> bool {
    tmt_core::native_install::Product::Cli
        .companions()
        .contains(&first_party_file(name).as_str())
}

/// The companion file a first-party driver named `name` ships as.
pub fn first_party_file(name: &str) -> String {
    format!("tmt-driver-{name}")
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryDocument {
    version: u8,
    drivers: Vec<DriverRecord>,
}

#[derive(Debug)]
pub enum RegistryError {
    /// The executable isn't one TMT may run.
    Unsafe(String),
    /// It isn't a host driver this tmt can use, or it collides with one.
    Refused(String),
    /// `drivers.json` isn't a registry this tmt reads.
    Invalid(String),
    Io(io::Error),
}

impl RegistryError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsafe(_) => "DRIVER_UNSAFE",
            Self::Refused(_) => "DRIVER_REFUSED",
            Self::Invalid(_) => "DRIVER_REGISTRY_INVALID",
            Self::Io(_) => "DRIVER_REGISTRY_UNAVAILABLE",
        }
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsafe(message) | Self::Refused(message) | Self::Invalid(message) => {
                output.write_str(message)
            }
            Self::Io(error) => write!(output, "The host driver registry is unavailable: {error}"),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<io::Error> for RegistryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<TrustError> for RegistryError {
    fn from(error: TrustError) -> Self {
        match error {
            TrustError::Unsafe(message) => Self::Unsafe(message),
            TrustError::Io(error) => Self::Io(error),
        }
    }
}

pub fn registry_path(global_dir: &Path) -> PathBuf {
    global_dir.join(REGISTRY_FILE)
}

/// The approved drivers; none when the registry doesn't exist.
pub fn read(global_dir: &Path) -> Result<Vec<DriverRecord>, RegistryError> {
    let path = registry_path(global_dir);
    let bytes = match crate::bounded_file::read_no_follow(&path, REGISTRY_LIMIT) {
        Ok(bytes) => bytes,
        Err(crate::bounded_file::FileReadError::Io(error))
            if error.kind() == io::ErrorKind::NotFound =>
        {
            return Ok(Vec::new());
        }
        Err(error) => return Err(RegistryError::Io(io::Error::other(error.to_string()))),
    };
    let document: RegistryDocument = serde_json::from_slice(&bytes)
        .map_err(|_| RegistryError::Invalid("The host driver registry is not valid.".into()))?;
    if document.version != 1 {
        return Err(RegistryError::Invalid(
            "The host driver registry uses an unsupported version.".into(),
        ));
    }
    Ok(document.drivers)
}

fn write(global_dir: &Path, drivers: &[DriverRecord]) -> Result<(), RegistryError> {
    let bytes = serde_json::to_vec_pretty(&RegistryDocument {
        version: 1,
        drivers: drivers.to_vec(),
    })
    .map_err(io::Error::other)?;
    Ok(crate::private_file::replace(
        &registry_path(global_dir),
        &bytes,
    )?)
}

/// Approves `executable` as a host driver, replacing an earlier approval of
/// the same name: [`inspect`], then [`commit`].
pub fn approve(
    global_dir: &Path,
    executable: &Path,
    runner: &impl CommandRunner,
) -> Result<DriverRecord, RegistryError> {
    commit(global_dir, inspect(global_dir, executable, runner)?)
}

/// Everything approval checks, without recording anything: the executable is
/// safely owned, answers `capabilities`, and could not be read as a built-in
/// host or another approved driver. The record is what `commit` would write,
/// so a user can be shown it before consenting.
pub fn inspect(
    global_dir: &Path,
    executable: &Path,
    runner: &impl CommandRunner,
) -> Result<DriverRecord, RegistryError> {
    let record = described(executable, DriverSource::Path, None, runner)?;
    admissible(&record, &read(global_dir)?)?;
    Ok(record)
}

/// [`inspect`] for the first-party driver `name` that the running `tmt`
/// (`executable`) ships as a companion of its active release. The file must
/// match the SHA-256 the release's receipt records, and the driver must call
/// itself `name`.
pub fn inspect_first_party(
    global_dir: &Path,
    name: &str,
    tmt: &Path,
    runner: &impl CommandRunner,
) -> Result<DriverRecord, RegistryError> {
    let companion = crate::native_install::active_companion(tmt, &first_party_file(name))
        .map_err(|error| {
            RegistryError::Refused(format!(
                "The {name} driver ships only with a managed native installation ({error}). Approve a driver executable with: tmt driver install <path>"
            ))
        })?
        .ok_or_else(|| {
            RegistryError::Refused(format!("This tmt release ships no {name} driver."))
        })?;
    let record = described(
        &companion.path,
        DriverSource::FirstParty,
        Some(&companion.sha256),
        runner,
    )?;
    if record.name != name {
        return Err(RegistryError::Refused(format!(
            "The shipped {name} driver calls itself {}.",
            record.name
        )));
    }
    admissible(&record, &read(global_dir)?)?;
    Ok(record)
}

/// The record approving `executable` would write: ownership, the digest
/// (equal to `expected` when one is given), and one `capabilities` probe.
fn described(
    executable: &Path,
    source: DriverSource,
    expected: Option<&str>,
    runner: &impl CommandRunner,
) -> Result<DriverRecord, RegistryError> {
    let executable = fs::canonicalize(executable)?;
    let metadata = executable_trust::verify_ownership(&executable)?;
    let digest = executable_trust::digest(&executable)?;
    if expected.is_some_and(|expected| expected != digest) {
        return Err(RegistryError::Unsafe(format!(
            "{} does not match its release receipt.",
            executable.display()
        )));
    }
    let (capabilities, _) = process::probe(runner, &executable).map_err(|reason| {
        RegistryError::Refused(format!(
            "{} is not a host driver: {reason}",
            executable.display()
        ))
    })?;
    Ok(DriverRecord {
        name: capabilities.name.clone(),
        path: executable,
        digest,
        fingerprint: Fingerprint::of(&metadata),
        protocol: PROTOCOL,
        capabilities,
        approved_at_ms: 0,
        source,
    })
}

/// Records an inspected driver, after checking again against the registry
/// as it is now, since another approval may have landed in between.
pub fn commit(global_dir: &Path, mut record: DriverRecord) -> Result<DriverRecord, RegistryError> {
    let mut drivers = read(global_dir)?;
    admissible(&record, &drivers)?;
    drivers.retain(|existing| existing.name != record.name);
    record.approved_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    drivers.push(record.clone());
    drivers.sort_by(|left, right| left.name.cmp(&right.name));
    write(global_dir, &drivers)?;
    Ok(record)
}

/// Whether `record` may join `drivers`, replacing one of its own name.
fn admissible(record: &DriverRecord, drivers: &[DriverRecord]) -> Result<(), RegistryError> {
    let grammar = Grammar::from_capabilities(&record.capabilities).map_err(|error| {
        RegistryError::Refused(format!(
            "{} is not a host driver: {error}",
            record.path.display()
        ))
    })?;
    let name = &record.name;
    if let Some(reason) = tmt_core::host::builtin_conflict(grammar.host()) {
        return Err(RegistryError::Refused(format!(
            "Host driver {name} can't be installed: {reason}."
        )));
    }
    let others: Vec<&DriverRecord> = drivers
        .iter()
        .filter(|existing| existing.name != *name)
        .collect();
    for existing in &others {
        let Ok(other) = Grammar::from_capabilities(&existing.capabilities) else {
            continue;
        };
        if let Some(reason) = grammar.conflict(&other) {
            return Err(RegistryError::Refused(format!(
                "Host driver {name} can't be installed beside {}: {reason}.",
                existing.name
            )));
        }
    }
    // Each approved host is registered for a process's life (`tmt_core::host`).
    if others.len() >= MAX_EXTERNAL_HOSTS {
        return Err(RegistryError::Refused(format!(
            "At most {MAX_EXTERNAL_HOSTS} host drivers can be installed."
        )));
    }
    Ok(())
}

/// Whether an approved driver can still run as approved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalState {
    Ok,
    /// The executable is no longer the one approved: changed, replaced, or
    /// no longer safely owned.
    Changed,
    /// Nothing is at the approved path.
    Missing,
}

impl ApprovalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Changed => "changed",
            Self::Missing => "missing",
        }
    }
}

/// The state a driver would be found in when next run: the same ownership,
/// fingerprint and digest checks a call makes. A first-party driver is
/// checked against the release that ships it now, so an upgrade keeps it
/// `ok` and only a file that doesn't match its receipt is `changed`.
pub fn state(record: &DriverRecord, tmt: &Path) -> ApprovalState {
    if record.source == DriverSource::FirstParty {
        let Ok(Some(companion)) =
            crate::native_install::active_companion(tmt, &first_party_file(&record.name))
        else {
            return ApprovalState::Missing;
        };
        return match executable_trust::digest(&companion.path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => ApprovalState::Missing,
            Ok(digest) if digest == companion.sha256 => ApprovalState::Ok,
            _ => ApprovalState::Changed,
        };
    }
    match fs::symlink_metadata(&record.path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return ApprovalState::Missing,
        _ => {}
    }
    let same = executable_trust::unchanged(&record.path, &record.fingerprint)
        && executable_trust::digest(&record.path).is_ok_and(|digest| digest == record.digest);
    if same {
        ApprovalState::Ok
    } else {
        ApprovalState::Changed
    }
}

/// A first-party record as the running release ships it. Unchanged while the
/// shipped driver is the approved one. After an upgrade the release ships a
/// new one: it is described again (its receipt digest, one `capabilities`
/// probe) and recorded under the same approval, nothing asked. `None` when
/// the release ships no such driver, it no longer calls itself by the
/// approved name, or it would now collide with another host; it then reads
/// as unavailable until approved again.
pub fn current_first_party(
    global_dir: &Path,
    record: &DriverRecord,
    tmt: &Path,
    runner: &impl CommandRunner,
) -> Option<DriverRecord> {
    let companion = crate::native_install::active_companion(tmt, &first_party_file(&record.name))
        .ok()
        .flatten()?;
    if companion.sha256 == record.digest && companion.path == record.path {
        return Some(record.clone());
    }
    let mut current = described(
        &companion.path,
        DriverSource::FirstParty,
        Some(&companion.sha256),
        runner,
    )
    .ok()?;
    if current.name != record.name {
        return None;
    }
    current.approved_at_ms = record.approved_at_ms;
    let mut drivers = read(global_dir).ok()?;
    admissible(&current, &drivers).ok()?;
    drivers.retain(|existing| existing.name != current.name);
    drivers.push(current.clone());
    drivers.sort_by(|left, right| left.name.cmp(&right.name));
    write(global_dir, &drivers).ok()?;
    Some(current)
}

/// Removes an approval; whether there was one.
pub fn remove(global_dir: &Path, name: &str) -> Result<bool, RegistryError> {
    let mut drivers = read(global_dir)?;
    let before = drivers.len();
    drivers.retain(|existing| existing.name != name);
    if drivers.len() == before {
        return Ok(false);
    }
    write(global_dir, &drivers)?;
    Ok(true)
}
