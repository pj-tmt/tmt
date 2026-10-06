//! The local filesystem object backend. `LocalFs` owns the installation ledger,
//! the in-flight bookkeeping and every extension's private payload tree
//! `<dataRoot>/<extension>/objects/`; it hands out extension-bound
//! [`LocalHandle`]s that all share that one ledger. It borrows the `Serving`
//! proof, so neither it nor a handle can outlive the serve lease.
//!
//! Ordering: the ledger commits an adoption before any byte; a part is
//! acknowledged only after its bytes and checkpoint are synced; a publication is
//! acknowledged only after the create-only link and directory are synced and the
//! receipt row is durable. Locks: the flight mutex and the ledger mutex are
//! never held together, and neither is held across file I/O.
use super::{
    BackendCaps, BackendError, BackendResult, BeginResult, BeginSpec, BlobKey, Clock, Digest,
    ExtensionId, IntentId, IoBudget, NamespaceId, ObjectBackend, OriginalIntent, Progress, Quotas,
    ReadPart, Receipt, Transfer, TransferState, Usage, UsageScope,
    ledger::{Adoption, Gate, Ledger, Phase, Row},
    tree::{self, Dir, TreeError, hex},
};
use crate::{error::RemoteError, limits, state::Serving, store::database};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::PathBuf,
    sync::{Condvar, Mutex, MutexGuard},
    time::{Duration, Instant},
};

const CHUNK: u64 = limits::OBJECT_CHUNK_BYTES as u64;
/// Waiting on another holder re-checks cancellation at least this often.
const SLICE: Duration = Duration::from_millis(25);

/// Named points inside the real algorithms; tests observe them to stop, block or
/// coordinate at an exact durable boundary. Production builds compile them away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Milestone {
    Adopted,
    ChunkSynced,
    ChunkAcked,
    CommitVerified,
    CommitAdopted,
    Linked,
    LinkSynced,
    Receipted,
    StagingUnlinked,
    DiscardAdopted,
    CloseUnlinked,
    Fenced,
    RemovalMarked,
    RemovalUnlinked,
    ReadBytes,
    /// Every successful return, just before its final budget check.
    Return,
}
#[cfg(test)]
pub(crate) type Observer = std::sync::Arc<dyn Fn(Milestone) -> bool + Send + Sync>;

struct Inner {
    data_root: PathBuf,
    ledger: Ledger,
    flight: Flight,
    quotas: Quotas,
    clock: Clock,
    #[cfg(test)]
    observer: Mutex<Option<Observer>>,
}

/// The installation coordinator, bound to the held serve lease for its lifetime:
/// the lease cannot be released while the coordinator or any handle exists.
///
/// ```compile_fail,E0505
/// use std::{sync::atomic::AtomicBool, time::Instant};
/// use tmt_remote::{objects::*, state::Layout};
/// fn escape(root: &std::path::Path) {
///     let serving = Layout::open(root).unwrap().serve_lock().unwrap();
///     let cancelled = AtomicBool::new(false);
///     let io = IoBudget { deadline: Instant::now(), cancelled: &cancelled };
///     let local = LocalFs::open(&serving, Quotas::contract(), system_clock(), &io).unwrap();
///     let handle = local.handle(ExtensionId::new("alpha").unwrap());
///     drop(serving);
///     let _ = handle.id();
/// }
/// ```
pub struct LocalFs<'s> {
    _lease: &'s Serving,
    inner: Inner,
}
impl<'s> LocalFs<'s> {
    /// Open the ledger and settle every interrupted original under the lease
    /// before readiness. Damaged or unsafe state refuses; it is never reset.
    pub fn open(
        serving: &'s Serving,
        quotas: Quotas,
        clock: Clock,
        io: &IoBudget<'_>,
    ) -> Result<Self, RemoteError> {
        // A spent budget creates and changes nothing, including the ledger itself.
        io.check().map_err(unavailable)?;
        let ledger = Ledger::open(serving)?;
        let data_root = serving
            .layout()
            .directory
            .parent()
            .ok_or_else(|| database("missing Remote data root"))?
            .to_owned();
        let local = Self {
            _lease: serving,
            inner: Inner {
                data_root,
                ledger,
                flight: Flight::default(),
                quotas,
                clock,
                #[cfg(test)]
                observer: Mutex::new(None),
            },
        };
        local.reconcile(io).map_err(unavailable)?;
        if !local.inner.ledger.settled(io).map_err(unavailable)? {
            return Err(unavailable(BackendError::Unavailable));
        }
        io.check().map_err(unavailable)?;
        Ok(local)
    }
    /// The backend of one trusted installed extension; the name grants nothing.
    pub fn handle(&self, extension: ExtensionId) -> LocalHandle<'_> {
        LocalHandle {
            inner: &self.inner,
            extension,
        }
    }
    pub fn installation_usage(&self, io: &IoBudget<'_>) -> BackendResult<Usage> {
        let usage = io
            .check()
            .and_then(|()| self.inner.ledger.installation_usage(io));
        self.inner.finish(io, usage)
    }
    /// Settle interrupted originals by their original IDs: trim unacknowledged
    /// tails, complete or refuse a possible publication, finish closes and
    /// removals. Not a timer: the owning service decides when to call it.
    pub fn reconcile(&self, io: &IoBudget<'_>) -> BackendResult<()> {
        io.check()?;
        let unsettled = self.inner.ledger.unsettled(io)?;
        for row in unsettled.rows {
            io.check()?;
            self.inner.settle(&row, io)?;
        }
        for (extension, namespace) in unsettled.namespaces {
            io.check()?;
            self.inner.finish_removal(&extension, &namespace, io)?;
        }
        io.check()
    }
    #[cfg(test)]
    pub(crate) fn observe(&self, observer: Observer) {
        *self.inner.observer.lock().unwrap() = Some(observer);
    }
}
fn unavailable(error: BackendError) -> RemoteError {
    RemoteError::new(
        "REMOTE_OBJECTS_UNAVAILABLE",
        &format!("Remote object state could not be settled: {error:?}."),
    )
}

