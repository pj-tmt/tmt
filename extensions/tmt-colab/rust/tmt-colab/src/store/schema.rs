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
];
pub(super) const CURRENT_VERSION: u32 = MIGRATIONS.len() as u32;
/// How many pre-update copies stay: the newest by the schema they were taken at, so two upgrades in
/// a row still leave a way back past the first.
const COPIES_KEPT: usize = 2;

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
            true
        }
        _ => false,
    };
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as u32)?;
    }
    tx.commit()?;
    if let (true, Some(store)) = (copied, &path) {
        // The migration is committed: failing to tidy an old copy must not fail the open.
        for (_, old) in copies(store).into_iter().skip(COPIES_KEPT) {
            let _ = std::fs::remove_file(old);
        }
    }
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
    let write = || -> io::Result<()> {
        match fs::remove_file(&partial) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        let mut source = fs::File::open(store)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits())
            .open(&partial)?;
        io::copy(&mut source, &mut file)?;
        file.sync_all()?;
        fs::rename(&partial, &target)?;
        fs::File::open(target.parent().unwrap_or(Path::new(".")))?.sync_all()
    };
    write().map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        Fault::Backup(error)
    })
}
