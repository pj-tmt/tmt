//! Resumable copy of Office rows out of the core database.
//!
//! `prepare` records the source identity in a private staging database,
//! `copy` writes every Office row from one consistent source snapshot in a
//! single staging transaction, and `verify` compares every typed cell against a
//! fresh snapshot. Each transition holds the migration lock and is atomic, so
//! an interruption leaves the previous state and the source is only read.
//! The source stays authoritative until [`switch`] publishes a verified copy.

use crate::{
    StorageLayout,
    cells::{self, Difference, Manifest, ManifestBuilder, TableShape},
    schema,
};

mod plan;
mod switch;
pub use plan::{Plan, PlanState, migrate, plan};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, config::DbConfig};
use std::{
    fmt, fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub(crate) use switch::ensure_active;
pub use switch::{
    Backup, Held, OfficeService, Quiesce, Quiesced, Recovery, Switched, recover, switch,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Absent,
    Prepared,
    Copied,
    Verified,
    /// `office.db` is published but not yet activated; recovery settles it.
    Switching,
    /// `office.db` is authoritative.
    Switched,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Prepared => "prepared",
            Self::Copied => "copied",
            Self::Verified => "verified",
            Self::Switching => "switching",
            Self::Switched => "switched",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "prepared" => Some(Self::Prepared),
            "copied" => Some(Self::Copied),
            "verified" => Some(Self::Verified),
            _ => None,
        }
    }
}

/// Observable migration progress. Row totals come from the staged copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    pub destination_exists: bool,
    pub source_schema_version: Option<i64>,
    pub source_manifest: Option<String>,
    pub rows: Vec<(String, u64)>,
    /// Retained switch backups, oldest first.
    pub backups: Vec<Backup>,
}

#[derive(Debug)]
pub enum MigrationError {
    /// Another migration step holds the lock.
    Busy,
    /// The source is missing, replaced, unreadable or has another Office shape.
    Source(String),
    /// Office rows changed since the copy; run `copy` again.
    SourceChanged,
    /// The staged copy differs from the source.
    Mismatch(String),
    /// The requested step does not follow the recorded state.
    State(String),
    /// Office storage or its staging area cannot be used.
    Destination(String),
    /// A directory the migration must write is not writable.
    NotWritable(PathBuf),
    /// The Office service could not be stopped and held stopped.
    Service(String),
    /// The backup could not be written or verified; nothing was switched.
    Backup(String),
    /// The data or storage changed after the user reviewed the plan.
    PlanChanged,
    /// The switch was recorded, but Office storage is missing or unusable.
    RecoveryRequired {
        database: PathBuf,
        condition: String,
        backup: Option<PathBuf>,
    },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => formatter.write_str("Another Office storage migration step is running."),
            Self::Source(message)
            | Self::Mismatch(message)
            | Self::State(message)
            | Self::Destination(message)
            | Self::Service(message)
            | Self::Backup(message) => formatter.write_str(message),
            Self::PlanChanged => formatter.write_str(
                "The migration plan changed after it was shown; review it and try again.",
            ),
            Self::NotWritable(directory) => write!(
                formatter,
                "{} is not writable; check its permissions and try again.",
                directory.display()
            ),
            Self::RecoveryRequired {
                database,
                condition,
                backup,
            } => {
                write!(
                    formatter,
                    "Office storage needs recovery: the switch was recorded but {} {condition}.",
                    database.display()
                )?;
                match backup {
                    Some(backup) => {
                        write!(formatter, " Your last backup is {}.", backup.display())?
                    }
                    None => formatter.write_str(" No backup was found.")?,
                }
                formatter.write_str(
                    " Restore steps: see the Office storage recovery section of the Office docs.",
                )
            }
            Self::SourceChanged => formatter.write_str(
                "Office data in the core database changed after the copy; copy it again.",
            ),
        }
    }
}

impl std::error::Error for MigrationError {}

