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
    fn publish(
        &self,
        key: &crate::keyring::Keyring,
        frozen: &page::FrozenPublication,
    ) -> crate::Result<page::Published> {
        self.flow.writer.publish(key, frozen)
    }
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
    fn write_source(&self, text: String) {
        let mut view = (self.world.source)().unwrap();
        let current = page::read(&view.store, &view.keyring, PAGE, &mut view.decoder).unwrap();
        let prepared = page::save::prepare(
            &self.world.source,
            &page::save::Save {
                page: PAGE.into(),
                operation_id: page::fresh_id().unwrap(),
                base_sha256: crypto::digest(current.source.as_bytes()),
                source: text,
                attachments: None,
            },
            &|| Ok(crate::registration::now_ms().unwrap()),
        )
        .unwrap();
        let page::save::Prepared::Write(frozen) = prepared else {
            panic!("changed source")
        };
        self.writer.publish(&view.keyring, &frozen).unwrap();
    }
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
fn the_second_save_repairs_full_and_unique_short_image_references_and_is_idempotent() {
    let mut flow = Flow::new();
    let old = flow.attached();
    let html = format!(
        "<h1>é</h1><img src='colab-attachment:{}'><img src=\"colab-attachment:{}\">🦀",
        old.attachment_id,
        &old.attachment_id[..8]
    );
    flow.write_source(html.clone());
    flow.forget_transfer();
    flow.advance();
    let before = flow.published();
    assert_eq!(flow.rekey().swapped, 1);
    let new = flow.listed()[0].attachment_id.clone();
    assert_eq!(
        flow.source_text(),
        html.replace(&old.attachment_id, &new).replace(
            &format!("colab-attachment:{}\"", &old.attachment_id[..8]),
            &format!("colab-attachment:{new}\"")
        )
    );
    assert_eq!(
        flow.published() - before,
        3,
        "proof, attachment swap, reference Save"
    );
    assert!(flow.slots.repairs(PAGE).unwrap().is_empty());
    assert_eq!(flow.rekey(), Pass::default());
    assert_eq!(flow.published() - before, 3);
}

#[test]
fn a_failed_second_save_keeps_its_mapping_past_ten_minutes_and_a_restart_repairs_it() {
    let mut flow = Flow::new();
    let old = flow.attached();
    flow.write_source(format!(
        "<img src='colab-attachment:{}'>",
        old.attachment_id
    ));
    flow.forget_transfer();
    flow.advance();
    flow.writer
        .fail_from
        .store(flow.published() + 3, Ordering::SeqCst);
    assert_eq!(flow.rekey().waiting, 1);
    let new = flow.listed()[0].attachment_id.clone();
    assert_ne!(new, old.attachment_id);
    assert!(flow.source_text().contains(&old.attachment_id));
    let ids = flow.slots.repairs(PAGE).unwrap();
    assert_eq!(ids.len(), 1);
    let slot = flow.slots.load(&ids[0]).unwrap();
    assert!(!slot.source_path().exists());
    assert!(slot.read_object(100).is_err());
    let now = slot.record.done_ms.unwrap() + crate::attachments::slots::DONE_AGE_MS + 1;
    assert!(flow.slots.sweep(now).unwrap().is_empty());
    assert!(flow.slots.load(&ids[0]).is_ok());
    let layout = crate::keyring::Layout::existing(&flow.world.state.root)
        .unwrap()
        .unwrap();
    flow.slots = StagingSlots::open(&layout).unwrap();
    flow.writer.fail_from.store(usize::MAX, Ordering::SeqCst);
    assert_eq!(flow.rekey(), Pass::default());
    assert_eq!(
        flow.source_text(),
        format!("<img src='colab-attachment:{new}'>")
    );
    assert!(flow.slots.repairs(PAGE).unwrap().is_empty());
    flow.slots.sweep(now).unwrap();
    assert!(flow.slots.load(&ids[0]).is_err());
}

#[test]
fn a_crash_after_the_swap_before_finishing_the_slot_recovers_without_another_upload() {
    let mut flow = Flow::new();
    let old = flow.attached();
    flow.write_source(format!(
        "<img src='colab-attachment:{}'>",
        old.attachment_id
    ));
    flow.forget_transfer();
    flow.advance();
    let mut slot = flow
        .slots
        .create_replacing(PAGE, &old.filename, &old.attachment_id, 1000)
        .unwrap();
    slot.stage_source(Flow::FILE).unwrap();
    let new = flow
        .run(&mut slot, Some(crypto::digest(Flow::FILE)))
        .unwrap()
        .descriptor;
    let id = slot.id().to_owned();
    assert!(flow.slots.load(&id).unwrap().record.done_ms.is_none());
    let layout = crate::keyring::Layout::existing(&flow.world.state.root)
        .unwrap()
        .unwrap();
    flow.slots = StagingSlots::open(&layout).unwrap();
    // Even after a long stop, the unfinished slot must not be swept before its committed swap
    // is reconciled. Its ciphertext goes when the recovered finish is recorded.
    assert!(
        flow.slots
            .sweep(1000 + crate::attachments::slots::STARTED_AGE_MS)
            .unwrap()
            .is_empty()
    );
    let before = flow.published();
    assert_eq!(flow.rekey(), Pass::default());
    assert_eq!(flow.published() - before, 1);
    assert_eq!(flow.begins(), 1);
    let slot = flow.slots.load(&id).unwrap();
    assert!(slot.record.done_ms.is_some() && slot.record.references_repaired);
    assert!(slot.read_object(100).is_err());
    assert_eq!(
        flow.source_text(),
        format!("<img src='colab-attachment:{}'>", new.attachment_id)
    );
}