/// One extension's view of the shared coordinator.
pub struct LocalHandle<'a> {
    inner: &'a Inner,
    extension: ExtensionId,
}

fn fs(_: TreeError) -> BackendError {
    BackendError::Unavailable
}
fn progress(row: &Row) -> Progress {
    Progress {
        intent: row.spec.intent,
        next_index: row.next_index,
        received: row.received,
    }
}
fn receipt(row: &Row) -> Receipt {
    Receipt {
        intent: row.spec.intent,
        key: row.spec.key,
        payload_sha256: row.spec.payload_sha256,
        payload_bytes: row.spec.payload_bytes,
        binding: row.spec.binding.clone(),
    }
}
fn original(row: &Row) -> OriginalIntent {
    OriginalIntent {
        spec: row.spec.clone(),
        adopted_at_ms: row.adopted_ms,
        expires_at_ms: row.expires_ms,
    }
}
/// Offset and length of a canonical part, or `None` past the payload.
fn part(index: u32, total: u64) -> Option<(u64, u64)> {
    let offset = u64::from(index) * CHUNK;
    (offset < total).then(|| (offset, CHUNK.min(total - offset)))
}

/// In-flight bookkeeping: one holder per intent and a count of holders per
/// namespace, so a namespace removal can drain exactly the work it fenced.
#[derive(Default)]
struct Flight {
    state: Mutex<FlightState>,
    changed: Condvar,
}
#[derive(Default)]
struct FlightState {
    intents: HashSet<(String, [u8; 32])>,
    namespaces: HashMap<(String, [u8; 32]), usize>,
}
struct Hold<'a> {
    flight: &'a Flight,
    extension: String,
    namespace: [u8; 32],
    intent: Option<[u8; 32]>,
}
impl Drop for Hold<'_> {
    fn drop(&mut self) {
        let mut state = match self.flight.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(intent) = self.intent {
            state.intents.remove(&(self.extension.clone(), intent));
        }
        let key = (self.extension.clone(), self.namespace);
        if let Some(count) = state.namespaces.get_mut(&key) {
            *count -= 1;
            if *count == 0 {
                state.namespaces.remove(&key);
            }
        }
        self.flight.changed.notify_all();
    }
}
impl Flight {
    fn lock(&self) -> BackendResult<MutexGuard<'_, FlightState>> {
        self.state.lock().map_err(|_| BackendError::Unavailable)
    }
    /// Register as a holder of the namespace, and of the intent when given;
    /// waiting for another intent holder is bounded by the budget.
    fn hold(
        &self,
        extension: &str,
        namespace: &NamespaceId,
        intent: Option<&IntentId>,
        io: &IoBudget<'_>,
    ) -> BackendResult<Hold<'_>> {
        let mut state = self.lock()?;
        let intent_key = intent.map(|intent| (extension.to_owned(), intent.0));
        while intent_key
            .as_ref()
            .is_some_and(|key| state.intents.contains(key))
        {
            io.check()?;
            let wait = io
                .deadline
                .saturating_duration_since(Instant::now())
                .min(SLICE);
            state = self
                .changed
                .wait_timeout(state, wait)
                .map_err(|_| BackendError::Unavailable)?
                .0;
        }
        if let Some(key) = intent_key {
            state.intents.insert(key);
        }
        *state
            .namespaces
            .entry((extension.to_owned(), namespace.0))
            .or_default() += 1;
        Ok(Hold {
            flight: self,
            extension: extension.to_owned(),
            namespace: namespace.0,
            intent: intent.map(|intent| intent.0),
        })
    }
    /// Wait until no holder remains in the namespace. The fence is already
    /// durable, so every later holder is refused by the ledger.
    fn drain(
        &self,
        extension: &str,
        namespace: &NamespaceId,
        io: &IoBudget<'_>,
    ) -> BackendResult<()> {
        let key = (extension.to_owned(), namespace.0);
        let mut state = self.lock()?;
        while state.namespaces.contains_key(&key) {
            io.check()?;
            let wait = io
                .deadline
                .saturating_duration_since(Instant::now())
                .min(SLICE);
            state = self
                .changed
                .wait_timeout(state, wait)
                .map_err(|_| BackendError::Unavailable)?
                .0;
        }
        Ok(())
    }
}

