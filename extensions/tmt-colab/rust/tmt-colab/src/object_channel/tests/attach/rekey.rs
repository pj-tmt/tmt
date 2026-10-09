//! Re-sealing document attachments under the current epoch (#2293), against a real owner store
//! with the object channel replaced by an in-memory backend. Every case checks the page: either
//! the old complete reference or the new complete one, never a half swap, and never a lost edit.
use super::*;
use crate::{
    attachments::rekey as pending,
    object_channel::rekey::{self, Lane, Pass},
};
use tmt_colab_model::attachment::AttachmentSelector;
use tmt_extension_objects::Limit;

/// The serve's channel with the old plaintext known and the attach run on the fake backend.
struct Test<'a> {
    flow: &'a Flow,
}
impl Lane for Test<'_> {
    fn plaintext(&self, _: &AttachmentSelector) -> crate::Result<Vec<u8>> {
        Ok(Flow::FILE.to_vec())
    }
    fn attach(
        &self,
        mut slot: StagingSlot,
        expected: Option<[u8; 32]>,
    ) -> crate::Result<flow::Attached> {
        let result = self.flow.run(&mut slot, expected);
        flow::settle(slot, result)
    }
}
impl Flow {
    /// The page's epoch moves on: every attachment now waits for a re-seal.
    fn advance(&mut self) {
        self.world
            .state
            .apply(OwnerAction::EpochAdvance { page: PAGE });
    }
    /// A fresh object, as a second upload to a backend that kept the first transfer's state.
    fn forget_transfer(&self) {
        *self.objects.backend.transfer_state() = fake::Transfer::Unobserved;
        *self.objects.backend.counts.lock().unwrap() = fake::Counts::default();
        *self.objects.backend.calls.lock().unwrap() = 0;
        *self.objects.backend.lose_reply_at.lock().unwrap() = None;
    }
    /// An attachment sealed and listed under the page's current epoch.
    fn attached(&self) -> Descriptor {
        let (mut slot, expected) = self.staged();
        let attached = self.run(&mut slot, Some(expected)).unwrap();
        slot.finish(2000).unwrap();
        attached.descriptor
    }
    fn rekey(&self) -> Pass {
        let deadline = Instant::now() + Duration::from_secs(20);
        rekey::pass(
            &Test { flow: self },
            PAGE,
            &self.slots,
            &self.world.source,
            deadline,
        )
        .unwrap()
    }
    fn waiting(&self) -> Vec<Descriptor> {
        pending::pending(
            &self.world.source,
            PAGE,
            Instant::now() + Duration::from_secs(20),
        )
        .unwrap()
    }
    /// The document's attachment list as the page has it.
    fn listed(&self) -> Vec<Descriptor> {
        let mut view = (self.world.source)().unwrap();
        let snapshot = page::snapshot(&view.store, &view.keyring, PAGE, true).unwrap();
        let folded = snapshot
            .materialize_until(
                &view.keyring,
                PAGE,
                &mut view.decoder,
                Instant::now() + Duration::from_secs(20),
            )
            .unwrap();
        folded
            .meta
            .get("attachments")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .map(|item| Descriptor::from_json(&serde_json::to_vec(item).unwrap()).unwrap())
            .collect()
    }
    fn epoch(&self) -> String {
        let view = (self.world.source)().unwrap();
        page::snapshot(&view.store, &view.keyring, PAGE, true)
            .unwrap()
            .epoch
            .to_string()
    }
    fn source_text(&self) -> String {
        let mut view = (self.world.source)().unwrap();
        page::read(&view.store, &view.keyring, PAGE, &mut view.decoder)
            .unwrap()
            .source
    }
}

