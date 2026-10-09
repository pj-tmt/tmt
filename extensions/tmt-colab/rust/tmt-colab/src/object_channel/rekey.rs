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
    /// Attachments swapped to the current epoch.
    pub swapped: usize,
    /// Attachments still on an earlier epoch after the pass.
    pub waiting: usize,
    /// Attachments whose attempt failed and kept their old reference.
    pub failed: usize,
}

/// What a re-seal needs from the object channel: the old plaintext, and a slot attached.
pub(super) trait Lane {
    fn plaintext(&self, selector: &AttachmentSelector) -> Result<Vec<u8>>;
    fn attach(&self, slot: StagingSlot, expected: Option<[u8; 32]>) -> Result<Attached>;
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
    fn plaintext(&self, selector: &AttachmentSelector) -> Result<Vec<u8>> {
        self.owner
            .read_root_local(self.page, selector, self.source.clone(), self.deadline)
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
    /// One bounded pass over a page. A failure of one attachment keeps its old reference and does
    /// not stop the others; nothing here can revert the advance.
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
    for (index, old) in rekey::pending(source, page_id, deadline)?
        .iter()
        .enumerate()
    {
        if index >= BATCH || Instant::now() >= deadline {
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
            Ok(_) => return Ok(()),
            // The old entry left the list meanwhile: nothing is left to re-seal.
            Err(error) if is(&error, page::Fault::Missing) => return Ok(()),
            // The page moved and the slot is gone: derive the swap again from the new page.
            Err(error) if is(&error, page::Fault::StaleBase) => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
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
        &old.media_type,
        &old.attachment_id,
        crate::registration::now_ms().map_err(|_| page::Fault::Unavailable)?,
    );
    let staged = slot.and_then(|slot| slot.stage_source(&plaintext).map(|()| slot));
    plaintext.fill(0);
    let slot = staged?;
    Ok(slot.id().to_owned())
}