impl MigrationError {
    /// Stable identifier for machine-readable output.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Busy => "OFFICE_STORAGE_BUSY",
            Self::Source(_) => "OFFICE_STORAGE_SOURCE",
            Self::SourceChanged => "OFFICE_STORAGE_SOURCE_CHANGED",
            Self::Mismatch(_) => "OFFICE_STORAGE_MISMATCH",
            Self::State(_) => "OFFICE_STORAGE_STATE",
            Self::Destination(_) => "OFFICE_STORAGE_DESTINATION",
            Self::NotWritable(_) => "OFFICE_STORAGE_NOT_WRITABLE",
            Self::Service(_) => "OFFICE_STORAGE_SERVICE",
            Self::Backup(_) => "OFFICE_STORAGE_BACKUP",
            Self::PlanChanged => "OFFICE_STORAGE_PLAN_CHANGED",
            Self::RecoveryRequired { .. } => "OFFICE_STORAGE_RECOVERY_REQUIRED",
        }
    }
}

type Result<T> = std::result::Result<T, MigrationError>;

fn destination(context: &str) -> impl Fn(rusqlite::Error) -> MigrationError + '_ {
    move |error| MigrationError::Destination(format!("{context}: {error}"))
}

fn source_error(context: &str) -> impl Fn(rusqlite::Error) -> MigrationError + '_ {
    move |error| MigrationError::Source(format!("{context}: {error}"))
}

/// Reports progress without taking the lock or touching the source.
pub fn status(layout: &StorageLayout) -> Result<Status> {
    let destination_exists = exists(&layout.database)?;
    let backups = backups(layout)?;
    if !exists(&layout.staging)? {
        let state = match destination_exists {
            false => State::Absent,
            true if activated(&layout.database)? => State::Switched,
            true => State::Switching,
        };
        return Ok(Status {
            state,
            destination_exists,
            source_schema_version: None,
            source_manifest: None,
            rows: Vec::new(),
            backups,
        });
    }
    let staging = open_staging_read_only(&layout.staging)?;
    let record = read_record(&staging)?;
    let rows = match record.as_ref().map(|record| record.state) {
        Some(State::Copied | State::Verified) => staged_rows(&staging)?,
        _ => Vec::new(),
    };
    Ok(Status {
        state: record.as_ref().map_or(State::Absent, |record| record.state),
        destination_exists,
        source_schema_version: record.as_ref().map(|record| record.source_schema_version),
        source_manifest: record.and_then(|record| record.source_manifest),
        rows,
        backups,
    })
}

/// Whether core recorded the Office storage switch, read query-only.
pub(crate) fn switched(layout: &StorageLayout) -> Result<bool> {
    switch::read_receipt(&layout.source).map(|receipt| receipt.is_some())
}

const BACKUP_PREFIX: &str = "office-storage-";

fn backups(layout: &StorageLayout) -> Result<Vec<Backup>> {
    let entries = match fs::read_dir(&layout.backups) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(MigrationError::Destination(format!(
                "Cannot read {}: {error}",
                layout.backups.display()
            )));
        }
    };
    let mut backups: Vec<Backup> = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(BACKUP_PREFIX)
                && entry.path().join("tmux-team.db").is_file()
        })
        .map(|entry| Backup {
            bytes: switch::directory_size(&entry.path()),
            directory: entry.path(),
        })
        .collect();
    backups.sort_by(|left, right| left.directory.cmp(&right.directory));
    Ok(backups)
}

/// Whether published Office storage records its activation.
fn activated(database: &Path) -> Result<bool> {
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(destination("Open Office storage"))?;
    connection
        .query_row("SELECT count(*) > 0 FROM _office_activation", [], |row| {
            row.get(0)
        })
        .map_err(destination("Read Office storage activation"))
}

/// Digest and shapes of every Office table, read inside the caller's snapshot.
fn office_manifest(connection: &Connection) -> Result<(Manifest, Vec<TableShape>)> {
    let mut manifest = ManifestBuilder::default();
    let mut shapes = Vec::new();
    for table in schema::OFFICE_TABLES {
        let shape =
            TableShape::read(connection, table).map_err(source_error("Read source table"))?;
        cells::digest_table(connection, &shape, &mut manifest)
            .map_err(source_error("Digest source rows"))?;
        shapes.push(shape);
    }
    Ok((manifest.finish(), shapes))
}

