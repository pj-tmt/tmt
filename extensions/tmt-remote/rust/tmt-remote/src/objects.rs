//! Remote's generic object backend boundary: opaque installed-extension
//! namespaces and keys, raw SHA-256/length and an immutable original intent.
//! Nothing here interprets a page, epoch, reference, envelope or media type, and
//! no operation grants permission: only Remote's admitted service calls it.
//! [`LocalFs`] is the one trusted local implementation; the adapter guide is
//! `.agents/skills/tmt-remote/references/object-backends.md`.
use crate::{canonical, limits};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod ledger;
mod local;
mod tree;
pub use local::{LocalFs, LocalHandle};

pub type Digest = [u8; 32];

/// A trusted installed extension's name; validating it grants nothing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExtensionId(String);
impl ExtensionId {
    pub fn new(name: &str) -> BackendResult<Self> {
        // `remote` is this owner's own subtree, never an installed extension's.
        if name.is_empty() || name == "remote" || !canonical::extension_name(name) {
            return Err(BackendError::Invalid);
        }
        Ok(Self(name.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NamespaceId(pub [u8; 32]);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OpaqueKey(pub [u8; 32]);
/// Service-derived identity of the original request (installed extension,
/// actual principal and caller transfer UUID); the backend treats it as opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntentId(pub [u8; 32]);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlobKey {
    pub namespace: NamespaceId,
    pub object: OpaqueKey,
}

/// The caller-frozen input of a begin. Adoption time and the staging deadline
/// are generated once at first durable adoption and never compared or renewed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginSpec {
    pub intent: IntentId,
    pub key: BlobKey,
    pub payload_sha256: Digest,
    pub payload_bytes: u64,
    /// Service-generated immutable input binding, at most 2 KiB; never a permission.
    pub binding: Vec<u8>,
}
/// The retained original of an intent: frozen input plus its adoption metadata.
/// Neither a receipt nor an attestation that bytes were stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalIntent {
    pub spec: BeginSpec,
    pub adopted_at_ms: u64,
    pub expires_at_ms: u64,
}
/// A committed, create-only publication of the original intent's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub intent: IntentId,
    pub key: BlobKey,
    pub payload_sha256: Digest,
    pub payload_bytes: u64,
    pub binding: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub intent: IntentId,
    pub next_index: u32,
    pub received: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeginResult {
    Pending(Progress),
    Committed(Receipt),
    Terminal(TransferState),
}
/// Absence, expiry and a deadline never prove that a possibly published effect did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferState {
    Pending(Progress),
    Committed(Receipt),
    /// Incomplete staging closed by its deadline; the identity is never executable again.
    Expired,
    /// The original is retained but its bytes are unavailable (deleted namespace) or the state is unreadable.
    Unavailable,
    /// A publication may have happened; only owned reconciliation settles it.
    Unknown,
    Discarded,
    /// No original record exists; this is not proof of no effect.
    NotObserved,
}
/// Read-only observation of an original intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub state: TransferState,
    pub original: Option<OriginalIntent>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadPart {
    pub offset: u64,
    pub total_bytes: u64,
    pub bytes: Vec<u8>,
}
/// Conservative logical charge of one scope; see the charge formula in the adapter guide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub charged_bytes: u64,
    pub entries: u32,
    pub active_uploads: u32,
    pub retained_identities: u32,
}
/// What an adapter supports. Parts must be canonical: every part but the last
/// is exactly `chunk_bytes`, the last carries the remainder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendCaps {
    pub max_payload_bytes: u64,
    pub chunk_bytes: u32,
}
/// Immutable deadline plus cooperative cancellation, checked at controlled
/// boundaries. A timeout never rolls back or interrupts an OS call: durable
/// effect, charge and state remain, and late disclosure is suppressed.
pub struct IoBudget<'a> {
    pub deadline: Instant,
    pub cancelled: &'a AtomicBool,
}
impl IoBudget<'_> {
    pub fn check(&self) -> BackendResult<()> {
        if self.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(BackendError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(BackendError::Deadline);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    NamespaceBytes,
    ExtensionBytes,
    InstallationBytes,
    NamespaceEntries,
    ExtensionEntries,
    InstallationEntries,
    ActiveIntents,
    RetainedExtension,
    RetainedInstallation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// Malformed input, or a non-canonical part, rejected before any effect.
    Invalid,
    /// Changed input under an original ID, or a state that forbids the operation.
    Conflict,
    Missing,
    /// Unsafe or damaged storage, an I/O failure, or a settled-elsewhere state; charged state stays.
    Unavailable,
    Cancelled,
    Deadline,
    /// A hierarchy limit refused the adoption with no effect.
    Capacity(Limit),
}
pub type BackendResult<T> = Result<T, BackendError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Namespace(NamespaceId),
    Extension,
}