/// One extension's payload tree: `<dataRoot>/<extension>/objects/{staging,blobs}`.
struct Tree {
    objects: Dir,
}
impl Tree {
    fn staging(&self, create: bool) -> BackendResult<Option<Dir>> {
        self.objects.child("staging", create).map_err(fs)
    }
    fn blobs(&self, create: bool) -> BackendResult<Option<Dir>> {
        self.objects.child("blobs", create).map_err(fs)
    }
    fn namespace(&self, namespace: &NamespaceId, create: bool) -> BackendResult<Option<Dir>> {
        match self.blobs(create)? {
            Some(blobs) => blobs.child(&hex(&namespace.0), create).map_err(fs),
            None => Ok(None),
        }
    }
}

impl Inner {
    fn now(&self) -> u64 {
        (self.clock)()
    }
    /// Every successful return passes here, so no result or acknowledgment is
    /// disclosed unless the budget still holds; an effect already made stays.
    fn finish<T>(&self, io: &IoBudget<'_>, value: BackendResult<T>) -> BackendResult<T> {
        let value = value?;
        self.milestone(Milestone::Return)?;
        io.check()?;
        Ok(value)
    }
    #[cfg(test)]
    fn milestone(&self, point: Milestone) -> BackendResult<()> {
        let observer = self.observer.lock().unwrap().clone();
        match observer {
            Some(observer) if !observer(point) => Err(BackendError::Unavailable),
            _ => Ok(()),
        }
    }
    #[cfg(not(test))]
    #[inline(always)]
    fn milestone(&self, _: Milestone) -> BackendResult<()> {
        Ok(())
    }
    /// The extension's payload tree, created only for a write; a read never creates it.
    fn tree(&self, extension: &str, create: bool) -> BackendResult<Option<Tree>> {
        let layout = if create {
            Some(
                tmt_extension_state::Layout::open(&self.data_root, extension, &[])
                    .map_err(|_| BackendError::Unavailable)?,
            )
        } else {
            tmt_extension_state::Layout::existing(&self.data_root, extension, &[])
                .map_err(|_| BackendError::Unavailable)?
        };
        let Some(layout) = layout else {
            return Ok(None);
        };
        let root = Dir::open_root(&layout.directory).map_err(fs)?;
        Ok(root
            .child("objects", create)
            .map_err(fs)?
            .map(|objects| Tree { objects }))
    }
    fn write_tree(&self, extension: &str) -> BackendResult<Tree> {
        self.tree(extension, true)?.ok_or(BackendError::Unavailable)
    }