struct RepairReply<'a> {
    flow: &'a Flow,
    lose: AtomicBool,
    edit: AtomicBool,
}
impl Lane for RepairReply<'_> {
    fn plaintext(&self, selector: &AttachmentSelector) -> crate::Result<Vec<u8>> {
        Test { flow: self.flow }.plaintext(selector)
    }
    fn attach(
        &self,
        slot: StagingSlot,
        expected: Option<[u8; 32]>,
    ) -> crate::Result<flow::Attached> {
        Test { flow: self.flow }.attach(slot, expected)
    }
    fn publish(
        &self,
        key: &crate::keyring::Keyring,
        frozen: &page::FrozenPublication,
    ) -> crate::Result<page::Published> {
        if self.edit.swap(false, Ordering::SeqCst) {
            self.flow.edit();
        }
        let result = self.flow.writer.publish(key, frozen)?;
        if self.lose.swap(false, Ordering::SeqCst) {
            return Err(page::Fault::Unavailable.into());
        }
        Ok(result)
    }
}

#[test]
fn stale_and_unknown_reference_saves_are_settled_from_the_current_page() {
    for lose in [false, true] {
        let mut flow = Flow::new();
        let old = flow.attached();
        flow.write_source(format!(
            "<img src='colab-attachment:{}'>",
            old.attachment_id
        ));
        flow.forget_transfer();
        flow.advance();
        let lane = RepairReply {
            flow: &flow,
            lose: AtomicBool::new(lose),
            edit: AtomicBool::new(!lose),
        };
        let first = rekey::pass(
            &lane,
            PAGE,
            &flow.slots,
            &flow.world.source,
            Instant::now() + Duration::from_secs(20),
        )
        .unwrap();
        let new = flow.listed()[0].attachment_id.clone();
        if lose {
            assert_eq!(first.waiting, 1, "a lost reply keeps the mapping");
            assert_eq!(flow.slots.repairs(PAGE).unwrap().len(), 1);
            let before = flow.published();
            assert_eq!(flow.rekey(), Pass::default());
            assert_eq!(
                flow.published(),
                before,
                "read confirms success without another Save"
            );
        } else {
            assert_eq!(first.swapped, 1);
            assert!(
                flow.source_text().ends_with("<p>edited</p>"),
                "concurrent edit kept"
            );
        }
        assert!(
            flow.source_text()
                .contains(&format!("colab-attachment:{new}"))
        );
        assert!(flow.slots.repairs(PAGE).unwrap().is_empty());
    }
}

#[test]
fn an_older_reference_is_repaired_before_the_next_epoch_replaces_its_attachment_again() {
    let mut flow = Flow::new();
    let old = flow.attached();
    flow.write_source(format!(
        "<img src='colab-attachment:{}'>",
        old.attachment_id
    ));
    flow.forget_transfer();
    flow.advance();
    flow.writer
        .fail_from
        .store(flow.published() + 3, Ordering::SeqCst);
    assert_eq!(flow.rekey().waiting, 1);
    let middle = flow.listed()[0].attachment_id.clone();
    flow.writer.fail_from.store(usize::MAX, Ordering::SeqCst);
    flow.advance();
    flow.forget_transfer();
    assert_eq!(flow.rekey().swapped, 1);
    let new = flow.listed()[0].attachment_id.clone();
    assert_ne!(middle, new);
    assert_eq!(
        flow.source_text(),
        format!("<img src='colab-attachment:{new}'>")
    );
    assert!(flow.slots.repairs(PAGE).unwrap().is_empty());
}

#[test]
fn an_ambiguous_short_reference_is_left_untouched_but_the_full_reference_is_repaired() {
    let flow = Flow::new();
    let old = flow.attached();
    let mut another = old.clone();
    another.attachment_id.replace_range(9..13, "abcd");
    let html = format!(
        "<img src='colab-attachment:{}'><img src='colab-attachment:{}'>",
        old.attachment_id,
        &old.attachment_id[..8]
    );
    let new = "30000000-0000-4000-8000-000000000092";
    assert_eq!(
        pending::repair_source(&html, &old.attachment_id, new, &[another]),
        Some(html.replace(&old.attachment_id, new))
    );
}

