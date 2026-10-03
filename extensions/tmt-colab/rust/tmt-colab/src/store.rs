//! Durable opaque envelopes. Callers own signature/session/role admission.
//! Receipt/hash rows survive payload pruning, including interleaved namespaces.
pub mod owner;
mod schema;
use crate::{Result, keyring::Layout, limits};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
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
    ResyncRequired,
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

/// Delivery cursor, scoped separately to a stream and namespace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NamespaceCursor {
    pub seq: u64,
    pub hash: [u8; 32],
}
pub struct ReadObject {
    pub cursor: NamespaceCursor,
    pub checkpoint: bool,
    pub bytes: Vec<u8>,
}

pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(layout: &Layout) -> Result<Self> {
        layout.file("space.db")?.sync_all()?;
        std::fs::File::open(&layout.directory)?.sync_all()?;
        let mut connection = Connection::open_with_flags(
            layout.directory.join("space.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(Duration::from_secs(2))?;
        schema::check_version(&connection)?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        schema::migrate(&mut connection)?;
        Ok(Self { connection })
    }
    /// Existing-state reads never create files, change pragmas or run migrations.
    pub fn read(layout: &Layout) -> Result<Self> {
        Self::existing(layout, false)
    }
    /// Root-local caller holds the lifecycle lock. Never creates or migrates state.
    pub fn write_existing(layout: &Layout) -> Result<Self> {
        Self::existing(layout, true)
    }
    fn existing(layout: &Layout, writable: bool) -> Result<Self> {
        use nix::fcntl::OFlag;
        use std::{
            fs::OpenOptions,
            os::unix::fs::{MetadataExt, OpenOptionsExt},
        };
        let path = layout.directory.join("space.db");
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != nix::unistd::Uid::effective().as_raw()
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(crate::keyring::StateFault::UnsafeFile.into());
        }
        let connection = Connection::open_with_flags(
            &path,
            (if writable {
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            } else {
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            }) | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version = schema::check_version(&connection)?;
        if writable && version != 4 {
            return Err(Fault::UnsupportedSchema(version).into());
        }
        Ok(Self { connection })
    }
    /// Management inspection requires current tables, without migrating legacy state.
    pub fn require_current_schema(&self) -> StoreResult<()> {
        schema::check_read_version(&self.connection)
    }
    pub fn create_page(&self, page: &str) -> StoreResult<()> {
        bounded_id(page)?;
        self.connection
            .execute("INSERT INTO pages VALUES (?,'1')", [page])?;
        Ok(())
    }
    /// Metadata seam for a caller's verified owner transition; no remote route in L2a.
    pub fn advance_epoch(&mut self, page: &str, expected: u64) -> StoreResult<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        advance_epoch(&transaction, page, expected)?;
        transaction.commit()?;
        Ok(())
    }
    pub fn append(&mut self, envelope: &Envelope<'_>) -> StoreResult<Accepted> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = append_in(&tx, envelope);
        if result.is_ok() {
            tx.commit()?;
        }
        result?.ok_or(Fault::Conflict)
    }
    /// Publish an admitted checkpoint; prune only a prefix covered by every namespace.
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
        // Sequence/hash continuity belongs to the shared stream, not a namespace.
        // Receipt rows survive pruning, so this test remains valid after compaction.
        let paired: bool = tx.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM receipts r WHERE r.page=?1 AND r.epoch=?2 AND r.stream=?3 AND r.seq<=?4
             AND NOT EXISTS(SELECT 1 FROM checkpoints c WHERE c.page=r.page AND c.epoch=r.epoch AND c.stream=r.stream
             AND c.namespace=r.namespace AND c.seq=?4 AND c.payload IS NOT NULL))",
            params![s.page, epoch, s.stream, seq], |r| r.get(0))?;
        if paired {
            tx.execute(
                "UPDATE receipts SET payload=NULL WHERE page=? AND epoch=? AND stream=? AND seq<=?",
                params![s.page, epoch, s.stream, seq],
            )?;
            tx.execute("UPDATE checkpoints SET payload=NULL WHERE page=? AND epoch=? AND stream=? AND seq<? AND pinned=0",
                params![s.page,epoch,s.stream,seq])?;
        }
        // Count the inserted payload after any paired reclaim. Failure rolls back
        // publication and pruning together, including the old checkpoint pair.
        capacity(&tx, s.page, 0, false)?;
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
    /// Bounded namespace inventory, including checkpoint-only namespaces. This
    /// read transaction prevents an epoch transition between admission and reads.
    pub fn namespaces(&self, page: &str, epoch: u64) -> StoreResult<Vec<(String, Namespace)>> {
        bounded_id(page)?;
        let tx = self.connection.unchecked_transaction()?;
        current(
            &tx,
            StreamScope {
                page,
                epoch,
                stream: "",
            },
        )?;
        let mut query = tx.prepare(
            "SELECT stream,namespace FROM receipts WHERE page=?1 AND epoch=?2
             UNION SELECT stream,namespace FROM checkpoints WHERE page=?1 AND epoch=?2
             ORDER BY stream,namespace LIMIT ?3",
        )?;
        let rows = query.query_map(
            params![
                page,
                epoch.to_string(),
                (limits::SYNC_NAMESPACES + 1) as i64
            ],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )?;
        let mut result = Vec::new();
        for row in rows {
            let (stream, namespace) = row?;
            bounded_id(&stream)?;
            let namespace = match namespace.as_str() {
                "content" => Namespace::Content,
                "own" => Namespace::Own,
                _ => return Err(Fault::Invalid),
            };
            result.push((stream, namespace));
        }
        if result.len() > limits::SYNC_NAMESPACES {
            return Err(Fault::Capacity);
        }
        Ok(result)
    }
    /// Resolves an exact retained update/checkpoint; an unknown or pruned cursor
    /// cannot silently become the current head. Zero explicitly requests bootstrap.
    pub fn resolve_cursor(
        &self,
        scope: StreamScope<'_>,
        namespace: Namespace,
        cursor: NamespaceCursor,
    ) -> StoreResult<()> {
        let tx = self.connection.unchecked_transaction()?;
        current(&tx, scope)?;
        resolve_cursor(&tx, scope, namespace, cursor)
    }
    /// One object per page, with SQL-side payload length admission before copying.
    /// Bootstrap uses the latest paired prefix checkpoint, then its full namespace tail.
    /// Compaction between pages invalidates the retained cursor instead of skipping data.
    pub fn namespace_next(
        &self,
        scope: StreamScope<'_>,
        namespace: Namespace,
        cursor: NamespaceCursor,
    ) -> StoreResult<Option<ReadObject>> {
        let tx = self.connection.unchecked_transaction()?;
        current(&tx, scope)?;
        resolve_cursor(&tx, scope, namespace, cursor)?;
        let initial: Option<(String, Vec<u8>, Option<i64>)> = if cursor.seq == 0 {
            tx.query_row("SELECT c.seq,c.hash,length(c.payload) FROM checkpoints c
                WHERE c.page=?1 AND c.epoch=?2 AND c.stream=?3 AND c.namespace=?4 AND c.payload IS NOT NULL
                AND NOT EXISTS(SELECT 1 FROM receipts r WHERE r.page=c.page AND r.epoch=c.epoch AND r.stream=c.stream AND r.seq<=c.seq
                    AND NOT EXISTS(SELECT 1 FROM checkpoints p WHERE p.page=c.page AND p.epoch=c.epoch AND p.stream=c.stream
                        AND p.namespace=r.namespace AND p.seq=c.seq AND p.payload IS NOT NULL))
                ORDER BY c.seq DESC LIMIT 1",
                params![scope.page, scope.epoch.to_string(), scope.stream, namespace.name()],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?
        } else {
            None
        };
        let checkpoint = initial.is_some();
        let row = match initial {
            Some(row) => Some(row),
            None => tx.query_row("SELECT seq,hash,length(payload) FROM receipts WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq>? ORDER BY seq LIMIT 1",
                params![scope.page, scope.epoch.to_string(), scope.stream, namespace.name(), sequence(cursor.seq)],
                |r| Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Option<i64>>(2)?))).optional()?,
        };
        let Some((seq, hash, size)) = row else {
            return Ok(None);
        };
        let size = size.ok_or(Fault::ResyncRequired)?;
        if size <= 0 || size > limits::OBJECT_BYTES as i64 {
            return Err(Fault::Capacity);
        }
        let sql = if checkpoint {
            "SELECT payload FROM checkpoints WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq=?"
        } else {
            "SELECT payload FROM receipts WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq=?"
        };
        let bytes = tx.query_row(
            sql,
            params![
                scope.page,
                scope.epoch.to_string(),
                scope.stream,
                namespace.name(),
                seq
            ],
            |r| r.get(0),
        )?;
        Ok(Some(ReadObject {
            cursor: NamespaceCursor {
                seq: seq.parse().map_err(|_| Fault::Invalid)?,
                hash: hash.try_into().map_err(|_| Fault::Invalid)?,
            },
            checkpoint,
            bytes,
        }))
    }
    pub fn close(self) -> Result<()> {
        self.connection.close().map_err(|(_, e)| e.into())
    }
}
fn advance_epoch(connection: &Connection, page: &str, expected: u64) -> StoreResult<()> {
    let next = expected.checked_add(1).ok_or(Fault::Invalid)?;
    if connection.execute(
        "UPDATE pages SET epoch=? WHERE page=? AND epoch=?",
        params![next.to_string(), page, expected.to_string()],
    )? != 1
    {
        return Err(Fault::StaleEpoch);
    }
    Ok(())
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
pub(super) fn capacity(c: &Connection, page: &str, added: usize, receipt: bool) -> StoreResult<()> {
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
    let baselines: i64 = c.query_row(
        "SELECT COALESCE(sum(length(envelope)),0) FROM baselines WHERE page=?",
        [page],
        |r| r.get(0),
    )?;
    if bytes
        .saturating_add(baselines)
        .saturating_add(checkpoints)
        .saturating_add(added as i64)
        > limits::PAGE_BYTES as i64
        || (receipt && count >= limits::PAGE_RECEIPTS as i64)
    {
        return Err(Fault::Capacity);
    }
    Ok(())
}

fn resolve_cursor(
    c: &Connection,
    scope: StreamScope<'_>,
    namespace: Namespace,
    cursor: NamespaceCursor,
) -> StoreResult<()> {
    bounded_id(scope.page)?;
    bounded_id(scope.stream)?;
    if cursor.seq == 0 {
        return if cursor.hash == [0; 32] {
            Ok(())
        } else {
            Err(Fault::Invalid)
        };
    }
    let found: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM receipts WHERE page=?1 AND epoch=?2 AND stream=?3 AND namespace=?4 AND seq=?5 AND hash=?6 AND payload IS NOT NULL
         UNION ALL SELECT 1 FROM checkpoints WHERE page=?1 AND epoch=?2 AND stream=?3 AND namespace=?4 AND seq=?5 AND hash=?6 AND payload IS NOT NULL)",
        params![scope.page, scope.epoch.to_string(), scope.stream, namespace.name(), sequence(cursor.seq), cursor.hash.as_slice()], |r| r.get(0))?;
    if found {
        Ok(())
    } else {
        Err(Fault::ResyncRequired)
    }
}

/// Shared create-only append; its caller owns commit/rollback and authority fencing.
fn append_in(tx: &Connection, envelope: &Envelope<'_>) -> StoreResult<Option<Accepted>> {
    validate(envelope)?;
    let s = envelope.scope;
    let epoch = s.epoch.to_string();
    let seq = sequence(envelope.seq);
    let digest = Sha256::digest(envelope.bytes).to_vec();
    current(tx, s)?;
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
            return Ok(Some(Accepted::Replay));
        }
        tx.execute(
            "UPDATE streams SET frozen=1 WHERE page=? AND epoch=? AND stream=?",
            params![s.page, epoch, s.stream],
        )?;
        return Ok(None);
    }
    unfrozen(tx, s)?;
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
    capacity(tx, s.page, envelope.bytes.len(), true)?;
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
    Ok(Some(Accepted::New))
}