    fn begin(
        &self,
        extension: &str,
        spec: &BeginSpec,
        io: &IoBudget<'_>,
    ) -> BackendResult<BeginResult> {
        io.check()?;
        if spec.payload_bytes > limits::OBJECT_PAYLOAD_BYTES
            || spec.binding.len() > limits::OBJECT_BINDING_BYTES
        {
            return Err(BackendError::Invalid);
        }
        let _hold = self
            .flight
            .hold(extension, &spec.key.namespace, Some(&spec.intent), io)?;
        io.check()?;
        let row = match self
            .ledger
            .adopt(extension, spec, self.now(), &self.quotas, io)?
        {
            Adoption::Fresh(row) => {
                self.milestone(Milestone::Adopted)?;
                io.check()?;
                return Ok(BeginResult::Pending(progress(&row)));
            }
            Adoption::Repeat(row) => row,
        };
        let state = match row.phase {
            Phase::Staging => return Ok(BeginResult::Pending(progress(&row))),
            Phase::Committed => return Ok(BeginResult::Committed(receipt(&row))),
            Phase::Committing | Phase::Unknown => TransferState::Unknown,
            Phase::Discarding => {
                self.close(extension, &row, io)?;
                TransferState::Discarded
            }
            Phase::Expiring => {
                self.close(extension, &row, io)?;
                TransferState::Expired
            }
            Phase::Discarded => TransferState::Discarded,
            Phase::Expired => TransferState::Expired,
            Phase::Removing | Phase::Deleted => TransferState::Unavailable,
        };
        io.check()?;
        Ok(BeginResult::Terminal(state))
    }

    fn append(
        &self,
        extension: &str,
        intent: IntentId,
        index: u32,
        bytes: &[u8],
        io: &IoBudget<'_>,
    ) -> BackendResult<Progress> {
        io.check()?;
        let known = self
            .ledger
            .row(extension, &intent, io)?
            .ok_or(BackendError::Missing)?;
        let _hold = self
            .flight
            .hold(extension, &known.spec.key.namespace, Some(&intent), io)?;
        io.check()?;
        let row = match self
            .ledger
            .gate(extension, &intent, self.now(), false, io)?
        {
            Gate::Go(row) => row,
            Gate::Expired(row) => {
                self.close(extension, &row, io)?;
                return Err(BackendError::Conflict);
            }
        };
        let total = row.spec.payload_bytes;
        if index < row.next_index {
            // An exact repeat of an acknowledged part compares the stored bytes.
            let (offset, length) = part(index, total).ok_or(BackendError::Conflict)?;
            if bytes.len() as u64 != length {
                return Err(BackendError::Conflict);
            }
            let tree = self
                .tree(extension, false)?
                .ok_or(BackendError::Unavailable)?;
            let staging = tree.staging(false)?.ok_or(BackendError::Unavailable)?;
            let file = staging
                .open_file(&hex(&intent.0), 1)
                .map_err(fs)?
                .ok_or(BackendError::Unavailable)?;
            let mut stored = vec![0; bytes.len()];
            tree::read_at(&file, offset, &mut stored).map_err(fs)?;
            io.check()?;
            return if stored == bytes {
                Ok(progress(&row))
            } else {
                Err(BackendError::Conflict)
            };
        }
        if index > row.next_index || row.received == total {
            return Err(BackendError::Conflict);
        }
        let length = CHUNK.min(total - row.received);
        if bytes.len() as u64 != length {
            return Err(BackendError::Invalid);
        }
        let tree = self.write_tree(extension)?;
        let staging = tree.staging(true)?.ok_or(BackendError::Unavailable)?;
        let name = hex(&intent.0);
        let file = match staging.create_file(&name).map_err(fs)? {
            Some(file) => {
                staging.sync().map_err(fs)?;
                file
            }
            None => staging
                .open_file(&name, 1)
                .map_err(fs)?
                .ok_or(BackendError::Unavailable)?,
        };
        let stored = tree::len(&file).map_err(fs)?;
        if stored < row.received {
            return Err(BackendError::Unavailable);
        }
        if stored > row.received {
            // An unacknowledged tail from an interrupted part is never trusted.
            file.set_len(row.received)
                .map_err(|_| BackendError::Unavailable)?;
        }
        io.check()?;
        tree::write_at(&file, row.received, bytes).map_err(fs)?;
        file.sync_all().map_err(|_| BackendError::Unavailable)?;
        self.milestone(Milestone::ChunkSynced)?;
        io.check()?;
        let next = self.ledger.advance(extension, &row, length, io)?;
        self.milestone(Milestone::ChunkAcked)?;
        io.check()?;
        Ok(progress(&next))
    }

