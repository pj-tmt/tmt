use crate::{
    StorageLayout,
    migration::{self, MigrationError, State},
    schema,
};
use rusqlite::Connection;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

/// Test-only interruption points inside migration steps.
pub(crate) mod fault {
    use rusqlite::Connection;
    use std::cell::Cell;

    thread_local! {
        static FAIL_AFTER: Cell<Option<&'static str>> = const { Cell::new(None) };
        static FULL: Cell<bool> = const { Cell::new(false) };
    }

    pub fn fail_after(table: Option<&'static str>) {
        FAIL_AFTER.with(|value| value.set(table));
    }

    /// Caps staging at its current size, like a full disk.
    pub fn staging_full(enabled: bool) {
        FULL.with(|value| value.set(enabled));
    }

    pub fn after_table(table: &str) -> Result<(), String> {
        match FAIL_AFTER.with(Cell::get) {
            Some(target) if target == table => Err(format!("Injected interruption after {table}")),
            _ => Ok(()),
        }
    }

    thread_local! {
        static CRASH_AT: Cell<Option<&'static str>> = const { Cell::new(None) };
        static AVAILABLE: Cell<Option<u64>> = const { Cell::new(None) };
        static CORRUPT_BACKUP: Cell<bool> = const { Cell::new(false) };
    }

    /// Unwinds at a switch boundary like a killed process: open transactions
    /// roll back and locks are released, but no cleanup code runs.
    pub fn crash_at_point(point: Option<&'static str>) {
        CRASH_AT.with(|value| value.set(point));
    }

    type Action = (&'static str, Box<dyn FnOnce()>);

    thread_local! {
        static ACTION: std::cell::RefCell<Option<Action>> = const { std::cell::RefCell::new(None) };
    }

    /// Runs `action` once when the switch reaches `point`.
    pub fn at_point(point: &'static str, action: impl FnOnce() + 'static) {
        ACTION.with(|value| *value.borrow_mut() = Some((point, Box::new(action))));
    }

    pub fn point(point: &str) {
        let action = ACTION.with(|value| {
            let mut value = value.borrow_mut();
            match value.as_ref() {
                Some((target, _)) if *target == point => value.take().map(|(_, action)| action),
                _ => None,
            }
        });
        if let Some(action) = action {
            action();
        }
        if CRASH_AT.with(Cell::get) == Some(point) {
            CRASH_AT.with(|value| value.set(None));
            std::panic::panic_any(super::Crash(point.to_owned()));
        }
    }

    pub fn set_available_bytes(bytes: Option<u64>) {
        AVAILABLE.with(|value| value.set(bytes));
    }

    pub fn available_bytes() -> Option<u64> {
        AVAILABLE.with(Cell::get)
    }

    pub fn corrupt_backup(enabled: bool) {
        CORRUPT_BACKUP.with(|value| value.set(enabled));
    }

    /// Overwrites the first page of the backup after it was written.
    pub fn after_backup(database: &std::path::Path) {
        if CORRUPT_BACKUP.with(Cell::get) {
            let mut bytes = std::fs::read(database).unwrap();
            bytes[..100].fill(0xA5);
            std::fs::write(database, bytes).unwrap();
        }
    }

    pub fn configure_staging(connection: &Connection) {
        if FULL.with(Cell::get) {
            let pages: i64 = connection
                .query_row("PRAGMA page_count", [], |row| row.get(0))
                .unwrap();
            connection
                .pragma_update(None, "max_page_count", pages + 1)
                .unwrap();
        }
    }
}

/// Payload of an injected crash.
#[derive(Debug)]
pub(crate) struct Crash(pub String);

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A disposable global directory with a real, fully migrated core database.
struct Root {
    path: PathBuf,
    layout: StorageLayout,
}

impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tmt-office-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        let source = path.join("tmux-team.db");
        // Public core entry point only: it creates and migrates a real schema.
        drop(tmt_adapters::storage::Storage::open(&source).unwrap());
        let layout = StorageLayout::within(&source, &path, &path.join("config.json"));
        Self { path, layout }
    }

    fn source(&self) -> Connection {
        let connection = Connection::open(&self.layout.source).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        connection
    }

    fn staging(&self) -> Connection {
        Connection::open(&self.layout.staging).unwrap()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.layout.directory, fs::Permissions::from_mode(0o700));
        let _ = fs::remove_dir_all(&self.path);
    }
}

