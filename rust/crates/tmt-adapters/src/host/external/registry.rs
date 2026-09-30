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
use tmt_core::host::HostKind;
use tmt_driver_protocol::{Capabilities, Grammar, PROTOCOL};

pub const REGISTRY_FILE: &str = "drivers.json";
const REGISTRY_LIMIT: usize = 64 * 1024;
/// Registered grammars are kept for the process's life, so the count is small.
pub const MAX_DRIVERS: usize = 16;

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
/// the same name.
pub fn approve(
    global_dir: &Path,
    executable: &Path,
    runner: &impl CommandRunner,
) -> Result<DriverRecord, RegistryError> {
    let executable = fs::canonicalize(executable)?;
    let metadata = executable_trust::verify_ownership(&executable)?;
    let digest = executable_trust::digest(&executable)?;
    let (capabilities, grammar) = process::probe(runner, &executable).map_err(|reason| {
        RegistryError::Refused(format!(
            "{} is not a host driver: {reason}",
            executable.display()
        ))
    })?;
    let name = grammar.name().to_owned();
    if let Some(reason) = builtin_conflict(&grammar) {
        return Err(RegistryError::Refused(format!(
            "Host driver {name} can't be installed: {reason}."
        )));
    }
    let mut drivers = read(global_dir)?;
    drivers.retain(|existing| existing.name != name);
    for existing in &drivers {
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
    if drivers.len() >= MAX_DRIVERS {
        return Err(RegistryError::Refused(format!(
            "At most {MAX_DRIVERS} host drivers can be installed."
        )));
    }
    let record = DriverRecord {
        name,
        path: executable,
        digest,
        fingerprint: Fingerprint::of(&metadata),
        protocol: PROTOCOL,
        capabilities,
        approved_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64),
    };
    drivers.push(record.clone());
    drivers.sort_by(|left, right| left.name.cmp(&right.name));
    write(global_dir, &drivers)?;
    Ok(record)
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

/// A built-in host's name, or IDs and targets a built-in host would read as
/// its own.
fn builtin_conflict(grammar: &Grammar) -> Option<String> {
    let sample_id = format!("{}1", grammar.pane_id_prefix());
    HostKind::ALL.into_iter().find_map(|host| {
        if grammar.name() == host.as_str() {
            Some(format!("{} is a built-in host", host.as_str()))
        } else if host.is_pane_id(&sample_id) {
            Some(format!("its pane IDs look like {}'s", host.as_str()))
        } else if grammar
            .sample_target()
            .is_some_and(|target| host.is_target(&target))
        {
            Some(format!("its targets look like {}'s", host.as_str()))
        } else {
            None
        }
    })
}
