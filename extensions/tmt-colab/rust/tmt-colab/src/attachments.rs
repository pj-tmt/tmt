//! Attachment reference admission. A descriptor or caller JSON is never authority.
use crate::{
    Result,
    decoder::Decoder,
    fold::{Snapshot, View},
    keyring::Keyring,
    page,
    store::Store,
};
use serde_json::Value;
use std::time::Instant;
use tmt_colab_model::{
    attachment::{AttachmentPublication, AttachmentSelector, Descriptor, Source},
    crypto, framing, values,
};

pub(crate) fn validate_attachment_record(
    root: &str,
    key: &str,
    value: &Value,
) -> tmt_colab_model::Result<()> {
    use tmt_colab_model::Invalid;
    if value.get("kind").and_then(Value::as_str) != Some("attachment-publication") {
        return Ok(());
    }
    let record =
        AttachmentPublication::from_json(&serde_json::to_vec(value).map_err(|_| Invalid)?)?;
    if root != "intents" || key != record.attachment_id {
        return Err(Invalid);
    }
    Ok(())
}
/// Internal object-owner seam. Implementations return only complete committed
/// bytes for this exact namespace/key, or an error for unknown/pending objects.
/// This is not a wire DTO or a caller-provided committed flag.
pub trait CommittedObjectVerifier {
    fn read_committed(
        &self,
        namespace: &[u8; 32],
        key: &[u8; 32],
        deadline: Instant,
    ) -> Result<Vec<u8>>;
}
fn remaining(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(page::Fault::Unavailable.into());
    }
    Ok(())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn object_key(d: &Descriptor) -> Result<[u8; 32]> {
    values::object_id(&d.object_id)?;
    let mut key = [0; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&d.object_id[i * 2..i * 2 + 2], 16)?;
    }
    Ok(key)
}
pub fn namespace(space: &str, page: &str) -> Result<[u8; 32]> {
    values::space_id(space)?;
    values::generated_id(page)?;
    Ok(crypto::digest(&framing::frame(&[
        b"tmt-colab-attachment-namespace-v1",
        space.as_bytes(),
        page.as_bytes(),
    ])?))
}
fn revision(key: &Keyring, page: &str, snapshot: &Snapshot) -> Result<String> {
    page::token(
        &key.space_id,
        page,
        &snapshot.authority.head,
        snapshot.epoch,
        &snapshot.cuts,
    )
}
/// The base this descriptor's creation is fenced by now. A document attachment binds the
/// source it was written against, so it needs the whole page revision; a message attachment
/// binds only its writer and message, so it needs the membership head and epoch it was sealed
/// under and nothing a foreign write can move.
pub(crate) fn current_base(
    key: &Keyring,
    descriptor: &Descriptor,
    snapshot: &Snapshot,
) -> Result<String> {
    match &descriptor.source {
        Source::Document { .. } => revision(key, &descriptor.page, snapshot),
        Source::Message { .. } => {
            let head = &snapshot.authority.head;
            Ok(tmt_colab_model::attachment::message_fence(
                &key.space_id,
                &descriptor.page,
                &snapshot.epoch.to_string(),
                &head.revision.to_string(),
                &head.hash,
                &descriptor.author_device,
            )?)
        }
    }
}
/// `current_base` from a fresh owner-store snapshot, for rechecks after slow work.
pub(crate) fn captured_base(
    store: &Store,
    key: &Keyring,
    descriptor: &Descriptor,
) -> Result<String> {
    current_base(
        key,
        descriptor,
        &Snapshot::capture(store, key, &descriptor.page)?,
    )
}
fn descriptor_in(
    view: &View,
    selector: &AttachmentSelector,
    page_revision: &str,
    space: &str,
    page: &str,
    epoch: u64,
) -> Result<Option<Descriptor>> {
    let list = match selector {
        AttachmentSelector::DocumentCurrent {
            content_revision, ..
        } => {
            if content_revision != page_revision {
                return Err(page::Fault::StaleBase.into());
            }
            view.meta.get("attachments")
        }
        AttachmentSelector::Message {
            writer_id,
            message_id,
            message_revision,
            ..
        } => {
            let Some(roots) = view.own.get(writer_id) else {
                return Ok(None);
            };
            let message = &roots["messages"][format!("{message_id}:{message_revision}")];
            let message_epoch = epoch.to_string();
            if message["kind"] != "comment"
                || message["deleted"] != false
                || message["senderDevice"] != *writer_id
                || message["messageId"] != *message_id
                || message["revision"] != *message_revision
                || message["spaceId"] != space
                || message["pageId"] != page
                || message["epoch"].as_str() != Some(message_epoch.as_str())
            {
                return Ok(None);
            }
            message.get("attachments")
        }
    };
    let (id, hash) = selector.attachment();
    let Some(list) = list.and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut found = None;
    for value in list {
        let d = Descriptor::from_json(&serde_json::to_vec(value)?)?;
        if d.attachment_id != id {
            continue;
        }
        if found.is_some() || hex(&d.hash()?) != hash || d.space != space || d.page != page {
            return Err(page::Fault::Invalid.into());
        }
        found = Some(d);
    }
    Ok(found)
}
pub(crate) fn creation_proof(view: &View, descriptor: &Descriptor) -> Result<()> {
    // This projection came only from authenticated positive-sequence own
    // envelopes/checkpoints, with every later revocation prefix checked by fold.
    if !view.status_writers.contains(&descriptor.author_device) {
        return Err(page::Fault::Denied.into());
    }
    let roots = view
        .own
        .get(&descriptor.author_device)
        .ok_or(page::Fault::Denied)?;
    let value = &roots["intents"][&descriptor.attachment_id];
    let proof = AttachmentPublication::from_json(&serde_json::to_vec(value)?)?;
    proof.matches_descriptor(descriptor)?;
    Ok(())
}
/// The existing registration/reader owner constructs this identity for its
/// actual principal. It is not deserializable or an object-policy credential.
#[derive(Clone)]
pub struct SessionReadOwner {
    admission: crate::registration::OwnerAdmission,
    principal: String,
    scope: crate::sync::SyncScope,
    context: [u8; 32],
}
impl crate::registration::OwnerAdmission {
    pub fn attachment_read_owner(
        &self,
        principal: &str,
        scope: &crate::sync::SyncScope,
    ) -> Result<SessionReadOwner> {
        let context =
            self.attachment_context(principal, scope, values::decimal(&scope.epoch, false)?)?;
        Ok(SessionReadOwner {
            admission: self.clone(),
            principal: principal.into(),
            scope: scope.clone(),
            context,
        })
    }
}
impl SessionReadOwner {
    fn admit_epoch(&self, epoch: u64) -> Result<()> {
        if self
            .admission
            .attachment_context(&self.principal, &self.scope, epoch)?
            != self.context
        {
            return Err(page::Fault::Denied.into());
        }
        Ok(())
    }
}
/// One immutable root-local read capture. It cannot be decoded from JSON or
/// reused as a remote reader permit; slice 3 binds callbacks to their actual peer.
pub struct AdmittedAttachmentRead {
    descriptor: Descriptor,
    snapshot: Snapshot,
    current_revision: String,
    creator_key: [u8; 32],
    deadline: Instant,
    session: Option<SessionReadOwner>,
}
impl AdmittedAttachmentRead {
    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    pub fn recheck(&self, store: &Store, key: &Keyring) -> Result<()> {
        remaining(self.deadline)?;
        if let Some(session) = &self.session {
            session.admit_epoch(self.snapshot.epoch)?;
        }
        if self.descriptor.space != key.space_id {
            return Err(page::Fault::Denied.into());
        }
        let current = Snapshot::capture_read(store, key, &self.descriptor.page, None)?;
        let original =
            Snapshot::capture_read(store, key, &self.descriptor.page, Some(self.snapshot.epoch))?;
        if original.cuts != self.snapshot.cuts
            || revision(key, &self.descriptor.page, &current)? != self.current_revision
        {
            return Err(page::Fault::StaleBase.into());
        }
        Ok(())
    }
    /// All owner transactions and decoder children have ended before I/O. No
    /// plaintext is returned if authority/cuts move during the committed read.
    pub fn disclose(
        &self,
        store: &Store,
        key: &Keyring,
        objects: &dyn CommittedObjectVerifier,
    ) -> Result<Vec<u8>> {
        self.recheck(store, key)?;
        let raw = objects.read_committed(
            &namespace(&self.descriptor.space, &self.descriptor.page)?,
            &object_key(&self.descriptor)?,
            self.deadline,
        )?;
        self.recheck(store, key)?;
        let mut plaintext = self.descriptor.open(
            &raw,
            &self.descriptor.context(),
            &self.snapshot.secret,
            &self.creator_key,
        )?;
        if let Err(error) = self.recheck(store, key) {
            plaintext.fill(0);
            return Err(error);
        }
        Ok(plaintext)
    }
}
/// Actual root-local keyring admission, distinct from owner Session/readers.
/// It scans at most the existing 64-epoch owner history window for exact records.
pub fn capture_root_local(
    store: &Store,
    key: &Keyring,
    page: &str,
    selector: &AttachmentSelector,
    decoder: &mut Decoder,
    deadline: Instant,
) -> Result<AdmittedAttachmentRead> {
    capture_read(store, key, page, selector, decoder, deadline, None)
}
/// Owner/reader Session admission, distinct from actual root-local access.
/// The private adapter additionally binds bus/origin and peer generation.
pub fn capture_session(
    store: &Store,
    key: &Keyring,
    owner: &SessionReadOwner,
    selector: &AttachmentSelector,
    decoder: &mut Decoder,
    deadline: Instant,
) -> Result<AdmittedAttachmentRead> {
    if owner.scope.space != key.space_id {
        return Err(page::Fault::Denied.into());
    }
    capture_read(
        store,
        key,
        &owner.scope.page,
        selector,
        decoder,
        deadline,
        Some(owner.clone()),
    )
}
fn capture_read(
    store: &Store,
    key: &Keyring,
    page: &str,
    selector: &AttachmentSelector,
    decoder: &mut Decoder,
    deadline: Instant,
    session: Option<SessionReadOwner>,
) -> Result<AdmittedAttachmentRead> {
    selector.validate()?;
    remaining(deadline)?;
    let current = Snapshot::capture_read(store, key, page, None)?;
    let current_revision = revision(key, page, &current)?;
    let epoch = current.epoch;
    if let Some(session) = &session {
        session.admit_epoch(epoch)?;
    }
    let view = current.materialize_until(key, page, decoder, deadline)?;
    let mut descriptor = descriptor_in(
        &view,
        selector,
        &current_revision,
        &key.space_id,
        page,
        epoch,
    )?;
    if descriptor.is_none() && matches!(selector, AttachmentSelector::Message { .. }) {
        for old in (epoch.saturating_sub(63).max(1)..epoch).rev() {
            remaining(deadline)?;
            if let Some(session) = &session
                && session.admit_epoch(old).is_err()
            {
                continue;
            }
            let snapshot = Snapshot::capture_read(store, key, page, Some(old))?;
            if revision(key, page, &Snapshot::capture_read(store, key, page, None)?)?
                != current_revision
            {
                return Err(page::Fault::StaleBase.into());
            }
            let view = snapshot.materialize_until(key, page, decoder, deadline)?;
            descriptor =
                descriptor_in(&view, selector, &current_revision, &key.space_id, page, old)?;
            if descriptor.is_some() {
                break;
            }
        }
    }
    let descriptor = descriptor.ok_or(page::Fault::Missing)?;
    let asset_epoch = values::decimal(&descriptor.epoch, false)?;
    if let Some(session) = &session {
        session.admit_epoch(asset_epoch)?;
    }
    let snapshot = Snapshot::capture_read(store, key, page, Some(asset_epoch))?;
    let view = snapshot.materialize_until(key, page, decoder, deadline)?;
    creation_proof(&view, &descriptor)?;
    let creator_key = snapshot.asset_author(key, &descriptor)?;
    let admitted = AdmittedAttachmentRead {
        descriptor,
        snapshot,
        current_revision,
        creator_key,
        deadline,
        session,
    };
    admitted.recheck(store, key)?;
    Ok(admitted)
}
/// Before object allocation, bind an upload to the actual mounted writer and
/// the authenticated target base. This admits no receipt or publication: the
/// complete committed envelope still has to be verified by the consumer.
pub(crate) fn check_upload_target(
    source: &mut crate::page::save::Source,
    principal: &str,
    descriptor: &Descriptor,
    base: &str,
    deadline: Instant,
) -> Result<()> {
    descriptor.validate()?;
    remaining(deadline)?;
    if descriptor.author_device != principal || descriptor.space != source.keyring.space_id {
        return Err(page::Fault::Denied.into());
    }
    let snapshot = Snapshot::capture(&source.store, &source.keyring, &descriptor.page)?;
    if descriptor.epoch != snapshot.epoch.to_string()
        || descriptor.membership_revision != snapshot.authority.head.revision.to_string()
        || current_base(&source.keyring, descriptor, &snapshot)? != base
    {
        return Err(page::Fault::StaleBase.into());
    }
    snapshot.asset_author(&source.keyring, descriptor)?;
    // Decoding is outside the Registration/sync locks. Subsequent callbacks
    // recheck this exact base, so they need not decode the same target again.
    match &descriptor.source {
        Source::Document { source_digest } => {
            let view = snapshot.materialize_until(
                &source.keyring,
                &descriptor.page,
                &mut source.decoder,
                deadline,
            )?;
            if *source_digest != hex(&crypto::digest(view.source.as_bytes())) {
                return Err(page::Fault::StaleBase.into());
            }
        }
        Source::Message { writer_id, .. } if writer_id == principal => {}
        _ => return Err(page::Fault::Denied.into()),
    }
    remaining(deadline)?;
    if captured_base(&source.store, &source.keyring, descriptor)? != base {
        return Err(page::Fault::StaleBase.into());
    }
    Ok(())
}