    fn commit(
        &self,
        extension: &str,
        intent: IntentId,
        io: &IoBudget<'_>,
    ) -> BackendResult<Receipt> {
        io.check()?;
        let known = self
            .ledger
            .row(extension, &intent, io)?
            .ok_or(BackendError::Missing)?;
        let _hold = self
            .flight
            .hold(extension, &known.spec.key.namespace, Some(&intent), io)?;
        io.check()?;
        let row = self
            .ledger
            .row(extension, &intent, io)?
            .ok_or(BackendError::Missing)?;
        match row.phase {
            Phase::Committed => return Ok(receipt(&row)),
            // The same original's publication continues; it never starts another.
            Phase::Committing => return self.publish(extension, &row, false, io),
            Phase::Unknown => return Err(BackendError::Unavailable),
            Phase::Staging => {}
            _ => return Err(BackendError::Conflict),
        }
        let row = match self.ledger.gate(extension, &intent, self.now(), true, io)? {
            Gate::Go(row) => row,
            Gate::Expired(row) => {
                self.close(extension, &row, io)?;
                return Err(BackendError::Conflict);
            }
        };
        let tree = self.write_tree(extension)?;
        let staging = tree.staging(true)?.ok_or(BackendError::Unavailable)?;
        let name = hex(&intent.0);
        let file = match staging.open_file(&name, 1).map_err(fs)? {
            Some(file) => file,
            // An empty payload has no part, so its staging file is created here.
            None if row.spec.payload_bytes == 0 => {
                let file = staging
                    .create_file(&name)
                    .map_err(fs)?
                    .ok_or(BackendError::Unavailable)?;
                staging.sync().map_err(fs)?;
                file
            }
            None => return Err(BackendError::Unavailable),
        };
        self.trim_and_verify(&file, &row, io)?;
        self.milestone(Milestone::CommitVerified)?;
        io.check()?;
        let adopted = match self
            .ledger
            .commit_adopt(extension, &intent, self.now(), io)?
        {
            Gate::Go(row) => row,
            Gate::Expired(row) => {
                self.close(extension, &row, io)?;
                return Err(BackendError::Conflict);
            }
        };
        self.milestone(Milestone::CommitAdopted)?;
        self.publish(extension, &adopted, true, io)
    }

    /// Exact payload length, then the full raw SHA-256, with budget checks per buffer.
    fn trim_and_verify(&self, file: &File, row: &Row, io: &IoBudget<'_>) -> BackendResult<()> {
        tree::admit_file(file, 1).map_err(fs)?;
        let total = row.spec.payload_bytes;
        let stored = tree::len(file).map_err(fs)?;
        if stored < total {
            return Err(BackendError::Unavailable);
        }
        if stored > total {
            file.set_len(total).map_err(|_| BackendError::Unavailable)?;
            file.sync_all().map_err(|_| BackendError::Unavailable)?;
        }
        if digest(file, total, io)? == row.spec.payload_sha256 {
            Ok(())
        } else {
            Err(BackendError::Invalid)
        }
    }

    /// Create-only publication of a `committing` original, shared by the commit
    /// and by reconciliation. A destination that already exists is accepted only
    /// when its actual length and SHA-256 equal the original; anything else is
    /// closed as `unknown` and never overwritten.
    fn publish(
        &self,
        extension: &str,
        row: &Row,
        verified: bool,
        io: &IoBudget<'_>,
    ) -> BackendResult<Receipt> {
        let tree = self.write_tree(extension)?;
        let staging = tree.staging(true)?.ok_or(BackendError::Unavailable)?;
        let directory = tree
            .namespace(&row.spec.key.namespace, true)?
            .ok_or(BackendError::Unavailable)?;
        let (staged_name, final_name) = (hex(&row.spec.intent.0), hex(&row.spec.key.object.0));
        let settle = |error: BackendError| {
            // Possible bytes at an unverifiable destination stay charged and closed.
            let _ = self.ledger.mark_unknown(extension, &row.spec.intent, io);
            error
        };
        loop {
            io.check()?;
            let destination = directory.open_file(&final_name, 2).map_err(fs)?;
            if let Some(destination) = destination {
                if tree::len(&destination).map_err(fs)? != row.spec.payload_bytes
                    || digest(&destination, row.spec.payload_bytes, io)? != row.spec.payload_sha256
                {
                    return Err(settle(BackendError::Unavailable));
                }
                break;
            }
            let Some(staged) = staging.open_file(&staged_name, 2).map_err(fs)? else {
                return Err(settle(BackendError::Unavailable));
            };
            if !verified {
                match self.trim_and_verify(&staged, row, io) {
                    Ok(()) => {}
                    Err(BackendError::Invalid) => return Err(settle(BackendError::Unavailable)),
                    Err(error) => return Err(error),
                }
            }
            // Create-only: losing a race re-verifies the winner's file above.
            if staging
                .link_into(&staged_name, &directory, &final_name)
                .map_err(fs)?
            {
                break;
            }
        }
        self.milestone(Milestone::Linked)?;
        directory.sync().map_err(fs)?;
        self.milestone(Milestone::LinkSynced)?;
        io.check()?;
        self.ledger
            .commit_receipt(extension, &row.spec.intent, io)?;
        self.milestone(Milestone::Receipted)?;
        // The receipt is durable: removing the staging name is cleanup, and a
        // failure leaves `staged` set for reconciliation to finish.
        if staging.unlink(&staged_name).is_ok()
            && staging.sync().is_ok()
            && self.ledger.unstage(extension, &row.spec.intent, io).is_ok()
        {
            self.milestone(Milestone::StagingUnlinked)?;
        }
        io.check()?;
        Ok(receipt(row))
    }

