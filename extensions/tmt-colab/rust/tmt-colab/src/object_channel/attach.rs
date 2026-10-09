//! Root-local native attach (#2291): upload one staged file as a document attachment through the
//! serve's established object channel, then publish its creation proof and the document's
//! attachment list through the serve's single writer. The serve alone seals, uploads and
//! publishes; the caller only staged bytes into a slot the serve named.
//!
//! A slot freezes the sealed original before the first request leaves, so every retry is the same
//! transfer. Where an attempt stopped is read from the page and the backend, never remembered:
//! an already listed attachment is done, an already proven one needs only its list change, and an
//! upload resumes from the backend's own status. A lost reply therefore never makes a second
//! object or a second publication.
use super::{
    CallbackOwner, ChannelOwner, Client, CommittedReader, admission::RootWriteAdmission,
    binding::FrozenUpload,
};
use crate::{
    Result,
    attachments::{
        self, CommittedObjectVerifier, PublicationIntent, seal,
        slots::{Aged, Frozen, StagingSlot},
    },
    keyring::Keyring,
    limits, page,
};
use std::{sync::Arc, time::Instant};
use tmt_colab_model::{
    attachment::{Descriptor, DocumentChange, Source},
    crypto, values,
};
use tmt_extension_objects::{
    AdmitInput, BeginInput, Call, ErrorCode, Origin, Outcome, State, Success,
};

/// One frozen publication through the serve's single writer, answered or refused.
pub(crate) trait Publish {
    fn publish(&self, key: &Keyring, frozen: &page::FrozenPublication) -> Result<page::Published>;
}
/// What an attach produced: the descriptor now listed on the page, and the page revision after.
#[derive(Debug)]
pub(crate) struct Attached {
    pub descriptor: Descriptor,
    pub revision: String,
}
/// A refusal and whether the slot it came from is finished. A slot is kept only while it still
/// holds an original that a later explicit resume can use.
#[derive(Debug)]
pub(super) struct AttachFailure {
    pub(super) error: Error,
    pub(super) dispose: bool,
}
impl AttachFailure {
    fn keep(error: impl Into<Error>) -> Self {
        Self {
            error: error.into(),
            dispose: false,
        }
    }
    fn dispose(error: impl Into<Error>) -> Self {
        Self {
            error: error.into(),
            dispose: true,
        }
    }
}
pub(super) type Error = Box<dyn std::error::Error + Send + Sync>;
pub(super) type Step<T> = std::result::Result<T, AttachFailure>;

/// Where an attempt stands on the page.
enum Stage {
    /// The descriptor is already in the document's attachment list.
    Listed,
    /// The creation proof is published; only the list change is left.
    Proven,
    /// Neither: the original may still have to be uploaded.
    Fresh,
}