/// Prepare a creation proof only after the exact committed bytes pass crypto
/// and the captured target/base still matches. The existing publication owner
/// signs/fences the result; no second writer or automatic publication is added.
pub struct PublicationIntent<'a> {
    pub descriptor: &'a Descriptor,
    pub base: &'a str,
}
pub fn prepare_publication(
    store: &Store,
    key: &Keyring,
    intent: PublicationIntent<'_>,
    objects: &dyn CommittedObjectVerifier,
    decoder: &mut Decoder,
    deadline: Instant,
    now: u64,
) -> Result<page::FrozenPublication> {
    let PublicationIntent { descriptor, base } = intent;
    descriptor.validate()?;
    remaining(deadline)?;
    let snapshot = Snapshot::capture(store, key, &descriptor.page)?;
    let current_revision = current_base(key, descriptor, &snapshot)?;
    if current_revision != base
        || descriptor.space != key.space_id
        || descriptor.author_device != key.local_writer()?.0
        || descriptor.membership_revision != snapshot.authority.head.revision.to_string()
    {
        return Err(page::Fault::StaleBase.into());
    }
    let view = snapshot.materialize_until(key, &descriptor.page, decoder, deadline)?;
    match &descriptor.source {
        Source::Document { source_digest }
            if *source_digest == hex(&crypto::digest(view.source.as_bytes())) => {}
        Source::Message { writer_id, .. } if *writer_id == descriptor.author_device => {}
        _ => return Err(page::Fault::Invalid.into()),
    }
    let creator_key = snapshot.asset_author(key, descriptor)?;
    let raw = objects.read_committed(
        &namespace(&descriptor.space, &descriptor.page)?,
        &object_key(descriptor)?,
        deadline,
    )?;
    let mut plaintext =
        descriptor.open(&raw, &descriptor.context(), &snapshot.secret, &creator_key)?;
    plaintext.fill(0);
    remaining(deadline)?;
    if captured_base(store, key, descriptor)? != current_revision {
        return Err(page::Fault::StaleBase.into());
    }
    let proof = AttachmentPublication {
        version: 1,
        kind: "attachment-publication".into(),
        space_id: descriptor.space.clone(),
        page_id: descriptor.page.clone(),
        epoch: descriptor.epoch.clone(),
        sender_device: descriptor.author_device.clone(),
        membership_revision: descriptor.membership_revision.clone(),
        attachment_id: descriptor.attachment_id.clone(),
        descriptor_hash: hex(&descriptor.hash()?),
        source: descriptor.source.clone(),
        base_revision: current_revision,
    };
    proof.validate()?;
    let record = crate::decoder::OwnRecord {
        root: "intents".into(),
        key: descriptor.attachment_id.clone(),
        value: serde_json::to_value(proof)?,
    };
    let publication = page::freeze_own_records(
        key,
        &descriptor.page,
        &snapshot,
        &view,
        &[record],
        decoder,
        now,
    )?;
    remaining(deadline)?;
    Ok(publication)
}

impl Drop for AdmittedAttachmentRead {
    fn drop(&mut self) {
        self.snapshot.secret.fill(0);
    }
}