    /// Remove the staging name of a closing original, then release its charge.
    fn close(&self, extension: &str, row: &Row, io: &IoBudget<'_>) -> BackendResult<()> {
        io.check()?;
        if let Some(tree) = self.tree(extension, false)?
            && let Some(staging) = tree.staging(false)?
        {
            staging.unlink(&hex(&row.spec.intent.0)).map_err(fs)?;
            staging.sync().map_err(fs)?;
        }
        self.milestone(Milestone::CloseUnlinked)?;
        io.check()?;
        self.ledger.close_confirmed(extension, &row.spec.intent, io)
    }

    fn status(
        &self,
        extension: &str,
        intent: IntentId,
        io: &IoBudget<'_>,
    ) -> BackendResult<Transfer> {
        io.check()?;
        let Some(row) = self.ledger.row(extension, &intent, io)? else {
            return Ok(Transfer {
                state: TransferState::NotObserved,
                original: None,
            });
        };
        let state = match row.phase {
            // Past its deadline a staging original can no longer commit; reporting
            // that writes nothing, and only incomplete staging is ever expired.
            Phase::Staging if self.now() >= row.expires_ms => TransferState::Expired,
            Phase::Staging => TransferState::Pending(progress(&row)),
            Phase::Committed => TransferState::Committed(receipt(&row)),
            Phase::Committing | Phase::Unknown => TransferState::Unknown,
            Phase::Discarding | Phase::Discarded => TransferState::Discarded,
            Phase::Expiring | Phase::Expired => TransferState::Expired,
            Phase::Removing | Phase::Deleted => TransferState::Unavailable,
        };
        io.check()?;
        Ok(Transfer {
            state,
            original: Some(original(&row)),
        })
    }

    fn stat(&self, extension: &str, key: BlobKey, io: &IoBudget<'_>) -> BackendResult<Receipt> {
        io.check()?;
        let _hold = self.flight.hold(extension, &key.namespace, None, io)?;
        let row = self
            .ledger
            .blob(extension, &key, io)?
            .ok_or(BackendError::Missing)?;
        io.check()?;
        Ok(receipt(&row))
    }

    fn read(
        &self,
        extension: &str,
        key: BlobKey,
        offset: u64,
        count: u32,
        io: &IoBudget<'_>,
    ) -> BackendResult<ReadPart> {
        io.check()?;
        if count == 0 || count > limits::OBJECT_CHUNK_BYTES {
            return Err(BackendError::Invalid);
        }
        let _hold = self.flight.hold(extension, &key.namespace, None, io)?;
        let row = self
            .ledger
            .blob(extension, &key, io)?
            .ok_or(BackendError::Missing)?;
        let total = row.spec.payload_bytes;
        if offset > total || (offset == total && total != 0) {
            return Err(BackendError::Invalid);
        }
        let length = u64::from(count).min(total - offset) as usize;
        let tree = self
            .tree(extension, false)?
            .ok_or(BackendError::Unavailable)?;
        let directory = tree
            .namespace(&key.namespace, false)?
            .ok_or(BackendError::Unavailable)?;
        // Only a pending staging cleanup may leave a second name for the payload.
        let file = directory
            .open_file(&hex(&key.object.0), if row.staged { 2 } else { 1 })
            .map_err(fs)?
            .ok_or(BackendError::Unavailable)?;
        if tree::len(&file).map_err(fs)? != total {
            return Err(BackendError::Unavailable);
        }
        let mut bytes = vec![0; length];
        tree::read_at(&file, offset, &mut bytes).map_err(fs)?;
        self.milestone(Milestone::ReadBytes)?;
        // The namespace may have been fenced or the budget spent during the read:
        // nothing is disclosed unless the same committed original is still open.
        self.ledger
            .blob(extension, &key, io)?
            .ok_or(BackendError::Missing)?;
        io.check()?;
        Ok(ReadPart {
            offset,
            total_bytes: total,
            bytes,
        })
    }

