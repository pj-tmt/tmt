//! Append-only schema history. Never edit a migration after it ships.
use super::{Fault, StoreResult};
use rusqlite::{Connection, TransactionBehavior};
use std::path::{Path, PathBuf};

const MIGRATIONS: &[&str] = &[
    // Schema 1: original opaque ciphertext store.
    r#"            CREATE TABLE IF NOT EXISTS pages(page TEXT PRIMARY KEY, epoch TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS streams(page TEXT, epoch TEXT, stream TEXT, frozen INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(page,epoch,stream), FOREIGN KEY(page) REFERENCES pages(page));
            CREATE TABLE IF NOT EXISTS receipts(page TEXT, epoch TEXT, stream TEXT, seq TEXT, namespace TEXT NOT NULL,
                hash BLOB NOT NULL, digest BLOB NOT NULL, payload BLOB,
                PRIMARY KEY(page,epoch,stream,seq), FOREIGN KEY(page,epoch,stream) REFERENCES streams(page,epoch,stream));
            CREATE TABLE IF NOT EXISTS checkpoints(page TEXT, epoch TEXT, stream TEXT, namespace TEXT, seq TEXT,
                hash BLOB NOT NULL, digest BLOB NOT NULL, head BLOB NOT NULL, payload BLOB, pinned INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(page,epoch,stream,namespace,seq), FOREIGN KEY(page,epoch,stream) REFERENCES streams(page,epoch,stream));
"#,
    // Schema 2: owner authority state; existing ciphertext tables remain intact.
    r#"
    CREATE TABLE owner_state(singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        space TEXT NOT NULL, root BLOB NOT NULL, revision TEXT NOT NULL, hash BLOB NOT NULL,
        member_id TEXT NOT NULL, member_sign BLOB NOT NULL, member_enc BLOB NOT NULL);
    CREATE TABLE membership_log(revision TEXT PRIMARY KEY, hash BLOB NOT NULL UNIQUE, envelope BLOB NOT NULL);
    CREATE TABLE recipients(kind TEXT NOT NULL, id TEXT NOT NULL, record BLOB NOT NULL,
        PRIMARY KEY(kind,id));
    CREATE TABLE devices(id TEXT PRIMARY KEY, record BLOB NOT NULL);
    CREATE TABLE epoch_secrets(page TEXT NOT NULL, epoch TEXT NOT NULL, secret BLOB NOT NULL,
        PRIMARY KEY(page,epoch), FOREIGN KEY(page) REFERENCES pages(page));
    CREATE TABLE wraps(page TEXT NOT NULL, epoch TEXT NOT NULL, kind TEXT NOT NULL,
        recipient TEXT NOT NULL, revision TEXT NOT NULL, envelope BLOB NOT NULL,
        PRIMARY KEY(page,epoch,kind,recipient,revision),
        FOREIGN KEY(page,epoch) REFERENCES epoch_secrets(page,epoch));
    CREATE TABLE owner_operations(id TEXT PRIMARY KEY, digest BLOB NOT NULL, outcome BLOB NOT NULL);
    "#,
    // Schema 3: remote device bindings and durable local revocation tombstones.
    r#"
    CREATE TABLE device_registrations(device_id TEXT PRIMARY KEY, binding BLOB,
        revoked INTEGER NOT NULL CHECK(revoked IN (0,1)), grant_revision TEXT NOT NULL);
    "#,
    // Schema 4: immutable epoch baselines; descriptors are owner-log commitments.
    r#"
    CREATE TABLE baselines(page TEXT NOT NULL, epoch TEXT NOT NULL, descriptor BLOB NOT NULL,
        envelope BLOB NOT NULL, PRIMARY KEY(page,epoch),
        FOREIGN KEY(page,epoch) REFERENCES epoch_secrets(page,epoch));
    "#,
    // Schema 5: server-observed content update evidence; legacy pages stay unknown.
    r#"
    ALTER TABLE pages ADD COLUMN last_update_at_ms INTEGER
        CHECK(last_update_at_ms IS NULL OR
            (typeof(last_update_at_ms)='integer' AND last_update_at_ms>=0 AND last_update_at_ms<=9007199254740991));
    "#,
    // Schema 6: original content publication outcomes; legacy operations stay unscoped.
    r#"
    ALTER TABLE owner_operations ADD COLUMN publication_kind TEXT;
    ALTER TABLE owner_operations ADD COLUMN space TEXT;
    ALTER TABLE owner_operations ADD COLUMN page TEXT;
    ALTER TABLE owner_operations ADD COLUMN original_epoch TEXT;
    ALTER TABLE owner_operations ADD COLUMN original_stream TEXT
        CHECK((publication_kind IS NULL AND space IS NULL AND page IS NULL
            AND original_epoch IS NULL AND original_stream IS NULL) OR
            (publication_kind IS NOT NULL AND publication_kind='content'
            AND typeof(space)='text' AND typeof(page)='text'
            AND typeof(original_epoch)='text' AND typeof(original_stream)='text'));
    CREATE INDEX owner_operations_page ON owner_operations(page) WHERE publication_kind IS NOT NULL;
    "#,
    // Schema 7: page labels, display-only names for the owner's page list. Not signed, not in the
    // owner log; a deleted page's rows are removed with its data and its tombstone row stays.
    r#"
    CREATE TABLE page_labels(page TEXT NOT NULL, label TEXT NOT NULL,
        PRIMARY KEY(page,label), FOREIGN KEY(page) REFERENCES pages(page)) WITHOUT ROWID;
    "#,
    // Schema 8: bounded publication progress; originals stay in the existing tables.
    r#"
    CREATE TABLE cloud_publications(
        page TEXT NOT NULL, epoch TEXT NOT NULL, stream TEXT NOT NULL,
        binding BLOB NOT NULL CHECK(length(binding)=32), uid TEXT NOT NULL,
        bootstrap_seq TEXT NOT NULL, confirmed_seq TEXT NOT NULL,
        PRIMARY KEY(page,epoch,stream), FOREIGN KEY(page) REFERENCES pages(page));
    "#,
    // Schema 9: a live link's cloud admission token. Derived from the seed at creation, the only
    // moment the seed exists; removed with the link. Not signed and not in the owner log.
    r#"
    CREATE TABLE link_admissions(link TEXT PRIMARY KEY, token TEXT NOT NULL) WITHOUT ROWID;
    "#,
    // Schema 10: the (page, link) pairs whose cloud `links` row this machine may have written, until
    // the link's removal from the cloud is complete. Intent is recorded before the row is created.
    r#"
    CREATE TABLE link_publications(page TEXT NOT NULL, link TEXT NOT NULL,
        PRIMARY KEY(page,link), FOREIGN KEY(page) REFERENCES pages(page)) WITHOUT ROWID;
    "#,
    // Schema 11: when the cloud last confirmed the page's owner row, and the expiry that row then
    // carried, for the page's current epoch. The cloud deletes a page at that expiry, so this is
    // the only honest source of when it ends there; nothing here is derived from local activity.
    // A row at a past epoch is replaced, never kept; removed with the page's data.
    r#"
    CREATE TABLE cloud_page_rows(page TEXT PRIMARY KEY, epoch TEXT NOT NULL,
        expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>=0),
        FOREIGN KEY(page) REFERENCES pages(page)) WITHOUT ROWID;
    "#,
    // Schema 12: what the owner's serve decided about each edit request a link device wrote, so a
    // request is applied at most once and a refused one is never evaluated again. Not signed, not
    // in the owner log; rows go with the page.
    r#"
    CREATE TABLE edit_decisions(page TEXT NOT NULL, epoch TEXT NOT NULL, writer TEXT NOT NULL,
        operation TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('applied','conflict','refused')),
        PRIMARY KEY(page,epoch,writer,operation), FOREIGN KEY(page) REFERENCES pages(page)) WITHOUT ROWID;
    "#,
    // Schema 13: projects replace page labels. A project exists on its own (a new one may hold no
    // page) and a page can be in several; every label becomes a project of the same name and keeps
    // its pages. Display-only like labels: not signed, not in the owner log; a deleted page's rows
    // are removed with its data.
    r#"
    CREATE TABLE projects(name TEXT PRIMARY KEY) WITHOUT ROWID;
    INSERT INTO projects(name) SELECT DISTINCT label FROM page_labels;
    CREATE TABLE page_projects(page TEXT NOT NULL, project TEXT NOT NULL,
        PRIMARY KEY(page,project), FOREIGN KEY(page) REFERENCES pages(page),
        FOREIGN KEY(project) REFERENCES projects(name)) WITHOUT ROWID;
    INSERT INTO page_projects(page,project) SELECT page,label FROM page_labels;
    DROP TABLE page_labels;
    "#,
    // Schema 14: who signed a checkpoint. NULL is the stream's own device, as every checkpoint
    // before this; an owner fold (#3983) names the owner-member device that signed it for the
    // stream, so a prune keeps its anchor and a revoke can list the folds its device signed.
    r#"
    ALTER TABLE checkpoints ADD COLUMN signer TEXT;
    "#,
    // Schema 15: server-observed page activity (#2600): the clock at the last admitted content or
    // own-stream update (edits, comments, replies) or explicit owner restore. Legacy pages stay
    // unknown until new activity; nothing is backfilled and a read never establishes it.
    r#"
    ALTER TABLE pages ADD COLUMN last_activity_at_ms INTEGER
        CHECK(last_activity_at_ms IS NULL OR
            (typeof(last_activity_at_ms)='integer' AND last_activity_at_ms>=0 AND last_activity_at_ms<=9007199254740991));
    "#,
    // Schema 16: the owner's local plaintext page titles (#4218), one row per active page, written
    // only from the owner's verified fold at the page's progress key (`page_progress`). Display-only
    // projection state: never a log statement, never synced, published, exported or shared, and
    // rebuildable from the fold, so losing it loses nothing.
    r#"
    CREATE TABLE page_titles(page TEXT PRIMARY KEY, progress TEXT NOT NULL,
        title TEXT NOT NULL CHECK(length(CAST(title AS BLOB)) BETWEEN 1 AND 512),
        FOREIGN KEY(page) REFERENCES pages(page)) WITHOUT ROWID;
    "#,
    // Schema 17: the owner's pinned pages (#4356), one row per pinned page; `seq` is the pin order.
    // Display-only like projects: never a log statement, never synced, published, exported or
    // shared. Archive and delete remove the row; unarchive does not bring it back.
    r#"
    CREATE TABLE page_pins(page TEXT PRIMARY KEY, seq INTEGER NOT NULL,
        FOREIGN KEY(page) REFERENCES pages(page)) WITHOUT ROWID;
    "#,
    // Schema 18: the owner's pinned projects (#5073), the sidebar's list in the owner's order.
    // `project_pins` holds one row per pinned project, `seq` its place; `project_pin_choice` has its
    // one row once the owner has written the list, so an empty chosen list differs from the
    // automatic default of a space that never chose. Display-only like projects: never a log
    // statement. Deleting a project removes its pin; renaming moves it.
    r#"
    CREATE TABLE project_pins(name TEXT PRIMARY KEY, seq INTEGER NOT NULL,
        FOREIGN KEY(name) REFERENCES projects(name)) WITHOUT ROWID;
    CREATE TABLE project_pin_choice(chosen INTEGER PRIMARY KEY CHECK(chosen=1));
    "#,
    // Schema 19: terminal metadata-request results survive a crash before their signed own
    // receipt is appended. Existing source-request decisions and their semantics are unchanged.
    r#"
    ALTER TABLE edit_decisions ADD COLUMN result TEXT;
    "#,
    // Schema 20: local evidence of a successful automatic page cleanup; never synced.
    r#"
    CREATE TABLE page_auto_cleaned(page TEXT PRIMARY KEY, at_ms INTEGER NOT NULL
        CHECK(typeof(at_ms)='integer' AND at_ms>=0 AND at_ms<=9007199254740991),
        FOREIGN KEY(page) REFERENCES pages(page) ON DELETE CASCADE) WITHOUT ROWID;
    "#,
];
pub(super) const CURRENT_VERSION: u32 = MIGRATIONS.len() as u32;
/// How many pre-update copies stay: the one just taken and the newest other by the schema it was
/// taken at, so two upgrades in a row still leave a way back past the first.
const COPIES_KEPT: usize = 2;
const RECLAIM_PAGES: u32 = 64;