impl ChannelOwner {
    /// Attach the file staged in `slot`. `expected` is the SHA-256 of the staged copy the caller
    /// reports, required exactly when the slot has not been sealed yet; a slot that already holds
    /// a sealed original resumes without it. A finished slot keeps only its answer, so a retry
    /// after a lost reply gets the same attachment. A slot that holds nothing a resume could use
    /// is disposed; any other is kept.
    pub(crate) fn attach_root_local(
        &self,
        mut slot: StagingSlot,
        expected: Option<[u8; 32]>,
        source: page::save::SourceOpener,
        publisher: &dyn Publish,
        deadline: Instant,
    ) -> Result<Attached> {
        let Some(client) = self.client() else {
            return Err(page::Fault::Unavailable.into());
        };
        let live = Live {
            client: &client,
            source: &source,
            deadline,
        };
        match run(&live, &mut slot, expected, &source, publisher, deadline) {
            Ok(attached) => {
                slot.finish(now()?)?;
                Ok(attached)
            }
            Err(failure) => {
                if failure.dispose {
                    let _ = slot.dispose();
                }
                Err(failure.error)
            }
        }
    }
    /// Discard the original of a slot past the backend's staging expiry, then dispose it. The
    /// discard is best effort and bounded by `deadline`: an original the backend already expired
    /// needs none, and once the budget is spent the slot goes without one.
    pub(crate) fn dispose_aged(
        &self,
        aged: Aged<'_>,
        source: &page::save::SourceOpener,
        deadline: Instant,
    ) {
        let Aged { slot, .. } = aged;
        if Instant::now() < deadline
            && let (Some(client), Some(frozen)) = (self.client(), slot.record.sealed.clone())
            && let Ok(upload) = FrozenUpload::metadata(
                frozen.descriptor.clone(),
                frozen.base.clone(),
                &frozen.transfer_id,
            )
        {
            let channel = UploadChannel::new(&client, source, &frozen, &upload, deadline);
            let _ = channel.call(upload.discard());
        }
        let _ = slot.dispose();
    }
}
fn original_of(upload: &FrozenUpload) -> BeginInput {
    let Call::Begin(input) = upload.begin() else {
        unreachable!("frozen begin")
    };
    input
}
/// How a call reaches the backend: through the object channel, admitted by the narrow native
/// owner. A seam so the upload protocol can be exercised without a bus.
pub(super) trait Transport {
    fn call(&self, call: Call) -> Result<Outcome>;
}
struct UploadChannel<'a> {
    client: &'a Client,
    source: &'a page::save::SourceOpener,
    frozen: &'a Frozen,
    upload: &'a FrozenUpload,
    original: BeginInput,
    deadline: Instant,
}
impl<'a> UploadChannel<'a> {
    fn new(
        client: &'a Client,
        source: &'a page::save::SourceOpener,
        frozen: &'a Frozen,
        upload: &'a FrozenUpload,
        deadline: Instant,
    ) -> Self {
        Self {
            client,
            source,
            frozen,
            upload,
            original: original_of(upload),
            deadline,
        }
    }
}
impl Transport for UploadChannel<'_> {
    fn call(&self, call: Call) -> Result<Outcome> {
        let input = match &call {
            Call::Begin(input) => AdmitInput::Begin(input.clone()),
            Call::Status(input) => AdmitInput::Status(input.clone()),
            Call::Part(_) | Call::Commit(_) | Call::Discard(_) => {
                self.upload.admission_input(&call)?
            }
            Call::Read(_) | Call::Config(_) => return Err(page::Fault::Invalid.into()),
        };
        let owner: Arc<dyn CallbackOwner> = Arc::new(RootWriteAdmission {
            client: self.client.clone(),
            source: Arc::clone(self.source),
            descriptor: self.frozen.descriptor.clone(),
            base: self.frozen.base.clone(),
            original: self.original.clone(),
            input: Some(input),
            deadline: self.deadline,
        });
        self.client
            .request(Origin::LocalExtension, call, owner, self.deadline)
            .map_err(|_| page::Fault::Unavailable.into())
    }
}
/// The object channel as the attach flow needs it: the upload protocol, the authenticated read-back
/// of the committed bytes, and a best-effort discard. A seam, so every stage of the flow, including
/// a page that moved under it, can be exercised without a bus.
pub(super) trait Objects {
    fn upload(&self, frozen: &Frozen, ciphertext: &[u8]) -> Step<()>;
    fn committed(&self, frozen: &Frozen) -> Result<Box<dyn CommittedObjectVerifier + '_>>;
    fn discard(&self, frozen: &Frozen);
}
/// The serve's established channel, with every call admitted by the narrow native owner.
struct Live<'a> {
    client: &'a Client,
    source: &'a page::save::SourceOpener,
    deadline: Instant,
}
impl Objects for Live<'_> {
    fn upload(&self, frozen: &Frozen, ciphertext: &[u8]) -> Step<()> {
        let frozen_upload = FrozenUpload::metadata(
            frozen.descriptor.clone(),
            frozen.base.clone(),
            &frozen.transfer_id,
        )
        .map_err(AttachFailure::keep)?;
        let channel = UploadChannel::new(
            self.client,
            self.source,
            frozen,
            &frozen_upload,
            self.deadline,
        );
        upload(&channel, &frozen_upload, ciphertext)
    }
    fn committed(&self, frozen: &Frozen) -> Result<Box<dyn CommittedObjectVerifier + '_>> {
        let upload = FrozenUpload::metadata(
            frozen.descriptor.clone(),
            frozen.base.clone(),
            &frozen.transfer_id,
        )?;
        let original = original_of(&upload);
        Ok(Box::new(CommittedReader {
            client: self.client.clone(),
            origin: Origin::LocalExtension,
            owner: Arc::new(RootWriteAdmission {
                client: self.client.clone(),
                source: Arc::clone(self.source),
                descriptor: frozen.descriptor.clone(),
                base: frozen.base.clone(),
                original: original.clone(),
                input: None,
                deadline: self.deadline,
            }),
            namespace: *original.namespace.as_bytes(),
            key: *original.opaque_key.as_bytes(),
            policy: original.policy.clone(),
            digest: *original.payload_sha256.as_bytes(),
            bytes: original.payload_bytes,
        }))
    }
    fn discard(&self, frozen: &Frozen) {
        // A short budget of its own: the caller is already refusing, and an original the backend
        // has expired or never saw needs no discard.
        let deadline = self
            .deadline
            .min(Instant::now() + std::time::Duration::from_secs(1));
        if let Ok(upload) = FrozenUpload::metadata(
            frozen.descriptor.clone(),
            frozen.base.clone(),
            &frozen.transfer_id,
        ) {
            let channel = UploadChannel::new(self.client, self.source, frozen, &upload, deadline);
            let _ = channel.call(upload.discard());
        }
    }
}
/// What a backend refusal means to the caller. `unknown` says a mutation may have taken effect.
fn refusal(code: ErrorCode) -> Error {
    match code {
        ErrorCode::Denied => page::Fault::Denied,
        ErrorCode::Invalid | ErrorCode::Conflict => page::Fault::Invalid,
        ErrorCode::Capacity(_) => page::Fault::Capacity,
        ErrorCode::NotFound => page::Fault::Missing,
        ErrorCode::Unavailable | ErrorCode::Unknown => page::Fault::Unavailable,
    }
    .into()
}
fn committed(published: &page::Published) -> Result<()> {
    use crate::publication::Outcome as Publication;
    match &published.record.outcome {
        Publication::Committed { .. } => Ok(()),
        Publication::Rejected { code, .. } => Err(page::Fault::from(*code).into()),
        // The write may have committed: the next attempt reads the page to find out.
        Publication::Unknown { .. } => Err(page::Fault::Unavailable.into()),
    }
}
pub(super) fn run(
    objects: &dyn Objects,
    slot: &mut StagingSlot,
    expected: Option<[u8; 32]>,
    source: &page::save::SourceOpener,
    publisher: &dyn Publish,
    deadline: Instant,
) -> Step<Attached> {
    let page_id = slot.record.page.clone();
    if slot.record.done_ms.is_some() {
        // A retry of a finished attach: the same attachment, never a second upload.
        let frozen = slot
            .record
            .sealed
            .clone()
            .ok_or(AttachFailure::keep(page::Fault::Invalid))?;
        return answer(source, &page_id, frozen.descriptor).map_err(AttachFailure::keep);
    }
    if slot.record.sealed.is_none() {
        // Nothing exists outside this slot yet, so any refusal here finishes it. Without the
        // hash the caller measured while copying there is nothing to hold the copy against.
        let Some(expected) = expected else {
            return Err(AttachFailure::keep(page::Fault::Invalid));
        };
        let mut plaintext = slot.read_source().map_err(AttachFailure::dispose)?;
        let sealed = (|| -> Result<seal::Sealed> {
            if crypto::digest(&plaintext) != expected {
                return Err(page::Fault::Invalid.into());
            }
            let mut view = source()?;
            seal::seal_document(
                &mut view,
                &page_id,
                &plaintext,
                &slot.record.filename,
                &slot.record.media_type,
                deadline,
            )
        })();
        plaintext.fill(0);
        let sealed = sealed.map_err(AttachFailure::dispose)?;
        slot.seal(
            &sealed.ciphertext,
            Frozen {
                transfer_id: page::fresh_id().map_err(AttachFailure::dispose)?,
                base: sealed.base,
                descriptor: sealed.descriptor,
            },
        )
        .map_err(AttachFailure::dispose)?;
    }
    let frozen = slot.record.sealed.clone().expect("sealed above");
    let descriptor = frozen.descriptor.clone();
    // Past this point a page that moved is terminal: no resume can make the frozen base current
    // again, so the original is discarded (best effort) and the slot disposed.
    let moved = |error: Error| {
        if is(&error, page::Fault::StaleBase) {
            objects.discard(&frozen);
            AttachFailure::dispose(page::Fault::StaleBase)
        } else {
            AttachFailure::keep(error)
        }
    };
    match stage(source, &page_id, &descriptor, deadline).map_err(AttachFailure::keep)? {
        Stage::Listed => {}
        Stage::Proven => save(source, publisher, &descriptor, deadline).map_err(moved)?,
        Stage::Fresh => {
            if !base_holds(source, &frozen).map_err(AttachFailure::keep)? {
                return Err(moved(page::Fault::StaleBase.into()));
            }
            let ciphertext = slot
                .read_object(limits::OBJECT_BYTES as u64)
                .map_err(AttachFailure::keep)?;
            if let Err(failure) = objects.upload(&frozen, &ciphertext) {
                // A refusal that follows the page moving is a stale base, not a permission.
                if is(&failure.error, page::Fault::Denied)
                    && !base_holds(source, &frozen).unwrap_or(true)
                {
                    return Err(moved(page::Fault::StaleBase.into()));
                }
                return Err(failure);
            }
            prove(objects, source, publisher, &frozen, deadline).map_err(moved)?;
            save(source, publisher, &descriptor, deadline).map_err(moved)?;
        }
    }
    answer(source, &page_id, descriptor).map_err(AttachFailure::keep)
}
fn is(error: &Error, fault: page::Fault) -> bool {
    error.downcast_ref::<page::Fault>() == Some(&fault)
}
/// Whether the page is still at the base the original was sealed against.
fn base_holds(source: &page::save::SourceOpener, frozen: &Frozen) -> Result<bool> {
    let view = source()?;
    let snapshot = page::snapshot(&view.store, &view.keyring, &frozen.descriptor.page, true)?;
    Ok(attachments::current_base(&view.keyring, &frozen.descriptor, &snapshot)? == frozen.base)
}
/// The attachment as the page has it now: its descriptor and the revision a reference names.
fn answer(
    source: &page::save::SourceOpener,
    page_id: &str,
    descriptor: Descriptor,
) -> Result<Attached> {
    let view = source()?;
    Ok(Attached {
        revision: page::revision(&view.store, &view.keyring, page_id)?,
        descriptor,
    })
}
/// Read where the page stands. An archived, deleted or read-only page refuses here, before any
/// request leaves, whatever the slot already holds.
fn stage(
    source: &page::save::SourceOpener,
    page_id: &str,
    descriptor: &Descriptor,
    deadline: Instant,
) -> Result<Stage> {
    let mut view = source()?;
    let snapshot = page::snapshot(&view.store, &view.keyring, page_id, true)?;
    let folded = snapshot.materialize_until(&view.keyring, page_id, &mut view.decoder, deadline)?;
    let listed = folded
        .meta
        .get("attachments")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|list| {
            list.iter()
                .any(|item| item["attachmentId"] == descriptor.attachment_id.as_str())
        });
    Ok(if listed {
        Stage::Listed
    } else if attachments::creation_proof(&folded, descriptor).is_ok() {
        Stage::Proven
    } else {
        Stage::Fresh
    })
}
/// Upload the frozen original, or finish what an earlier attempt started. A doubtful answer keeps
/// the slot: the next explicit resume asks the backend before sending anything.
pub(super) fn upload(
    transport: &dyn Transport,
    upload: &FrozenUpload,
    ciphertext: &[u8],
) -> Step<()> {
    let original = original_of(upload);
    if ciphertext.len() as u64 != original.payload_bytes
        || crypto::digest(ciphertext) != *original.payload_sha256.as_bytes()
    {
        return Err(AttachFailure::keep(page::Fault::Invalid));
    }
    let call = |call: Call| transport.call(call).map_err(AttachFailure::keep);
    let outcome = |outcome: Outcome| -> Step<Success> {
        match outcome {
            Outcome::Success(success) => Ok(success),
            Outcome::Failure(code) => Err(AttachFailure::keep(refusal(code))),
        }
    };
    // Ask first: the backend knows whether this exact transfer began, how far it got, or that it
    // is gone. Only a transfer it never observed is begun.
    let start = match outcome(call(upload.status())?)? {
        Success::Pending { next_index, .. } | Success::Progress { next_index, .. } => {
            Some(next_index)
        }
        Success::Committed { .. } => None,
        Success::State(State::NotObserved) => {
            match outcome(call(Call::Begin(original.clone()))?)? {
                Success::Pending { next_index, .. } => Some(next_index),
                _ => return Err(AttachFailure::keep(page::Fault::Invalid)),
            }
        }
        // The staged upload expired or was discarded: the original is gone for good.
        Success::State(State::Expired | State::Discarded) => {
            return Err(AttachFailure::dispose(page::Fault::Missing));
        }
        Success::State(_) => return Err(AttachFailure::keep(page::Fault::Unavailable)),
        _ => return Err(AttachFailure::keep(page::Fault::Invalid)),
    };
    let Some(start) = start else {
        return Ok(());
    };
    let total = original.payload_bytes.div_ceil(limits::CHUNK_BYTES as u64);
    for index in
        start..u32::try_from(total).map_err(|_| AttachFailure::keep(page::Fault::Capacity))?
    {
        let from = index as usize * limits::CHUNK_BYTES;
        let part = ciphertext
            .get(from..)
            .map(|rest| &rest[..rest.len().min(limits::CHUNK_BYTES)])
            .filter(|part| !part.is_empty())
            .ok_or_else(|| AttachFailure::keep(page::Fault::Invalid))?;
        let sent = upload
            .streamed_part(index, part.to_vec())
            .map_err(AttachFailure::keep)?;
        match outcome(call(sent)?)? {
            Success::Progress { next_index, .. } if next_index == index + 1 => {}
            _ => return Err(AttachFailure::keep(page::Fault::Invalid)),
        }
    }
    match outcome(call(upload.commit())?)? {
        Success::Committed { .. } => Ok(()),
        _ => Err(AttachFailure::keep(page::Fault::Invalid)),
    }
}
/// Publish the creation proof after reading the committed bytes back and authenticating them.
fn prove(
    objects: &dyn Objects,
    source: &page::save::SourceOpener,
    publisher: &dyn Publish,
    frozen: &Frozen,
    deadline: Instant,
) -> Result<()> {
    let reader = objects.committed(frozen)?;
    let mut view = source()?;
    let publication = attachments::prepare_publication(
        &view.store,
        &view.keyring,
        PublicationIntent {
            descriptor: &frozen.descriptor,
            base: &frozen.base,
        },
        &*reader,
        &mut view.decoder,
        deadline,
        now()?,
    )?;
    committed(&publisher.publish(&view.keyring, &publication)?)
}
/// Add the descriptor to the document's attachment list, bound to the source it was sealed
/// against: a page that changed since refuses as a stale base and is never re-authored.
fn save(
    source: &page::save::SourceOpener,
    publisher: &dyn Publish,
    descriptor: &Descriptor,
    deadline: Instant,
) -> Result<()> {
    let Source::Document { source_digest } = &descriptor.source else {
        return Err(page::Fault::Invalid.into());
    };
    values::object_id(source_digest)?;
    let mut base = [0u8; 32];
    for (index, byte) in base.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&source_digest[index * 2..index * 2 + 2], 16)?;
    }
    let mut view = source()?;
    if Instant::now() >= deadline {
        return Err(page::Fault::Unavailable.into());
    }
    let current = page::read(
        &view.store,
        &view.keyring,
        &descriptor.page,
        &mut view.decoder,
    )?;
    drop(view);
    let change: DocumentChange =
        serde_json::from_value(serde_json::json!({ "set": [descriptor] }))?;
    let prepared = page::save::prepare(
        source,
        &page::save::Save {
            page: descriptor.page.clone(),
            operation_id: page::fresh_id()?,
            base_sha256: base,
            source: current.source,
            attachments: Some(change),
        },
        &now,
    )?;
    match prepared {
        page::save::Prepared::Write(frozen) => {
            let view = source()?;
            committed(&publisher.publish(&view.keyring, &frozen)?)
        }
        // A list change always writes; "unchanged" means the page cannot take it as it stands.
        page::save::Prepared::Unchanged(_) => Err(page::Fault::Invalid.into()),
    }
}
fn now() -> Result<u64> {
    crate::registration::now_ms().map_err(|_| page::Fault::Unavailable.into())
}