#[test]
fn a_rekey_swaps_an_earlier_epoch_attachment_for_a_current_one_in_one_write() {
    let mut flow = Flow::new();
    let old = flow.attached();
    assert!(flow.waiting().is_empty(), "sealed under the current epoch");
    flow.forget_transfer();
    flow.advance();
    assert_eq!(flow.waiting().len(), 1);
    assert_eq!(flow.listed(), vec![old.clone()], "the old reference stays");
    let before = flow.published();
    assert_eq!(
        flow.rekey(),
        Pass {
            swapped: 1,
            waiting: 0,
            failed: 0
        }
    );
    assert_eq!(flow.published() - before, 2, "the proof, then the swap");
    let listed = flow.listed();
    assert_eq!(listed.len(), 1, "swapped, not added beside");
    assert_ne!(listed[0].attachment_id, old.attachment_id);
    assert_eq!(listed[0].epoch, flow.epoch());
    assert_eq!(
        (&listed[0].filename, &listed[0].media_type),
        (&old.filename, &old.media_type)
    );
    // Nothing waits any more, so a second pass changes nothing.
    assert!(flow.waiting().is_empty());
    assert_eq!(flow.rekey(), Pass::default());
    assert_eq!(flow.published() - before, 2);
}
#[test]
fn a_refused_rekey_keeps_the_old_reference_and_a_later_pass_finishes_it_with_the_same_transfer() {
    let mut flow = Flow::new();
    let old = flow.attached();
    flow.forget_transfer();
    flow.advance();
    flow.objects
        .backend
        .refuse_begin_with(ErrorCode::Capacity(Limit::ActiveIntents));
    let before = flow.published();
    assert_eq!(
        flow.rekey(),
        Pass {
            swapped: 0,
            waiting: 1,
            failed: 1
        }
    );
    assert_eq!(
        flow.listed(),
        vec![old.clone()],
        "the old complete reference"
    );
    assert_eq!(flow.published(), before, "nothing was published");
    let held = flow
        .slots
        .replacing(PAGE, &old.attachment_id)
        .unwrap()
        .unwrap();
    let frozen = flow.slots.load(&held).unwrap().record.sealed.unwrap();
    // The quota clears: the same frozen original is uploaded and swapped in.
    flow.objects.backend.accept_begin();
    assert_eq!(flow.rekey().swapped, 1);
    let finished = flow.slots.load(&held).unwrap();
    assert_eq!(
        finished.record.sealed.unwrap().transfer_id,
        frozen.transfer_id,
        "the original transfer ID"
    );
    assert_eq!(flow.begins(), 1);
    assert_eq!(
        flow.listed()[0].attachment_id,
        frozen.descriptor.attachment_id
    );
}
#[test]
fn an_interrupted_rekey_resumes_the_frozen_transfer_without_a_second_object() {
    let mut flow = Flow::new();
    flow.attached();
    flow.forget_transfer();
    flow.advance();
    // The first pass sealed, began and sent its part, then lost the reply.
    flow.objects.backend.lose_reply_at(3);
    assert_eq!(flow.rekey().failed, 1);
    assert_eq!(flow.waiting().len(), 1, "still the old reference");
    let held = flow.slots.replacing(PAGE, &flow.waiting()[0].attachment_id);
    assert!(held.unwrap().is_some(), "the frozen slot is kept");
    assert_eq!(flow.rekey().swapped, 1);
    assert_eq!(flow.begins(), 1, "one begin across both passes");
    assert!(flow.waiting().is_empty());
}
#[test]
fn a_foreign_edit_before_the_swap_is_never_overwritten_and_the_swap_is_derived_again() {
    let mut flow = Flow::new();
    let old = flow.attached();
    flow.forget_transfer();
    flow.advance();
    let source = Arc::clone(&flow.world.source);
    let server = Arc::clone(&flow.writer.server);
    let published = Arc::clone(&flow.writer.published);
    *flow.objects.before_upload.lock().unwrap() =
        Some(Box::new(move || edit_page(&source, &server, &published)));
    assert_eq!(
        flow.rekey(),
        Pass {
            swapped: 1,
            waiting: 0,
            failed: 0
        }
    );
    assert!(
        flow.source_text().ends_with("<p>edited</p>"),
        "the author's edit survives the swap"
    );
    let listed = flow.listed();
    assert_eq!(listed.len(), 1);
    assert_ne!(listed[0].attachment_id, old.attachment_id);
    assert_eq!(listed[0].epoch, flow.epoch());
    assert_eq!(
        flow.objects.discards.load(Ordering::SeqCst),
        1,
        "the original sealed against the old base is discarded"
    );
}
#[test]
fn an_advance_during_a_rekey_discards_the_stale_original_and_seals_for_the_newer_epoch() {
    let mut flow = Flow::new();
    flow.attached();
    flow.forget_transfer();
    flow.advance();
    flow.objects.backend.lose_reply_at(3);
    assert_eq!(flow.rekey().failed, 1);
    let second = flow.epoch();
    flow.advance();
    assert_ne!(flow.epoch(), second);
    flow.forget_transfer();
    assert_eq!(flow.rekey().swapped, 1);
    let listed = flow.listed();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].epoch, flow.epoch(), "sealed for the newest epoch");
    assert!(flow.waiting().is_empty());
}
#[test]
fn a_rekey_pass_swaps_at_most_a_batch_and_the_rest_wait_for_the_next_pass() {
    let mut flow = Flow::new();
    for _ in 0..rekey::BATCH + 1 {
        flow.forget_transfer();
        flow.attached();
    }
    flow.forget_transfer();
    flow.advance();
    let first = flow.rekey();
    assert_eq!(first.swapped, rekey::BATCH);
    assert_eq!(first.waiting, 1);
    flow.forget_transfer();
    assert_eq!(flow.rekey().swapped, 1);
    assert!(flow.waiting().is_empty());
    assert_eq!(flow.listed().len(), rekey::BATCH + 1);
}