pub(super) fn check_version(connection: &Connection) -> StoreResult<u32> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > CURRENT_VERSION {
        // The newest copy at a schema this build can open is the way back to it.
        let copy = store_path(connection).and_then(|store| {
            copies(&store)
                .into_iter()
                .find(|(schema, _)| *schema <= CURRENT_VERSION)
                .map(|(_, copy)| copy)
        });
        return Err(Fault::UnsupportedSchema(version, copy));
    }
    Ok(version)
}

/// The store file behind `connection`; none for an in-memory database.
fn store_path(connection: &Connection) -> Option<PathBuf> {
    connection
        .path()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

pub(super) fn check_read_version(connection: &Connection) -> StoreResult<()> {
    let version = check_version(connection)?;
    if version < CURRENT_VERSION {
        return Err(Fault::OutdatedSchema(version));
    }
    Ok(())
}

pub(super) fn migrate(connection: &mut Connection) -> StoreResult<()> {
    connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    let path = store_path(connection);
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Recheck after acquiring the writer lock: another opener may have migrated.
    let version = check_version(&tx)?;
    // An existing store is copied once before the first step. The writer lock is held and the
    // journal mode is DELETE, so the file is complete and nothing can change it. A failed copy stops
    // here, with the store untouched; a fresh store (version 0) holds nothing to keep.
    let copied = match &path {
        Some(store) if version > 0 && version < CURRENT_VERSION => {
            backup(store, version)?;
            Some(version)
        }
        _ => None,
    };
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as u32)?;
    }
    if version == CURRENT_VERSION {
        // No schema write occurred. Release the writer reservation without COMMIT's
        // exclusive-lock upgrade: a reader may defer physical conversion, never this open.
        tx.rollback()?;
    } else {
        tx.commit()?;
    }
    if let (Some(taken), Some(store)) = (copied, &path) {
        // The migration is committed: failing to tidy an old copy must not fail the open. The copy
        // just taken is the way back from this update, so it stays whatever schemas the others
        // are at (a newer Colab's copies outrank it by number); the newest other one stays with it.
        let others = copies(store)
            .into_iter()
            .filter(|(schema, _)| *schema != taken);
        for (_, old) in others.skip(COPIES_KEPT - 1) {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(())
}

/// VACUUM takes its own writer lock after migrations finish. Its mode is the durable marker:
/// failed conversion leaves a usable store and retries on next open, without copying the file.
pub(super) fn enable_incremental(connection: &Connection) {
    let convert = || -> rusqlite::Result<()> {
        if connection.pragma_query_value(None, "auto_vacuum", |r| r.get::<_, u32>(0))? == 2 {
            return Ok(());
        }
        connection.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        connection.execute_batch("VACUUM")
    };
    if let Err(error) = convert() {
        let mut output = tmt_cli_style::stream::stderr();
        let terminal = output.terminal();
        // A diagnostic failure cannot turn best-effort maintenance into a failed open.
        let _ = tmt_cli_style::message::warning(
            &mut output,
            terminal,
            &format!(
                "Colab could not enable disk reclaim: {error}. It will retry on the next open."
            ),
            None,
        );
    }
}

/// Physical reclaim follows the committed prune, so an I/O error cannot roll its write back.
pub(super) fn reclaim(connection: &Connection) {
    // Match best-effort backup cleanup: repeated maintenance failures stay silent, and the
    // next committed prune retries. The durable write's result never depends on reclaim.
    let _ = reclaim_free_pages(connection);
}

fn reclaim_free_pages(connection: &Connection) -> rusqlite::Result<()> {
    // Bound routine work to 256 KiB at SQLite's default page size, as Core does. SQLite yields
    // one row per page: execute_batch would stop at the first row without finishing the reclaim.
    let mut statement =
        connection.prepare(&format!("PRAGMA incremental_vacuum({RECLAIM_PAGES})"))?;
    let mut rows = statement.query([])?;
    while rows.next()?.is_some() {}
    Ok(())
}

/// Where the copy of a store taken before migrating it from schema `version` lives: next to the
/// store, in the same private data root.
pub fn backup_path(store: &Path, version: u32) -> PathBuf {
    let mut name = store.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".pre-v{version}.bak"));
    store.with_file_name(name)
}

