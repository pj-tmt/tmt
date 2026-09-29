//! Makes a verified copy authoritative.
//!
//! Under the migration lock, with the Office service stopped and held
//! stopped, the switch backs up the core database, rechecks the Office rows
//! inside one immediate core transaction, publishes staging as `office.db`,
//! and records core's cutover receipt in that same transaction. The commit is
//! the single decision point: before it the core database is unchanged and
//! recovery renames `office.db` back to staging; after it core's schema 36
//! triggers fence the retained Office rows and recovery only finishes
//! activation. Writing the receipt is the one sanctioned Office write to the
//! core database; it leaves with the in-process core link (#355).

use super::{
    MigrationError, Record, Result, State, destination, exists, lock, now_ms, office_manifest,
    open_existing_staging, open_source, require_record, require_schema, source_error,
    source_identity,
};
use crate::{StorageLayout, schema};
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior};
use std::{
    fs::{self, File},
    io,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
use tmt_adapters::{config::ConfigPaths, office_service};

const EXTENSION: &str = "office";

/// Stops the Office service and keeps it from serving until the guard drops.
pub trait Quiesce {
    fn quiesce(&self) -> std::result::Result<Quiesced<'_>, String>;
}

/// Anything whose drop releases the service again.
pub trait Held {}
impl<T> Held for T {}

pub struct Quiesced<'a> {
    pub was_running: bool,
    pub guard: Box<dyn Held + 'a>,
}

/// The installed local Office service of one configuration root.
pub struct OfficeService<'a>(pub &'a ConfigPaths);

impl Quiesce for OfficeService<'_> {
    fn quiesce(&self) -> std::result::Result<Quiesced<'_>, String> {
        let was_running = office_service::stop(self.0).map_err(|error| {
            format!("The Office service could not be stopped: {error} Stop it and try again.")
        })?;
        let guard = office_service::service_lock(self.0).map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                "The Office service started again during the migration; stop it and try again."
                    .to_owned()
            } else {
                format!("Cannot keep the Office service stopped: {error}")
            }
        })?;
        Ok(Quiesced {
            was_running,
            guard: Box::new(guard),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switched {
    pub database: PathBuf,
    pub backup: Backup,
    pub service_was_running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub directory: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// No switch was in progress.
    None,
    /// An undecided switch was undone; the verified copy is staged again.
    Reverted,
    /// A recorded switch finished activating `office.db`.
    Activated,
    /// `office.db` is already authoritative.
    Current,
}

impl Recovery {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Reverted => "reverted",
            Self::Activated => "activated",
            Self::Current => "current",
        }
    }
}

/// Core's record that Office storage became authoritative.
pub(super) struct Receipt {
    device: i64,
    inode: i64,
    manifest: String,
    pub(super) switched_at_ms: i64,
}

/// Switches a verified copy. Any interrupted earlier switch is settled first.
pub fn switch(layout: &StorageLayout, service: &dyn Quiesce) -> Result<Switched> {
    let _lock = lock(layout)?;
    settle(layout)?;
    if exists(&layout.database)? {
        return Err(MigrationError::State(
            "Office storage is already switched.".to_owned(),
        ));
    }
    let record = {
        let staging = open_existing_staging(layout)?;
        let record = require_record(&staging)?;
        if record.state != State::Verified {
            return Err(MigrationError::State(
                "The Office copy is not verified; run verify first.".to_owned(),
            ));
        }
        record
    };
    record.require_same_source(&layout.source)?;
    let quiesced = service.quiesce().map_err(MigrationError::Service)?;
    crash_point("stopped");
    let backup = back_up(layout, &record)?;
    crash_point("backed-up");
    decide(layout, &record)?;
    crash_point("committed");
    activate(layout)?;
    drop(quiesced.guard);
    Ok(Switched {
        database: layout.database.clone(),
        backup,
        service_was_running: quiesced.was_running,
    })
}