/// Hierarchy ceilings. [`Quotas::contract`] is the proposed contract set; tests
/// inject small values to saturate each limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quotas {
    pub namespace_bytes: u64,
    pub extension_bytes: u64,
    pub installation_bytes: u64,
    pub namespace_entries: u32,
    pub extension_entries: u32,
    pub installation_entries: u32,
    pub active_intents: u32,
    pub retained_extension: u32,
    pub retained_installation: u32,
}
impl Quotas {
    pub const fn contract() -> Self {
        Self {
            namespace_bytes: limits::OBJECT_NAMESPACE_BYTES,
            extension_bytes: limits::OBJECT_EXTENSION_BYTES,
            installation_bytes: limits::OBJECT_INSTALLATION_BYTES,
            namespace_entries: limits::OBJECT_NAMESPACE_ENTRIES,
            extension_entries: limits::OBJECT_EXTENSION_ENTRIES,
            installation_entries: limits::OBJECT_INSTALLATION_ENTRIES,
            active_intents: limits::OBJECT_ACTIVE_INTENTS,
            retained_extension: limits::OBJECT_RETAINED_EXTENSION,
            retained_installation: limits::OBJECT_RETAINED_INSTALLATION,
        }
    }
}

/// Wall-clock milliseconds since the Unix epoch, injected so tests are deterministic.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64)
    })
}

/// One extension-bound backend. Every operation takes an explicit [`IoBudget`].
/// Bytes are visible only through a committed receipt; an original ID never
/// becomes executable again after expiry, discard or deletion.
pub trait ObjectBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> BackendCaps;
    /// `None` is the whole extension.
    fn usage(&self, namespace: Option<NamespaceId>, io: &IoBudget<'_>) -> BackendResult<Usage>;
    /// Adopt the original intent and reserve its entry and bytes, or observe the original.
    fn begin(&self, spec: &BeginSpec, io: &IoBudget<'_>) -> BackendResult<BeginResult>;
    fn append(
        &self,
        intent: IntentId,
        index: u32,
        bytes: &[u8],
        io: &IoBudget<'_>,
    ) -> BackendResult<Progress>;
    fn commit(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<Receipt>;
    /// Strictly observational: never adopts, expires, trims, publishes or reconciles.
    fn status(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<Transfer>;
    /// The committed receipt of a key; an incomplete object is `Missing`.
    fn stat(&self, key: BlobKey, io: &IoBudget<'_>) -> BackendResult<Receipt>;
    fn read(
        &self,
        key: BlobKey,
        offset: u64,
        count: u32,
        io: &IoBudget<'_>,
    ) -> BackendResult<ReadPart>;
    /// Close incomplete staging; a committing, committed or unknown original conflicts.
    fn discard(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<()>;
    /// Fence the namespace, then remove its payloads; originals stay as tombstones.
    fn remove_namespace(&self, namespace: NamespaceId, io: &IoBudget<'_>) -> BackendResult<()>;
}

/// Bytes charged for a payload: rounded up to the physical block, zero for an empty payload.
pub(crate) fn payload_charge(bytes: u64) -> u64 {
    bytes.div_ceil(limits::OBJECT_BLOCK_BYTES) * limits::OBJECT_BLOCK_BYTES
}
