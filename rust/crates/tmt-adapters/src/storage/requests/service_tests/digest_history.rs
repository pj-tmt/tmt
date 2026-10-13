use super::*;

fn claim(
    f: &mut Fixture,
    target: &str,
    opportunity: DigestOpportunity,
) -> tmt_core::request::digest::DigestChecklist {
    if opportunity == DigestOpportunity::Idle {
        service(f).make_digest_due(target).unwrap();
    }
    service(f)
        .claim_digest_checklist(target, new_operation_id(), new_operation_id(), opportunity)
        .unwrap()
        .unwrap()
}

#[test]
fn history_appears_once_at_settlement_even_when_the_clock_rolls_back() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "first");
    let first = claim(&mut f, &target, DigestOpportunity::TurnBoundary);
    let before = service(&mut f)
        .digest_checklist_history(&target, None, 0, 128)
        .unwrap();
    assert!(before.checklists.is_empty());
    assert_eq!(before.legacy_settled_count, 0);
    let watermark = before.through_sequence;
    f.set_now(NOW_MS + 5);
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &first.id,
            &first.attempt_token,
            DigestState::Delivered,
        )
        .unwrap();
    let page = service(&mut f)
        .digest_checklist_history(&target, None, watermark, 1)
        .unwrap();
    assert_eq!(page.checklists.len(), 1);
    let row = &page.checklists[0];
    assert_eq!(row.evidence.checklist.id, first.id);
    assert_eq!(
        row.evidence.opportunity,
        Some(DigestOpportunity::TurnBoundary)
    );
    assert_eq!(row.evidence.sealed_count, Some(1));
    assert_eq!(row.evidence.settled_at_ms, Some(NOW_MS + 5));
    assert_eq!(row.items[0].request_id, "first");
    assert_eq!(row.items[0].context_tokens_at_arrival, None);
    assert_eq!(row.retained_count, 1);
    assert_eq!(page.delivered_digests, 1);
    let sequence = row.evidence.settlement_sequence;
    let cursor = f.storage.change_cursor().unwrap();
    f.set_now(NOW_MS + 10);
    assert!(
        !service(&mut f)
            .settle_digest_checklist(
                &target,
                &first.id,
                &first.attempt_token,
                DigestState::Delivered
            )
            .unwrap()
    );
    assert_eq!(f.storage.change_cursor().unwrap(), cursor);
    assert_eq!(
        service(&mut f)
            .digest_checklist_history(&target, None, 0, 128)
            .unwrap()
            .checklists,
        page.checklists
    );
    assert_eq!(
        f.storage.change_cursor().unwrap(),
        cursor,
        "history must not write or prune"
    );
    held(&mut f, &target, &sender, "second");
    let second = claim(&mut f, &target, DigestOpportunity::Idle);
    f.set_now(NOW_MS);
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &second.id,
            &second.attempt_token,
            DigestState::Uncertain,
        )
        .unwrap();
    let later = service(&mut f)
        .digest_checklist_history(&target, None, sequence, 128)
        .unwrap();
    assert_eq!(later.checklists.len(), 1);
    assert_eq!(later.checklists[0].evidence.checklist.id, second.id);
    assert!(later.checklists[0].evidence.settlement_sequence > sequence);
    assert_eq!(
        later.checklists[0].evidence.opportunity,
        Some(DigestOpportunity::Idle)
    );
    assert_eq!(later.checklists[0].evidence.settled_at_ms, Some(NOW_MS));
    assert_eq!(later.delivered_digests, 1);
    f.storage.close().unwrap();
}

#[test]
fn history_bounds_inline_inputs_and_preserves_oversized_continuation() {
    let (mut f, target, _, sender) = start();
    let mut batches = Vec::new();
    for (index, count) in [70, 70, 129].into_iter().enumerate() {
        for item in 0..count {
            held(&mut f, &target, &sender, &format!("batch-{index}-{item}"));
        }
        let batch = claim(&mut f, &target, DigestOpportunity::TurnBoundary);
        service(&mut f)
            .settle_digest_checklist(
                &target,
                &batch.id,
                &batch.attempt_token,
                DigestState::Delivered,
            )
            .unwrap();
        batches.push(batch);
    }
    let first = service(&mut f)
        .digest_checklist_history(&target, None, 0, 128)
        .unwrap();
    assert_eq!(first.checklists.len(), 1);
    assert_eq!(first.checklists[0].items.len(), 70);
    assert!(first.checklists[0].next_item_after.is_none());
    let second = service(&mut f)
        .digest_checklist_history(&target, None, first.next_after.unwrap(), 128)
        .unwrap();
    assert_eq!(second.checklists.len(), 1);
    assert_eq!(second.checklists[0].items.len(), 70);
    let third = service(&mut f)
        .digest_checklist_history(&target, None, second.next_after.unwrap(), 128)
        .unwrap();
    assert_eq!(third.checklists.len(), 1);
    assert!(third.next_after.is_none());
    let row = &third.checklists[0];
    assert_eq!(row.evidence.sealed_count, Some(129));
    assert_eq!(row.items.len(), 128);
    let rest = service(&mut f)
        .digest_checklist_history(
            &target,
            Some(&batches[2].id),
            row.next_item_after.unwrap(),
            128,
        )
        .unwrap();
    assert_eq!(rest.checklists[0].items.len(), 1);
    assert_eq!(rest.checklists[0].items[0].request_id, "batch-2-128");
    assert!(rest.checklists[0].next_item_after.is_none());
    assert_eq!(rest.checklists[0].retained_count, 129);
    assert!(
        service(&mut f)
            .digest_checklist_history(&target, None, third.through_sequence, 128)
            .unwrap()
            .checklists
            .is_empty()
    );
    f.storage.close().unwrap();
}