/// Switches an install whose shared Office tables hold no user data, through
/// the same prepare, copy, verify, decision and activation as `switch`, so
/// core records the receipt and its fences apply. Only the full-database
/// backup and the service quiesce are skipped: there is nothing to restore,
/// and the decision's manifest recheck aborts if any Office write raced in.
/// Waits a bounded time for another process's switch, then reports whether
/// this call switched.
pub(crate) fn switch_fresh(layout: &StorageLayout) -> Result<bool> {
    let _lock = lock_waiting(layout, Duration::from_secs(10))?;
    settle(layout)?;
    if exists(&layout.database)? || super::user_rows(layout)? != 0 {
        return Ok(false);
    }
    super::prepare_locked(layout)?;
    super::copy_locked(layout)?;
    super::verify_locked(layout)?;
    let record = require_record(&open_existing_staging(layout)?)?;
    crash_point("fresh-verified");
    decide(layout, &record)?;
    crash_point("fresh-committed");
    activate(layout)?;
    Ok(true)
}

/// Bounded polling for the migration lock held by another process.
fn lock_waiting(layout: &StorageLayout, limit: Duration) -> Result<impl Drop> {
    let deadline = std::time::Instant::now() + limit;
    loop {
        match lock(layout) {
            Err(MigrationError::Busy) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            result => return result,
        }
    }
}

/// Ensures switched Office storage is activated; for store selection after
/// core reported a receipt.
pub(crate) fn ensure_active(layout: &StorageLayout) -> Result<()> {
    if activation(&layout.database).unwrap_or(false) {
        return Ok(());
    }
    match recover(layout)? {
        Recovery::Current | Recovery::Activated => Ok(()),
        Recovery::None | Recovery::Reverted => Err(MigrationError::State(
            "Office storage has no recorded switch.".to_owned(),
        )),
    }
}

/// Settles an interrupted switch from core's receipt and the activation marker.
pub fn recover(layout: &StorageLayout) -> Result<Recovery> {
    let _lock = lock(layout)?;
    settle(layout)
}

fn settle(layout: &StorageLayout) -> Result<Recovery> {
    let receipt = read_receipt(&layout.source)?;
    let published = exists(&layout.database)?;
    match (receipt, published) {
        (None, false) => Ok(Recovery::None),
        (None, true) => {
            if exists(&layout.staging)? {
                return Err(MigrationError::Destination(format!(
                    "Both {} and its staging copy exist without a recorded switch; remove the staging copy and prepare again.",
                    layout.database.display()
                )));
            }
            rename_durably(&layout.database, &layout.staging, &layout.directory)?;
            Ok(Recovery::Reverted)
        }
        (Some(_), false) => Err(recovery_required(layout, "is missing")),
        (Some(receipt), true) => {
            let identity = source_identity(&layout.database)
                .map_err(|_| recovery_required(layout, "is unreadable"))?;
            if (identity.device, identity.inode) != (receipt.device, receipt.inode) {
                return Err(recovery_required(layout, "was replaced"));
            }
            if activation(&layout.database)
                .map_err(|_| recovery_required(layout, "is unreadable"))?
            {
                Ok(Recovery::Current)
            } else {
                activate(layout)?;
                Ok(Recovery::Activated)
            }
        }
    }
}

fn recovery_required(layout: &StorageLayout, condition: &str) -> MigrationError {
    MigrationError::RecoveryRequired {
        database: layout.database.clone(),
        condition: condition.to_owned(),
        backup: super::backups(layout)
            .ok()
            .and_then(|backups| backups.into_iter().last())
            .map(|backup| backup.directory),
    }
}