#[cfg(test)]
pub(in crate::object_channel) mod tests {
    use super::*;
    use std::sync::Mutex;
    use tmt_extension_objects::Limit;

    const TRANSFER: &str = "20000000-0000-4000-8000-000000000094";
    pub(in crate::object_channel) fn bytes(length: usize) -> Vec<u8> {
        (0..length).map(|i| (i * 131 + 7) as u8).collect()
    }
    /// A frozen upload of `ciphertext`, as a slot would hold it.
    pub(in crate::object_channel) fn frozen(ciphertext: &[u8]) -> FrozenUpload {
        let descriptor: Descriptor = serde_json::from_value(serde_json::json!({
            "version": 1,
            "attachmentId": "20000000-0000-4000-8000-000000000092",
            "space": "a".repeat(32),
            "page": "20000000-0000-4000-8000-000000000091",
            "epoch": "1",
            "namespace": "content",
            "objectId": "a".repeat(64),
            "authorDevice": "20000000-0000-4000-8000-000000000093",
            "membershipRevision": "1",
            "source": {"kind": "document", "sourceDigest": "b".repeat(64)},
            "envelopeHash": "c".repeat(64),
            "signature": values::encode_binary(&[1u8; 64]),
            "payloadSha256": crypto::digest(ciphertext).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "payloadBytes": ciphertext.len().to_string(),
            "plaintextBytes": "5",
            "filename": "a.txt",
            "mediaType": "text/plain",
        }))
        .unwrap();
        FrozenUpload::metadata(descriptor, format!("v1:{}", "e".repeat(64)), TRANSFER).unwrap()
    }