const ADA: &str = "11111111-1111-4111-8111-111111111111";
const RETIRED: &str = "22222222-2222-4222-8222-222222222222";
const WORLD: &str = "33333333-3333-4333-8333-333333333333";
const ROOT: &str = "44444444-4444-4444-8444-444444444444";
const MISSING_ROOM: &str = "55555555-5555-4555-8555-555555555555";
const SNAPSHOT: &str = "66666666-6666-4666-8666-666666666666";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Real-shaped Office rows with the cases a lossy copy would break: NULL vs
/// empty, non-UTF-8 text, REAL and TEXT in INTEGER columns, rowid gaps,
/// WITHOUT ROWID tables, tombstones, retired authors, dangling room
/// categories, and retained blocks under a saved world layout. Core seeds the
/// three singleton rows, which the fixture replaces with non-default values.
fn seed(connection: &Connection) {
    connection
        .execute_batch(&format!(
            "
INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime, retired_at_ms) VALUES
  ('{ADA}', 'Ada', 'ada', 't', 't', 'saved', NULL),
  ('{RETIRED}', 'Old', 'old', 't', 't', 'temporary', 5);
INSERT INTO office_local_blocks (block_id, target_kind, identity_id, revision, layout, updated_at_ms) VALUES
  ('b1111111-1111-4111-8111-111111111111', 'identity', '{ADA}', 2, '{{\"v\": 1,  \"s\":\"\u{e9}\u{2603}\"}}', 10),
  ('b2222222-2222-4222-8222-222222222222', 'identity', '{RETIRED}', 1, CAST(x'C3FF00' AS TEXT), 11),
  ('b3333333-3333-4333-8333-333333333333', 'lobby', NULL, 1, '', 12);
DELETE FROM office_local_blocks WHERE block_id = 'b1111111-1111-4111-8111-111111111111';
INSERT INTO office_local_worlds (singleton, id, created_at_ms, layout_revision, layout_json, layout_updated_at_ms) VALUES
  (1, '{WORLD}', 1, 4, '{{\"areas\":[{{\"identity\":\"{RETIRED}\"}}]}}', 7);
INSERT INTO office_local_profiles (identity_id, revision, profile, updated_at_ms) VALUES
  ('{RETIRED}', 3, '{{\"name\":\"R\"}}', 9), ('{ADA}', 1, '', 2);
INSERT OR REPLACE INTO office_board_state (singleton, revision, next_sequence) VALUES (1, 5, 4);
INSERT INTO office_board_entries (id, thread_id, is_root, category_kind, category_id, author_kind, author_id, author_name, revision, deleted, created_sequence, activity_sequence, created_at_ms, updated_at_ms, title, body) VALUES
  ('{ROOT}', '{ROOT}', 1, 'general', NULL, 'identity', '{RETIRED}', 'Old', 1, 0, 1, 3, 1, 1, '', NULL),
  ('e2222222-2222-4222-8222-222222222222', '{ROOT}', 0, 'general', NULL, 'owner', 'owner', NULL, 2, 0, 2, 2, 2, 3, NULL, CAST(x'FF' AS TEXT)),
  ('e3333333-3333-4333-8333-333333333333', 'e3333333-3333-4333-8333-333333333333', 1, 'room', '{MISSING_ROOM}', 'identity', '{ADA}', 'Ada', 3, 1, 3, 3, 4, 5, NULL, NULL);
INSERT INTO office_board_operations (actor_key, operation_id, intent_digest, result_kind, entry_id, thread_id, revision, created, changed, deleted, moderated) VALUES
  ('identity:{RETIRED}', 'o1111111-1111-4111-8111-111111111111', '{DIGEST}', 'create', '{ROOT}', NULL, 1, 1, 1.5, NULL, 'x'),
  ('owner', 'o2222222-2222-4222-8222-222222222222', '{DIGEST}', 'delete', 'e3333333-3333-4333-8333-333333333333', '{ROOT}', 3, NULL, 0, 1, 1);
INSERT INTO office_whiteboards (document_id, world_id, revision, scene, updated_at_ms) VALUES
  ('lobby', '{WORLD}', 2, '{{\"elements\":[]}}', 5);
INSERT INTO office_whiteboard_operations (world_id, operation_id, intent_digest, document_id, revision, changed, updated_at_ms) VALUES
  ('{WORLD}', 'w1111111-1111-4111-8111-111111111111', '{DIGEST}', 'lobby', 2, 1, 6);
INSERT INTO office_whiteboard_snapshots (snapshot_id, world_id, intent_digest, document_id, document_revision, scene, selected_element_ids, annotation, created_at_ms) VALUES
  ('{SNAPSHOT}', '{WORLD}', '{DIGEST}', 'lobby', 2, '{{}}', '[]', '', 8);
INSERT INTO office_whiteboard_snapshot_images (snapshot_id, pixel_digest, png) VALUES
  ('{SNAPSHOT}', '{DIGEST}', x'89504E47000D0A1A0A00');
INSERT INTO office_prop_packs (digest, bytes, prop_count, installed_revision, installed_at_ms) VALUES
  ('sha256:{DIGEST}', x'00FF7B7D', 2, 1, 3);
INSERT OR REPLACE INTO office_prop_catalog (singleton, revision, previous_kind, previous_digest, previous_base_revision, previous_result_revision) VALUES
  (1, 1, 'install', 'sha256:{DIGEST}', 0, 1);
INSERT INTO office_avatar_packs (digest, bytes, avatar_count, installed_revision, installed_at_ms) VALUES
  ('sha256:{DIGEST}', x'7B7D', 1, 2, 4);
INSERT OR REPLACE INTO office_avatar_catalog (singleton, revision, previous_kind, previous_digest, previous_base_revision, previous_result_revision) VALUES
  (1, 0, NULL, NULL, NULL, NULL);
"
        ))
        .unwrap();
}

