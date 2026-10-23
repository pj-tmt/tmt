//! Consented host and runtime drivers, in `<global>/drivers.json` (0600, replaced
//! atomically). There is no PATH discovery: a driver runs only from a record
//! made when the user approved that exact executable. Approval checks that
//! the user owns it and nothing else can write it, asks for its
//! capabilities, and refuses a declaration that could be read as a built-in
//! host's or another driver's. `tmt driver install|ls|rm` (#570 slice 6) is
//! its command front end.

use super::{Declaration, process};
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
use tmt_driver_protocol::{
    Grammar, LocationsRequest, LocationsResponse, Op, PROTOCOL, RuntimeDeclaration,
};

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
    pub capabilities: Declaration,
    /// Runtime locations disclosed at approval. Setup refreshes through the same admission helper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locations: Option<LocationsResponse>,
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
    /// First-party drivers the user removed. They are not approved again
    /// automatically ([`approve_shipped`]); `tmt driver install` clears the
    /// name. Written only when not empty, so a registry that never saw a
    /// removal reads the same in a tmt from before this list existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    declined: Vec<String>,
}

#[derive(Debug)]
pub enum RegistryError {
    /// The executable isn't one TMT may run.
    Unsafe(String),
    /// It is not a driver this tmt can use, or its declaration collides.
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
            Self::Io(error) => write!(output, "The driver registry is unavailable: {error}"),
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
    Ok(read_document(global_dir)?.drivers)
}

fn read_document(global_dir: &Path) -> Result<RegistryDocument, RegistryError> {
    let path = registry_path(global_dir);
    let bytes = match crate::bounded_file::read_no_follow(&path, REGISTRY_LIMIT) {
        Ok(bytes) => bytes,
        Err(crate::bounded_file::FileReadError::Io(error))
            if error.kind() == io::ErrorKind::NotFound =>
        {
            return Ok(RegistryDocument {
                version: 1,
                ..RegistryDocument::default()
            });
        }
        Err(error) => return Err(RegistryError::Io(io::Error::other(error.to_string()))),
    };
    let document: RegistryDocument = serde_json::from_slice(&bytes)
        .map_err(|_| RegistryError::Invalid("The driver registry is not valid.".into()))?;
    if document.version != 1 {
        return Err(RegistryError::Invalid(
            "The driver registry uses an unsupported version.".into(),
        ));
    }
    Ok(document)
}

fn write(global_dir: &Path, document: &RegistryDocument) -> Result<(), RegistryError> {
    let bytes = serde_json::to_vec_pretty(document).map_err(io::Error::other)?;
    if bytes.len() > REGISTRY_LIMIT {
        return Err(RegistryError::Invalid(
            "The approvals exceed the registry byte limit; nothing changed.".into(),
        ));
    }
    Ok(crate::private_file::replace(
        &registry_path(global_dir),
        &bytes,
    )?)
}

/// Approves `executable` as either kind, replacing an earlier approval of
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
    let mut record = described(executable, DriverSource::Path, None, runner)?;
    admissible(&record, &read(global_dir)?)?;
    locate(&mut record, runner)?;
    Ok(record)
}

/// Resolve and admit runtime write targets before either approval path can consent.
fn locate(record: &mut DriverRecord, runner: &impl CommandRunner) -> Result<(), RegistryError> {
    if let Declaration::Runtime(value) = &record.capabilities {
        let home = std::env::home_dir()
            .and_then(|path| path.into_os_string().into_string().ok())
            .ok_or_else(|| RegistryError::Refused("Could not resolve the current home.".into()))?;
        let env = value
            .env
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
            .collect();
        let process = process::DriverProcess::open_runtime(record.clone(), runner)
            .map_err(|error| RegistryError::Refused(error.to_string()))?;
        record.locations = Some(
            process
                .locations(
                    LocationsRequest { home, env },
                    std::time::Instant::now() + Op::Locations.bounds().deadline,
                )
                .map_err(|error| RegistryError::Refused(error.to_string()))?,
        );
    }
    Ok(())
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
    let mut record = record;
    locate(&mut record, runner)?;
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
    let capabilities = process::probe(runner, &executable).map_err(|reason| {
        RegistryError::Refused(format!(
            "{} is not a driver: {reason}",
            executable.display()
        ))
    })?;
    Ok(DriverRecord {
        name: capabilities.name().to_owned(),
        path: executable,
        digest,
        fingerprint: Fingerprint::of(&metadata),
        protocol: PROTOCOL,
        capabilities,
        locations: None,
        approved_at_ms: 0,
        source,
    })
}

