//! Durable opaque envelopes. Callers own signature/session/role admission.
//! Receipt/hash rows survive payload pruning, including interleaved namespaces.
mod edits;
pub use edits::{EDIT_DECISIONS, EditState};
pub mod owner;
pub mod publication;
mod schema;
#[cfg(test)]
mod tests;
/// The schema version this build writes. Migration tests read it instead of naming the latest.
pub const SCHEMA_VERSION: u32 = schema::CURRENT_VERSION;
use crate::{Result, keyring::Layout, limits};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
pub use schema::backup_path;
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Namespace {
    Content,
    Own,
}
impl Namespace {
    pub(crate) fn name(self) -> &'static str {
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
    OutdatedSchema(u32),
    /// The store's schema, and the newest pre-update copy this build can open when one exists.
    UnsupportedSchema(u32, Option<std::path::PathBuf>),
    /// The copy taken before a migration could not be written; nothing was migrated.
    Backup(std::io::Error),
    Gap,
    Conflict,
    Capacity,
    Sql(rusqlite::Error),
    /// Preserve clock provenance so domain admission never treats it as rejection.
    Clock(Box<Fault>),
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutdatedSchema(_) => {
                f.write_str("This space was saved by an older Colab service.")
            }
            Self::UnsupportedSchema(_, None) => {
                f.write_str("This space was saved by a newer Colab.")
            }
            Self::UnsupportedSchema(_, Some(copy)) => write!(
                f,
                "This space was saved by a newer Colab. Run tmt upgrade to open it. To stay on this version instead, stop Colab and replace space.db with the backup from before that update ({}); changes made since then are lost.",
                copy.display()
            ),
            Self::Backup(_) => f.write_str(
                "Colab could not save a backup of this space before updating it. Nothing was changed.",
            ),
            _ => write!(f, "{self:?}"),
        }
    }
}
impl Fault {
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::OutdatedSchema(_) => Some("COLAB_STORE_OUTDATED"),
            Self::UnsupportedSchema(..) => Some("COLAB_STORE_NEWER"),
            Self::Backup(_) => Some("COLAB_STORE_BACKUP_FAILED"),
            _ => None,
        }
    }
    pub fn next(&self) -> Option<&'static [&'static str]> {
        match self {
            Self::OutdatedSchema(_) => Some(&["tmt colab stop", "tmt colab serve --background"]),
            Self::UnsupportedSchema(..) => Some(&["tmt upgrade"]),
            _ => None,
        }
    }
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Self::OutdatedSchema(_) => {
                Some("Run tmt colab stop, then tmt colab serve --background.")
            }
            Self::UnsupportedSchema(..) => Some("Run tmt upgrade, then try again."),
            Self::Backup(_) => Some(
                "Free disk space or fix the permissions of the Colab data folder, then try again.",
            ),
            _ => None,
        }
    }
    pub fn schema_versions(&self) -> Option<(u32, u32)> {
        match self {
            Self::OutdatedSchema(version) | Self::UnsupportedSchema(version, _) => {
                Some((*version, schema::CURRENT_VERSION))
            }
            _ => None,
        }
    }
}
impl std::error::Error for Fault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Clock(error) => Some(error.as_ref()),
            Self::Sql(error) => Some(error),
            Self::Backup(error) => Some(error),
            _ => None,
        }
    }
}
/// Whether `error` or one of its causes is SQLite reporting the database busy or locked: a store
/// that could not be read, which says nothing about the request that needed it (#2956).
pub(crate) fn contended(error: &(dyn std::error::Error + 'static)) -> bool {
    std::iter::successors(Some(error), |error| error.source()).any(|error| {
        matches!(
            error.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(failure, _))
                if matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
        )
    })
}
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
/// The update envelope an owner fold binds, kept past the prune of the prefix it covers.
pub struct Anchor {
    pub namespace: Namespace,
    pub hash: [u8; 32],
    pub bytes: Vec<u8>,
}
/// One checkpoint (or owner fold, with the device that signed it) of a [`Store::bootstrap`].
pub struct Bootstrapped<'a> {
    pub envelope: Envelope<'a>,
    pub signer: Option<&'a str>,
}
pub struct ReadObject {
    pub cursor: NamespaceCursor,
    pub checkpoint: bool,
    pub bytes: Vec<u8>,
}

/// How long a connection waits for another one before the store refuses with busy: the shared
/// state wait bound (`tmt_extension_state`'s five-second write wait), for a reader and a writer alike.
pub const BUSY: Duration = Duration::from_secs(5);

