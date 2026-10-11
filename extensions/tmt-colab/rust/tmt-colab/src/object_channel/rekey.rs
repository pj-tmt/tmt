//! Re-seal document attachments under the page's current epoch (#2293). `epoch.advance` has
//! already committed and narrowed access; the attachments still reference objects sealed under the
//! old epoch key. Each one is read back through the root-local read, sealed again under the
//! current epoch as the local writer, uploaded, proven, and swapped for its old list entry in one
//! write. Until that write, readers see the old complete reference; after it, the new one.
//!
//! This reuses the native attach pipeline: a re-seal is a slot whose `replaces` names the old
//! attachment, so its frozen transfer, resume and stale-base rules are the attach rules. A foreign
//! write between the advance and the swap makes the attempt stale; it is derived again from the
//! new page under a fixed bound and never overwrites the edit.
//! A second ordinary owner Save repairs page references from the current source. The slot keeps
//! its replacement mapping until that Save is settled, including after a crash or lost reply.
use super::{
    ChannelOwner,
    attach::{Attached, Error, Publish, is},
};
use crate::{
    Result,
    attachments::{
        rekey,
        slots::{StagingSlot, StagingSlots},
    },
    page,
};
use std::time::{Duration, Instant};
use tmt_colab_model::{
    attachment::{AttachmentSelector, Descriptor},
    crypto,
};

/// Attachments one pass re-seals at most; the rest wait for the next pass.
pub(crate) const BATCH: usize = 4;
/// How many times one attachment is derived again after the page moved under it.
const STALE_TRIES: usize = 3;

/// What a pass found and did, for the page's status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pass {
    /// Attachments swapped to the current epoch with their reference repair settled.
    pub swapped: usize,
    /// Attachment replacements or reference repairs still waiting after the pass.
    pub waiting: usize,
    /// Attachment replacements or reference Saves whose attempt failed.
    pub failed: usize,
}

/// What a re-seal needs from the object channel: the old plaintext, and a slot attached.
pub(super) trait Lane {
    fn plaintext(&self, selector: &AttachmentSelector) -> Result<Vec<u8>>;
    fn attach(&self, slot: StagingSlot, expected: Option<[u8; 32]>) -> Result<Attached>;
    fn publish(
        &self,
        key: &crate::keyring::Keyring,
        frozen: &page::FrozenPublication,
    ) -> Result<page::Published>;
}
/// The serve's established channel.
struct Established<'a> {
    owner: &'a ChannelOwner,
    page: &'a str,
    source: &'a page::save::SourceOpener,
    publisher: &'a dyn Publish,
    deadline: Instant,
}
impl Lane for Established<'_> {
    fn publish(
        &self,
        key: &crate::keyring::Keyring,
        frozen: &page::FrozenPublication,
    ) -> Result<page::Published> {
        self.publisher.publish(key, frozen)
    }
    fn plaintext(&self, selector: &AttachmentSelector) -> Result<Vec<u8>> {
        Ok(self
            .owner
            .read_root_local(self.page, selector, self.source.clone(), self.deadline)?
            .0)
    }
    fn attach(&self, slot: StagingSlot, expected: Option<[u8; 32]>) -> Result<Attached> {
        self.owner.attach_root_local(
            slot,
            expected,
            self.source.clone(),
            self.publisher,
            self.deadline,
        )
    }
}