/// Serializes every read-modify-write of the registry, so an approval, a
/// removal and a first-party adoption in concurrent tmt processes can't
/// drop one another. The write itself is a staged file renamed into place.
fn locked(global_dir: &Path) -> Result<nix::fcntl::Flock<fs::File>, RegistryError> {
    fs::create_dir_all(global_dir)?;
    Ok(crate::file_lock::exclusive(
        &global_dir.join("drivers.lock"),
    )?)
}

/// Records an inspected driver, after checking again against the registry
/// as it is now, since another approval may have landed in between.
/// An explicit approval also lifts an earlier removal of that name.
pub fn commit(global_dir: &Path, record: DriverRecord) -> Result<DriverRecord, RegistryError> {
    let _lock = locked(global_dir)?;
    let mut document = read_document(global_dir)?;
    admissible(&record, &document.drivers)?;
    document
        .declined
        .retain(|declined| *declined != record.name);
    let record = insert(&mut document, record);
    write(global_dir, &document)?;
    Ok(record)
}

/// Puts `record` in its place, replacing an approval of the same name.
fn insert(document: &mut RegistryDocument, mut record: DriverRecord) -> DriverRecord {
    document
        .drivers
        .retain(|existing| existing.name != record.name);
    record.approved_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    document.drivers.push(record.clone());
    document
        .drivers
        .sort_by(|left, right| left.name.cmp(&right.name));
    record
}

/// What an automatic approval of a shipped driver changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Automatic {
    /// The first use of the host it serves: there was no approval.
    First(DriverRecord),
    /// A release shipped a driver that declares more than the approval
    /// covered, and `reason` says what ([`beyond`]).
    Widened {
        record: DriverRecord,
        reason: String,
    },
}

/// Approves the first-party driver `name` that the running `tmt` ships when
/// no one needs to be asked: the file is the one its release receipt records,
/// so it carries no trust beyond the `tmt` already running (#3220). Nothing
/// is recorded for a driver the user removed, for a path approval of that
/// name, or when the release ships no usable driver.
///
/// - No approval and `host_present` (the caller's environment shows the host
///   the driver serves): approved, as `tmt driver install <name> --yes` would.
/// - An approval of a shipped driver the release replaced with one declaring
///   more than was approved: approved again. A replacement within the
///   approval is adopted by [`resolve_first_party`] without a word.
///
/// Both checks repeat under the registry lock, so concurrent processes
/// approve once and only one of them reports it.
pub fn approve_shipped(
    global_dir: &Path,
    name: &str,
    tmt: &Path,
    host_present: bool,
    runner: &impl CommandRunner,
) -> Result<Option<Automatic>, RegistryError> {
    if !is_first_party(name) {
        return Ok(None);
    }
    let document = read_document(global_dir)?;
    if document.declined.iter().any(|declined| declined == name) {
        return Ok(None);
    }
    match document.drivers.iter().find(|record| record.name == name) {
        None if !host_present => Ok(None),
        None => {
            let record = inspect_first_party(global_dir, name, tmt, runner)?;
            Ok(replace_unless_changed(global_dir, record, None)?.map(Automatic::First))
        }
        Some(approved) if approved.source != DriverSource::FirstParty => Ok(None),
        Some(approved) => {
            let Ok(Some(current)) = shipped_current(approved, tmt, runner) else {
                return Ok(None);
            };
            let Some(reason) = beyond(approved, &current) else {
                return Ok(None);
            };
            Ok(replace_unless_changed(global_dir, current, Some(approved))?
                .map(|record| Automatic::Widened { record, reason }))
        }
    }
}