/// Independent oracle: storage class and hex bytes of every cell, rowid first
/// for rowid tables, in key order. It shares no code with the migration.
fn dump(connection: &Connection, table: &str) -> Vec<String> {
    let columns: Vec<String> = connection
        .prepare(&format!(
            "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
        ))
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let without_rowid: bool = connection
        .query_row(
            "SELECT instr(upper(sql), 'WITHOUT ROWID') > 0 FROM sqlite_master WHERE name = ?",
            [table],
            |row| row.get(0),
        )
        .unwrap();
    let mut cells: Vec<String> = columns
        .iter()
        .map(|column| format!("typeof(\"{column}\") || ':' || hex(\"{column}\")"))
        .collect();
    let order = if without_rowid {
        "1".to_owned()
    } else {
        cells.insert(0, "'rowid:' || rowid".to_owned());
        "rowid".to_owned()
    };
    connection
        .prepare(&format!(
            "SELECT {} FROM \"{table}\" ORDER BY {order}",
            cells.join(" || '|' || ")
        ))
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap()
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Every table in the core database, including core-owned ones.
fn whole_source(connection: &Connection) -> Vec<(String, Vec<String>)> {
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let rows = dump(connection, &table);
            (table, rows)
        })
        .collect()
}

fn state(root: &Root) -> State {
    migration::status(&root.layout).unwrap().state
}

fn assert_staged_equals_source(root: &Root) {
    let (source, staging) = (root.source(), root.staging());
    for table in schema::OFFICE_TABLES {
        assert_eq!(dump(&source, table), dump(&staging, table), "{table}");
    }
}

#[test]
fn fresh_core_office_objects_match_office_schema_except_identity_references() {
    let root = Root::new();
    assert!(schema::source_matches(&root.source()).unwrap().is_ok());
    let core: Vec<String> = root
        .source()
        .prepare("SELECT sql FROM sqlite_master WHERE name IN ('office_local_blocks', 'office_local_profiles')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        core.iter()
            .all(|sql| sql.contains(" REFERENCES identities(id)"))
    );
    let office = Connection::open_in_memory().unwrap();
    schema::install(&office).unwrap();
    let references: i64 = office
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE sql LIKE '%REFERENCES identities%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(references, 0);
}