/// Starts a fresh migration: validates the source and records its identity in
/// a new staging database. Any earlier staging area is discarded; before a
/// switch it is never authoritative.
pub fn prepare(layout: &StorageLayout) -> Result<Status> {
    let _lock = lock(layout)?;
    require_no_destination(layout)?;
    let source = open_source(&layout.source)?;
    let identity = source_identity(&layout.source)?;
    let version = source_schema_version(&source)?;
    remove_staging(&layout.staging)?;
    let staging = create_staging(&layout.staging)?;
    staging
        .execute(
            "INSERT INTO _office_migration (singleton, state, source_path, source_device, source_inode, source_schema_version, prepared_at_ms) \
             VALUES (1, 'prepared', ?, ?, ?, ?, ?)",
            rusqlite::params![
                layout.source.to_string_lossy(),
                identity.device,
                identity.inode,
                version,
                now_ms(),
            ],
        )
        .map_err(destination("Record migration preparation"))?;
    drop(staging);
    status(layout)
}

/// Copies every Office row from one source snapshot. A repeated copy replaces
/// the staged rows atomically, so resuming after any interruption is safe.
pub fn copy(layout: &StorageLayout) -> Result<Status> {
    let _lock = lock(layout)?;
    require_no_destination(layout)?;
    let mut staging = open_existing_staging(layout)?;
    let record = require_record(&staging)?;
    let source = open_source(&layout.source)?;
    record.require_same_source(&layout.source)?;
    let snapshot = source
        .unchecked_transaction()
        .map_err(source_error("Open source snapshot"))?;
    let transaction = staging
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(destination("Begin staged copy"))?;
    for table in schema::OFFICE_TABLES.iter().rev() {
        transaction
            .execute(&format!("DELETE FROM \"{table}\""), [])
            .map_err(destination("Clear staged rows"))?;
    }
    let mut manifest = ManifestBuilder::default();
    for table in schema::OFFICE_TABLES {
        let shape =
            TableShape::read(&snapshot, table).map_err(source_error("Read source table"))?;
        cells::copy_table(&snapshot, &transaction, &shape, &mut manifest)
            .map_err(|error| MigrationError::Destination(format!("Copy {table}: {error}")))?;
        #[cfg(test)]
        crate::tests::fault::after_table(table).map_err(MigrationError::Destination)?;
    }
    require_consistent(&transaction)?;
    let manifest = manifest.finish();
    transaction
        .execute(
            "UPDATE _office_migration SET state = 'copied', source_manifest = ?, copied_at_ms = ?, verified_at_ms = NULL WHERE singleton = 1",
            rusqlite::params![manifest.digest, now_ms()],
        )
        .map_err(destination("Record copy"))?;
    transaction
        .commit()
        .map_err(destination("Commit staged copy"))?;
    drop(snapshot);
    status(layout)
}

/// Compares every staged cell with a fresh source snapshot. A changed source
/// requires another copy; a differing copy is never marked verified.
pub fn verify(layout: &StorageLayout) -> Result<Status> {
    let _lock = lock(layout)?;
    require_no_destination(layout)?;
    let staging = open_existing_staging(layout)?;
    let record = require_record(&staging)?;
    if record.state == State::Prepared {
        return Err(MigrationError::State(
            "Office rows have not been copied yet; run copy first.".to_owned(),
        ));
    }
    let source = open_source(&layout.source)?;
    record.require_same_source(&layout.source)?;
    let snapshot = source
        .unchecked_transaction()
        .map_err(source_error("Open source snapshot"))?;
    let (manifest, shapes) = office_manifest(&snapshot)?;
    if Some(manifest.digest) != record.source_manifest {
        return Err(MigrationError::SourceChanged);
    }
    for shape in &shapes {
        if let Some(Difference { table, row, detail }) =
            cells::compare_table(&snapshot, &staging, shape)
                .map_err(destination("Compare staged rows"))?
        {
            return Err(MigrationError::Mismatch(format!(
                "Staged Office data differs from the source: {table} row {row} {detail}."
            )));
        }
    }
    let integrity: String = staging
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(destination("Check staging integrity"))?;
    if integrity != "ok" {
        return Err(MigrationError::Mismatch(format!(
            "Staged Office storage failed its integrity check: {integrity}"
        )));
    }
    require_consistent(&staging)?;
    staging
        .execute(
            "UPDATE _office_migration SET state = 'verified', verified_at_ms = ? WHERE singleton = 1",
            [now_ms()],
        )
        .map_err(destination("Record verification"))?;
    drop(snapshot);
    drop(staging);
    status(layout)
}