/// Records `record` if the registry still holds `expected` under its name
/// (none for a first approval) and the user has not removed that name.
fn replace_unless_changed(
    global_dir: &Path,
    record: DriverRecord,
    expected: Option<&DriverRecord>,
) -> Result<Option<DriverRecord>, RegistryError> {
    let _lock = locked(global_dir)?;
    let mut document = read_document(global_dir)?;
    let held = document
        .drivers
        .iter()
        .find(|existing| existing.name == record.name);
    if held != expected || document.declined.contains(&record.name) {
        return Ok(None);
    }
    admissible(&record, &document.drivers)?;
    let record = insert(&mut document, record);
    write(global_dir, &document)?;
    Ok(Some(record))
}

/// Whether `record` may join `drivers`, replacing one of its own name.
fn admissible(record: &DriverRecord, drivers: &[DriverRecord]) -> Result<(), RegistryError> {
    let name = &record.name;
    if record.capabilities.name() != name
        || tmt_core::driver::ALL
            .iter()
            .any(|driver| driver.name == name)
    {
        return Err(RegistryError::Refused(format!(
            "Driver {name} uses a reserved or mismatched name."
        )));
    }
    let others: Vec<&DriverRecord> = drivers
        .iter()
        .filter(|existing| existing.name != *name)
        .collect();
    match &record.capabilities {
        Declaration::Host(value) => {
            let grammar = Grammar::from_capabilities(value).map_err(|error| {
                RegistryError::Refused(format!(
                    "{} is not a host driver: {error}",
                    record.path.display()
                ))
            })?;
            if let Some(reason) = tmt_core::host::builtin_conflict(grammar.host()) {
                return Err(RegistryError::Refused(format!(
                    "Host driver {name} can't be installed: {reason}."
                )));
            }
            for existing in &others {
                if let Some(value) = existing.capabilities.host() {
                    let Ok(other) = Grammar::from_capabilities(value) else {
                        continue;
                    };
                    if let Some(reason) = grammar.conflict(&other) {
                        return Err(RegistryError::Refused(format!(
                            "Host driver {name} can't be installed beside {}: {reason}.",
                            existing.name
                        )));
                    }
                }
            }
        }
        Declaration::Runtime(value) => {
            let declaration = RuntimeDeclaration::new(value)
                .map_err(|error| RegistryError::Refused(error.to_string()))?;
            if tmt_core::host::HostKind::ALL
                .iter()
                .any(|host| host.as_str() == name)
            {
                return Err(RegistryError::Refused(format!(
                    "Driver {name} uses a built-in host name."
                )));
            }
            for executable in &value.executables {
                if !declaration.claims(executable) {
                    continue;
                }
                let builtin = tmt_core::driver::ALL
                    .iter()
                    .any(|driver| driver.executables.contains(&executable.as_str()));
                let approved = others.iter().any(|existing| match &existing.capabilities {
                    Declaration::Runtime(value) => {
                        value.claims && value.executables.contains(executable)
                    }
                    _ => false,
                });
                if builtin || approved {
                    return Err(RegistryError::Refused(format!(
                        "Command {executable} is already claimed."
                    )));
                }
            }
        }
    }
    if drivers.iter().any(|existing| {
        existing.name == *name && existing.capabilities.kind() != record.capabilities.kind()
    }) {
        return Err(RegistryError::Refused(format!(
            "Driver name {name} is already approved for another kind."
        )));
    }
    // Each approved host is registered for a process's life (`tmt_core::host`).
    if others.len() >= MAX_EXTERNAL_HOSTS {
        return Err(RegistryError::Refused(format!(
            "At most {MAX_EXTERNAL_HOSTS} drivers can be installed."
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

/// The state a driver would be found in when next run, and why when it
/// isn't `ok`: the same ownership, fingerprint and digest checks a call
/// makes. A first-party driver is resolved as a run would resolve it
/// ([`resolve_first_party`]), then its bytes are checked against the receipt.
pub fn state(
    global_dir: &Path,
    record: &DriverRecord,
    tmt: &Path,
    runner: &impl CommandRunner,
) -> (ApprovalState, Option<String>) {
    if record.source == DriverSource::FirstParty {
        return match resolve_first_party(global_dir, record, tmt, runner) {
            Ok(current) => match executable_trust::digest(&current.path) {
                Ok(digest) if digest == current.digest => (ApprovalState::Ok, None),
                _ => (
                    ApprovalState::Changed,
                    Some("the shipped driver doesn't match its release receipt".into()),
                ),
            },
            Err(gap) => (gap.state(), Some(gap.reason)),
        };
    }
    match fs::symlink_metadata(&record.path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return (
                ApprovalState::Missing,
                Some("nothing is at the approved path".into()),
            );
        }
        _ => {}
    }
    let same = executable_trust::unchanged(&record.path, &record.fingerprint)
        && executable_trust::digest(&record.path).is_ok_and(|digest| digest == record.digest);
    if same {
        (ApprovalState::Ok, None)
    } else {
        (
            ApprovalState::Changed,
            Some("the executable changed since it was approved".into()),
        )
    }
}

/// Why a first-party driver can't run as approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstPartyGap {
    /// The running tmt ships no such driver, as opposed to shipping one that
    /// can't be used as approved.
    pub not_shipped: bool,
    pub reason: String,
}

impl FirstPartyGap {
    fn not_shipped(reason: String) -> Self {
        Self {
            not_shipped: true,
            reason,
        }
    }

    fn unusable(reason: String) -> Self {
        Self {
            not_shipped: false,
            reason,
        }
    }

    pub fn state(&self) -> ApprovalState {
        if self.not_shipped {
            ApprovalState::Missing
        } else {
            ApprovalState::Changed
        }
    }
}

/// What a new description declares beyond the approved one, if anything.
/// An approval covers what the user was shown: the same protocol, pane-ID
/// and target syntax, and no operation or environment variable beyond it.
fn beyond(approved: &DriverRecord, current: &DriverRecord) -> Option<String> {
    let (Declaration::Host(old), Declaration::Host(new)) =
        (&approved.capabilities, &current.capabilities)
    else {
        return Some("runtime or changed-kind upgrades require approval again".into());
    };
    if !new.protocols.contains(&approved.protocol) {
        return Some(format!(
            "it no longer speaks protocol {}",
            approved.protocol
        ));
    }
    if new.pane_id != old.pane_id || new.target != old.target {
        return Some("it changed its pane-ID or target syntax".into());
    }
    let ops: Vec<&str> = new
        .ops
        .iter()
        .filter(|op| !old.ops.contains(op))
        .map(String::as_str)
        .collect();
    if !ops.is_empty() {
        return Some(format!("it now also runs {}", ops.join(", ")));
    }
    let env: Vec<&str> = new
        .caller_env
        .iter()
        .filter(|name| !old.caller_env.contains(name))
        .map(String::as_str)
        .collect();
    if !env.is_empty() {
        return Some(format!("it now also reads {}", env.join(", ")));
    }
    // The launch template is a command line Ops runs; a new or changed one was not approved.
    if new.launch.is_some() && new.launch != old.launch {
        return Some("it now opens panes with a command you have not approved".into());
    }
    None
}

/// A first-party record as the running release ships it.
///
/// While the release ships the approved digest, the record as written. After
/// an upgrade the new driver is described (its receipt digest and one
/// `capabilities` probe) and adopted under the same approval, asking nothing,
/// only when it declares nothing beyond what was approved ([`beyond`]); the
/// record is then rewritten under the registry lock. Same name and syntax
/// leave every conflict check as it was. Anything else is a gap: not
/// shipped (`missing`), or shipped but needing approval again (`changed`).
pub fn resolve_first_party(
    global_dir: &Path,
    record: &DriverRecord,
    tmt: &Path,
    runner: &impl CommandRunner,
) -> Result<DriverRecord, FirstPartyGap> {
    let Some(mut current) = shipped_current(record, tmt, runner)? else {
        return Ok(record.clone());
    };
    if let Some(reason) = beyond(record, &current) {
        return Err(FirstPartyGap::unusable(format!(
            "the upgraded driver needs approval again: {reason}"
        )));
    }
    current.approved_at_ms = record.approved_at_ms;
    let unwritable = |error: RegistryError| FirstPartyGap::unusable(error.to_string());
    let _lock = locked(global_dir).map_err(unwritable)?;
    let mut document = read_document(global_dir).map_err(unwritable)?;
    // Removed or replaced since it was read: that approval stands.
    if !document.drivers.iter().any(|existing| existing == record) {
        return Err(FirstPartyGap::unusable(
            "the approval changed while it was being updated".into(),
        ));
    }
    document
        .drivers
        .retain(|existing| existing.name != current.name);
    document.drivers.push(current.clone());
    document
        .drivers
        .sort_by(|left, right| left.name.cmp(&right.name));
    write(global_dir, &document).map_err(unwritable)?;
    Ok(current)
}

/// The driver the running release ships under `record`'s name, described
/// (its receipt digest and one `capabilities` probe), or none when it is the
/// very file `record` approves.
fn shipped_current(
    record: &DriverRecord,
    tmt: &Path,
    runner: &impl CommandRunner,
) -> Result<Option<DriverRecord>, FirstPartyGap> {
    let name = &record.name;
    let companion = crate::native_install::active_companion(tmt, &first_party_file(name))
        .map_err(|error| {
            FirstPartyGap::not_shipped(format!("this tmt is not a managed installation: {error}"))
        })?
        .ok_or_else(|| {
            FirstPartyGap::not_shipped(format!("this tmt release ships no {name} driver"))
        })?;
    if companion.sha256 == record.digest && companion.path == record.path {
        return Ok(None);
    }
    let current = described(
        &companion.path,
        DriverSource::FirstParty,
        Some(&companion.sha256),
        runner,
    )
    .map_err(|error| FirstPartyGap::unusable(error.to_string()))?;
    if current.name != *name {
        return Err(FirstPartyGap::unusable(format!(
            "the shipped driver now calls itself {}",
            current.name
        )));
    }
    Ok(Some(current))
}

/// [`resolve_first_party`] for a run: the record to use, or none, so the
/// host reads as unavailable until the driver is approved again.
pub fn current_first_party(
    global_dir: &Path,
    record: &DriverRecord,
    tmt: &Path,
    runner: &impl CommandRunner,
) -> Option<DriverRecord> {
    resolve_first_party(global_dir, record, tmt, runner).ok()
}

/// Removes an approval; whether there was one. Removing a first-party
/// driver is remembered, so it is not approved again automatically.
pub fn remove(global_dir: &Path, name: &str) -> Result<bool, RegistryError> {
    let _lock = locked(global_dir)?;
    let mut document = read_document(global_dir)?;
    let before = document.drivers.len();
    document.drivers.retain(|existing| existing.name != name);
    if document.drivers.len() == before {
        return Ok(false);
    }
    if is_first_party(name) && !document.declined.iter().any(|declined| declined == name) {
        document.declined.push(name.to_owned());
    }
    write(global_dir, &document)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(launch: Option<&[&str]>) -> DriverRecord {
        let mut capabilities = json!({"kind": "host", "name": "screen", "version": "test",
            "protocols": [1], "ops": ["snapshot"], "paneId": {"prefix": "term_"},
            "target": "w{n}:p{n}", "callerEnv": []});
        if let Some(launch) = launch {
            capabilities["launch"] = json!(launch);
        }
        serde_json::from_value(json!({
            "name": "screen", "path": "/tmp/driver", "digest": "test", "protocol": 1,
            "fingerprint": {"device": 0, "inode": 0, "size": 0, "modifiedNs": 0,
                "changedNs": 0, "uid": 0, "mode": 0}, "approvedAtMs": 0,
            "capabilities": capabilities
        }))
        .unwrap()
    }

    #[test]
    fn a_new_or_changed_launch_template_needs_approval_again() {
        let open = ["screen-open", "{session}", "{command}"];
        let other = ["screen-open", "--detach", "{session}", "{command}"];
        // The command line Ops would run was never shown to the user.
        let added = beyond(&record(None), &record(Some(&open))).unwrap();
        assert!(added.contains("opens panes with a command you have not approved"));
        assert!(beyond(&record(Some(&open)), &record(Some(&other))).is_some());
        // The same template, or none at all, widens nothing.
        assert_eq!(beyond(&record(Some(&open)), &record(Some(&open))), None);
        assert_eq!(beyond(&record(Some(&open)), &record(None)), None);
        assert_eq!(beyond(&record(None), &record(None)), None);
    }
}