type Clock = dyn Fn() -> StoreResult<u64> + Send + Sync;
fn now_ms() -> StoreResult<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Fault::Invalid)?
        .as_millis();
    let now = u64::try_from(now).map_err(|_| Fault::Invalid)?;
    tmt_colab_model::values::time(now).map_err(|_| Fault::Invalid)?;
    Ok(now)
}
pub struct Store {
    connection: Connection,
    clock: Arc<Clock>,
}
impl Store {
    pub fn open(layout: &Layout) -> Result<Self> {
        layout.file("space.db")?.sync_all()?;
        std::fs::File::open(&layout.directory)?.sync_all()?;
        let mut connection = Connection::open_with_flags(
            layout.directory.join("space.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(BUSY)?;
        schema::check_version(&connection)?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        schema::migrate(&mut connection)?;
        Ok(Self {
            connection,
            clock: Arc::new(now_ms),
        })
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
        connection.busy_timeout(BUSY)?;
        schema::check_version(&connection)?;
        if writable {
            schema::check_read_version(&connection)?;
        }
        Ok(Self {
            connection,
            clock: Arc::new(now_ms),
        })
    }
    /// Trusted server clock; injected by tests, never selected by a request.
    pub fn with_clock(
        mut self,
        clock: impl Fn() -> StoreResult<u64> + Send + Sync + 'static,
    ) -> Self {
        self.clock = Arc::new(clock);
        self
    }
    /// Management inspection requires current tables, without migrating legacy state.
    pub fn require_current_schema(&self) -> StoreResult<()> {
        schema::check_read_version(&self.connection)
    }
    pub fn create_page(&self, page: &str) -> StoreResult<()> {
        bounded_id(page)?;
        self.connection
            .execute("INSERT INTO pages(page,epoch) VALUES (?,'1')", [page])?;
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
        self.append_with(envelope, Equivocation::Freeze)
    }
    /// `append` for a stream that every holder of one link writes with the same key: a different
    /// original at a sequence already taken is a lost race, not a fork. It is refused as
    /// `Conflict` and nothing is written, so the stream stays open to the next holder.
    pub fn append_shared(&mut self, envelope: &Envelope<'_>) -> StoreResult<Accepted> {
        self.append_with(envelope, Equivocation::Refuse)
    }
    fn append_with(
        &mut self,
        envelope: &Envelope<'_>,
        equivocation: Equivocation,
    ) -> StoreResult<Accepted> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = append_in(&tx, envelope, self.clock.as_ref(), equivocation);
        if result.is_ok() {
            tx.commit()?;
        }
        result?.ok_or(Fault::Conflict)
    }
    /// Publish an admitted checkpoint; prune only a prefix covered by every namespace.
    pub fn checkpoint(&mut self, envelope: &Envelope<'_>) -> StoreResult<Accepted> {
        self.checkpoint_by(envelope, None)
    }
    /// Publish an admitted owner fold (colab-v1, "Owner fold") that `signer` signed for the
    /// stream the envelope names. It is stored as that stream's checkpoint; the signer is kept so
    /// that a prune keeps the update the fold binds and a revoke of the signer can list the fold.
    pub fn fold(&mut self, envelope: &Envelope<'_>, signer: &str) -> StoreResult<Accepted> {
        bounded_id(signer)?;
        self.checkpoint_by(envelope, Some(signer))
    }
    fn checkpoint_by(
        &mut self,
        envelope: &Envelope<'_>,
        signer: Option<&str>,
    ) -> StoreResult<Accepted> {
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
            "INSERT INTO checkpoints(page,epoch,stream,namespace,seq,hash,digest,head,payload,signer) VALUES (?,?,?,?,?,?,?,?,?,?)",
            params![
                s.page,
                epoch,
                s.stream,
                envelope.namespace.name(),
                seq,
                envelope.hash.as_slice(),
                digest,
                envelope.previous.as_slice(),
                envelope.bytes,
                signer
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
            publication::prune(&tx, s, &seq)?;
        }
        // Count the inserted payload after any paired reclaim. Failure rolls back
        // publication and pruning together, including the old checkpoint pair.
        capacity(&tx, s.page, 0, false)?;
        tx.commit()?;
        Ok(Accepted::New)
    }
    /// Install a verified checkpoint pair (or owner fold pair) for a stream this machine holds
    /// nothing of, so its tail can follow from the sequence after it. A fold brings the update it
    /// binds, which is stored as the receipt at that sequence and kept un-pruned like a local fold's
    /// anchor; an ordinary checkpoint has no such update to bring and leaves no receipt.
    ///
    /// One transaction. The stream must hold no receipt and no checkpoint: the same objects again
    /// are a `Replay`, anything else is a `Conflict` and nothing is written, so a held prefix is
    /// never merged with a pruned one. Callers own signature, role and authority admission.
    pub fn bootstrap(
        &mut self,
        anchor: Option<&Envelope<'_>>,
        objects: &[Bootstrapped<'_>],
    ) -> StoreResult<Accepted> {
        let first = &objects.first().ok_or(Fault::Invalid)?.envelope;
        let s = first.scope;
        if objects.len() > 2 {
            return Err(Fault::Invalid);
        }
        for (index, object) in objects.iter().enumerate() {
            let e = &object.envelope;
            validate(e)?;
            if e.scope.page != s.page
                || e.scope.epoch != s.epoch
                || e.scope.stream != s.stream
                || e.seq != first.seq
                || e.previous != first.previous
                || objects[..index]
                    .iter()
                    .any(|other| other.envelope.namespace == e.namespace)
            {
                return Err(Fault::Invalid);
            }
            if let Some(signer) = object.signer {
                bounded_id(signer)?;
            }
        }
        if let Some(anchor) = anchor {
            validate(anchor)?;
            if anchor.scope.page != s.page
                || anchor.scope.epoch != s.epoch
                || anchor.scope.stream != s.stream
                || anchor.seq != first.seq
                || anchor.hash != first.previous
            {
                return Err(Fault::Invalid);
            }
        }
        let epoch = s.epoch.to_string();
        let seq = sequence(first.seq);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, s)?;
        tx.execute(
            "INSERT OR IGNORE INTO streams(page,epoch,stream) VALUES (?,?,?)",
            params![s.page, epoch, s.stream],
        )?;
        let mut held = true;
        for object in objects {
            let e = &object.envelope;
            let row: Option<(Vec<u8>, Vec<u8>)> = tx
                .query_row(
                    "SELECT hash,digest FROM checkpoints WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq=?",
                    params![s.page, epoch, s.stream, e.namespace.name(), seq],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            held &= row.is_some_and(|(hash, digest)| {
                hash == e.hash && digest == Sha256::digest(e.bytes).to_vec()
            });
        }
        if let Some(anchor) = anchor {
            let row: Option<(Vec<u8>, Vec<u8>, String)> = tx
                .query_row(
                    "SELECT hash,digest,namespace FROM receipts WHERE page=? AND epoch=? AND stream=? AND seq=?",
                    params![s.page, epoch, s.stream, seq],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            held &= row.is_some_and(|(hash, digest, namespace)| {
                hash == anchor.hash
                    && digest == Sha256::digest(anchor.bytes).to_vec()
                    && namespace == anchor.namespace.name()
            });
        }
        if held {
            return Ok(Accepted::Replay);
        }
        unfrozen(&tx, s)?;
        if position(&tx, s)?.is_some() {
            return Err(Fault::Conflict);
        }
        if let Some(anchor) = anchor {
            tx.execute(
                "INSERT INTO receipts VALUES (?,?,?,?,?,?,?,?)",
                params![
                    s.page,
                    epoch,
                    s.stream,
                    seq,
                    anchor.namespace.name(),
                    anchor.hash.as_slice(),
                    Sha256::digest(anchor.bytes).to_vec(),
                    anchor.bytes
                ],
            )?;
        }
        for object in objects {
            let e = &object.envelope;
            tx.execute(
                "INSERT INTO checkpoints(page,epoch,stream,namespace,seq,hash,digest,head,payload,signer) VALUES (?,?,?,?,?,?,?,?,?,?)",
                params![
                    s.page,
                    epoch,
                    s.stream,
                    e.namespace.name(),
                    seq,
                    e.hash.as_slice(),
                    Sha256::digest(e.bytes).to_vec(),
                    e.previous.as_slice(),
                    e.bytes,
                    object.signer
                ],
            )?;
        }
        // The installed originals are page content that arrived: the page is not idle.
        let now = touch_activity(&tx, s.page, self.clock.as_ref())?;
        if objects
            .iter()
            .any(|object| object.envelope.namespace == Namespace::Content)
        {
            tx.execute("UPDATE pages SET last_update_at_ms=CASE WHEN last_update_at_ms IS NULL OR last_update_at_ms<?1 THEN ?1 ELSE last_update_at_ms END WHERE page=?2",
                params![now as i64, s.page])?;
        }
        // Count the inserted payloads; a failure rolls back every row above.
        capacity(&tx, s.page, 0, anchor.is_some())?;
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
    /// The retained update envelope at `seq` of a stream, in whichever namespace it is: the anchor
    /// an owner fold of the stream at `seq` is checked against. `None` once it is pruned.
    pub fn anchor(&self, scope: StreamScope<'_>, seq: u64) -> StoreResult<Option<Anchor>> {
        current(&self.connection, scope)?;
        read_anchor(&self.connection, scope, seq)
    }
    /// [`Self::anchor`] for a historical epoch, which only detached history reads.
    pub(crate) fn retained_anchor(
        &self,
        scope: StreamScope<'_>,
        seq: u64,
    ) -> StoreResult<Option<Anchor>> {
        read_epoch(&self.connection, scope, true)?;
        read_anchor(&self.connection, scope, seq)
    }
    /// The position of a stream (see [`position`]); a stream that has never been stored has none.
    pub fn stream_head(&self, scope: StreamScope<'_>) -> StoreResult<Option<(u64, [u8; 32])>> {
        current(&self.connection, scope)?;
        position(&self.connection, scope)
    }
    /// When the page last saw an admitted content/own-stream update or owner restore, by this
    /// machine's clock at admission. A page with no activity evidence, or no such page, is `None`
    /// (unknown, never old): the later of the activity time and the older content-edit time, whichever exist.
    pub fn last_activity_ms(&self, page: &str) -> StoreResult<Option<u64>> {
        bounded_id(page)?;
        let times: Option<(Option<i64>, Option<i64>)> = self
            .connection
            .query_row(
                "SELECT last_activity_at_ms,last_update_at_ms FROM pages WHERE page=?",
                params![page],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (activity, update) = times.unwrap_or_default();
        activity
            .max(update)
            .map(|ms| u64::try_from(ms).map_err(|_| Fault::Invalid))
            .transpose()
    }
    /// Bounded namespace inventory, including checkpoint-only namespaces. This
    /// read transaction prevents an epoch transition between admission and reads.
    pub fn namespaces(&self, page: &str, epoch: u64) -> StoreResult<Vec<(String, Namespace)>> {
        self.namespace_inventory(page, epoch, false)
    }
    /// Only detached history uses retained paging, after current caller admission.
    pub(crate) fn retained_namespaces(
        &self,
        page: &str,
        epoch: u64,
    ) -> StoreResult<Vec<(String, Namespace)>> {
        self.namespace_inventory(page, epoch, true)
    }
    fn namespace_inventory(
        &self,
        page: &str,
        epoch: u64,
        historical: bool,
    ) -> StoreResult<Vec<(String, Namespace)>> {
        bounded_id(page)?;
        let tx = self.connection.unchecked_transaction()?;
        read_epoch(
            &tx,
            StreamScope {
                page,
                epoch,
                stream: "",
            },
            historical,
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
        resolve_cursor(&tx, scope, namespace, cursor, true)
    }
    /// ACK grants frame credit, not bootstrap authority. Compaction may prune
    /// admitted bytes, but their exact receipt/checkpoint identity remains.
    pub fn resolve_ack_cursor(
        &self,
        scope: StreamScope<'_>,
        namespace: Namespace,
        cursor: NamespaceCursor,
    ) -> StoreResult<()> {
        let tx = self.connection.unchecked_transaction()?;
        current(&tx, scope)?;
        resolve_cursor(&tx, scope, namespace, cursor, false)
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
        self.namespace_object(scope, namespace, cursor, false)
    }
    pub(crate) fn retained_namespace_next(
        &self,
        scope: StreamScope<'_>,
        namespace: Namespace,
        cursor: NamespaceCursor,
    ) -> StoreResult<Option<ReadObject>> {
        self.namespace_object(scope, namespace, cursor, true)
    }
    fn namespace_object(
        &self,
        scope: StreamScope<'_>,
        namespace: Namespace,
        cursor: NamespaceCursor,
        historical: bool,
    ) -> StoreResult<Option<ReadObject>> {
        let tx = self.connection.unchecked_transaction()?;
        read_epoch(&tx, scope, historical)?;
        resolve_cursor(&tx, scope, namespace, cursor, true)?;
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
fn read_anchor(c: &Connection, scope: StreamScope<'_>, seq: u64) -> StoreResult<Option<Anchor>> {
    let row: Option<(String, Vec<u8>, Option<Vec<u8>>)> = c
        .query_row(
            "SELECT namespace,hash,payload FROM receipts WHERE page=? AND epoch=? AND stream=? AND seq=?",
            params![scope.page, scope.epoch.to_string(), scope.stream, sequence(seq)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((namespace, hash, Some(bytes))) = row else {
        return Ok(None);
    };
    Ok(Some(Anchor {
        namespace: match namespace.as_str() {
            "content" => Namespace::Content,
            "own" => Namespace::Own,
            _ => return Err(Fault::Invalid),
        },
        hash: hash.try_into().map_err(|_| Fault::Invalid)?,
        bytes,
    }))
}
/// The one definition of a stream's position: its latest sequence and the hash it has there
/// (colab-v1, "Stream position"). A stream built by ordinary appends and checkpoints has a receipt
/// at every sequence it holds, and a checkpoint needs the receipt at its own, so the latest
/// receipt decides. Only a stream installed by [`Store::bootstrap`] can hold a checkpoint past
/// its receipts; then that checkpoint's head is the position. Callers never read the receipts
/// table for a position.
fn position(c: &Connection, s: StreamScope<'_>) -> StoreResult<Option<(u64, [u8; 32])>> {
    let seq: Option<String> = c
        .query_row(
            "SELECT seq FROM (SELECT seq FROM receipts WHERE page=?1 AND epoch=?2 AND stream=?3
             UNION ALL SELECT seq FROM checkpoints WHERE page=?1 AND epoch=?2 AND stream=?3)
             ORDER BY seq DESC LIMIT 1",
            params![s.page, s.epoch.to_string(), s.stream],
            |r| r.get(0),
        )
        .optional()?;
    let Some(seq) = seq else {
        return Ok(None);
    };
    let seq: u64 = seq.parse().map_err(|_| Fault::Invalid)?;
    let hash = hash_at(c, s.page, &s.epoch.to_string(), s.stream, seq)?.ok_or(Fault::Invalid)?;
    Ok(Some((seq, hash)))
}
/// The hash a stream has at `seq`: the update's receipt, or, where a bootstrapped stream holds no
/// receipt, the head a checkpoint at `seq` binds. Receipt rows survive pruning of their payload.
pub(crate) fn hash_at(
    c: &Connection,
    page: &str,
    epoch: &str,
    stream: &str,
    seq: u64,
) -> StoreResult<Option<[u8; 32]>> {
    let hash: Option<Vec<u8>> = c
        .query_row(
            "SELECT hash FROM receipts WHERE page=?1 AND epoch=?2 AND stream=?3 AND seq=?4
             UNION ALL SELECT head FROM checkpoints WHERE page=?1 AND epoch=?2 AND stream=?3 AND seq=?4
             LIMIT 1",
            params![page, epoch, stream, sequence(seq)],
            |r| r.get(0),
        )
        .optional()?;
    hash.map(|hash| hash.try_into().map_err(|_| Fault::Invalid))
        .transpose()
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
fn read_epoch(c: &Connection, s: StreamScope<'_>, historical: bool) -> StoreResult<()> {
    if !historical {
        return current(c, s);
    }
    let epoch: Option<String> = c
        .query_row("SELECT epoch FROM pages WHERE page=?", [s.page], |r| {
            r.get(0)
        })
        .optional()?;
    let current = epoch
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(Fault::StaleEpoch)?;
    if s.epoch == 0 || s.epoch >= current || current - s.epoch >= 64 {
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
    admit_page(c, page, added, usize::from(receipt))
}

/// Shared page budget: retained identities survive pruning and epoch changes.
pub(super) fn admit_page(
    c: &Connection,
    page: &str,
    added: usize,
    identities: usize,
) -> StoreResult<()> {
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
    let (outcomes, operations): (i64, i64) = c.query_row(
        "SELECT COALESCE(sum(length(outcome)+length(digest)+length(CAST(id AS BLOB))+
         length(CAST(publication_kind AS BLOB))+length(CAST(space AS BLOB))+length(CAST(page AS BLOB))+
         length(CAST(original_epoch AS BLOB))+length(CAST(original_stream AS BLOB))),0),count(*)
         FROM owner_operations WHERE page=? AND publication_kind IS NOT NULL",
        [page], |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let publication: i64 = c.query_row(
        "SELECT COALESCE(sum(length(binding)+length(CAST(uid AS BLOB))+
         length(CAST(epoch AS BLOB))+length(CAST(stream AS BLOB))+
         length(CAST(bootstrap_seq AS BLOB))+length(CAST(confirmed_seq AS BLOB))),0)
         FROM cloud_publications WHERE page=?",
        [page],
        |r| r.get(0),
    )?;
    let total = [bytes, checkpoints, baselines, outcomes, publication]
        .into_iter()
        .try_fold(0usize, |n, v| {
            n.checked_add(usize::try_from(v).map_err(|_| Fault::Invalid)?)
                .ok_or(Fault::Capacity)
        })?;
    let count = usize::try_from(count)
        .map_err(|_| Fault::Invalid)?
        .checked_add(usize::try_from(operations).map_err(|_| Fault::Invalid)?)
        .ok_or(Fault::Capacity)?;
    if total.checked_add(added).ok_or(Fault::Capacity)? > limits::PAGE_BYTES
        || count.checked_add(identities).ok_or(Fault::Capacity)? > limits::PAGE_RECEIPTS
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
    require_payload: bool,
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
        "SELECT EXISTS(SELECT 1 FROM receipts WHERE page=?1 AND epoch=?2 AND stream=?3 AND namespace=?4 AND seq=?5 AND hash=?6 AND (NOT ?7 OR payload IS NOT NULL)
         UNION ALL SELECT 1 FROM checkpoints WHERE page=?1 AND epoch=?2 AND stream=?3 AND namespace=?4 AND seq=?5 AND hash=?6 AND (NOT ?7 OR payload IS NOT NULL))",
        params![scope.page, scope.epoch.to_string(), scope.stream, namespace.name(), sequence(cursor.seq), cursor.hash.as_slice(), require_payload], |r| r.get(0))?;
    if found {
        Ok(())
    } else {
        Err(Fault::ResyncRequired)
    }
}

/// Shared create-only append; its caller owns commit/rollback and authority fencing.
/// What a different original at an already-taken sequence does to its stream.
#[derive(Clone, Copy)]
enum Equivocation {
    /// The stream is frozen: its author signed two originals for one sequence.
    Freeze,
    /// The original is refused and the stream is untouched.
    Refuse,
}
fn append_in(
    tx: &Connection,
    envelope: &Envelope<'_>,
    clock: &Clock,
    equivocation: Equivocation,
) -> StoreResult<Option<Accepted>> {
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
        if matches!(equivocation, Equivocation::Refuse) {
            return Err(Fault::Conflict);
        }
        tx.execute(
            "UPDATE streams SET frozen=1 WHERE page=? AND epoch=? AND stream=?",
            params![s.page, epoch, s.stream],
        )?;
        return Ok(None);
    }
    unfrozen(tx, s)?;
    let (expected, previous) = match position(tx, s)? {
        Some((seq, hash)) => (seq.checked_add(1).ok_or(Fault::Capacity)?, hash.to_vec()),
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
    // Every new update counts, whichever namespace it is in: the store only sees sealed bytes,
    // so it cannot tell a comment or reply from an Ask record in the `own` stream.
    let now = touch_activity(tx, s.page, clock)?;
    if envelope.namespace == Namespace::Content {
        tx.execute("UPDATE pages SET last_update_at_ms=CASE WHEN last_update_at_ms IS NULL OR last_update_at_ms<?1 THEN ?1 ELSE last_update_at_ms END WHERE page=?2",
            params![now as i64, s.page])?;
    }
    Ok(Some(Accepted::New))
}

/// Record admitted activity atomically, never on an exact replay; content-edit time is separate.
fn touch_activity(tx: &Connection, page: &str, clock: &Clock) -> StoreResult<u64> {
    let now = clock().map_err(|e| Fault::Clock(Box::new(e)))?;
    tmt_colab_model::values::time(now).map_err(|_| Fault::Invalid)?;
    let changed = tx.execute("UPDATE pages SET last_activity_at_ms=CASE WHEN last_activity_at_ms IS NULL OR last_activity_at_ms<?1 THEN ?1 ELSE last_activity_at_ms END WHERE page=?2",
        params![now as i64, page])?;
    if changed != 1 {
        return Err(Fault::Invalid);
    }
    Ok(now)
}