impl ChannelOwner {
    /// One bounded pass over a page. Unfinished reference repairs precede further attachment
    /// replacements; otherwise a failed replacement does not stop the others. Nothing reverts
    /// the page's key change.
    pub(crate) fn rekey_page(
        &self,
        page_id: &str,
        slots: &StagingSlots,
        source: &page::save::SourceOpener,
        publisher: &dyn Publish,
        budget: Duration,
    ) -> Result<Pass> {
        let deadline = Instant::now() + budget;
        let lane = Established {
            owner: self,
            page: page_id,
            source,
            publisher,
            deadline,
        };
        pass(&lane, page_id, slots, source, deadline)
    }
}
pub(super) fn pass(
    lane: &dyn Lane,
    page_id: &str,
    slots: &StagingSlots,
    source: &page::save::SourceOpener,
    deadline: Instant,
) -> Result<Pass> {
    let mut pass = Pass::default();
    let mut repaired = 0;
    for id in slots.repairs(page_id)? {
        if repaired >= BATCH || Instant::now() >= deadline {
            pass.waiting += 1;
            continue;
        }
        match repair(lane, slots, &id, source, deadline) {
            Ok(true) => repaired += 1,
            Ok(false) => {} // Its attachment swap has not committed yet.
            Err(_) => pass.waiting += 1,
        }
    }
    // An earlier mapping must be settled before another swap can replace its new ID.
    if pass.waiting > 0 {
        return Ok(pass);
    }
    for (index, old) in rekey::pending(source, page_id, deadline)?
        .iter()
        .enumerate()
    {
        if index + repaired >= BATCH || Instant::now() >= deadline {
            pass.waiting += 1;
        } else if one(lane, page_id, old, slots, source, deadline).is_ok() {
            pass.swapped += 1;
        } else {
            pass.failed += 1;
            pass.waiting += 1;
        }
    }
    Ok(pass)
}
/// Re-seal one attachment, resuming the transfer an earlier pass froze when there is one.
fn one(
    lane: &dyn Lane,
    page_id: &str,
    old: &Descriptor,
    slots: &StagingSlots,
    source: &page::save::SourceOpener,
    deadline: Instant,
) -> Result<()> {
    let mut last: Error = page::Fault::Unavailable.into();
    for _ in 0..STALE_TRIES {
        let id = match slots.replacing(page_id, &old.attachment_id)? {
            Some(id) => id,
            None => stage(lane, page_id, old, slots, source, deadline)?,
        };
        let claim = slots.claim(&id, deadline)?;
        let slot = slots.load(&id)?;
        let expected = match slot.record.sealed {
            Some(_) => None,
            None => Some(crypto::digest(&slot.read_source()?)),
        };
        let result = lane.attach(slot, expected);
        drop(claim);
        match result {
            Ok(_) => {
                repair(lane, slots, &id, source, deadline)?;
                return Ok(());
            }
            // The old entry left the list meanwhile: nothing is left to re-seal.
            Err(error) if is(&error, page::Fault::Missing) => return Ok(()),
            // The page moved and the slot is gone: derive the swap again from the new page.
            Err(error) if is(&error, page::Fault::StaleBase) => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}
/// Read the current page before every attempt. A lost Save reply keeps the mapping; the next
/// read observes either the repaired source or the latest edit, never blindly repeats a Save.
fn repair(
    lane: &dyn Lane,
    slots: &StagingSlots,
    id: &str,
    source: &page::save::SourceOpener,
    deadline: Instant,
) -> Result<bool> {
    let _claim = slots.claim(id, deadline)?;
    let mut slot = slots.load(id)?;
    let old = slot.record.replaces.clone().ok_or(page::Fault::Invalid)?;
    tmt_colab_model::values::generated_id(&old)?;
    let descriptor = slot
        .record
        .sealed
        .as_ref()
        .ok_or(page::Fault::Invalid)?
        .descriptor
        .clone();
    for _ in 0..STALE_TRIES {
        if Instant::now() >= deadline {
            return Err(page::Fault::Unavailable.into());
        }
        let mut view = source()?;
        let deleted =
            view.store
                .owner_read(&view.keyring.space_id, &view.keyring.owner_public(), |tx| {
                    Ok(tx
                        .page_policy_at(
                            &slot.record.page,
                            tx.head().ok_or(page::Fault::Invalid)?.revision,
                        )?
                        .deleted)
                })?;
        if deleted {
            slot.dispose()?;
            return Ok(true);
        }
        // Source equality alone cannot detect an attachment-only removal. Keep the complete
        // page revision as the ordinary publication's preparation fence as well.
        let revision = page::revision(&view.store, &view.keyring, &slot.record.page)?;
        let snapshot = page::snapshot(&view.store, &view.keyring, &slot.record.page, true)?;
        let folded = snapshot.materialize_until(
            &view.keyring,
            &slot.record.page,
            &mut view.decoder,
            deadline,
        )?;
        let listed: Vec<_> = folded
            .meta
            .get("attachments")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .map(|item| -> Result<Descriptor> {
                Ok(Descriptor::from_json(&serde_json::to_vec(item)?)?)
            })
            .collect::<Result<_>>()?;
        let Some(current) = listed
            .iter()
            .find(|item| item.attachment_id == descriptor.attachment_id)
        else {
            if listed.iter().any(|item| item.attachment_id == old) {
                // This original never swapped and outlived Remote's upload expiry. Normal age
                // cleanup can discard it; a later pass derives a fresh replacement.
                if crate::registration::now_ms()?.saturating_sub(slot.record.created_ms)
                    >= crate::attachments::slots::STARTED_AGE_MS
                {
                    slot.repaired()?;
                }
                return Ok(false);
            }
            // Both attachments were removed; no references may be recreated.
            slot.dispose()?;
            return Ok(true);
        };
        if current != &descriptor {
            return Err(page::Fault::Invalid.into());
        }
        // The replacement may itself now be from an earlier epoch. Reuse attachment read
        // admission, which checks its original creation proof and the current page reference.
        let admitted = crate::attachments::capture_root_local(
            &view.store,
            &view.keyring,
            &slot.record.page,
            &AttachmentSelector::DocumentCurrent {
                attachment_id: descriptor.attachment_id.clone(),
                descriptor_hash: descriptor
                    .hash()?
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                content_revision: revision.clone(),
            },
            &mut view.decoder,
            deadline,
        )?;
        if admitted.descriptor() != &descriptor {
            return Err(page::Fault::Invalid.into());
        }
        // Recover a swap committed before finish, dropping any leftover file bytes first.
        if slot.record.done_ms.is_none() {
            slot.finish(crate::registration::now_ms()?)?;
        }
        let repaired =
            rekey::repair_source(&folded.source, &old, &descriptor.attachment_id, &listed);
        if repaired == folded.source {
            slot.repaired()?;
            return Ok(true);
        }
        let result = (|| -> Result<()> {
            match page::prepare_publication_with_clock(
                &view.store,
                &view.keyring,
                &slot.record.page,
                crate::decoder::ContentEdit {
                    source: &repaired,
                    publisher_agent: None,
                    attachments: None,
                },
                page::PublishOptions {
                    expected_revision: Some(&revision),
                    base_source_sha256: Some(crypto::digest(folded.source.as_bytes())),
                    operation_id: Some(&page::fresh_id()?),
                },
                &mut view.decoder,
                &|| Ok(crate::registration::now_ms()?),
            )? {
                page::PublicationPreparation::Noop { .. } => Ok(()),
                page::PublicationPreparation::Write(frozen) => {
                    let published = lane.publish(&view.keyring, &frozen)?;
                    match published.record.outcome {
                        crate::publication::Outcome::Committed { .. } => Ok(()),
                        crate::publication::Outcome::Rejected { code, .. } => {
                            Err(page::Fault::from(code).into())
                        }
                        crate::publication::Outcome::Unknown { .. } => {
                            Err(page::Fault::Unavailable.into())
                        }
                    }
                }
            }
        })();
        match result {
            Ok(()) => {
                slot.repaired()?;
                return Ok(true);
            }
            Err(error) if is(&error, page::Fault::StaleBase) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(page::Fault::StaleBase.into())
}
/// A slot holding the old attachment's plaintext, read back through the root-local read.
fn stage(
    lane: &dyn Lane,
    page_id: &str,
    old: &Descriptor,
    slots: &StagingSlots,
    source: &page::save::SourceOpener,
    deadline: Instant,
) -> Result<String> {
    let selector = {
        let view = source()?;
        AttachmentSelector::DocumentCurrent {
            attachment_id: old.attachment_id.clone(),
            descriptor_hash: old.hash()?.iter().map(|b| format!("{b:02x}")).collect(),
            content_revision: page::revision(&view.store, &view.keyring, page_id)?,
        }
    };
    if Instant::now() >= deadline {
        return Err(page::Fault::Unavailable.into());
    }
    let mut plaintext = lane.plaintext(&selector)?;
    let slot = slots.create_replacing(
        page_id,
        &old.filename,
        &old.attachment_id,
        crate::registration::now_ms().map_err(|_| page::Fault::Unavailable)?,
    );
    let staged = slot.and_then(|slot| slot.stage_source(&plaintext).map(|()| slot));
    plaintext.fill(0);
    let slot = staged?;
    Ok(slot.id().to_owned())
}