pub(super) fn read_receipt(source: &Path) -> Result<Option<Receipt>> {
    let connection = super::open_source_unchecked(source)?;
    let fenced: bool = connection
        .query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'extension_storage_cutovers'",
            [],
            |row| row.get(0),
        )
        .map_err(source_error("Read the core schema"))?;
    if !fenced {
        return Ok(None);
    }
    connection
        .query_row(
            "SELECT destination_device, destination_inode, manifest, switched_at_ms FROM extension_storage_cutovers WHERE extension = ?",
            [EXTENSION],
            |row| {
                Ok(Receipt {
                    device: row.get(0)?,
                    inode: row.get(1)?,
                    manifest: row.get(2)?,
                    switched_at_ms: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(source_error("Read the Office storage receipt"))
}

/// Recomputes the Office rows, publishes staging and records the receipt in
/// one immediate core transaction.
fn decide(layout: &StorageLayout, record: &Record) -> Result<()> {
    let mut source = open_source(&layout.source)?;
    source
        .pragma_update(None, "query_only", false)
        .map_err(source_error("Open the core database for the switch"))?;
    require_fence(&source)?;
    let not_writable = |error: rusqlite::Error| writable(error, &layout.source);
    let transaction = source
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(not_writable)?;
    crash_point("decision-begun");
    let (manifest, _) = office_manifest(&transaction)?;
    if Some(&manifest.digest) != record.source_manifest.as_ref() {
        return Err(MigrationError::SourceChanged);
    }
    publish(layout)?;
    crash_point("published");
    let decided = (|| {
        let identity = source_identity(&layout.database)?;
        transaction
            .execute(
                "INSERT INTO extension_storage_cutovers (extension, destination_device, destination_inode, storage_schema_version, manifest, switched_at_ms) VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![EXTENSION, identity.device, identity.inode, schema::VERSION, manifest.digest, now_ms()],
            )
            .map_err(not_writable)?;
        transaction.commit().map_err(not_writable)
    })();
    if let Err(error) = decided {
        // Without a committed receipt the source still owns Office data.
        settle(layout)?;
        return Err(error);
    }
    Ok(())
}

/// Core schema 36 must fence the retained rows before a receipt can mean anything.
pub(super) fn require_fence(source: &Connection) -> Result<()> {
    let fences: Vec<String> = schema::core_fences().collect();
    let placeholders = vec!["?"; fences.len()].join(", ");
    let present: i64 = source
        .query_row(
            &format!(
                "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name IN ({placeholders})"
            ),
            rusqlite::params_from_iter(&fences),
            |row| row.get(0),
        )
        .map_err(source_error("Read the core schema"))?;
    if present != fences.len() as i64 {
        return Err(MigrationError::Source(
            "The core database does not fence moved Office data; update tmt and run it once before migrating.".to_owned(),
        ));
    }
    Ok(())
}

/// Upgrades staging to the current Office schema and renames it into place.
fn publish(layout: &StorageLayout) -> Result<()> {
    let staging = open_existing_staging(layout)?;
    require_schema(&staging)?;
    schema::upgrade_existing(&staging)
        .map_err(destination("Upgrade staged Office storage"))?
        .map_err(MigrationError::Destination)?;
    drop(staging);
    File::open(&layout.staging)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_failure(error, &layout.directory, "Sync staged Office storage"))?;
    rename_durably(&layout.staging, &layout.database, &layout.directory)
}

fn activation(database: &Path) -> rusqlite::Result<bool> {
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.query_row("SELECT count(*) > 0 FROM _office_activation", [], |row| {
        row.get(0)
    })
}

/// Writes the activation marker once and moves `office.db` to WAL.
fn activate(layout: &StorageLayout) -> Result<()> {
    let receipt = read_receipt(&layout.source)?
        .ok_or_else(|| MigrationError::State("No recorded Office storage switch.".to_owned()))?;
    let unreadable = |_| recovery_required(layout, "is unreadable");
    let connection = Connection::open_with_flags(
        &layout.database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(unreadable)?;
    connection
        .busy_timeout(Duration::from_millis(5000))
        .map_err(destination("Configure Office storage"))?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(unreadable)?;
    let copied: Option<String> = connection
        .query_row(
            "SELECT source_manifest FROM _office_migration WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(unreadable)?;
    if integrity != "ok"
        || copied.as_ref() != Some(&receipt.manifest)
        || require_schema(&connection).is_err()
    {
        return Err(recovery_required(layout, "is unreadable"));
    }
    // Storage switched by an earlier build activates at the current schema.
    schema::upgrade_existing(&connection)
        .map_err(destination("Upgrade Office storage"))?
        .map_err(MigrationError::Destination)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(destination("Configure Office storage"))?;
    connection
        .execute(
            "INSERT INTO _office_activation (singleton, manifest, switched_at_ms, activated_at_ms) VALUES (1, ?, ?, ?) ON CONFLICT (singleton) DO NOTHING",
            rusqlite::params![receipt.manifest, receipt.switched_at_ms, now_ms()],
        )
        .map_err(destination("Record Office storage activation"))?;
    Ok(())
}

/// Copies the whole core database and Office configuration into a new private
/// backup directory, verified before anything is fenced.
fn back_up(layout: &StorageLayout, record: &Record) -> Result<Backup> {
    // `VACUUM INTO` writes only the target, which a query-only connection refuses.
    let source = Connection::open_with_flags(
        &layout.source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(source_error("Open the core database for the backup"))?;
    source
        .busy_timeout(Duration::from_millis(5000))
        .map_err(source_error("Configure the source busy timeout"))?;
    let (pages, page_size): (i64, i64) = source
        .query_row(
            "SELECT page_count, page_size FROM pragma_page_count, pragma_page_size",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(source_error("Measure the core database"))?;
    let extra = recovery_files(layout)?
        .iter()
        .map(|(path, _)| fs::metadata(path).map_or(0, |metadata| metadata.len()))
        .sum::<u64>();
    let needed = (pages * page_size).max(0) as u64 + extra;
    let parent = existing_ancestor(&layout.backups);
    let available = available_bytes(&parent)
        .map_err(|error| io_failure(error, &parent, "Measure free space"))?;
    if available < needed {
        return Err(MigrationError::Backup(format!(
            "The backup needs {needed} bytes in {}, but only {available} are available. Free space and try again.",
            parent.display()
        )));
    }
    let directory = new_backup_directory(layout)?;
    let result = (|| {
        let database = directory.join("tmux-team.db");
        source
            .execute("VACUUM INTO ?", [database.to_string_lossy()])
            .map_err(|error| {
                MigrationError::Backup(format!("Back up the core database: {error}"))
            })?;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600))
            .and_then(|()| File::open(&database)?.sync_all())
            .map_err(|error| io_failure(error, &directory, "Secure the backup"))?;
        #[cfg(test)]
        crate::tests::fault::after_backup(&database);
        verify_backup(&source, &database, record)?;
        let office = directory.join("office");
        for (path, name) in recovery_files(layout)? {
            let target = if name == "config.json" && path == layout.config {
                directory.join(&name)
            } else {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&office)
                    .map_err(|error| io_failure(error, &directory, "Create the backup"))?;
                office.join(&name)
            };
            fs::copy(&path, &target)
                .and_then(|_| File::open(&target)?.sync_all())
                .map_err(|error| io_failure(error, &directory, "Copy Office configuration"))?;
        }
        sync_directory(&directory)?;
        sync_directory(&layout.backups)?;
        Ok(Backup {
            bytes: directory_size(&directory),
            directory: directory.clone(),
        })
    })();
    if result.is_err() {
        // This unique directory belongs to this switch; nothing was fenced.
        let _ = fs::remove_dir_all(&directory);
    }
    result
}

fn verify_backup(source: &Connection, database: &Path, record: &Record) -> Result<()> {
    let invalid = |detail: String| {
        MigrationError::Backup(format!("The backup failed verification: {detail}"))
    };
    let backup = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| invalid(error.to_string()))?;
    let integrity: String = backup
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| invalid(error.to_string()))?;
    if integrity != "ok" {
        return Err(invalid(integrity));
    }
    let history = |connection: &Connection| -> rusqlite::Result<Vec<(i64, String)>> {
        connection
            .prepare("SELECT version, name FROM _migrations ORDER BY version")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect()
    };
    if history(&backup).map_err(|error| invalid(error.to_string()))?
        != history(source).map_err(source_error("Read the core migrations"))?
    {
        return Err(invalid("its migration history differs".to_owned()));
    }
    let (manifest, _) = office_manifest(&backup)?;
    if Some(&manifest.digest) != record.source_manifest.as_ref() {
        return Err(MigrationError::SourceChanged);
    }
    Ok(())
}

/// The global configuration and the protected Office files: installation
/// metadata and pairing records, never runtime state, locks or databases.
fn recovery_files(layout: &StorageLayout) -> Result<Vec<(PathBuf, String)>> {
    let mut files = Vec::new();
    if fs::symlink_metadata(&layout.config).is_ok_and(|metadata| metadata.is_file()) {
        files.push((layout.config.clone(), "config.json".to_owned()));
    }
    let entries = match fs::read_dir(&layout.directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(files),
        Err(error) => {
            return Err(io_failure(
                error,
                &layout.directory,
                "Read the Office directory",
            ));
        }
    };
    for entry in entries {
        let entry = entry
            .map_err(|error| io_failure(error, &layout.directory, "Read the Office directory"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let regular = entry.file_type().is_ok_and(|kind| kind.is_file());
        if regular
            && !name.starts_with('.')
            && !name.starts_with("office.db")
            && !name.ends_with(".lock")
        {
            files.push((entry.path(), name));
        }
    }
    files.sort();
    Ok(files)
}

fn new_backup_directory(layout: &StorageLayout) -> Result<PathBuf> {
    let create = |path: &Path, recursive: bool| {
        fs::DirBuilder::new()
            .recursive(recursive)
            .mode(0o700)
            .create(path)
    };
    create(&layout.backups, true)
        .map_err(|error| io_failure(error, &layout.backups, "Create the backup directory"))?;
    let stamp = utc_stamp(now_ms() / 1000);
    for attempt in 0..100 {
        let name = match attempt {
            0 => format!("{}{stamp}", super::BACKUP_PREFIX),
            _ => format!("{}{stamp}-{attempt}", super::BACKUP_PREFIX),
        };
        let path = layout.backups.join(name);
        match create(&path, false) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(io_failure(
                    error,
                    &layout.backups,
                    "Create the backup directory",
                ));
            }
        }
    }
    Err(MigrationError::Backup(
        "Cannot choose a new backup directory name.".to_owned(),
    ))
}

pub(super) fn directory_size(directory: &Path) -> u64 {
    fs::read_dir(directory).map_or(0, |entries| {
        entries
            .flatten()
            .map(|entry| match entry.file_type() {
                Ok(kind) if kind.is_dir() => directory_size(&entry.path()),
                _ => entry.metadata().map_or(0, |metadata| metadata.len()),
            })
            .sum()
    })
}

fn existing_ancestor(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|candidate| candidate.exists())
        .unwrap_or(Path::new("/"))
        .to_path_buf()
}

