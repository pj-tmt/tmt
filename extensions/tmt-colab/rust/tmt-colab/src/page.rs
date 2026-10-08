//! Root-local source access: isolated preparation, then fenced ciphertext commit.
pub mod compact;
pub mod ipc;
pub mod save;
use crate::{
    Result,
    decoder::{Decoder, MemoryLimit},
    fold::{self, Snapshot},
    keyring::Keyring,
    store::{
        Accepted, Envelope, Namespace, Store, StreamScope,
        owner::{Cut, Device, OwnerTransaction},
    },
};
use serde::{Deserialize, Serialize};
use tmt_colab_model::{certificate, crypto, framing, object, statement, values};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    StaleBase,
    Invalid,
    Capacity,
    Missing,
    Inactive,
    Denied,
    Unavailable,
    StreamGap,
}
impl Fault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StaleBase => "COLAB_STALE_BASE",
            Self::Invalid => "COLAB_INPUT_INVALID",
            Self::Capacity => "COLAB_CAPACITY",
            Self::Missing => "COLAB_STATE_MISSING",
            Self::Inactive => "COLAB_PAGE_INACTIVE",
            Self::Denied => "COLAB_DENIED",
            Self::Unavailable => "COLAB_UNAVAILABLE",
            Self::StreamGap => "COLAB_STREAM_GAP",
        }
    }
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::store::owner::size;
        f.write_str(match self {
            Self::StaleBase => "Page changed since the editing base; read it again before writing.",
            Self::Invalid => "Invalid page source or input.",
            Self::Capacity => {
                return write!(
                    f,
                    "The write is larger than the page can take: a source is at most {}, an update at most {}, and the write plus the changes the page already keeps at most {} updates and {}. Nothing was written.",
                    size(crate::decoder::BASELINE_BYTES),
                    size(crate::decoder::UPDATE_BYTES),
                    crate::decoder::WRITE_TAIL_UPDATES,
                    size(crate::decoder::WRITE_TAIL_BYTES)
                );
            }
            Self::Missing => "Existing Colab state is required.",
            Self::Inactive => "Archived or deleted pages cannot be written.",
            Self::Denied => {
                "The local writer is revoked or its certificate does not match the owner."
            }
            Self::Unavailable => {
                "Serving page write unavailable; no offline fallback was attempted."
            }
            Self::StreamGap => {
                "This device's write stream moved on before the write was admitted. Nothing was written; read the page again before retrying."
            }
        })
    }
}
impl std::error::Error for Fault {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Head {
    pub revision: String,
    pub statement_hash: String,
}
impl From<&statement::Head> for Head {
    fn from(head: &statement::Head) -> Self {
        Self {
            revision: head.revision.to_string(),
            statement_hash: hex(&head.hash),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub space_id: String,
    pub page_id: String,
    pub source: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_recipient: Option<crate::decoder::CreationRecipient>,
    pub epoch: String,
    pub membership_head: Head,
    pub revision: String,
    pub memory_limit: MemoryLimit,
}
/// What one whole-source write reports. A no-op publishes nothing and carries no publication.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub space_id: String,
    pub page_id: String,
    pub epoch: String,
    pub membership_head: Head,
    pub revision: String,
    pub source_sha256: String,
    pub memory_limit: MemoryLimit,
    pub changed: bool,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub publication: Option<PublicationReceipt>,
}
/// The original operation and the last envelope of an admitted batch.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationReceipt {
    pub operation_id: String,
    pub stream_id: String,
    pub count: usize,
    pub seq: String,
    pub envelope_hash: String,
}
impl Receipt {
    /// The page already holds this source: nothing was prepared, signed or published.
    pub fn unchanged(
        key: &Keyring,
        page: &str,
        source: &str,
        epoch: String,
        membership_head: Head,
        revision: String,
        memory_limit: MemoryLimit,
    ) -> Self {
        Self {
            space_id: key.space_id.clone(),
            page_id: page.into(),
            epoch,
            membership_head,
            revision,
            source_sha256: hex(&crypto::digest(source.as_bytes())),
            memory_limit,
            changed: false,
            publication: None,
        }
    }
}
/// The source is over the most one page can hold; names both numbers. A reader that stops at
/// the limit cannot know the whole size and says "at least".
#[derive(Debug, PartialEq, Eq)]
pub struct SourceTooLarge {
    size: usize,
    limit: usize,
    at_least: bool,
}
impl SourceTooLarge {
    pub fn exact(size: usize, limit: usize) -> Self {
        Self {
            size,
            limit,
            at_least: false,
        }
    }
    pub fn at_least(size: usize, limit: usize) -> Self {
        Self {
            size,
            limit,
            at_least: true,
        }
    }
    pub fn code(&self) -> &'static str {
        Fault::Capacity.code()
    }
}
impl std::fmt::Display for SourceTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::store::owner::size;
        write!(
            f,
            "The page source is {}{} bytes ({}); one page holds at most {} bytes ({}). Nothing was written.",
            if self.at_least { "at least " } else { "" },
            self.size,
            size(self.size),
            self.limit,
            size(self.limit)
        )
    }
}
impl std::error::Error for SourceTooLarge {}
/// A published write whose result this process could not read: it may or may not be durable.
/// The original operation is the only identity; a later write prepares from a fresh snapshot.
#[derive(Debug, PartialEq, Eq)]
pub struct OutcomeUnknown {
    pub operation_id: String,
}
impl OutcomeUnknown {
    pub fn code(&self) -> &'static str {
        "COLAB_OUTCOME_UNKNOWN"
    }
}
impl std::fmt::Display for OutcomeUnknown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "The write (operation {}) may or may not have been published, and nothing was resent. Read the page to see its current source before writing again.",
            self.operation_id
        )
    }
}
impl std::error::Error for OutcomeUnknown {}
impl From<crate::publication::Rejection> for Fault {
    fn from(code: crate::publication::Rejection) -> Self {
        use crate::publication::Rejection;
        match code {
            Rejection::StaleBase => Self::StaleBase,
            Rejection::Capacity => Self::Capacity,
            Rejection::PageInactive => Self::Inactive,
            Rejection::StateMissing => Self::Missing,
            Rejection::StreamGap => Self::StreamGap,
        }
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn token(
    space: &str,
    page: &str,
    head: &statement::Head,
    epoch: u64,
    cuts: &[Cut],
) -> Result<String> {
    let cuts = cuts.iter().map(Cut::payload).collect::<Result<Vec<_>>>()?;
    Ok(format!(
        "v1:{}",
        hex(&crypto::digest(&framing::frame(&[
            b"tmt-colab-page-revision-v1",
            space.as_bytes(),
            page.as_bytes(),
            head.revision.to_string().as_bytes(),
            &head.hash,
            epoch.to_string().as_bytes(),
            &serde_json::to_vec(&cuts)?,
        ])?))
    ))
}
fn current_token(tx: &OwnerTransaction<'_>, key: &Keyring, page: &str) -> Result<String> {
    let epoch = tx.current_epoch(page)?;
    token(
        &key.space_id,
        page,
        tx.head().ok_or(Fault::Missing)?,
        epoch,
        &tx.cuts(page, epoch)?,
    )
}
pub(crate) fn snapshot(
    store: &Store,
    key: &Keyring,
    page: &str,
    writing: bool,
) -> Result<Snapshot> {
    values::generated_id(page)?;
    store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        let (states, _) = fold::verify_log(&tx.log()?, key, page)?;
        if states.last().is_some_and(|a| !a.policy.writable()) && writing {
            return Err(Fault::Inactive.into());
        }
        Ok(())
    })?;
    Snapshot::capture(store, key, page)
}
/// The revision a next write fences on, read in one owner snapshot. A write's own best-effort
/// combine can move it after the write's outcome was retained.
pub fn revision(store: &Store, key: &Keyring, page: &str) -> Result<String> {
    values::generated_id(page)?;
    store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        current_token(tx, key, page)
    })
}
pub fn read(store: &Store, key: &Keyring, page: &str, decoder: &mut Decoder) -> Result<Page> {
    let s = snapshot(store, key, page, false)?;
    let view = s.materialize(key, page, decoder)?;
    Ok(Page {
        space_id: key.space_id.clone(),
        page_id: page.into(),
        source: view.source,
        title: view.title,
        publisher_agent: view.publisher_agent,
        creation_recipient: view.creation_recipient,
        epoch: s.epoch.to_string(),
        membership_head: Head::from(&s.authority.head),
        revision: token(&key.space_id, page, &s.authority.head, s.epoch, &s.cuts)?,
        memory_limit: view.memory_limit,
    })
}
/// NOOP carries no original operation or certificate; it reports the captured base.
pub enum PublicationPreparation {
    Noop {
        epoch: String,
        membership_head: Head,
        base_revision: String,
        memory_limit: MemoryLimit,
    },
    Write(FrozenPublication),
}
/// Exact sealed bytes are frozen once. Authority is still checked by the commit owner.
pub struct FrozenPublication {
    job: Box<crate::publication::SignedJob>,
    packet: Vec<u8>,
    chain: Vec<u8>,
}
impl FrozenPublication {
    pub fn job(&self) -> &crate::publication::SignedJob {
        &self.job
    }
    pub fn packet(&self) -> &[u8] {
        &self.packet
    }
    pub fn chain(&self) -> &[u8] {
        &self.chain
    }
}
/// What a caller requires of the page, or supplies, before a source replacement is prepared.
/// `Option<&str>` converts to the revision-only form the CLI uses.
#[derive(Clone, Copy, Default)]
pub struct PublishOptions<'a> {
    /// Refuse unless the page is at this revision token.
    pub expected_revision: Option<&'a str>,
    /// Refuse unless the page's current source has this SHA-256: the browser's editing base.
    pub base_source_sha256: Option<[u8; 32]>,
    /// The caller's own operation ID, so it can ask about the original after a lost reply.
    pub operation_id: Option<&'a str>,
}
impl<'a> From<Option<&'a str>> for PublishOptions<'a> {
    fn from(expected_revision: Option<&'a str>) -> Self {
        Self {
            expected_revision,
            ..Self::default()
        }
    }
}
pub fn prepare_publication<'a>(
    store: &Store,
    key: &Keyring,
    page: &str,
    edit: crate::decoder::ContentEdit<'_>,
    options: impl Into<PublishOptions<'a>>,
    decoder: &mut Decoder,
    now: u64,
) -> Result<PublicationPreparation> {
    use crate::{decoder::ContentBatch, publication::PublicationKind};
    let options = options.into();
    values::generated_id(page)?;
    values::time(now)?;
    if let Some(id) = options.operation_id {
        values::generated_id(id)?;
    }
    if edit.source.len() > crate::decoder::BASELINE_BYTES {
        return Err(
            SourceTooLarge::exact(edit.source.len(), crate::decoder::BASELINE_BYTES).into(),
        );
    }
    if edit
        .publisher_agent
        .is_some_and(|v| !crate::decoder::valid_publisher_agent(v))
    {
        return Err(Fault::Invalid.into());
    }
    // The commit owner re-decides writability; refusing here keeps an archived page from
    // recording a terminal rejection for a write that was never going to be admitted.
    let s = snapshot(store, key, page, true)?;
    let revision = token(&key.space_id, page, &s.authority.head, s.epoch, &s.cuts)?;
    if options.expected_revision.is_some_and(|r| r != revision) {
        return Err(Fault::StaleBase.into());
    }
    let author = Author::of(key, &s, now)?;
    let prepared = s.prepare_content_batch(
        key,
        page,
        edit,
        options.base_source_sha256.as_ref(),
        decoder,
    )?;
    let ContentBatch::Updates(updates) = prepared.batch else {
        return Ok(PublicationPreparation::Noop {
            epoch: s.epoch.to_string(),
            membership_head: Head::from(&s.authority.head),
            base_revision: revision,
            memory_limit: prepared.memory_limit,
        });
    };
    let frozen = freeze(
        key,
        page,
        &s,
        author,
        Sealing {
            kind: PublicationKind::Content,
            updates: &updates,
            revision,
            source: edit.source,
            memory_limit: prepared.memory_limit,
            operation_id: options.operation_id,
        },
        now,
    )?;
    Ok(PublicationPreparation::Write(frozen))
}
/// One immutable own-namespace update of the local writer's stream, frozen through the same
/// signed publication as a content batch (status actions; the decoder prepared `update`).
pub(crate) fn prepare_own_publication(
    key: &Keyring,
    page: &str,
    s: &Snapshot,
    update: &[u8],
    source: &str,
    memory_limit: MemoryLimit,
    now: u64,
) -> Result<FrozenPublication> {
    values::generated_id(page)?;
    values::time(now)?;
    use crate::publication::PublicationKind;
    s.require_update_capacity(page)?;
    let revision = token(&key.space_id, page, &s.authority.head, s.epoch, &s.cuts)?;
    let author = Author::of(key, s, now)?;
    freeze(
        key,
        page,
        s,
        author,
        Sealing {
            kind: PublicationKind::Own,
            updates: &[update.to_vec()],
            revision,
            source,
            memory_limit,
            operation_id: None,
        },
        now,
    )
}
/// Own records of the local writer, prepared against one materialization and frozen as one
/// own publication. Only that writer's authenticated structs enter the edit, never the shared
/// document or another writer's plain projection re-encoded as a new document.
pub(crate) fn freeze_own_records(
    key: &Keyring,
    page: &str,
    s: &Snapshot,
    folded: &fold::View,
    records: &[crate::decoder::OwnRecord],
    decoder: &mut Decoder,
    now: u64,
) -> Result<FrozenPublication> {
    use crate::decoder::UpdateBatch;
    let updates: Vec<&[u8]> = folded
        .local_own_update
        .as_ref()
        .into_iter()
        .map(Vec::as_slice)
        .collect();
    let prepared = decoder.prepare_own(
        UpdateBatch {
            namespace: Namespace::Own,
            baseline: &[],
            updates: &updates,
        },
        records,
        None,
    )?;
    prepare_own_publication(
        key,
        page,
        s,
        &prepared.merged,
        &folded.source,
        prepared.memory_limit,
        now,
    )
}
/// Publishes immutable records into the local writer's own stream through the same job a
/// status action uses. A record already present with the same value is left out of the edit.
pub fn prepare_own_records(
    store: &Store,
    key: &Keyring,
    page: &str,
    records: &[crate::decoder::OwnRecord],
    decoder: &mut Decoder,
    now: u64,
) -> Result<FrozenPublication> {
    let s = snapshot(store, key, page, true)?;
    let folded = s.materialize(key, page, decoder)?;
    freeze_own_records(key, page, &s, &folded, records, decoder, now)
}
/// The local writer and the unexpired certificate chain it already holds, if any.
struct Author {
    id: String,
    sign: Vec<u8>,
    existing: Option<Vec<u8>>,
}
impl Author {
    fn of(key: &Keyring, s: &Snapshot, now: u64) -> Result<Self> {
        let (id, sign, _) = key.local_writer()?;
        if s.authority.revoked_devices.contains(&id) {
            return Err(Fault::Denied.into());
        }
        let issuer = s.genesis_hash()?;
        // Every captured device row is parsed, so malformed state cannot be bypassed by renewal.
        let mut existing = None;
        for device in &s.devices {
            let chain = certificate::Chain::from_json(&device.chain)?;
            let cert = chain.certificate()?;
            if cert.device_id == id {
                if device.revoked {
                    return Err(Fault::Denied.into());
                }
                let expiry = local_chain(&chain, key, &s.authority.head, &issuer, now)?;
                if expiry > now {
                    existing = Some(device.chain.clone());
                }
            }
        }
        Ok(Self {
            id,
            sign: sign.to_vec(),
            existing,
        })
    }
}
struct Sealing<'a> {
    kind: crate::publication::PublicationKind,
    updates: &'a [Vec<u8>],
    revision: String,
    source: &'a str,
    memory_limit: MemoryLimit,
    /// The caller's own operation ID; a fresh one when absent.
    operation_id: Option<&'a str>,
}
/// Seals, signs and verifies the exact bytes of one publication of `kind`.
fn freeze(
    key: &Keyring,
    page: &str,
    s: &Snapshot,
    author: Author,
    sealing: Sealing<'_>,
    now: u64,
) -> Result<FrozenPublication> {
    use crate::publication::{Manifest, MembershipHead, NativeEvidence, PublicationEntry};
    let Sealing {
        kind,
        updates,
        revision,
        source,
        memory_limit,
        operation_id,
    } = sealing;
    let Author { id, sign, existing } = author;
    let issuer = s.genesis_hash()?;
    let chain = match existing {
        Some(bytes) => bytes,
        None => writer_chain(key, &s.authority.head, &issuer, now)?,
    };
    if chain.len() > crate::publication::CHAIN_BYTES {
        return Err(Fault::Capacity.into());
    }
    let chain_model = certificate::Chain::from_json(&chain)?;
    if local_chain(&chain_model, key, &s.authority.head, &issuer, now)? <= now {
        return Err(Fault::Denied.into());
    }
    let head = s
        .cuts
        .iter()
        .filter(|c| c.stream == id)
        .max_by_key(|c| c.tail_seq);
    let mut seq = head.map_or(0, |c| c.tail_seq);
    let mut previous = head.map_or([0; 32], |c| c.tail_hash);
    let mut packet = Vec::new();
    let mut entries = Vec::with_capacity(updates.len());
    for delta in updates {
        seq = seq.checked_add(1).ok_or(Fault::Capacity)?;
        let envelope = key.seal_content(
            &object::Context {
                space: key.space_id.clone(),
                page: page.into(),
                epoch: s.epoch.to_string(),
                kind: "update".into(),
                namespace: kind.name().into(),
                author_device: id.clone(),
                membership_revision: s.authority.head.revision.to_string(),
                stream_seq: seq.to_string(),
                prev_hash: previous,
            },
            &s.secret,
            delta,
        )?;
        let bytes = envelope.to_json()?;
        if bytes.len() > crate::limits::UPDATE_BYTES {
            return Err(Fault::Capacity.into());
        }
        let total = packet
            .len()
            .checked_add(bytes.len())
            .ok_or(Fault::Capacity)?;
        if total > crate::publication::packet_limit(updates.len())? {
            return Err(Fault::Capacity.into());
        }
        previous = envelope.hash()?;
        entries.push(PublicationEntry {
            namespace: kind,
            seq: seq.to_string(),
            envelope_hash: values::encode_binary(&previous),
            envelope_bytes: bytes.len(),
        });
        packet.extend_from_slice(&bytes);
    }
    let manifest = Manifest {
        version: 1,
        operation_id: match operation_id {
            Some(given) => given.to_owned(),
            None => fresh_id()?,
        },
        space_id: key.space_id.clone(),
        page_id: page.into(),
        epoch: s.epoch.to_string(),
        stream_id: id,
        kind,
        membership_head: MembershipHead {
            revision: s.authority.head.revision.to_string(),
            statement_hash: hex(&s.authority.head.hash),
        },
        base_revision: revision,
        entries,
        packet_bytes: packet.len(),
        packet_hash: values::encode_binary(&crypto::digest(&packet)),
        native_evidence: Some(NativeEvidence {
            source_sha256: hex(&crypto::digest(source.as_bytes())),
            memory_limit,
            chain_hash: values::encode_binary(&crypto::digest(&chain)),
        }),
    };
    let job = key.sign_content_publication(manifest)?;
    job.verify_packet(&packet, &sign)?;
    Ok(FrozenPublication {
        job: Box::new(job),
        packet,
        chain,
    })
}
pub(crate) fn fresh_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let h = hex(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}
pub(crate) fn writer_chain(
    key: &Keyring,
    head: &statement::Head,
    issuer: &[u8; 32],
    now: u64,
) -> Result<Vec<u8>> {
    let (id, sign, enc) = key.local_writer()?;
    let cert = certificate::Certificate {
        space: &key.space_id,
        issuer_kind: "member",
        issuer_id: &head.owner_member.id,
        device_id: &id,
        signing_key: &sign,
        encryption_key: &enc,
        membership_revision: "1",
        issued_at: now,
        expires_at: now
            .checked_add(crate::registration::CERTIFICATE_MS)
            .ok_or(Fault::Invalid)?,
    };
    Ok(serde_json::to_vec(
        &serde_json::json!({"version":1,"issuerStatement":values::encode_binary(issuer),
            "deviceCertificate":values::encode_binary(&certificate::input(&cert)?),
            "issuerSignature":values::encode_binary(&key.sign_device_certificate(&cert)?)}),
    )?)
}
fn local_chain(
    chain: &certificate::Chain,
    key: &Keyring,
    head: &statement::Head,
    issuer: &[u8; 32],
    now: u64,
) -> Result<u64> {
    let cert = chain.certificate()?;
    let (id, sign, enc) = key.local_writer()?;
    if cert.space != key.space_id
        || cert.issuer_kind != "member"
        || cert.issuer_id != head.owner_member.id
        || cert.device_id != id
        || cert.signing_key != &sign
        || cert.encryption_key != &enc
        || cert.membership_revision != "1"
        || cert.issued_at > now
    {
        return Err(Fault::Denied.into());
    }
    chain.verify(issuer, &cert, &head.owner_member.signing_key)?;
    Ok(cert.expires_at)
}
/// Exact terminal bytes are retained for recovery; Outcome remains the single wire model.
pub struct PublicationRecord {
    pub outcome: crate::publication::Outcome,
    pub bytes: Vec<u8>,
}
/// An answered publish: the retained original outcome and, when the writer could read it while no
/// other writer could run, the page revision after the write and its combine.
pub struct Published {
    pub record: PublicationRecord,
    pub revision: Option<String>,
}
pub struct PublicationCommitted {
    pub record: PublicationRecord,
    /// New terminal result (including rejection), or exact original-operation replay.
    pub accepted: Accepted,
}
fn publication_authority(
    tx: &OwnerTransaction<'_>,
    key: &Keyring,
    original: &crate::publication::JobKey,
    chain: &certificate::Chain,
    now: u64,
) -> Result<bool> {
    values::time(now)?;
    let head = tx.head().ok_or(Fault::Missing)?;
    let issuer = tx.statement(1)?.ok_or(Fault::Missing)?.hash()?;
    let device = key.local_writer()?.0;
    if original.space_id != key.space_id
        || original.stream_id != device
        || local_chain(chain, key, head, &issuer, now)? <= now
    {
        return Err(Fault::Denied.into());
    }
    let (states, _) = fold::verify_log(&tx.log()?, key, &original.page_id)?;
    let authority = states.last().ok_or(Fault::Missing)?;
    if authority.revoked_devices.contains(&device) || tx.device(&device)?.is_some_and(|d| d.revoked)
    {
        return Err(Fault::Denied.into());
    }
    Ok(authority.policy.writable())
}
fn publication_rejection(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> Option<crate::publication::Rejection> {
    use crate::{publication::Rejection, store::Fault as StoreFault};
    match error.downcast_ref::<StoreFault>() {
        Some(StoreFault::Gap) => Some(Rejection::StreamGap),
        Some(StoreFault::Capacity) => Some(Rejection::Capacity),
        Some(StoreFault::StaleEpoch) => Some(Rejection::StaleBase),
        _ => match error.downcast_ref::<crate::store::owner::OwnerFault>() {
            Some(crate::store::owner::OwnerFault::Capacity) => Some(Rejection::Capacity),
            _ => None, // Clock, SQL, malformed state and changed intent remain outer errors.
        },
    }
}
/// Native adapter: verifies exact sealed bytes, then atomically retains one original result.
pub fn commit_publication(
    store: &mut Store,
    key: &Keyring,
    job: &crate::publication::SignedJob,
    packet: &[u8],
    chain: &[u8],
    now: u64,
) -> Result<PublicationCommitted> {
    use crate::publication::{Outcome, Position, PublicationKind, Rejection};
    let original = job.key()?;
    let evidence = job
        .manifest
        .native_evidence
        .as_ref()
        .ok_or(Fault::Invalid)?;
    if chain.len() > 16 * 1024
        || evidence.chain_hash != values::encode_binary(&crypto::digest(chain))
    {
        return Err(Fault::Invalid.into());
    }
    let certificate = certificate::Chain::from_json(chain)?;
    let (_, signing_key, _) = key.local_writer()?;
    let entries = job.verify_packet(packet, &signing_key)?;
    store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        publication_authority(tx, key, &original, &certificate, now).map(|_| ())
    })?;
    let mut accepted = Accepted::Replay;
    let bytes = store.device_transaction(&key.space_id, &key.owner_public(), |tx| {
        let writable = publication_authority(tx, key, &original, &certificate, now)?;
        if let Some(saved) = tx.saved_publication(&original, Some(job))? {
            return Ok(saved);
        }
        // Reserve terminal room before deciding any admitted effect rejection.
        tx.admit_publication(&original, 0, 0)?;
        let effect = tx.content_savepoint(|tx| {
            if !writable {
                return Ok(Err(Rejection::PageInactive));
            }
            let Some(epoch) = tx.page_epoch(&original.page_id)? else {
                return Ok(Err(Rejection::StateMissing));
            };
            let head = tx.head().ok_or(Fault::Missing)?;
            if epoch != original.original_epoch
                || job.manifest.membership_head.revision != head.revision.to_string()
                || job.manifest.membership_head.statement_hash != hex(&head.hash)
                || job.manifest.base_revision != current_token(tx, key, &original.page_id)?
            {
                return Ok(Err(Rejection::StaleBase));
            }
            tx.admit_publication(&original, packet.len(), entries.len())?;
            tx.put_device(&Device {
                chain: chain.to_vec(),
                revoked: false,
            })?;
            let epoch = values::decimal(&original.original_epoch, false)?;
            for entry in &entries {
                let context = &entry.header.context;
                let result = tx.append_update(&Envelope {
                    scope: StreamScope {
                        page: &original.page_id,
                        epoch,
                        stream: &original.stream_id,
                    },
                    namespace: match job.manifest.kind {
                        PublicationKind::Content => Namespace::Content,
                        PublicationKind::Own => Namespace::Own,
                    },
                    seq: values::decimal(&context.stream_seq, false)?,
                    hash: entry.envelope.hash()?,
                    previous: context.prev_hash,
                    bytes: entry.bytes,
                })?;
                // Only the original outcome row grants replay; a fresh identity cannot adopt old content.
                if result != Accepted::New {
                    return Err(crate::store::Fault::Gap.into());
                }
            }
            let last = entries.last().ok_or(Fault::Invalid)?;
            Ok(Ok(Outcome::Committed {
                key: original.clone(),
                count: entries.len(),
                final_position: Position {
                    seq: last.header.context.stream_seq.clone(),
                    envelope_hash: values::encode_binary(&last.envelope.hash()?),
                },
                committed_revision: current_token(tx, key, &original.page_id)?,
                native_evidence: Some(evidence.clone()),
            }))
        });
        let outcome = match effect {
            Ok(Ok(committed)) => committed,
            Ok(Err(code)) => Outcome::Rejected {
                key: original.clone(),
                code,
            },
            Err(error) => match publication_rejection(error.as_ref()) {
                Some(code) => Outcome::Rejected {
                    key: original.clone(),
                    code,
                },
                None => return Err(error),
            },
        };
        let bytes = outcome.to_json(&original, Some(job))?;
        tx.save_publication(&original, job, &bytes)?;
        accepted = Accepted::New;
        Ok(bytes)
    })?;
    Ok(PublicationCommitted {
        record: PublicationRecord {
            outcome: Outcome::from_json(&bytes, &original, Some(job))?,
            bytes,
        },
        accepted,
    })
}
/// Observational original-key lookup; never issues or persists a certificate or UNKNOWN.
/// Read-only: what this page's root-local stream recorded for a caller-chosen operation ID.
/// `None` means the original never reached a terminal outcome here, so nothing was committed.
pub fn publication_status_by_operation(
    store: &Store,
    key: &Keyring,
    page: &str,
    operation_id: &str,
) -> Result<Option<PublicationRecord>> {
    values::generated_id(page)?;
    let (stream, _, _) = key.local_writer()?;
    store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        let Some(original) = tx.publication_key_by_operation(page, &stream, operation_id)? else {
            return Ok(None);
        };
        let bytes = tx
            .saved_publication(&original, None)?
            .ok_or(Fault::Invalid)?;
        Ok(Some(PublicationRecord {
            outcome: crate::publication::Outcome::from_json(&bytes, &original, None)?,
            bytes,
        }))
    })
}
pub fn publication_status(
    store: &Store,
    key: &Keyring,
    original: &crate::publication::JobKey,
    chain: &[u8],
    now: u64,
) -> Result<PublicationRecord> {
    use crate::publication::Outcome;
    original.validate()?;
    if chain.len() > 16 * 1024 {
        return Err(Fault::Invalid.into());
    }
    let certificate = certificate::Chain::from_json(chain)?;
    store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        publication_authority(tx, key, original, &certificate, now)?;
        let bytes = match tx.saved_publication(original, None)? {
            Some(bytes) => bytes,
            None => Outcome::Unknown {
                key: original.clone(),
            }
            .to_json(original, None)?,
        };
        Ok(PublicationRecord {
            outcome: Outcome::from_json(&bytes, original, None)?,
            bytes,
        })
    })
}
/// Maps the retained original-operation result to the write receipt: a rejection is the refusal
/// it names, and a missing result is the unknown outcome of that operation.
pub fn publication_receipt(
    job: &crate::publication::SignedJob,
    record: &PublicationRecord,
) -> Result<Receipt> {
    use crate::publication::Outcome;
    let manifest = &job.manifest;
    match &record.outcome {
        Outcome::Committed {
            key,
            count,
            final_position,
            committed_revision,
            native_evidence,
        } => {
            let evidence = native_evidence.as_ref().ok_or(Fault::Invalid)?;
            Ok(Receipt {
                space_id: manifest.space_id.clone(),
                page_id: manifest.page_id.clone(),
                epoch: manifest.epoch.clone(),
                membership_head: Head {
                    revision: manifest.membership_head.revision.clone(),
                    statement_hash: manifest.membership_head.statement_hash.clone(),
                },
                revision: committed_revision.clone(),
                source_sha256: evidence.source_sha256.clone(),
                memory_limit: evidence.memory_limit,
                changed: true,
                publication: Some(PublicationReceipt {
                    operation_id: key.operation_id.clone(),
                    stream_id: key.stream_id.clone(),
                    count: *count,
                    seq: final_position.seq.clone(),
                    envelope_hash: final_position.envelope_hash.clone(),
                }),
            })
        }
        Outcome::Rejected { code, .. } => Err(Fault::from(*code).into()),
        Outcome::Unknown { key } => Err(OutcomeUnknown {
            operation_id: key.operation_id.clone(),
        }
        .into()),
    }
}