struct Record {
    state: State,
    source_device: i64,
    source_inode: i64,
    source_schema_version: i64,
    source_manifest: Option<String>,
}

impl Record {
    fn require_same_source(&self, path: &Path) -> Result<()> {
        let identity = source_identity(path)?;
        if identity.device != self.source_device || identity.inode != self.source_inode {
            return Err(MigrationError::Source(
                "The core database was replaced after preparation; prepare again.".to_owned(),
            ));
        }
        Ok(())
    }
}

fn read_record(staging: &Connection) -> Result<Option<Record>> {
    staging
        .query_row(
            "SELECT state, source_device, source_inode, source_schema_version, source_manifest FROM _office_migration WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(destination("Read migration record"))?
        .map(|(state, source_device, source_inode, source_schema_version, source_manifest)| {
            Ok(Record {
                state: State::parse(&state).ok_or_else(|| {
                    MigrationError::Destination(format!("Unknown migration state {state}."))
                })?,
                source_device,
                source_inode,
                source_schema_version,
                source_manifest,
            })
        })
        .transpose()
}

fn require_record(staging: &Connection) -> Result<Record> {
    read_record(staging)?.ok_or_else(|| {
        MigrationError::State("No prepared Office storage migration; run prepare first.".to_owned())
    })
}

fn staged_rows(staging: &Connection) -> Result<Vec<(String, u64)>> {
    schema::OFFICE_TABLES
        .iter()
        .map(|table| {
            staging
                .query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| {
                    row.get::<_, i64>(0)
                })
                .map(|count| ((*table).to_owned(), count as u64))
                .map_err(destination("Count staged rows"))
        })
        .collect()
}

fn require_consistent(connection: &Connection) -> Result<()> {
    let violation: Option<String> = connection
        .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
        .optional()
        .map_err(destination("Check staged references"))?;
    match violation {
        Some(table) => Err(MigrationError::Mismatch(format!(
            "Staged Office data has a dangling reference in {table}."
        ))),
        None => Ok(()),
    }
}

fn require_no_destination(layout: &StorageLayout) -> Result<()> {
    if exists(&layout.database)? {
        return Err(MigrationError::State(
            "Office storage already exists; there is nothing to migrate.".to_owned(),
        ));
    }
    Ok(())
}

struct SourceIdentity {
    device: i64,
    inode: i64,
}

fn source_identity(path: &Path) -> Result<SourceIdentity> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        MigrationError::Source(format!("Cannot inspect the core database: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(MigrationError::Source(
            "The core database is not a regular file.".to_owned(),
        ));
    }
    Ok(SourceIdentity {
        device: metadata.dev() as i64,
        inode: metadata.ino() as i64,
    })
}

/// Opens the core database without creating, migrating or writing it: the
/// connection is query-only and does not checkpoint on close.
fn open_source(path: &Path) -> Result<Connection> {
    let connection = open_source_unchecked(path)?;
    match schema::source_matches(&connection).map_err(source_error("Read the source schema"))? {
        Ok(()) => Ok(connection),
        Err(message) => Err(MigrationError::Source(message)),
    }
}

fn open_source_unchecked(path: &Path) -> Result<Connection> {
    source_identity(path)?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(source_error("Open the core database"))?;
    connection
        .set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)
        .map_err(source_error("Disable checkpoint on close"))?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(source_error("Make the source query-only"))?;
    connection
        .busy_timeout(Duration::from_millis(5000))
        .map_err(source_error("Configure the source busy timeout"))?;
    Ok(connection)
}

fn source_schema_version(source: &Connection) -> Result<i64> {
    source
        .query_row("SELECT max(version) FROM _migrations", [], |row| row.get(0))
        .map_err(source_error("Read the core schema version"))
}

fn lock(layout: &StorageLayout) -> Result<impl Drop> {
    secure_directory(&layout.directory)?;
    tmt_adapters::file_lock::exclusive(&layout.lock).map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            MigrationError::Busy
        } else {
            MigrationError::Destination(format!("Cannot lock Office storage migration: {error}"))
        }
    })
}

