//! The summary is a notification, not a second inbox or captured request input.
use super::{
    digest::{held as hold_request, identity, policy, request, start},
    support::{Fixture, NOW_MS, service},
};
use crate::{
    orchestration::{DigestReceiving, DigestReceivingSettings},
    storage::Storage,
};
use rusqlite::{Connection, params};
use tmt_core::{
    operation::new_operation_id,
    request::digest::{
        DeliveryPolicy, DigestChecklist, DigestOpportunity, DigestProjection, DigestState,
    },
};

// Stable fixture labels remain canonical request IDs for the decision journal.
fn request_id(label: &str) -> String {
    let suffix = label
        .bytes()
        .fold(0u64, |n, b| n.wrapping_mul(31).wrapping_add(u64::from(b)))
        & 0xffffffffffff;
    format!("req_10000000-0000-4000-8000-{suffix:012x}")
}
fn held(f: &mut Fixture, target: &str, sender: &str, label: &str) {
    hold_request(f, target, sender, &request_id(label));
}
fn restricted(state: &str) -> (Fixture, String, String, String, String) {
    let (mut f, target, owner, sender) = start();
    service(&mut f)
        .write_digest(policy(&target, &owner, 1, NOW_MS + 120_000))
        .unwrap();
    f.storage
        .sync_digest_receiving(
            0,
            DigestReceivingSettings {
                enabled: true,
                default_receiving: DigestReceiving::Auto {},
                members: vec![],
            },
        )
        .unwrap();
    let plan = new_operation_id();
    let db = Connection::open(&f.database).unwrap();
    db.execute("INSERT INTO orchestration_settings VALUES(1,1,'UTC')", [])
        .unwrap();
    db.execute(
        "INSERT INTO orchestration_plans VALUES(?1,'Quiet',1,1,1,'{\"kind\":\"always\"}',?2)",
        params![plan, format!(r#"{{"state":"{state}"}}"#)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO orchestration_plan_targets VALUES(?1,0,'{\"kind\":\"all\"}')",
        [&plan],
    )
    .unwrap();
    (f, target, owner, sender, plan)
}
fn state(f: &Fixture, plan: &str, value: &str) {
    Connection::open(&f.database)
        .unwrap()
        .execute(
            "UPDATE orchestration_plans SET fields=?2,revision=revision+1 WHERE id=?1",
            params![plan, format!(r#"{{"state":"{value}"}}"#)],
        )
        .unwrap();
}
fn claim(f: &mut Fixture, target: &str) -> Option<DigestChecklist> {
    service(f)
        .claim_digest_summary(
            target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            || true,
        )
        .unwrap()
}
fn settle(f: &mut Fixture, b: &DigestChecklist, result: DigestState) {
    service(f)
        .settle_digest_checklist(&b.identity_id, &b.id, &b.attempt_token, result)
        .unwrap();
}
fn urgent(f: &mut Fixture, target: &str, sender: &str, id: &str) {
    service(f)
        .prepare_delivery(
            request(target, Some(sender), &request_id(id)),
            format!("attempt-{id}"),
            7,
            DeliveryPolicy {
                urgent: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
}
fn count(f: &Fixture, table: &str) -> i64 {
    Connection::open(&f.database)
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn restricted_holds_coalesce_and_sleep_to_focus_releases_only_urgent_without_reoffer_or_capture() {
    let (mut f, target, _, sender, plan) = restricted("sleep");
    held(&mut f, &target, &sender, "ordinary");
    urgent(&mut f, &target, &sender, "urgent");
    assert_eq!(count(&f, "orchestration_pending_wakes"), 1);
    assert_eq!(count(&f, "orchestration_wake_clock"), 0);
    assert!(claim(&mut f, &target).is_none());
    assert_eq!(
        count(&f, "orchestration_decisions"),
        2,
        "blocked observation is not a decision"
    );
    state(&f, &plan, "focus");
    let b = claim(&mut f, &target).unwrap();
    assert_eq!(
        b.projection,
        DigestProjection::IncomingSummary {
            incoming_count: 1,
            urgent_count: 1
        }
    );
    assert_eq!(
        crate::digest::text(&mut f.storage, &b).unwrap(),
        "you have 1 messages, 1 urgent: tmt inbox"
    );
    assert_eq!(count(&f, "capture_entries"), 0);
    settle(&mut f, &b, DigestState::Delivered);
    assert!(!f.storage.digest_continuation_open(&target).unwrap());
    let db = Connection::open(&f.database).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM focus_checklists WHERE confirmed_at_ms IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT released_classes FROM orchestration_pending_wakes",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM request_attempts WHERE response_submitted_at_ms IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        service(&mut f)
            .digest_claimable_items(&target, DigestOpportunity::TurnBoundary, 128)
            .unwrap()
            .is_empty()
    );
    state(&f, &plan, "awake");
    f.set_now(NOW_MS + 999);
    assert!(claim(&mut f, &target).is_none());
    f.set_now(NOW_MS + 1000);
    let next = claim(&mut f, &target).unwrap();
    assert_eq!(
        next.projection,
        DigestProjection::IncomingSummary {
            incoming_count: 1,
            urgent_count: 0
        }
    );
    settle(&mut f, &next, DigestState::Uncertain);
    assert_eq!(count(&f, "orchestration_pending_wakes"), 0);
    f.set_now(NOW_MS + 2000);
    assert!(claim(&mut f, &target).is_none());
    assert!(
        service(&mut f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None
            )
            .unwrap()
            .is_none()
    );
}
#[test]
fn base_auto_and_preupgrade_holds_keep_the_full_checklist_lifecycle() {
    let (mut f, target, _, sender) = start();
    f.storage
        .sync_digest_receiving(
            0,
            DigestReceivingSettings {
                enabled: true,
                default_receiving: DigestReceiving::Auto {},
                members: vec![],
            },
        )
        .unwrap();
    held(&mut f, &target, &sender, "ordinary-auto");
    assert_eq!(count(&f, "orchestration_pending_wakes"), 0);
    assert!(claim(&mut f, &target).is_none());
    let b = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(b.projection, DigestProjection::Full);
}
#[test]
fn exact_counts_cross_pages_and_exclude_closed_requests_notices_and_late_arrivals() {
    let (mut f, target, _, sender, plan) = restricted("sleep");
    for n in 0..140 {
        held(&mut f, &target, &sender, &format!("open-{n}"));
    }
    urgent(&mut f, &target, &sender, "open-urgent");
    for id in ["answered", "withdrawn", "expired", "result", "timeout"] {
        held(&mut f, &target, &sender, id);
    }
    let db = Connection::open(&f.database).unwrap();
    // Exercise the canonical persisted terminal facts that first_final_refusal reads.
    db.execute(
        "UPDATE request_attempts SET response_submitted_at_ms=?1 WHERE request_id=?2",
        params![NOW_MS as i64, request_id("answered")],
    )
    .unwrap();
    service(&mut f)
        .withdraw_request(&sender, &request_id("withdrawn"), "obsolete")
        .unwrap();
    db.execute(
        "UPDATE request_attempts SET expires_at_ms=?1,prepared_at_ms=?3 WHERE request_id=?2",
        params![
            (NOW_MS + 1) as i64,
            request_id("expired"),
            (NOW_MS - tmt_core::retention::RESPONSE_ACCEPTANCE_WINDOW_MS) as i64
        ],
    )
    .unwrap();
    for source in ["result", "timeout"] {
        db.execute(
            "UPDATE focus_items SET source=?1 WHERE request_id=?2",
            params![source, request_id(source)],
        )
        .unwrap();
    }
    f.set_now(NOW_MS + 2);
    state(&f, &plan, "awake");
    let b = claim(&mut f, &target).unwrap();
    assert_eq!(
        b.projection,
        DigestProjection::IncomingSummary {
            incoming_count: 141,
            urgent_count: 1
        }
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM focus_items WHERE checklist_id=?",
            [&b.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        141
    );
    state(&f, &plan, "sleep");
    held(&mut f, &target, &sender, "late");
    settle(&mut f, &b, DigestState::Delivered);
    assert_eq!(
        count(&f, "orchestration_pending_wakes"),
        1,
        "later held arrivals retain a separate opportunity"
    );
    assert!(
        db.query_row(
            "SELECT checklist_id IS NULL FROM focus_items WHERE request_id=?",
            [request_id("late")],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM focus_items WHERE checklist_id=?",
            [&b.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        141
    );
}
#[test]
fn unsent_releases_only_its_members_and_restart_preserves_admission_spacing_and_no_replay() {
    for terminal in [
        DigestState::Delivered,
        DigestState::Uncertain,
        DigestState::Unsent,
    ] {
        let (mut f, target, _, sender, plan) = restricted("focus");
        held(&mut f, &target, &sender, "one");
        state(&f, &plan, "awake");
        let b = claim(&mut f, &target).unwrap();
        settle(&mut f, &b, terminal);
        f.storage.close().unwrap();
        f.storage = Storage::open(&f.database).unwrap();
        f.set_now(NOW_MS - 1);
        assert!(claim(&mut f, &target).is_none());
        f.set_now(NOW_MS + 999);
        assert!(claim(&mut f, &target).is_none());
        f.set_now(NOW_MS + 1000);
        assert_eq!(
            claim(&mut f, &target).is_some(),
            terminal == DigestState::Unsent
        );
    }
}
#[test]
fn incomplete_budget_and_no_eligible_work_never_publish_partial_counts() {
    use std::cell::Cell;
    let (mut f, target, _, sender, plan) = restricted("sleep");
    for n in 0..140 {
        held(&mut f, &target, &sender, &format!("item-{n}"));
    }
    state(&f, &plan, "awake");
    let calls = Cell::new(0);
    assert!(
        service(&mut f)
            .claim_digest_summary(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::Idle,
                || {
                    calls.set(calls.get() + 1);
                    calls.get() < 3
                }
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(count(&f, "focus_checklists"), 0);
    assert_eq!(count(&f, "orchestration_wake_clock"), 0);
    let db = Connection::open(&f.database).unwrap();
    db.execute(
        "UPDATE request_attempts SET response_submitted_at_ms=?",
        [NOW_MS as i64],
    )
    .unwrap();
    assert!(claim(&mut f, &target).is_none());
    assert_eq!(count(&f, "orchestration_pending_wakes"), 0);
    assert_eq!(count(&f, "focus_checklists"), 0);
}
#[test]
fn bounded_uuid_pass_continues_past_an_offline_first_identity_after_restart() {
    let (mut f, target, owner, sender, plan) = restricted("sleep");
    let other = identity(&mut f, "Second seat");
    service(&mut f)
        .write_digest(policy(&other, &owner, 0, NOW_MS + 120000))
        .unwrap();
    for id in [&target, &other] {
        held(&mut f, id, &sender, &format!("request-{id}"));
    }
    state(&f, &plan, "awake");
    let (first, _) = service(&mut f).digest_wake_candidates(1).unwrap();
    assert_eq!(first.len(), 1);
    let low = first[0].clone();
    service(&mut f)
        .advance_digest_wake_scan(Some(&low))
        .unwrap();
    f.storage.close().unwrap();
    f.storage = Storage::open(&f.database).unwrap();
    let (next, _) = service(&mut f).digest_wake_candidates(1).unwrap();
    assert_eq!(next.len(), 1);
    assert!(next[0] > low);
    let b = claim(&mut f, &next[0]).unwrap();
    let (_, clock) = service(&mut f).digest_wake_candidates(1).unwrap();
    assert_eq!(clock.scan_after_identity_id, None);
    assert_eq!(clock.next_admission_ms, NOW_MS + 1000);
    assert!(claim(&mut f, &low).is_none());
    settle(&mut f, &b, DigestState::Delivered);
    f.set_now(NOW_MS + 1000);
    assert!(claim(&mut f, &low).is_some());
}

#[test]
fn a_failed_claim_rolls_back_marker_clock_membership_and_decision_together() {
    let (mut f, target, _, sender, plan) = restricted("focus");
    held(&mut f, &target, &sender, "one");
    state(&f, &plan, "awake");
    let db = Connection::open(&f.database).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_wake BEFORE UPDATE ON orchestration_pending_wakes WHEN NEW.checklist_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'forced wake rollback'); END;").unwrap();
    assert!(
        service(&mut f)
            .claim_digest_summary(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::Idle,
                || true
            )
            .is_err()
    );
    assert_eq!(count(&f, "focus_checklists"), 0);
    assert_eq!(count(&f, "orchestration_wake_clock"), 0);
    assert_eq!(count(&f, "orchestration_decisions"), 1);
    assert!(
        db.query_row(
            "SELECT checklist_id IS NULL FROM orchestration_pending_wakes",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    db.execute_batch("DROP TRIGGER refuse_wake;").unwrap();
    assert!(claim(&mut f, &target).is_some());
}

#[test]
fn a_deferred_summary_never_grants_an_unmeasured_full_checklist_prefix() {
    let (mut f, target, _, sender, plan) = restricted("focus");
    held(&mut f, &target, &sender, "one");
    state(&f, &plan, "awake");
    let first = claim(&mut f, &target).unwrap();
    settle(&mut f, &first, DigestState::Unsent); // Spacing now defers this opportunity.
    assert_eq!(
        crate::digest::fitting_through(
            &mut f.storage,
            &target,
            DigestOpportunity::TurnBoundary,
            false
        )
        .unwrap(),
        Some(0)
    );
    let deferred = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            Some(0),
        )
        .unwrap();
    assert!(deferred.is_none());
    assert_eq!(count(&f, "focus_checklists"), 1);
}

mod authority;