#[test]
fn removed_attachments_are_not_recreated_and_archived_pages_are_not_saved() {
    for archive in [false, true] {
        let mut flow = Flow::new();
        let old = flow.attached();
        let html = format!("<img src='colab-attachment:{}'>", old.attachment_id);
        flow.write_source(html.clone());
        flow.forget_transfer();
        flow.advance();
        flow.writer
            .fail_from
            .store(flow.published() + 3, Ordering::SeqCst);
        assert_eq!(flow.rekey().waiting, 1);
        flow.writer.fail_from.store(usize::MAX, Ordering::SeqCst);
        let id = flow.slots.repairs(PAGE).unwrap().pop().unwrap();
        if archive {
            flow.world.state.apply(OwnerAction::Archive { page: PAGE });
        } else {
            let descriptor = flow.listed()[0].clone();
            let prepared = page::save::prepare(
                &flow.world.source,
                &page::save::Save {
                    page: PAGE.into(),
                    operation_id: page::fresh_id().unwrap(),
                    base_sha256: crypto::digest(html.as_bytes()),
                    source: html.clone(),
                    attachments: Some(
                        serde_json::from_value(
                            serde_json::json!({"remove": [descriptor.attachment_id]}),
                        )
                        .unwrap(),
                    ),
                },
                &|| Ok(crate::registration::now_ms().unwrap()),
            )
            .unwrap();
            let page::save::Prepared::Write(frozen) = prepared else {
                panic!("remove writes")
            };
            let view = (flow.world.source)().unwrap();
            flow.writer.publish(&view.keyring, &frozen).unwrap();
        }
        let before = flow.published();
        let pass = flow.rekey();
        assert_eq!(flow.published(), before);
        assert_eq!(flow.source_text(), html);
        assert_eq!(pass.waiting, usize::from(archive));
        assert_eq!(
            flow.slots.repairs(PAGE).unwrap().len(),
            usize::from(archive)
        );
        // Removed references have nothing left to resume; archived pages keep their pending
        // mapping without attempting a write.
        assert_eq!(flow.slots.load(&id).is_ok(), archive);
    }
}

#[test]
fn a_malformed_retained_old_id_refuses_repair_without_saving_or_losing_the_map() {
    let mut flow = Flow::new();
    let old = flow.attached();
    flow.write_source(format!(
        "<img src='colab-attachment:{}'>",
        old.attachment_id
    ));
    flow.forget_transfer();
    flow.advance();
    flow.writer
        .fail_from
        .store(flow.published() + 3, Ordering::SeqCst);
    assert_eq!(flow.rekey().waiting, 1);
    let id = flow.slots.repairs(PAGE).unwrap().pop().unwrap();
    let mut slot = flow.slots.load(&id).unwrap();
    slot.record.replaces = Some("bad".into());
    slot.finish(slot.record.done_ms.unwrap()).unwrap();
    flow.writer.fail_from.store(usize::MAX, Ordering::SeqCst);
    let before = flow.published();
    assert_eq!(flow.rekey().waiting, 1);
    assert_eq!(flow.published(), before);
    assert_eq!(flow.slots.repairs(PAGE).unwrap(), vec![id]);
    assert!(flow.source_text().contains(&old.attachment_id));
}

#[test]
fn deleting_the_page_settles_its_pending_reference_map_without_another_save() {
    let mut flow = Flow::new();
    let old = flow.attached();
    flow.write_source(format!(
        "<img src='colab-attachment:{}'>",
        old.attachment_id
    ));
    flow.forget_transfer();
    flow.advance();
    flow.writer
        .fail_from
        .store(flow.published() + 3, Ordering::SeqCst);
    assert_eq!(flow.rekey().waiting, 1);
    let id = flow.slots.repairs(PAGE).unwrap().pop().unwrap();
    flow.world.state.apply(OwnerAction::Delete { page: PAGE });
    let before = flow.published();
    assert_eq!(flow.rekey(), Pass::default());
    assert_eq!(flow.published(), before);
    assert!(flow.slots.repairs(PAGE).unwrap().is_empty());
    assert!(flow.slots.load(&id).is_err());
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

#[test]
fn a_source_parse_mismatch_waits_without_saving_and_can_resume_after_an_owner_edit() {
    let mut flow = Flow::new();
    let old = flow.attached();
    let html = format!("<img src='colab-attachment&#58;{}'>", old.attachment_id);
    flow.write_source(html.clone());
    flow.forget_transfer();
    flow.advance();
    let before = flow.published();
    assert_eq!(flow.rekey().waiting, 1);
    assert_eq!(
        flow.published() - before,
        2,
        "proof and attachment swap only"
    );
    assert_eq!(flow.source_text(), html);
    assert_eq!(flow.slots.repairs(PAGE).unwrap().len(), 1);
    assert_eq!(flow.rekey().waiting, 1);
    assert_eq!(flow.published() - before, 2);
    flow.write_source(format!(
        "<img src='colab-attachment:{}'>",
        old.attachment_id
    ));
    assert_eq!(flow.rekey().waiting, 0);
    let new = &flow.listed()[0].attachment_id;
    assert_eq!(
        flow.source_text(),
        format!("<img src='colab-attachment:{new}'>")
    );
    assert!(flow.slots.repairs(PAGE).unwrap().is_empty());
}