/// Mirrors core storage: an existing directory without owner write permission
/// is reported, not repaired.
fn secure_directory(path: &Path) -> Result<()> {
    if fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o200 == 0) {
        return Err(MigrationError::NotWritable(path.to_path_buf()));
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .and_then(|()| fs::set_permissions(path, fs::Permissions::from_mode(0o700)))
        .map_err(|error| {
            MigrationError::Destination(format!("Cannot secure the Office directory: {error}"))
        })
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(MigrationError::Destination(format!(
            "Cannot inspect {}: {error}",
            path.display()
        ))),
    }
}

fn sidecars(path: &Path) -> [std::path::PathBuf; 4] {
    ["", "-journal", "-wal", "-shm"].map(|suffix| {
        let mut file = path.as_os_str().to_os_string();
        file.push(suffix);
        file.into()
    })
}

fn remove_staging(path: &Path) -> Result<()> {
    for file in sidecars(path) {
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(MigrationError::Destination(format!(
                    "Cannot discard earlier staging data: {error}"
                )));
            }
        }
    }
    Ok(())
}

/// Staging uses a rollback journal with full synchronous commits, so each
/// committed state is durable in the single file a later switch publishes.
fn open_staging(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(destination("Open staging"))?;
    for (name, value) in [
        ("foreign_keys", "ON"),
        ("journal_mode", "DELETE"),
        ("synchronous", "FULL"),
    ] {
        connection
            .pragma_update(None, name, value)
            .map_err(destination("Configure staging"))?;
    }
    connection
        .busy_timeout(Duration::from_millis(5000))
        .map_err(destination("Configure staging"))?;
    #[cfg(test)]
    crate::tests::fault::configure_staging(&connection);
    require_schema(&connection)?;
    Ok(connection)
}

/// Reads progress without changing journal settings under a running step.
fn open_staging_read_only(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(destination("Open staging"))?;
    connection
        .busy_timeout(Duration::from_millis(5000))
        .map_err(destination("Configure staging"))?;
    require_schema(&connection)?;
    Ok(connection)
}

/// Accepts staging from this or an earlier Office schema; publishing upgrades it.
fn require_schema(connection: &Connection) -> Result<i64> {
    let version: Option<i64> = connection
        .query_row("SELECT max(version) FROM _office_schema", [], |row| {
            row.get(0)
        })
        .map_err(destination("Read staging schema"))?;
    match version {
        Some(version) if (1..=schema::VERSION).contains(&version) => Ok(version),
        _ => Err(MigrationError::Destination(
            "Staging data has an unsupported Office schema; prepare again.".to_owned(),
        )),
    }
}

fn open_existing_staging(layout: &StorageLayout) -> Result<Connection> {
    if !exists(&layout.staging)? {
        return Err(MigrationError::State(
            "No prepared Office storage migration; run prepare first.".to_owned(),
        ));
    }
    open_staging(&layout.staging)
}

fn create_staging(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(destination("Create staging"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| MigrationError::Destination(format!("Cannot secure staging: {error}")))?;
    connection
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(destination("Configure staging"))?;
    schema::install(&connection).map_err(destination("Create the Office schema"))?;
    drop(connection);
    open_staging(path)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

impl Status {
    /// Machine-readable progress for diagnostics.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "state": self.state.as_str(),
            "destinationExists": self.destination_exists,
            "sourceSchemaVersion": self.source_schema_version,
            "sourceManifest": self.source_manifest,
            "rows": self.rows.iter().map(|(table, count)| serde_json::json!({"table": table, "count": count})).collect::<Vec<_>>(),
            "backups": self.backups.iter().map(Backup::json).collect::<Vec<_>>(),
        })
    }
}

impl Backup {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"directory": self.directory, "bytes": self.bytes})
    }
}

impl Recovery {
    pub fn json(self) -> serde_json::Value {
        serde_json::json!({"recovery": self.as_str()})
    }
}

impl Switched {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "state": State::Switched.as_str(),
            "database": self.database,
            "backup": self.backup.json(),
            "serviceWasRunning": self.service_was_running,
        })
    }
}

impl MigrationError {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"error": {"code": self.code(), "message": self.to_string()}})
    }
}