// `statvfs` field widths differ by platform, so a conversion is redundant on some.
#[allow(clippy::useless_conversion)]
fn available_bytes(path: &Path) -> io::Result<u64> {
    #[cfg(test)]
    if let Some(bytes) = crate::tests::fault::available_bytes() {
        return Ok(bytes);
    }
    let stat = nix::sys::statvfs::statvfs(path).map_err(io::Error::from)?;
    Ok(u64::from(stat.blocks_available()).saturating_mul(u64::from(stat.fragment_size())))
}

fn rename_durably(from: &Path, to: &Path, directory: &Path) -> Result<()> {
    fs::rename(from, to).map_err(|error| io_failure(error, directory, "Publish Office storage"))?;
    sync_directory(directory)
}

fn sync_directory(directory: &Path) -> Result<()> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_failure(error, directory, "Sync the directory"))
}

/// Permission and read-only failures name the directory to fix.
fn io_failure(error: io::Error, directory: &Path, context: &str) -> MigrationError {
    match error.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            MigrationError::NotWritable(directory.to_path_buf())
        }
        _ => MigrationError::Destination(format!("{context}: {error}")),
    }
}

fn writable(error: rusqlite::Error, source: &Path) -> MigrationError {
    match error.sqlite_error_code() {
        Some(ErrorCode::ReadOnly | ErrorCode::CannotOpen | ErrorCode::PermissionDenied) => {
            MigrationError::NotWritable(
                source
                    .parent()
                    .map_or_else(|| source.to_path_buf(), Path::to_path_buf),
            )
        }
        _ => MigrationError::Source(format!("Record the Office storage switch: {error}")),
    }
}

/// `yyyymmddThhmmssZ` for a UNIX time in seconds.
fn utc_stamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    // Civil-from-days (proleptic Gregorian), valid for the whole i64 day range used here.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

#[cfg(test)]
fn crash_point(point: &'static str) {
    crate::tests::fault::point(point);
}

#[cfg(not(test))]
#[inline(always)]
fn crash_point(_point: &'static str) {}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_stamps_are_sortable_civil_times() {
        assert_eq!(super::utc_stamp(0), "19700101T000000Z");
        assert_eq!(super::utc_stamp(951_782_400), "20000229T000000Z");
        assert_eq!(super::utc_stamp(1_790_000_000), "20260921T141320Z");
    }
}
