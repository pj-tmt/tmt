//! Durable opaque envelopes. Callers own signature/session/role admission.
//! Receipt/hash rows survive payload pruning, including interleaved namespaces.
pub(crate) mod auth;

use crate::{Result, keyring::Layout, limits};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    Content,
    Own,
}
impl Namespace {
    fn name(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Own => "own",
        }
    }
}
#[derive(Clone, Copy)]
pub struct StreamScope<'a> {
    pub page: &'a str,
    pub epoch: u64,
    pub stream: &'a str,
}
pub struct Envelope<'a> {
    pub scope: StreamScope<'a>,
    pub namespace: Namespace,
    pub seq: u64,
    pub hash: [u8; 32],
    pub previous: [u8; 32],
    pub bytes: &'a [u8],
}
#[derive(Debug, PartialEq, Eq)]
pub enum Accepted {
    New,
    Replay,
}
#[derive(Debug)]
pub enum Fault {
    Invalid,
    StaleEpoch,
    StaleCheckpoint,
    UnsupportedSchema(u32),
    Gap,
    Conflict,
    Capacity,
    Sql(rusqlite::Error),
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Self::UnsupportedSchema(version) = self {
            write!(
                f,
                "Unsupported colab store schema {version}; database was not changed."
            )
        } else {
            write!(f, "{self:?}")
        }
    }
}
impl std::error::Error for Fault {}
impl From<rusqlite::Error> for Fault {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}
type StoreResult<T> = std::result::Result<T, Fault>;

pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(layout: &Layout) -> Result<Self> {
        layout.file("space.db")?.sync_all()?;
        std::fs::File::open(&layout.directory)?.sync_all()?;
        let connection = Connection::open_with_flags(
            layout.directory.join("space.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 2 {
            return Err(Fault::UnsupportedSchema(version).into());
        }
        connection.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
        )?;
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS pages(page TEXT PRIMARY KEY, epoch TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS streams(page TEXT, epoch TEXT, stream TEXT, frozen INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(page,epoch,stream), FOREIGN KEY(page) REFERENCES pages(page));
            CREATE TABLE IF NOT EXISTS receipts(page TEXT, epoch TEXT, stream TEXT, seq TEXT, namespace TEXT NOT NULL,
                hash BLOB NOT NULL, digest BLOB NOT NULL, payload BLOB,
                PRIMARY KEY(page,epoch,stream,seq), FOREIGN KEY(page,epoch,stream) REFERENCES streams(page,epoch,stream));
            CREATE TABLE IF NOT EXISTS checkpoints(page TEXT, epoch TEXT, stream TEXT, namespace TEXT, seq TEXT,
                hash BLOB NOT NULL, digest BLOB NOT NULL, head BLOB NOT NULL, payload BLOB, pinned INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(page,epoch,stream,namespace,seq), FOREIGN KEY(page,epoch,stream) REFERENCES streams(page,epoch,stream));
            PRAGMA user_version=1; COMMIT;")?;
        }
        Ok(Self { connection })
    }
    pub fn create_page(&self, page: &str) -> StoreResult<()> {
        bounded_id(page)?;
        self.connection
            .execute("INSERT INTO pages VALUES (?,'1')", [page])?;
        Ok(())
    }
    /// Metadata seam for a caller's verified owner transition; no remote route in L2a.
    pub fn advance_epoch(&mut self, page: &str, expected: u64) -> StoreResult<()> {
        let next = expected.checked_add(1).ok_or(Fault::Invalid)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE pages SET epoch=? WHERE page=? AND epoch=?",
            params![next.to_string(), page, expected.to_string()],
        )? != 1
        {
            return Err(Fault::StaleEpoch);
        }
        transaction.commit()?;
        Ok(())
    }
    pub fn append(&mut self, envelope: &Envelope<'_>) -> StoreResult<Accepted> {
        validate(envelope)?;
        let s = envelope.scope;
        let epoch = s.epoch.to_string();
        let seq = sequence(envelope.seq);
        let digest = Sha256::digest(envelope.bytes).to_vec();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, s)?;
        tx.execute(
            "INSERT OR IGNORE INTO streams(page,epoch,stream) VALUES (?,?,?)",
            params![s.page, epoch, s.stream],
        )?;
        let old: Option<(Vec<u8>, Vec<u8>, String)> = tx
            .query_row(
                "SELECT hash,digest,namespace FROM receipts WHERE page=? AND epoch=? AND stream=? AND seq=?",
                params![s.page, epoch, s.stream, seq],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((hash, stored_digest, namespace)) = old {
            if hash == envelope.hash
                && stored_digest == digest
                && namespace == envelope.namespace.name()
            {
                return Ok(Accepted::Replay);
            }
            tx.execute(
                "UPDATE streams SET frozen=1 WHERE page=? AND epoch=? AND stream=?",
                params![s.page, epoch, s.stream],
            )?;
            tx.commit()?;
            return Err(Fault::Conflict);
        }
        unfrozen(&tx, s)?;
        let head: Option<(String,Vec<u8>)> = tx.query_row("SELECT seq,hash FROM receipts WHERE page=? AND epoch=? AND stream=? ORDER BY seq DESC LIMIT 1",
            params![s.page,epoch,s.stream], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (expected, previous) = match head {
            Some((seq, hash)) => (
                seq.parse::<u64>()
                    .map_err(|_| Fault::Invalid)?
                    .checked_add(1)
                    .ok_or(Fault::Capacity)?,
                hash,
            ),
            None => (1, vec![0; 32]),
        };
        if envelope.seq != expected {
            return Err(Fault::Gap);
        }
        if previous != envelope.previous {
            return Err(Fault::Conflict);
        }
        capacity(&tx, s.page, envelope.bytes.len(), true)?;
        tx.execute(
            "INSERT INTO receipts VALUES (?,?,?,?,?,?,?,?)",
            params![
                s.page,
                epoch,
                s.stream,
                seq,
                envelope.namespace.name(),
                envelope.hash.as_slice(),
                digest,
                envelope.bytes
            ],
        )?;
        tx.commit()?;
        Ok(Accepted::New)
    }
    /// Publish the admitted namespace checkpoint and prune its prefix in one transaction.
    pub fn checkpoint(&mut self, envelope: &Envelope<'_>) -> StoreResult<Accepted> {
        validate(envelope)?;
        let s = envelope.scope;
        let epoch = s.epoch.to_string();
        let seq = sequence(envelope.seq);
        let digest = Sha256::digest(envelope.bytes).to_vec();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, s)?;
        unfrozen(&tx, s)?;
        let head: Option<Vec<u8>> = tx
            .query_row(
                "SELECT hash FROM receipts WHERE page=? AND epoch=? AND stream=? AND seq=?",
                params![s.page, epoch, s.stream, seq],
                |r| r.get(0),
            )
            .optional()?;
        if head.as_deref() != Some(envelope.previous.as_slice()) {
            return Err(Fault::Gap);
        }
        let old: Option<(Vec<u8>,Vec<u8>)> = tx.query_row("SELECT hash,digest FROM checkpoints WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq=?",
            params![s.page,epoch,s.stream,envelope.namespace.name(),seq], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((hash, stored)) = old {
            return if hash == envelope.hash && stored == digest {
                Ok(Accepted::Replay)
            } else {
                Err(Fault::Conflict)
            };
        }
        let latest: Option<String> = tx.query_row(
            "SELECT max(seq) FROM checkpoints WHERE page=? AND epoch=? AND stream=? AND namespace=?",
            params![s.page,epoch,s.stream,envelope.namespace.name()], |r| r.get(0))?;
        if latest.is_some_and(|latest| seq <= latest) {
            return Err(Fault::StaleCheckpoint);
        }
        tx.execute("UPDATE receipts SET payload=NULL WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq<=?", params![s.page,epoch,s.stream,envelope.namespace.name(),seq])?;
        tx.execute("UPDATE checkpoints SET payload=NULL WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq<? AND pinned=0",
            params![s.page,epoch,s.stream,envelope.namespace.name(),seq])?;
        capacity(&tx, s.page, envelope.bytes.len(), false)?;
        tx.execute(
            "INSERT INTO checkpoints(page,epoch,stream,namespace,seq,hash,digest,head,payload) VALUES (?,?,?,?,?,?,?,?,?)",
            params![
                s.page,
                epoch,
                s.stream,
                envelope.namespace.name(),
                seq,
                envelope.hash.as_slice(),
                digest,
                envelope.previous.as_slice(),
                envelope.bytes
            ],
        )?;
        tx.commit()?;
        Ok(Accepted::New)
    }
    /// L2b's verified authority-cut caller pins before committing the cut; no HTTP authority.
    pub fn pin_checkpoint(
        &mut self,
        scope: StreamScope<'_>,
        namespace: Namespace,
        seq: u64,
    ) -> StoreResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, scope)?;
        if tx.execute("UPDATE checkpoints SET pinned=1 WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq=? AND payload IS NOT NULL",
            params![scope.page,scope.epoch.to_string(),scope.stream,namespace.name(),sequence(seq)])? != 1 {
            return Err(Fault::Gap);
        }
        tx.commit()?;
        Ok(())
    }
    pub fn payload(&self, scope: StreamScope<'_>, seq: u64) -> StoreResult<Option<Vec<u8>>> {
        current(&self.connection, scope)?;
        Ok(self
            .connection
            .query_row(
                "SELECT payload FROM receipts WHERE page=? AND epoch=? AND stream=? AND seq=?",
                params![
                    scope.page,
                    scope.epoch.to_string(),
                    scope.stream,
                    sequence(seq)
                ],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten())
    }
    pub fn close(self) -> Result<()> {
        self.connection.close().map_err(|(_, e)| e.into())
    }
}
fn sequence(n: u64) -> String {
    format!("{n:020}")
}
fn bounded_id(id: &str) -> StoreResult<()> {
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(Fault::Invalid);
    }
    Ok(())
}
fn validate(e: &Envelope<'_>) -> StoreResult<()> {
    bounded_id(e.scope.page)?;
    bounded_id(e.scope.stream)?;
    if e.scope.epoch == 0
        || e.seq == 0
        || e.bytes.is_empty()
        || e.bytes.len() > limits::OBJECT_BYTES
    {
        return Err(Fault::Invalid);
    }
    Ok(())
}
fn current(c: &Connection, s: StreamScope<'_>) -> StoreResult<()> {
    let epoch: Option<String> = c
        .query_row("SELECT epoch FROM pages WHERE page=?", [s.page], |r| {
            r.get(0)
        })
        .optional()?;
    if epoch.as_deref() != Some(s.epoch.to_string().as_str()) {
        return Err(Fault::StaleEpoch);
    }
    Ok(())
}
fn unfrozen(c: &Connection, s: StreamScope<'_>) -> StoreResult<()> {
    let frozen: Option<bool> = c
        .query_row(
            "SELECT frozen FROM streams WHERE page=? AND epoch=? AND stream=?",
            params![s.page, s.epoch.to_string(), s.stream],
            |r| r.get(0),
        )
        .optional()?;
    if frozen != Some(false) {
        return Err(Fault::Conflict);
    }
    Ok(())
}
fn capacity(c: &Connection, page: &str, added: usize, receipt: bool) -> StoreResult<()> {
    let (bytes, count): (i64, i64) = c.query_row(
        "SELECT COALESCE(sum(length(payload)),0),count(*) FROM receipts WHERE page=?",
        [page],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let checkpoints: i64 = c.query_row(
        "SELECT COALESCE(sum(length(payload)),0) FROM checkpoints WHERE page=?",
        [page],
        |r| r.get(0),
    )?;
    if bytes
        .saturating_add(checkpoints)
        .saturating_add(added as i64)
        > limits::PAGE_BYTES as i64
        || (receipt && count >= limits::PAGE_RECEIPTS as i64)
    {
        return Err(Fault::Capacity);
    }
    Ok(())
}