    #[derive(Default)]
    pub(in crate::object_channel) struct Counts {
        pub(in crate::object_channel) status: usize,
        pub(in crate::object_channel) begin: usize,
        pub(in crate::object_channel) parts: Vec<u32>,
        pub(in crate::object_channel) commit: usize,
    }
    pub(in crate::object_channel) enum Transfer {
        Unobserved,
        Pending(Vec<Vec<u8>>),
        Committed,
        Gone(State),
    }
    /// An in-memory backend that survives the "restart" of the caller, and can lose the reply of
    /// the Nth call after applying it, or refuse with a code.
    pub(in crate::object_channel) struct Backend {
        transfer: Mutex<Transfer>,
        pub(in crate::object_channel) counts: Mutex<Counts>,
        pub(in crate::object_channel) calls: Mutex<usize>,
        pub(in crate::object_channel) lose_reply_at: Mutex<Option<usize>>,
        refuse_begin: Mutex<Option<ErrorCode>>,
    }
    impl Backend {
        pub(in crate::object_channel) fn new(transfer: Transfer) -> Self {
            Self {
                transfer: Mutex::new(transfer),
                counts: Mutex::new(Counts::default()),
                calls: Mutex::new(0),
                lose_reply_at: Mutex::new(None),
                refuse_begin: Mutex::new(None),
            }
        }
        pub(in crate::object_channel) fn refuse_begin_with(&self, code: ErrorCode) {
            *self.refuse_begin.lock().unwrap() = Some(code);
        }
        pub(in crate::object_channel) fn transfer_state(
            &self,
        ) -> std::sync::MutexGuard<'_, Transfer> {
            self.transfer.lock().unwrap()
        }
        pub(in crate::object_channel) fn lose_reply_at(&self, call: usize) {
            *self.lose_reply_at.lock().unwrap() = Some(call);
        }
    }
    impl Transport for Backend {
        fn call(&self, call: Call) -> Result<Outcome> {
            let number = {
                let mut calls = self.calls.lock().unwrap();
                *calls += 1;
                *calls
            };
            let mut transfer = self.transfer.lock().unwrap();
            let mut counts = self.counts.lock().unwrap();
            let outcome = match call {
                Call::Status(_) => {
                    counts.status += 1;
                    match &*transfer {
                        Transfer::Unobserved => Success::State(State::NotObserved),
                        Transfer::Pending(parts) => Success::Pending {
                            next_index: parts.len() as u32,
                            received: parts.iter().map(|p| p.len() as u64).sum(),
                            expires_at_ms: None,
                        },
                        Transfer::Committed => Success::Committed {
                            opaque_key: tmt_extension_objects::Bytes32::from_bytes([0; 32]),
                            payload_sha256: tmt_extension_objects::Sha256Hex::from_bytes([0; 32]),
                            payload_bytes: 0,
                        },
                        Transfer::Gone(state) => Success::State(*state),
                    }
                }
                Call::Begin(_) => {
                    if let Some(code) = *self.refuse_begin.lock().unwrap() {
                        return Ok(Outcome::Failure(code));
                    }
                    counts.begin += 1;
                    *transfer = Transfer::Pending(Vec::new());
                    Success::Pending {
                        next_index: 0,
                        received: 0,
                        expires_at_ms: None,
                    }
                }
                Call::Part(part) => {
                    counts.parts.push(part.index);
                    let Transfer::Pending(parts) = &mut *transfer else {
                        return Ok(Outcome::Failure(ErrorCode::Invalid));
                    };
                    if part.index as usize != parts.len() {
                        return Ok(Outcome::Failure(ErrorCode::Invalid));
                    }
                    parts.push(part.bytes.as_bytes().to_vec());
                    Success::Progress {
                        next_index: part.index + 1,
                        received: parts.iter().map(|p| p.len() as u64).sum(),
                    }
                }
                Call::Commit(_) => {
                    counts.commit += 1;
                    *transfer = Transfer::Committed;
                    Success::Committed {
                        opaque_key: tmt_extension_objects::Bytes32::from_bytes([0; 32]),
                        payload_sha256: tmt_extension_objects::Sha256Hex::from_bytes([0; 32]),
                        payload_bytes: 0,
                    }
                }
                _ => return Ok(Outcome::Failure(ErrorCode::Invalid)),
            };
            if *self.lose_reply_at.lock().unwrap() == Some(number) {
                return Err(page::Fault::Unavailable.into());
            }
            Ok(Outcome::Success(outcome))
        }
    }
    fn code(failure: &AttachFailure) -> Option<&'static str> {
        failure
            .error
            .downcast_ref::<page::Fault>()
            .map(page::Fault::code)
    }
    fn parts_needed(ciphertext: &[u8]) -> usize {
        ciphertext.len().div_ceil(limits::CHUNK_BYTES)
    }

    #[test]
    fn a_fresh_upload_asks_then_begins_once_sends_every_part_once_and_commits_once() {
        let ciphertext = bytes(limits::CHUNK_BYTES * 3 + 100);
        let backend = Backend::new(Transfer::Unobserved);
        upload(&backend, &frozen(&ciphertext), &ciphertext).unwrap();
        let counts = backend.counts.lock().unwrap();
        assert_eq!((counts.status, counts.begin, counts.commit), (1, 1, 1));
        assert_eq!(counts.parts, [0, 1, 2, 3]);
        assert!(matches!(
            &*backend.transfer.lock().unwrap(),
            Transfer::Committed
        ));
    }
    #[test]
    fn a_restart_mid_transfer_resumes_from_the_backend_with_exactly_one_begin() {
        let ciphertext = bytes(limits::CHUNK_BYTES * 5 + 7);
        let backend = Backend::new(Transfer::Unobserved);
        // The reply of the fourth call (status, begin, part 0, part 1) is lost, as if the serve
        // died there; the part itself was stored.
        backend.lose_reply_at(4);
        let failure = upload(&backend, &frozen(&ciphertext), &ciphertext).unwrap_err();
        assert_eq!(code(&failure), Some("COLAB_UNAVAILABLE"));
        assert!(!failure.dispose, "an interrupted upload keeps its slot");
        // A later run builds the same frozen upload again and finishes it.
        upload(&backend, &frozen(&ciphertext), &ciphertext).unwrap();
        let counts = backend.counts.lock().unwrap();
        assert_eq!(counts.begin, 1, "one begin across the interruption");
        assert_eq!(counts.status, 2);
        let mut sent = counts.parts.clone();
        sent.sort_unstable();
        assert_eq!(
            sent,
            (0..parts_needed(&ciphertext) as u32).collect::<Vec<_>>()
        );
        assert_eq!(counts.commit, 1);
    }
    #[test]
    fn a_reply_lost_after_begin_commits_or_commit_never_repeats_what_the_backend_has() {
        let ciphertext = bytes(limits::CHUNK_BYTES * 2);
        // Lost after begin: the next run sees pending and sends parts, never a second begin.
        let backend = Backend::new(Transfer::Unobserved);
        backend.lose_reply_at(2);
        assert!(upload(&backend, &frozen(&ciphertext), &ciphertext).is_err());
        upload(&backend, &frozen(&ciphertext), &ciphertext).unwrap();
        assert_eq!(backend.counts.lock().unwrap().begin, 1);
        // Lost after commit (status, begin, part, part, commit): the next run sees committed and
        // sends nothing at all.
        let backend = Backend::new(Transfer::Unobserved);
        backend.lose_reply_at(5);
        assert!(upload(&backend, &frozen(&ciphertext), &ciphertext).is_err());
        upload(&backend, &frozen(&ciphertext), &ciphertext).unwrap();
        let counts = backend.counts.lock().unwrap();
        assert_eq!((counts.begin, counts.parts.len(), counts.commit), (1, 2, 1));
    }
    #[test]
    fn a_gone_original_finishes_the_slot_and_a_doubtful_one_never_does() {
        let ciphertext = bytes(100);
        for state in [State::Expired, State::Discarded] {
            let backend = Backend::new(Transfer::Gone(state));
            let failure = upload(&backend, &frozen(&ciphertext), &ciphertext).unwrap_err();
            assert_eq!(code(&failure), Some("COLAB_STATE_MISSING"));
            assert!(failure.dispose);
            assert_eq!(backend.counts.lock().unwrap().begin, 0);
        }
        for state in [State::Unavailable, State::Unknown] {
            let backend = Backend::new(Transfer::Gone(state));
            let failure = upload(&backend, &frozen(&ciphertext), &ciphertext).unwrap_err();
            assert_eq!(code(&failure), Some("COLAB_UNAVAILABLE"));
            assert!(!failure.dispose);
        }
    }
    #[test]
    fn backend_refusals_keep_the_slot_and_name_their_code() {
        let ciphertext = bytes(100);
        for (refused, expected) in [
            (ErrorCode::Denied, "COLAB_DENIED"),
            (ErrorCode::Capacity(Limit::ActiveIntents), "COLAB_CAPACITY"),
            (ErrorCode::Invalid, "COLAB_INPUT_INVALID"),
            (ErrorCode::Unavailable, "COLAB_UNAVAILABLE"),
        ] {
            let backend = Backend::new(Transfer::Unobserved);
            backend.refuse_begin_with(refused);
            let failure = upload(&backend, &frozen(&ciphertext), &ciphertext).unwrap_err();
            assert_eq!(code(&failure), Some(expected));
            assert!(!failure.dispose);
        }
    }
    #[test]
    fn a_ciphertext_that_is_not_the_frozen_original_is_refused_before_any_call() {
        let ciphertext = bytes(100);
        let backend = Backend::new(Transfer::Unobserved);
        let mut other = ciphertext.clone();
        other[0] ^= 1;
        let failure = upload(&backend, &frozen(&ciphertext), &other).unwrap_err();
        assert_eq!(code(&failure), Some("COLAB_INPUT_INVALID"));
        assert_eq!(*backend.calls.lock().unwrap(), 0);
        let failure = upload(&backend, &frozen(&ciphertext), &ciphertext[..99]).unwrap_err();
        assert_eq!(code(&failure), Some("COLAB_INPUT_INVALID"));
        assert_eq!(*backend.calls.lock().unwrap(), 0);
    }
}