    fn discard(&self, extension: &str, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<()> {
        io.check()?;
        let known = self
            .ledger
            .row(extension, &intent, io)?
            .ok_or(BackendError::Missing)?;
        let _hold = self
            .flight
            .hold(extension, &known.spec.key.namespace, Some(&intent), io)?;
        io.check()?;
        let row = self.ledger.discard_adopt(extension, &intent, io)?;
        self.milestone(Milestone::DiscardAdopted)?;
        if matches!(row.phase, Phase::Discarding | Phase::Expiring) {
            self.close(extension, &row, io)?;
        }
        io.check()
    }

    fn remove_namespace(
        &self,
        extension: &str,
        namespace: NamespaceId,
        io: &IoBudget<'_>,
    ) -> BackendResult<()> {
        io.check()?;
        if !self.ledger.fence(extension, &namespace, &self.quotas, io)? {
            return Ok(());
        }
        self.milestone(Milestone::Fenced)?;
        self.finish_removal(extension, &namespace, io)
    }
    /// Drain the fenced work, then remove exactly the files the ledger names.
    fn finish_removal(
        &self,
        extension: &str,
        namespace: &NamespaceId,
        io: &IoBudget<'_>,
    ) -> BackendResult<()> {
        self.flight.drain(extension, namespace, io)?;
        for row in self.ledger.namespace_rows(extension, namespace, io)? {
            io.check()?;
            self.ledger.mark_removing(extension, &row.spec.intent, io)?;
        }
        self.milestone(Milestone::RemovalMarked)?;
        let rows = self.ledger.namespace_rows(extension, namespace, io)?;
        if let Some(tree) = self.tree(extension, false)? {
            let staging = tree.staging(false)?;
            let directory = tree.namespace(namespace, false)?;
            for row in &rows {
                io.check()?;
                if let Some(staging) = &staging {
                    staging.unlink(&hex(&row.spec.intent.0)).map_err(fs)?;
                }
                if let Some(directory) = &directory {
                    directory.unlink(&hex(&row.spec.key.object.0)).map_err(fs)?;
                }
            }
            if let Some(staging) = &staging {
                staging.sync().map_err(fs)?;
            }
            if directory.is_some()
                && let Some(blobs) = tree.blobs(false)?
            {
                // A foreign entry inside keeps the namespace charged and unsettled.
                blobs.remove_dir(&hex(&namespace.0)).map_err(fs)?;
                blobs.sync().map_err(fs)?;
            }
        }
        self.milestone(Milestone::RemovalUnlinked)?;
        io.check()?;
        self.ledger.removal_confirmed(extension, namespace, io)
    }

    /// One interrupted original, settled by its original ID.
    fn settle(&self, listed: &Row, io: &IoBudget<'_>) -> BackendResult<()> {
        let extension = listed.extension.as_str();
        let intent = listed.spec.intent;
        let _hold = self
            .flight
            .hold(extension, &listed.spec.key.namespace, Some(&intent), io)?;
        let Some(row) = self.ledger.row(extension, &intent, io)? else {
            return Ok(());
        };
        match row.phase {
            Phase::Staging => {
                if let Some(row) = self.ledger.expire(extension, &intent, self.now(), io)? {
                    return self.close(extension, &row, io);
                }
                self.trim(extension, &row, io)
            }
            Phase::Committing => match self.publish(extension, &row, false, io) {
                Ok(_) => Ok(()),
                Err(BackendError::Unavailable)
                    if self
                        .ledger
                        .row(extension, &intent, io)?
                        .is_some_and(|row| row.phase == Phase::Unknown) =>
                {
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Phase::Committed if row.staged => {
                if let Some(tree) = self.tree(extension, false)?
                    && let Some(staging) = tree.staging(false)?
                {
                    staging.unlink(&hex(&intent.0)).map_err(fs)?;
                    staging.sync().map_err(fs)?;
                }
                self.ledger.unstage(extension, &intent, io)
            }
            Phase::Discarding | Phase::Expiring => self.close(extension, &row, io),
            // A fenced namespace's removal owns these.
            _ => Ok(()),
        }
    }
    /// Cut an unacknowledged tail back to the durable checkpoint; bytes lost
    /// below it close the original as unknown rather than guessing.
    fn trim(&self, extension: &str, row: &Row, io: &IoBudget<'_>) -> BackendResult<()> {
        let staging = self
            .tree(extension, false)?
            .map(|tree| tree.staging(false))
            .transpose()?
            .flatten();
        let file = match &staging {
            Some(staging) => staging.open_file(&hex(&row.spec.intent.0), 1).map_err(fs)?,
            None => None,
        };
        let stored = match &file {
            Some(file) => tree::len(file).map_err(fs)?,
            None => 0,
        };
        if stored < row.received {
            return self.ledger.mark_unknown(extension, &row.spec.intent, io);
        }
        if let Some(file) = &file
            && stored > row.received
        {
            file.set_len(row.received)
                .map_err(|_| BackendError::Unavailable)?;
            file.sync_all().map_err(|_| BackendError::Unavailable)?;
        }
        Ok(())
    }
}

/// SHA-256 of the first `total` bytes, checking the budget every buffer.
fn digest(file: &File, total: u64, io: &IoBudget<'_>) -> BackendResult<Digest> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; limits::OBJECT_CHUNK_BYTES as usize];
    let mut offset = 0;
    while offset < total {
        io.check()?;
        let length = (total - offset).min(CHUNK) as usize;
        tree::read_at(file, offset, &mut buffer[..length]).map_err(fs)?;
        hasher.update(&buffer[..length]);
        offset += length as u64;
    }
    Ok(hasher.finalize().into())
}

impl ObjectBackend for LocalHandle<'_> {
    fn id(&self) -> &'static str {
        "local-fs"
    }
    fn capabilities(&self) -> BackendCaps {
        BackendCaps {
            max_payload_bytes: limits::OBJECT_PAYLOAD_BYTES,
            chunk_bytes: limits::OBJECT_CHUNK_BYTES,
        }
    }
    fn usage(&self, namespace: Option<NamespaceId>, io: &IoBudget<'_>) -> BackendResult<Usage> {
        let scope = namespace.map_or(UsageScope::Extension, UsageScope::Namespace);
        let usage = io
            .check()
            .and_then(|()| self.inner.ledger.usage(self.extension.as_str(), scope, io));
        self.inner.finish(io, usage)
    }
    fn begin(&self, spec: &BeginSpec, io: &IoBudget<'_>) -> BackendResult<BeginResult> {
        let result = self.inner.begin(self.extension.as_str(), spec, io);
        self.inner.finish(io, result)
    }
    fn append(
        &self,
        intent: IntentId,
        index: u32,
        bytes: &[u8],
        io: &IoBudget<'_>,
    ) -> BackendResult<Progress> {
        let result = self
            .inner
            .append(self.extension.as_str(), intent, index, bytes, io);
        self.inner.finish(io, result)
    }
    fn commit(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<Receipt> {
        let result = self.inner.commit(self.extension.as_str(), intent, io);
        self.inner.finish(io, result)
    }
    fn status(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<Transfer> {
        let result = self.inner.status(self.extension.as_str(), intent, io);
        self.inner.finish(io, result)
    }
    fn stat(&self, key: BlobKey, io: &IoBudget<'_>) -> BackendResult<Receipt> {
        let result = self.inner.stat(self.extension.as_str(), key, io);
        self.inner.finish(io, result)
    }
    fn read(
        &self,
        key: BlobKey,
        offset: u64,
        count: u32,
        io: &IoBudget<'_>,
    ) -> BackendResult<ReadPart> {
        let result = self
            .inner
            .read(self.extension.as_str(), key, offset, count, io);
        self.inner.finish(io, result)
    }
    fn discard(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<()> {
        let result = self.inner.discard(self.extension.as_str(), intent, io);
        self.inner.finish(io, result)
    }
    fn remove_namespace(&self, namespace: NamespaceId, io: &IoBudget<'_>) -> BackendResult<()> {
        let result = self
            .inner
            .remove_namespace(self.extension.as_str(), namespace, io);
        self.inner.finish(io, result)
    }
}

#[cfg(test)]
mod tests;
