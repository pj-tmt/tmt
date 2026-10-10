//! Attachment reference admission. A descriptor or caller JSON is never authority.
use crate::{
    Result,
    decoder::Decoder,
    export::conversations::{self, Conversations},
    fold::{Snapshot, View},
    keyring::Keyring,
    page,
    store::Store,
};
pub mod attach_ipc;
pub mod ipc;
pub(crate) mod rekey;
pub(crate) mod seal;
pub mod slots;
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
pub(crate) fn revision(key: &Keyring, page: &str, snapshot: &Snapshot) -> Result<String> {
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
        Source::Message { .. } => message_base(key, descriptor, snapshot),
    }
}
/// The membership head, epoch and author a message attachment is fenced by.
fn message_base(key: &Keyring, descriptor: &Descriptor, snapshot: &Snapshot) -> Result<String> {
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
    /// What this read stays valid against (`current_base`): the whole page revision for a
    /// document attachment, which is bound to the source it was written against, and only the
    /// membership head, epoch and author for a message attachment, which nothing a foreign or
    /// unrelated write can move. A read fails when its reference or its authority changed,
    /// never because the page merely advanced.
    base: String,
    /// A message reference is fenced by authority; a document reference by the page revision.
    by_message: bool,
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
        let base = if self.by_message {
            message_base(key, &self.descriptor, &current)?
        } else {
            revision(key, &self.descriptor.page, &current)?
        };
        if base != self.base {
            return Err(page::Fault::StaleBase.into());
        }
        if !self.by_message {
            let original = Snapshot::capture_read(
                store,
                key,
                &self.descriptor.page,
                Some(self.snapshot.epoch),
            )?;
            if original.cuts != self.snapshot.cuts {
                return Err(page::Fault::StaleBase.into());
            }
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
            // Only authority moving makes the scan stale; an unrelated record does not.
            let latest = Snapshot::capture_read(store, key, page, None)?;
            if latest.epoch != current.epoch
                || latest.authority.head.revision != current.authority.head.revision
                || latest.authority.head.hash != current.authority.head.hash
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
    let by_message = matches!(selector, AttachmentSelector::Message { .. });
    let base = if by_message {
        message_base(key, &descriptor, &current)?
    } else {
        revision(key, page, &current)?
    };
    let admitted = AdmittedAttachmentRead {
        descriptor,
        snapshot,
        base,
        by_message,
        creator_key,
        deadline,
        session,
    };
    admitted.recheck(store, key)?;
    Ok(admitted)
}
/// The exact selectors whose attachment ID starts with `prefix` (#2464): the files of live
/// comments in the verified discussion projection, which already owns revision chains, deletion,
/// scope and epoch, and, with a page revision, the page's document attachments. This only finds
/// a reference; the read still verifies everything.
fn candidates(
    conversations: &Conversations,
    meta: &Value,
    prefix: &str,
    space: &str,
    page: &str,
    page_revision: Option<&str>,
) -> Result<Vec<AttachmentSelector>> {
    let mut found = Vec::new();
    if let (Some(revision), Some(list)) = (page_revision, meta.get("attachments")) {
        for value in list.as_array().into_iter().flatten() {
            let d = Descriptor::from_json(&serde_json::to_vec(value)?)?;
            if d.attachment_id.starts_with(prefix) && d.space == space && d.page == page {
                found.push(AttachmentSelector::DocumentCurrent {
                    attachment_id: d.attachment_id.clone(),
                    descriptor_hash: hex(&d.hash()?),
                    content_revision: revision.to_owned(),
                });
            }
        }
    }
    for comment in conversations.threads.iter().flat_map(|t| &t.comments) {
        for file in comment
            .attachments
            .iter()
            .filter(|f| f.id.starts_with(prefix))
        {
            found.push(AttachmentSelector::Message {
                writer_id: comment.writer.clone(),
                message_id: comment.id.clone(),
                message_revision: comment.revision.clone(),
                attachment_id: file.id.clone(),
                descriptor_hash: file.descriptor_hash.clone(),
            });
        }
    }
    Ok(found)
}
/// The discussion projection of one materialized epoch.
fn projected(key: &Keyring, page: &str, snapshot: &Snapshot, view: &View) -> Conversations {
    Conversations::project(
        conversations::Capture {
            space_id: &key.space_id,
            page_id: page,
            title: &view.title,
            epoch: &snapshot.epoch.to_string(),
            head: conversations::Head {
                revision: snapshot.authority.head.revision.to_string(),
                statement_hash: hex(&snapshot.authority.head.hash),
            },
        },
        &view.own,
        &view.signing_keys,
        &view.status_writers,
    )
}
/// Resolve an attachment ID, or an unambiguous prefix of at least 8 hex characters, against the
/// current verified page and the same earlier-epoch window `capture_read` searches. The result is
/// the exact reference `attachment read --reference` takes, so the serve's read, with its access
/// and epoch checks, is unchanged. No match is `Missing`; more than one is `Invalid`.
pub fn resolve(
    store: &Store,
    key: &Keyring,
    page: &str,
    prefix: &str,
    decoder: &mut Decoder,
    deadline: Instant,
) -> Result<AttachmentSelector> {
    use page::Fault;
    if prefix.len() < 8
        || prefix.len() > 36
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
    {
        return Err(Fault::Invalid.into());
    }
    remaining(deadline)?;
    let current = Snapshot::capture_read(store, key, page, None)?;
    let current_revision = revision(key, page, &current)?;
    let epoch = current.epoch;
    let view = current.materialize_until(key, page, decoder, deadline)?;
    let mut found = candidates(
        &projected(key, page, &current, &view),
        &view.meta,
        prefix,
        &key.space_id,
        page,
        Some(&current_revision),
    )?;
    if found.is_empty() {
        for old in (epoch.saturating_sub(63).max(1)..epoch).rev() {
            remaining(deadline)?;
            let snapshot = Snapshot::capture_read(store, key, page, Some(old))?;
            let view = snapshot.materialize_until(key, page, decoder, deadline)?;
            found = candidates(
                &projected(key, page, &snapshot, &view),
                &view.meta,
                prefix,
                &key.space_id,
                page,
                None,
            )?;
            if !found.is_empty() {
                break;
            }
        }
    }
    match found.len() {
        0 => Err(Fault::Missing.into()),
        1 => Ok(found.remove(0)),
        _ => Err(Fault::Invalid.into()),
    }
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

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn descriptor() -> Descriptor {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../../contracts/vectors/attachment-v1.json"
        ))
        .unwrap();
        let case = corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "message-asset")
            .unwrap();
        Descriptor::from_json(case["input"].as_str().unwrap().as_bytes()).unwrap()
    }
    /// The projection of one writer's thread with the given comment records (message ID, revision,
    /// deleted), each listing the descriptor unless deleted.
    fn project(d: &Descriptor, comments: &[(&str, &str, bool)]) -> Conversations {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../contracts/vectors/discussion-v1.json"
        ))
        .unwrap();
        let writer = d.author_device.clone();
        let mut thread = fixture["thread"].clone();
        let mut roots = json!({"threads": {}, "messages": {}, "intents": {}, "replies": {}});
        for (field, value) in [
            ("senderDevice", &writer),
            ("spaceId", &d.space),
            ("pageId", &d.page),
        ] {
            thread[field] = value.as_str().into();
        }
        roots["threads"][format!("{}:1", thread["threadId"].as_str().unwrap())] = thread.clone();
        for (id, revision, deleted) in comments {
            let mut comment = fixture["comment"].clone();
            for (field, value) in [
                ("senderDevice", writer.as_str()),
                ("spaceId", &d.space),
                ("pageId", &d.page),
                ("messageId", id),
                ("revision", revision),
            ] {
                comment[field] = value.into();
            }
            comment["thread"] = json!({"writer": writer, "id": thread["threadId"]});
            comment["deleted"] = (*deleted).into();
            if *deleted {
                comment["body"] = "".into();
            } else {
                comment["attachments"] =
                    json!([serde_json::from_slice::<Value>(&d.to_json().unwrap()).unwrap()]);
            }
            roots["messages"][format!("{id}:{revision}")] = comment;
        }
        let own = BTreeMap::from([(writer.clone(), roots)]);
        let keys = BTreeMap::from([(writer, [0; 32])]);
        Conversations::project(
            conversations::Capture {
                space_id: &d.space,
                page_id: &d.page,
                title: "Files",
                epoch: "1",
                head: conversations::Head {
                    revision: "1".into(),
                    statement_hash: "00".repeat(32),
                },
            },
            &own,
            &keys,
            &BTreeSet::new(),
        )
    }
    fn find(d: &Descriptor, conv: &Conversations, prefix: &str) -> Vec<AttachmentSelector> {
        candidates(conv, &Value::Null, prefix, &d.space, &d.page, None).unwrap()
    }
    const M1: &str = "00000000-0000-4000-8000-0000000000a1";
    const M2: &str = "00000000-0000-4000-8000-0000000000a2";

    #[test]
    fn a_prefix_finds_the_exact_reference_of_a_projected_live_comment_attachment() {
        let d = descriptor();
        let found = find(&d, &project(&d, &[(M1, "1", false)]), &d.attachment_id[..8]);
        assert_eq!(
            found,
            vec![AttachmentSelector::Message {
                writer_id: d.author_device.clone(),
                message_id: M1.into(),
                message_revision: "1".into(),
                attachment_id: d.attachment_id.clone(),
                descriptor_hash: hex(&d.hash().unwrap()),
            }]
        );
        assert!(find(&d, &project(&d, &[(M1, "1", false)]), "ffffffff").is_empty());
    }
    #[test]
    fn a_deleted_message_is_not_in_the_projection_so_it_never_resolves() {
        let d = descriptor();
        let deleted = project(&d, &[(M1, "1", false), (M1, "2", true)]);
        assert!(find(&d, &deleted, &d.attachment_id[..8]).is_empty());
    }
    #[test]
    fn two_messages_listing_one_id_are_ambiguous_and_a_document_needs_the_current_revision() {
        let d = descriptor();
        let two = project(&d, &[(M1, "1", false), (M2, "1", false)]);
        assert_eq!(find(&d, &two, &d.attachment_id[..8]).len(), 2);
        let meta = json!({"attachments": [serde_json::from_slice::<Value>(&d.to_json().unwrap()).unwrap()]});
        let none = project(&d, &[]);
        let id = &d.attachment_id[..8];
        let at = |revision| candidates(&none, &meta, id, &d.space, &d.page, revision).unwrap();
        assert!(at(None).is_empty());
        assert!(matches!(
            at(Some("v1:r"))[..],
            [AttachmentSelector::DocumentCurrent { .. }]
        ));
    }
}
