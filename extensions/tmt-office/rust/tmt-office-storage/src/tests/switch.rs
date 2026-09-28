//! Switch, backup and recovery against disposable roots.
use super::{Crash, Root, dump, fault, seed, state, whole_source};
use crate::{
    migration::{self, MigrationError, Quiesce, Quiesced, Recovery, State},
    schema,
};
use rusqlite::Connection;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tmt_adapters::storage::Storage;

/// A stand-in for the local service that records whether it is held stopped.
struct Service {
    running: bool,
    stoppable: bool,
    held: Arc<AtomicBool>,
}

impl Service {
    fn new(running: bool) -> Self {
        Self {
            running,
            stoppable: true,
            held: Arc::default(),
        }
    }
}

struct Release(Arc<AtomicBool>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Quiesce for Service {
    fn quiesce(&self) -> Result<Quiesced<'_>, String> {
        if !self.stoppable {
            return Err("The Office service could not be stopped.".to_owned());
        }
        self.held.store(true, Ordering::SeqCst);
        Ok(Quiesced {
            was_running: self.running,
            guard: Box::new(Release(self.held.clone())),
        })
    }
}

const FENCE: &str = "Office data moved to Office storage";

/// A root with seeded Office rows, configuration and protected Office files,
/// verified and ready to switch.
fn verified() -> Root {
    let root = Root::new();
    seed(&root.source());
    fs::write(&root.layout.config, b"{\"global\":true}").unwrap();
    fs::set_permissions(&root.layout.config, fs::Permissions::from_mode(0o640)).unwrap();
    migration::prepare(&root.layout).unwrap();
    fs::write(root.layout.directory.join("installation-id"), b"install").unwrap();
    fs::set_permissions(
        root.layout.directory.join("installation-id"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    fs::write(root.layout.directory.join("installation.lock"), b"").unwrap();
    fs::write(root.layout.directory.join(".installation-x.tmp"), b"").unwrap();
    fs::create_dir(root.layout.directory.join("runtime")).unwrap();
    fs::write(root.layout.directory.join("runtime/receipt.json"), b"{}").unwrap();
    migration::copy(&root.layout).unwrap();
    migration::verify(&root.layout).unwrap();
    root
}

fn receipt(root: &Root) -> Option<tmt_adapters::storage::StorageCutover> {
    Storage::open(&root.layout.source)
        .unwrap()
        .extension_storage_cutover("office")
        .unwrap()
}

fn backup_directories(root: &Root) -> Vec<std::path::PathBuf> {
    let mut directories: Vec<_> = fs::read_dir(&root.layout.backups)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    directories.sort();
    directories
}

fn fenced(result: rusqlite::Result<usize>) -> bool {
    matches!(result, Err(error) if error.to_string().contains(FENCE))
}

/// The switch outcome before its decision point: nothing but staging moved.
fn assert_undecided(root: &Root, before: &[(String, Vec<String>)]) {
    assert_eq!(receipt(root), None);
    assert!(!root.layout.database.exists());
    assert_eq!(state(root), State::Verified);
    assert_eq!(whole_source(&root.source()), before);
    assert_eq!(
        root.source()
            .execute("UPDATE office_board_state SET revision = revision", [])
            .unwrap(),
        1
    );
}

fn mode(path: &std::path::Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn switch_backs_up_publishes_and_fences_with_one_receipt() {
    let root = verified();
    let manifest = migration::status(&root.layout)
        .unwrap()
        .source_manifest
        .unwrap();
    let before = whole_source(&root.source());
    // A writer whose connection predates the switch, like an older process.
    let old_writer = root.source();
    let service = Service::new(true);

    let switched = migration::switch(&root.layout, &service).unwrap();

    assert!(switched.service_was_running);
    assert!(!service.held.load(Ordering::SeqCst), "service released");
    assert_eq!(switched.database, root.layout.database);
    assert!(!root.layout.staging.exists());
    assert_eq!(mode(&root.layout.database), 0o600);
    let status = migration::status(&root.layout).unwrap();
    assert_eq!(status.state, State::Switched);
    assert_eq!(status.backups, vec![switched.backup.clone()]);

    let office = Connection::open(&root.layout.database).unwrap();
    for table in schema::OFFICE_TABLES {
        assert_eq!(dump(&office, table), dump(&root.source(), table), "{table}");
    }
    let journal: String = office
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "wal");
    let (marker, version): (String, i64) = office
        .query_row(
            "SELECT manifest, (SELECT max(version) FROM _office_schema) FROM _office_activation",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (marker.as_str(), version),
        (manifest.as_str(), schema::VERSION)
    );

    let receipt = receipt(&root).unwrap();
    let published = fs::metadata(&root.layout.database).unwrap();
    assert_eq!(
        (receipt.destination_device, receipt.destination_inode),
        (published.dev() as i64, published.ino() as i64)
    );
    assert_eq!(receipt.manifest, manifest);
    assert_eq!(receipt.storage_schema_version, schema::VERSION);

    // The backup is a private, verified copy taken before the receipt.
    let backup = &switched.backup.directory;
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("office-storage-")
    );
    assert_eq!(mode(backup), 0o700);
    assert_eq!(mode(&backup.join("tmux-team.db")), 0o600);
    let copy = Connection::open(backup.join("tmux-team.db")).unwrap();
    assert_eq!(whole_source(&copy), before);
    assert_eq!(
        fs::read(backup.join("config.json")).unwrap(),
        b"{\"global\":true}"
    );
    assert_eq!(mode(&backup.join("config.json")), 0o640);
    let mut office_files: Vec<String> = fs::read_dir(backup.join("office"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    office_files.sort();
    assert_eq!(office_files, ["installation-id"]);
    assert_eq!(mode(&backup.join("office/installation-id")), 0o600);
    assert!(switched.backup.bytes >= fs::metadata(backup.join("tmux-team.db")).unwrap().len());

    // Retained Office rows are read-only for every writer; core rows are not.
    assert!(fenced(old_writer.execute(
        "UPDATE office_board_state SET revision = revision + 1",
        []
    )));
    assert!(fenced(
        old_writer.execute("DELETE FROM office_local_profiles", [])
    ));
    old_writer
        .execute(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES ('77777777-7777-4777-8777-777777777777', 'New', 'new', 't', 't', 'saved')",
            [],
        )
        .unwrap();
    assert!(matches!(
        migration::switch(&root.layout, &service),
        Err(MigrationError::State(_))
    ));
    assert_eq!(migration::recover(&root.layout).unwrap(), Recovery::Current);
}

#[test]
fn a_crash_at_every_boundary_settles_to_exactly_one_outcome() {
    for (point, decided) in [
        ("stopped", false),
        ("backed-up", false),
        ("decision-begun", false),
        ("published", false),
        ("committed", true),
    ] {
        let root = verified();
        let before = whole_source(&root.source());
        let service = Service::new(false);
        fault::crash_at_point(Some(point));
        let crashed = catch_unwind(AssertUnwindSafe(|| {
            migration::switch(&root.layout, &service)
        }));
        fault::crash_at_point(None);
        let payload = crashed.expect_err(point);
        assert_eq!(payload.downcast_ref::<Crash>().unwrap().0, point);
        assert!(!service.held.load(Ordering::SeqCst), "{point}");

        if decided {
            assert_eq!(state(&root), State::Switching, "{point}");
            assert_eq!(
                migration::recover(&root.layout).unwrap(),
                Recovery::Activated
            );
            assert_eq!(migration::recover(&root.layout).unwrap(), Recovery::Current);
            assert_eq!(state(&root), State::Switched);
            assert!(fenced(root.source().execute(
                "UPDATE office_board_state SET revision = revision",
                []
            )));
            let office = Connection::open(&root.layout.database).unwrap();
            for table in schema::OFFICE_TABLES {
                assert_eq!(dump(&office, table), dump(&root.source(), table), "{table}");
            }
            continue;
        }
        let expected = if point == "published" {
            Recovery::Reverted
        } else {
            Recovery::None
        };
        assert_eq!(
            migration::recover(&root.layout).unwrap(),
            expected,
            "{point}"
        );
        assert_eq!(
            migration::recover(&root.layout).unwrap(),
            Recovery::None,
            "{point}"
        );
        assert_undecided(&root, &before);
        migration::switch(&root.layout, &service).unwrap();
        assert_eq!(state(&root), State::Switched, "{point}");
    }
}

#[test]
fn an_interrupted_publish_is_reverted_by_the_next_switch() {
    let root = verified();
    let service = Service::new(false);
    fault::crash_at_point(Some("published"));
    let _ = catch_unwind(AssertUnwindSafe(|| {
        migration::switch(&root.layout, &service)
    }));
    fault::crash_at_point(None);
    assert_eq!(state(&root), State::Switching);
    migration::switch(&root.layout, &service).unwrap();
    assert_eq!(state(&root), State::Switched);
    assert!(receipt(&root).is_some());
}

#[test]
fn an_office_write_before_the_decision_blocks_the_switch_until_copied_again() {
    let root = verified();
    let service = Service::new(false);
    let path = root.layout.source.clone();
    fault::at_point("backed-up", move || {
        Connection::open(path)
            .unwrap()
            .execute("UPDATE office_board_state SET revision = revision + 1", [])
            .unwrap();
    });
    let result = migration::switch(&root.layout, &service);
    assert!(
        matches!(result, Err(MigrationError::SourceChanged)),
        "{result:?}"
    );
    assert!(!service.held.load(Ordering::SeqCst));
    let before = whole_source(&root.source());
    assert_undecided(&root, &before);
    // The same change before the backup is caught by the backup check, which
    // removes its incomplete directory.
    root.source()
        .execute("UPDATE office_board_state SET revision = revision + 1", [])
        .unwrap();
    let retained = backup_directories(&root);
    assert!(matches!(
        migration::switch(&root.layout, &service),
        Err(MigrationError::SourceChanged)
    ));
    assert_eq!(backup_directories(&root), retained);
    migration::copy(&root.layout).unwrap();
    migration::verify(&root.layout).unwrap();
    migration::switch(&root.layout, &service).unwrap();
    let office = Connection::open(&root.layout.database).unwrap();
    assert_eq!(
        dump(&office, "office_board_state"),
        dump(&root.source(), "office_board_state")
    );
}

#[test]
fn backup_failures_leave_nothing_fenced() {
    let root = verified();
    let before = whole_source(&root.source());
    let service = Service::new(false);

    fault::set_available_bytes(Some(1024));
    let result = migration::switch(&root.layout, &service);
    fault::set_available_bytes(None);
    let Err(MigrationError::Backup(message)) = &result else {
        panic!("{result:?}");
    };
    assert!(
        message.contains("needs") && message.contains("1024"),
        "{message}"
    );
    assert_eq!(result.unwrap_err().code(), "OFFICE_STORAGE_BACKUP");
    assert!(!root.layout.backups.exists());
    assert_undecided(&root, &before);

    fault::corrupt_backup(true);
    let result = migration::switch(&root.layout, &service);
    fault::corrupt_backup(false);
    assert!(
        matches!(result, Err(MigrationError::Backup(_))),
        "{result:?}"
    );
    assert_eq!(backup_directories(&root), Vec::<std::path::PathBuf>::new());
    assert_undecided(&root, &before);
}

#[test]
fn an_unstoppable_service_blocks_the_switch_before_any_backup() {
    let root = verified();
    let before = whole_source(&root.source());
    let mut service = Service::new(true);
    service.stoppable = false;
    let result = migration::switch(&root.layout, &service);
    assert!(
        matches!(&result, Err(MigrationError::Service(_))),
        "{result:?}"
    );
    assert_eq!(result.unwrap_err().code(), "OFFICE_STORAGE_SERVICE");
    assert!(!root.layout.backups.exists());
    assert_undecided(&root, &before);
}

#[test]
fn an_unwritable_backup_directory_is_named() {
    let root = verified();
    let before = whole_source(&root.source());
    fs::create_dir(&root.layout.backups).unwrap();
    fs::set_permissions(&root.layout.backups, fs::Permissions::from_mode(0o500)).unwrap();
    if fs::write(root.layout.backups.join("probe"), b"").is_ok() {
        eprintln!("skipped: this user can write read-only directories");
        return;
    }
    let result = migration::switch(&root.layout, &Service::new(false));
    fs::set_permissions(&root.layout.backups, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        matches!(&result, Err(MigrationError::NotWritable(directory)) if *directory == root.layout.backups),
        "{result:?}"
    );
    assert!(result.unwrap_err().to_string().contains("is not writable"));
    assert_undecided(&root, &before);
}

#[test]
fn a_core_database_without_the_fence_cannot_be_switched() {
    let root = verified();
    root.source()
        .execute_batch("DROP TRIGGER office_board_state_after_office_cutover_update")
        .unwrap();
    let result = migration::switch(&root.layout, &Service::new(false));
    assert!(
        matches!(&result, Err(MigrationError::Source(message)) if message.contains("does not fence")),
        "{result:?}"
    );
    assert_eq!(receipt(&root), None);
    assert!(!root.layout.database.exists());
}

#[test]
fn core_fences_every_office_table_and_nothing_else() {
    let root = Root::new();
    let mut fences: Vec<String> = root
        .source()
        .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' AND sql LIKE '%extension_storage_cutovers%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    fences.sort();
    let mut expected: Vec<String> = schema::core_fences().collect();
    expected.sort();
    assert_eq!(fences, expected);
}

#[test]
fn staging_from_an_earlier_office_schema_is_upgraded_when_published() {
    let root = verified();
    root.staging()
        .execute_batch(
            "DROP TABLE _office_activation; DELETE FROM _office_schema WHERE version > 1;",
        )
        .unwrap();
    migration::switch(&root.layout, &Service::new(false)).unwrap();
    let versions: Vec<i64> = Connection::open(&root.layout.database)
        .unwrap()
        .prepare("SELECT version FROM _office_schema ORDER BY version")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(versions, [1, 2]);
    assert_eq!(state(&root), State::Switched);
}

#[test]
fn a_recorded_switch_with_missing_or_replaced_storage_requires_recovery() {
    let root = verified();
    let switched = migration::switch(&root.layout, &Service::new(false)).unwrap();
    let moved = root.path.join("office.db.moved");
    fs::rename(&root.layout.database, &moved).unwrap();
    let error = migration::recover(&root.layout).unwrap_err();
    assert_eq!(error.code(), "OFFICE_STORAGE_RECOVERY_REQUIRED");
    let message = error.to_string();
    assert!(message.contains("is missing"), "{message}");
    assert!(
        message.contains(&switched.backup.directory.display().to_string()),
        "{message}"
    );
    assert!(message.contains("Office storage recovery"), "{message}");
    // Never re-import or revert once the receipt exists.
    assert!(matches!(
        migration::switch(&root.layout, &Service::new(false)),
        Err(MigrationError::RecoveryRequired { .. })
    ));
    assert!(!root.layout.staging.exists());

    fs::copy(&moved, &root.layout.database).unwrap();
    let error = migration::recover(&root.layout).unwrap_err();
    assert!(error.to_string().contains("was replaced"), "{error}");
    fs::remove_file(&root.layout.database).unwrap();
    fs::rename(&moved, &root.layout.database).unwrap();
    assert_eq!(migration::recover(&root.layout).unwrap(), Recovery::Current);
}

#[test]
fn a_corrupt_published_database_is_never_activated() {
    let root = verified();
    fault::crash_at_point(Some("committed"));
    let _ = catch_unwind(AssertUnwindSafe(|| {
        migration::switch(&root.layout, &Service::new(false))
    }));
    fault::crash_at_point(None);
    let mut bytes = fs::read(&root.layout.database).unwrap();
    bytes[..100].fill(0xA5);
    fs::write(&root.layout.database, bytes).unwrap();
    let error = migration::recover(&root.layout).unwrap_err();
    assert!(error.to_string().contains("is unreadable"), "{error}");
    assert_eq!(error.code(), "OFFICE_STORAGE_RECOVERY_REQUIRED");
}
