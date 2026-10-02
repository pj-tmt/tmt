//! Append-only schema history. Never edit a migration after it ships.
use super::{Fault, StoreResult};
use rusqlite::{Connection, TransactionBehavior};

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
];

pub(super) fn check_version(connection: &Connection) -> StoreResult<u32> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version as usize > MIGRATIONS.len() {
        return Err(Fault::UnsupportedSchema(version));
    }
    Ok(version)
}

pub(super) fn migrate(connection: &mut Connection) -> StoreResult<()> {
    connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Recheck after acquiring the writer lock: another opener may have migrated.
    let version = check_version(&tx)?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as u32)?;
    }
    tx.commit()?;
    Ok(())
}