/// Every pre-update copy next to `store` as (schema it was taken at, path), newest schema first.
fn copies(store: &Path) -> Vec<(u32, PathBuf)> {
    let (Some(directory), Some(name)) = (store.parent(), store.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}.pre-v", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut found: Vec<(u32, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().into_string().ok()?;
            let digits = file.strip_prefix(&prefix)?.strip_suffix(".bak")?;
            let schema: u32 = digits.parse().ok()?;
            // Only the exact name `backup_path` writes: no sign, no leading zero.
            (schema.to_string() == digits).then(|| (schema, entry.path()))
        })
        .collect();
    found.sort_by_key(|(schema, _)| std::cmp::Reverse(*schema));
    found
}

/// Writes the copy owner-only and durably, through a partial file renamed into place, replacing an
/// older copy at the same schema. Any failure leaves no partial file and is a [`Fault::Backup`].
fn backup(store: &Path, version: u32) -> StoreResult<()> {
    use std::{fs, io, os::unix::fs::OpenOptionsExt};
    let target = backup_path(store, version);
    let mut partial_name = target.file_name().unwrap_or_default().to_os_string();
    partial_name.push(".partial");
    let partial = target.with_file_name(partial_name);
    let write = || -> crate::Result<()> {
        match fs::remove_file(&partial) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits())
            .open(&partial)?;
        // The migration already holds a writer reservation on another connection. Read through
        // SQLite so closing the source cannot discard that connection's POSIX lock.
        let source = Connection::open_with_flags(
            store,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        let mut destination = Connection::open_with_flags(
            &partial,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        destination.pragma_update(None, "synchronous", "FULL")?;
        {
            let copy = rusqlite::backup::Backup::new(&source, &mut destination)?;
            let result = copy.step(-1)?;
            if result != rusqlite::backup::StepResult::Done {
                return Err(format!("SQLite backup did not complete: {result:?}").into());
            }
        }
        destination.close().map_err(|(_, error)| error)?;
        source.close().map_err(|(_, error)| error)?;
        file.sync_all()?;
        fs::rename(&partial, &target)?;
        fs::File::open(target.parent().unwrap_or(Path::new(".")))?.sync_all()?;
        Ok(())
    };
    write().map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        Fault::Backup(std::io::Error::other(error))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The peer runs in a different process because POSIX locks are process-scoped. Its own
    /// SQLite connection must fail for a lock, and must succeed after the holder releases it.
    #[test]
    fn sqlite_files_keep_other_connections_locks() {
        use crate::{keyring::Layout, store::Store};
        use std::{
            env,
            ffi::OsString,
            time::{Duration, Instant},
        };
        const PROBE: &str = "TMT_COLAB_SQLITE_LOCK_PROBE";
        const EXPECT: &str = "TMT_COLAB_SQLITE_LOCK_EXPECT";
        const TEST: &str = "store::schema::tests::sqlite_files_keep_other_connections_locks";
        if let Some(path) = env::var_os(PROBE) {
            let connection = Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                    | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )
            .unwrap();
            connection.busy_timeout(Duration::ZERO).unwrap();
            let attempt = connection.execute_batch("BEGIN IMMEDIATE; ROLLBACK");
            if env::var(EXPECT).unwrap() == "blocked" {
                assert!(
                    matches!(&attempt, Err(rusqlite::Error::SqliteFailure(error, _))
                    if error.code == rusqlite::ErrorCode::DatabaseBusy),
                    "peer acquired the held lock: {attempt:?}"
                );
            } else {
                attempt.unwrap();
            }
            return;
        }
        let root = env::temp_dir().join(format!("tmt-colab-sqlite-lock-{}", std::process::id()));
        assert!(!root.exists());
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let layout = Layout::open(&root).unwrap();
        let _cleanup = Cleanup(root.clone());
        Store::open(&layout).unwrap().close().unwrap();
        let path = layout.directory.join("space.db");
        let held = Connection::open(&path).unwrap();
        let peer = |expected: &str| {
            let mut database = OsString::from(format!("{PROBE}="));
            database.push(&path);
            let args = [
                database,
                format!("{EXPECT}={expected}").into(),
                env::current_exe().unwrap().into_os_string(),
                "--exact".into(),
                TEST.into(),
                "--nocapture".into(),
            ];
            let output = tmt_invoke::invoke(
                tmt_invoke::Request {
                    program: Path::new("/usr/bin/env"),
                    args: &args,
                    input: &[],
                    deadline: Instant::now() + Duration::from_secs(10),
                    max_stream_bytes: 8192,
                    launch: tmt_invoke::LaunchOptions {
                        environment: tmt_invoke::EnvironmentPolicy::ClearAllowlist(&[]),
                        ..Default::default()
                    },
                },
                None,
            )
            .unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        for operation in ["read", "write_existing", "open", "backup"] {
            held.execute_batch("BEGIN IMMEDIATE").unwrap();
            peer("blocked");
            let opened = match operation {
                "read" => Some(Store::read(&layout).unwrap()),
                "write_existing" => Some(Store::write_existing(&layout).unwrap()),
                "open" => {
                    let error = Store::open(&layout)
                        .err()
                        .expect("a second writer must wait");
                    assert!(crate::store::contended(error.as_ref()), "{error}");
                    None
                }
                "backup" => {
                    backup(&path, CURRENT_VERSION).unwrap();
                    None
                }
                _ => unreachable!(),
            };
            peer("blocked");
            drop(opened);
            peer("blocked");
            held.execute_batch("ROLLBACK").unwrap();
            peer("available");
            if operation == "open" {
                Store::open(&layout).unwrap().close().unwrap();
            }
        }
        drop(held);
    }

    #[test]
    fn failed_reclaim_keeps_the_committed_write() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "PRAGMA auto_vacuum=INCREMENTAL;
            CREATE TABLE payloads(value BLOB);
            INSERT INTO payloads VALUES(zeroblob(524288));
            BEGIN IMMEDIATE; DELETE FROM payloads;
            INSERT INTO payloads VALUES(X'0102'); COMMIT;
            PRAGMA query_only=ON;",
            )
            .unwrap();
        assert!(matches!(
            reclaim_free_pages(&connection).unwrap_err(),
            rusqlite::Error::SqliteFailure(error, _) if error.code == rusqlite::ErrorCode::ReadOnly
        ));
        reclaim(&connection);
        let saved: Vec<u8> = connection
            .query_row("SELECT value FROM payloads", [], |r| r.get(0))
            .unwrap();
        assert_eq!(saved, [1, 2]);
        // A later successful attempt really reclaims pages; failure is not a silent no-op fixture.
        connection.pragma_update(None, "query_only", false).unwrap();
        let before: u32 = connection
            .pragma_query_value(None, "freelist_count", |r| r.get(0))
            .unwrap();
        reclaim_free_pages(&connection).unwrap();
        let after: u32 = connection
            .pragma_query_value(None, "freelist_count", |r| r.get(0))
            .unwrap();
        assert!(before >= RECLAIM_PAGES);
        assert_eq!(after, before - RECLAIM_PAGES);
    }
}