#[test]
fn copy_and_verify_preserve_every_typed_cell_rowid_and_retained_reference() {
    let root = Root::new();
    seed(&root.source());
    let before = whole_source(&root.source());
    assert_eq!(state(&root), State::Absent);
    assert_eq!(
        migration::prepare(&root.layout).unwrap().state,
        State::Prepared
    );
    let copied = migration::copy(&root.layout).unwrap();
    assert_eq!(copied.state, State::Copied);
    let verified = migration::verify(&root.layout).unwrap();
    assert_eq!(verified.state, State::Verified);
    assert_eq!(verified.source_manifest, copied.source_manifest);
    assert_staged_equals_source(&root);
    let rows: std::collections::BTreeMap<_, _> = verified.rows.into_iter().collect();
    assert_eq!(rows["office_local_blocks"], 2);
    assert_eq!(rows["office_board_entries"], 3);
    assert_eq!(rows["office_whiteboard_snapshot_images"], 1);
    // Spot-check the oracle itself on cells a lossy copy would alter.
    let staging = root.staging();
    let kinds: Vec<(String, String)> = staging
        .prepare("SELECT typeof(changed), typeof(moderated) FROM office_board_operations ORDER BY operation_id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(kinds[0], ("real".into(), "text".into()));
    let gap: Vec<i64> = staging
        .prepare("SELECT rowid FROM office_local_blocks ORDER BY rowid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(gap, vec![2, 3]);
    let bytes: Vec<u8> = staging
        .query_row(
            "SELECT CAST(layout AS BLOB) FROM office_local_blocks WHERE rowid = 2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bytes, [0xC3, 0xFF, 0x00]);
    assert!(
        !root.layout.database.exists(),
        "no destination before a switch"
    );
    assert_eq!(whole_source(&root.source()), before);
}

#[test]
fn interrupted_copy_keeps_the_prepared_state_and_resumes() {
    let root = Root::new();
    seed(&root.source());
    let before = whole_source(&root.source());
    migration::prepare(&root.layout).unwrap();
    fault::fail_after(Some("office_board_entries"));
    let error = migration::copy(&root.layout).unwrap_err();
    fault::fail_after(None);
    assert!(matches!(error, MigrationError::Destination(_)), "{error}");
    assert_eq!(state(&root), State::Prepared);
    let staged: i64 = root
        .staging()
        .query_row("SELECT count(*) FROM office_local_blocks", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(staged, 0, "a failed copy commits no rows");
    assert_eq!(whole_source(&root.source()), before);
    migration::copy(&root.layout).unwrap();
    migration::verify(&root.layout).unwrap();
    assert_staged_equals_source(&root);
}

#[test]
fn full_staging_storage_rolls_back_the_copy() {
    let root = Root::new();
    seed(&root.source());
    root.source()
        .execute(
            "UPDATE office_whiteboard_snapshot_images SET png = randomblob(262144)",
            [],
        )
        .unwrap();
    let before = whole_source(&root.source());
    migration::prepare(&root.layout).unwrap();
    fault::staging_full(true);
    let error = migration::copy(&root.layout).unwrap_err();
    fault::staging_full(false);
    assert!(error.to_string().contains("full"), "{error}");
    assert_eq!(state(&root), State::Prepared);
    assert_eq!(whole_source(&root.source()), before);
    migration::copy(&root.layout).unwrap();
    assert_eq!(
        migration::verify(&root.layout).unwrap().state,
        State::Verified
    );
}

#[test]
fn verification_rejects_a_changed_source_until_copied_again() {
    let root = Root::new();
    seed(&root.source());
    migration::prepare(&root.layout).unwrap();
    migration::copy(&root.layout).unwrap();
    root.source()
        .execute("UPDATE office_board_state SET revision = 6", [])
        .unwrap();
    let error = migration::verify(&root.layout).unwrap_err();
    assert!(matches!(error, MigrationError::SourceChanged), "{error}");
    assert_eq!(state(&root), State::Copied);
    migration::copy(&root.layout).unwrap();
    assert_eq!(
        migration::verify(&root.layout).unwrap().state,
        State::Verified
    );
    assert_staged_equals_source(&root);
}

#[test]
fn verification_rejects_a_tampered_copy_by_cell_not_by_count() {
    let root = Root::new();
    seed(&root.source());
    migration::prepare(&root.layout).unwrap();
    migration::copy(&root.layout).unwrap();
    // Same row count and blob length; one byte differs.
    root.staging()
        .execute(
            "UPDATE office_whiteboard_snapshot_images SET png = x'89504E47000D0A1A0A01'",
            [],
        )
        .unwrap();
    let error = migration::verify(&root.layout).unwrap_err();
    assert!(matches!(error, MigrationError::Mismatch(_)), "{error}");
    assert!(
        error
            .to_string()
            .contains("office_whiteboard_snapshot_images"),
        "{error}"
    );
    assert_eq!(state(&root), State::Copied);
}

#[test]
fn repeated_steps_are_idempotent() {
    let root = Root::new();
    seed(&root.source());
    migration::prepare(&root.layout).unwrap();
    let first = migration::copy(&root.layout).unwrap();
    let second = migration::copy(&root.layout).unwrap();
    assert_eq!(first.source_manifest, second.source_manifest);
    migration::verify(&root.layout).unwrap();
    let again = migration::verify(&root.layout).unwrap();
    assert_eq!(again.state, State::Verified);
    assert_staged_equals_source(&root);
}

#[test]
fn steps_require_their_predecessor_and_no_destination() {
    let root = Root::new();
    assert!(matches!(
        migration::copy(&root.layout),
        Err(MigrationError::State(_))
    ));
    migration::prepare(&root.layout).unwrap();
    assert!(matches!(
        migration::verify(&root.layout),
        Err(MigrationError::State(_))
    ));
    fs::write(&root.layout.database, b"").unwrap();
    for result in [
        migration::prepare(&root.layout),
        migration::copy(&root.layout),
    ] {
        assert!(matches!(result, Err(MigrationError::State(_))));
    }
}

#[test]
fn a_replaced_or_reshaped_source_is_rejected() {
    let root = Root::new();
    seed(&root.source());
    migration::prepare(&root.layout).unwrap();
    let moved = root.path.join("moved.db");
    fs::copy(&root.layout.source, &moved).unwrap();
    fs::rename(&moved, &root.layout.source).unwrap();
    let error = migration::copy(&root.layout).unwrap_err();
    assert!(error.to_string().contains("replaced"), "{error}");

    let reshaped = Root::new();
    reshaped
        .source()
        .execute("ALTER TABLE office_board_state ADD COLUMN extra TEXT", [])
        .unwrap();
    let error = migration::prepare(&reshaped.layout).unwrap_err();
    assert!(matches!(error, MigrationError::Source(_)), "{error}");
    assert!(!reshaped.layout.staging.exists());
}

#[test]
fn a_concurrent_step_is_reported_busy() {
    let root = Root::new();
    fs::create_dir_all(&root.layout.directory).unwrap();
    let held = tmt_adapters::file_lock::exclusive(&root.layout.lock).unwrap();
    assert!(matches!(
        migration::prepare(&root.layout),
        Err(MigrationError::Busy)
    ));
    drop(held);
    migration::prepare(&root.layout).unwrap();
}

#[test]
fn an_unwritable_office_directory_fails_without_state_or_source_changes() {
    let root = Root::new();
    seed(&root.source());
    let before = whole_source(&root.source());
    fs::create_dir_all(&root.layout.directory).unwrap();
    // A preexisting lock file lets the step reach staging creation.
    drop(tmt_adapters::file_lock::exclusive(&root.layout.lock).unwrap());
    fs::set_permissions(&root.layout.directory, fs::Permissions::from_mode(0o500)).unwrap();
    if fs::write(root.layout.directory.join("probe"), b"").is_ok() {
        eprintln!("skipped: this user can write read-only directories");
        return;
    }
    let result = migration::prepare(&root.layout);
    fs::set_permissions(&root.layout.directory, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        matches!(&result, Err(MigrationError::NotWritable(directory)) if *directory == root.layout.directory),
        "{result:?}"
    );
    assert_eq!(result.unwrap_err().code(), "OFFICE_STORAGE_NOT_WRITABLE");
    assert_eq!(state(&root), State::Absent);
    assert_eq!(whole_source(&root.source()), before);
}

#[test]
fn staging_is_private() {
    let root = Root::new();
    migration::prepare(&root.layout).unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&root.layout.directory), 0o700);
    assert_eq!(mode(&root.layout.staging), 0o600);
}

mod switch;

mod plan;
mod selection;