#[test]
fn history_marks_released_expired_and_legacy_membership_unknown() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "released");
    let released = claim(&mut f, &target, DigestOpportunity::TurnBoundary);
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &released.id,
            &released.attempt_token,
            DigestState::Unsent,
        )
        .unwrap();
    let page = service(&mut f)
        .digest_checklist_history(&target, None, 0, 128)
        .unwrap();
    assert_eq!(page.checklists[0].retained_count, 0);
    assert_eq!(page.checklists[0].evidence.sealed_count, Some(1));
    let next = claim(&mut f, &target, DigestOpportunity::TurnBoundary);
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &next.id,
            &next.attempt_token,
            DigestState::Delivered,
        )
        .unwrap();
    let db = rusqlite::Connection::open(&f.database).unwrap();
    db.execute(
        "UPDATE request_attempts SET retention_expires_at_ms=? WHERE request_id='released'",
        [NOW_MS as i64],
    )
    .unwrap();
    let cursor = f.storage.change_cursor().unwrap();
    let page = service(&mut f)
        .digest_checklist_history(&target, None, 0, 128)
        .unwrap();
    assert_eq!(page.checklists.len(), 2);
    assert_eq!(page.checklists[1].retained_count, 0);
    assert_eq!(f.storage.change_cursor().unwrap(), cursor);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM focus_items", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1,
        "read hides expiry without housekeeping"
    );
    db.execute("UPDATE focus_checklists SET settlement_sequence=NULL,settled_at_ms=NULL,opportunity=NULL,sealed_count=NULL WHERE id=?", [&released.id]).unwrap();
    let legacy = service(&mut f)
        .digest_checklist_history(&target, None, 0, 128)
        .unwrap();
    assert_eq!(legacy.legacy_settled_count, 1);
    assert_eq!(legacy.checklists.len(), 1);
    assert!(
        service(&mut f)
            .digest_checklist_history(&target, Some(&released.id), 0, 128)
            .unwrap()
            .checklists
            .is_empty()
    );
    assert!(
        service(&mut f)
            .digest_checklist_history(&sender, Some(&next.id), 0, 128)
            .is_err()
    );
    assert!(
        service(&mut f)
            .digest_checklist_history(&target, None, 0, 0)
            .is_err()
    );
    assert!(
        service(&mut f)
            .digest_checklist_history(&target, None, 0, 129)
            .is_err()
    );
    f.storage.close().unwrap();
    let mut reopened = crate::storage::Storage::open(&f.database).unwrap();
    let persisted = RequestService::new(&mut reopened, || NOW_MS)
        .digest_checklist_history(&target, None, 0, 128)
        .unwrap();
    assert_eq!(persisted.checklists, legacy.checklists);
    reopened.close().unwrap();
}

#[test]
fn failed_settlement_rolls_back_history_metadata_and_preserves_claim_membership() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "atomic");
    let batch = claim(&mut f, &target, DigestOpportunity::TurnBoundary);
    let db = rusqlite::Connection::open(&f.database).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_delivered BEFORE INSERT ON digest_counters
        WHEN NEW.delivered_digests>0 BEGIN SELECT RAISE(ABORT,'injected delivered counter failure'); END;").unwrap();
    let cursor = f.storage.change_cursor().unwrap();
    assert!(
        service(&mut f)
            .settle_digest_checklist(
                &target,
                &batch.id,
                &batch.attempt_token,
                DigestState::Delivered
            )
            .is_err()
    );
    assert_eq!(f.storage.change_cursor().unwrap(), cursor);
    assert!(
        service(&mut f)
            .digest_checklist_history(&target, None, 0, 128)
            .unwrap()
            .checklists
            .is_empty()
    );
    assert_eq!(
        db.query_row(
            "SELECT state,settled_at_ms,settlement_sequence FROM focus_checklists WHERE id=?",
            [&batch.id],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?
            ))
        )
        .unwrap(),
        ("claimed".into(), None, None)
    );
    db.execute_batch("DROP TRIGGER refuse_delivered").unwrap();
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &batch.id,
            &batch.attempt_token,
            DigestState::Delivered,
        )
        .unwrap();
    assert_eq!(
        service(&mut f)
            .digest_checklist_history(&target, None, 0, 128)
            .unwrap()
            .checklists
            .len(),
        1
    );
    f.storage.close().unwrap();
}
