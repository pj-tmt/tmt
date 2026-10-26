//! Existing receiving authority and ordinary notice behavior survive the summary projection.
use super::super::digest::mark_user;
use super::*;

#[test]
fn person_immunity_prevents_markers_and_removes_a_stale_opportunity_without_admission() {
    for recipient_is_person in [false, true] {
        let (mut f, target, _, sender, _) = restricted("sleep");
        mark_user(
            &f,
            if recipient_is_person {
                &target
            } else {
                &sender
            },
            "true",
        );
        let held = hold_request(&mut f, &target, &sender, &request_id("immune"));
        assert_eq!(held.digest_until_ms, None);
        assert_eq!(count(&f, "orchestration_pending_wakes"), 0);
        mark_user(
            &f,
            if recipient_is_person {
                &target
            } else {
                &sender
            },
            "false",
        );
        self::super::held(&mut f, &target, &sender, "stale");
        assert_eq!(count(&f, "orchestration_pending_wakes"), 1);
        mark_user(
            &f,
            if recipient_is_person {
                &target
            } else {
                &sender
            },
            "true",
        );
        assert!(
            !service(&mut f)
                .digest_wake_released(&target, || true)
                .unwrap()
        );
        assert_eq!(count(&f, "orchestration_pending_wakes"), 0);
        assert_eq!(count(&f, "orchestration_wake_clock"), 0);
    }
}
#[test]
fn global_off_is_still_the_master_switch_and_an_expired_lease_grants_no_hold() {
    for global_off in [false, true] {
        let (mut f, target, _, sender, _) = restricted("sleep");
        held(&mut f, &target, &sender, "before");
        if global_off {
            f.storage
                .sync_digest_receiving(
                    1,
                    DigestReceivingSettings {
                        enabled: false,
                        default_receiving: DigestReceiving::Auto {},
                        members: vec![],
                    },
                )
                .unwrap();
        } else {
            f.set_now(NOW_MS + 120000);
        }
        let next = hold_request(&mut f, &target, &sender, &request_id("after"));
        assert_eq!(next.digest_until_ms, None);
        let b = claim(&mut f, &target).unwrap();
        assert_eq!(
            b.projection,
            DigestProjection::IncomingSummary {
                incoming_count: 1,
                urgent_count: 0
            }
        );
    }
}
#[test]
fn an_overlapping_sleep_restriction_prevents_release_when_the_original_focus_ends() {
    let (mut f, target, _, sender, plan) = restricted("focus");
    held(&mut f, &target, &sender, "one");
    let db = Connection::open(&f.database).unwrap();
    let higher = new_operation_id();
    db.execute("INSERT INTO orchestration_plans VALUES(?1,'Other quiet',1,2,1,'{\"kind\":\"always\"}','{\"state\":\"sleep\"}')",[&higher]).unwrap();
    db.execute(
        "INSERT INTO orchestration_plan_targets VALUES(?1,0,'{\"kind\":\"all\"}')",
        [&higher],
    )
    .unwrap();
    db.execute(
        "UPDATE orchestration_plans SET enabled=0 WHERE id=?",
        [&plan],
    )
    .unwrap();
    assert!(claim(&mut f, &target).is_none());
    assert_eq!(count(&f, "orchestration_pending_wakes"), 1);
    assert_eq!(count(&f, "orchestration_wake_clock"), 0);
    db.execute(
        "UPDATE orchestration_plans SET enabled=0 WHERE id=?",
        [&higher],
    )
    .unwrap();
    assert!(claim(&mut f, &target).is_some());
}
#[test]
fn results_and_timeouts_use_full_checklists_while_a_later_summary_waits_on_spacing() {
    let (mut f, target, _, sender, plan) = restricted("sleep");
    for name in ["ordinary", "result", "timeout"] {
        held(&mut f, &target, &sender, name);
    }
    let db = Connection::open(&f.database).unwrap();
    for source in ["result", "timeout"] {
        db.execute(
            "UPDATE focus_items SET source=?1 WHERE request_id=?2",
            params![source, request_id(source)],
        )
        .unwrap();
    }
    state(&f, &plan, "awake");
    let summary = claim(&mut f, &target).unwrap();
    state(&f, &plan, "sleep");
    held(&mut f, &target, &sender, "late");
    settle(&mut f, &summary, DigestState::Delivered);
    state(&f, &plan, "awake");
    assert!(
        claim(&mut f, &target).is_none(),
        "spacing still gates the late summary"
    );
    let full = service(&mut f)
        .claim_digest_checklist(
            &target,
            new_operation_id(),
            new_operation_id(),
            DigestOpportunity::TurnBoundary,
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(full.projection, DigestProjection::Full);
    let sources: Vec<String> = db
        .prepare("SELECT source FROM focus_items WHERE checklist_id=? ORDER BY sequence")
        .unwrap()
        .query_map([&full.id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(sources, ["result", "timeout"]);
    assert!(
        db.query_row(
            "SELECT checklist_id IS NULL FROM focus_items WHERE request_id=?",
            [request_id("late")],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}

#[test]
fn a_summary_read_is_count_only_and_retirement_keeps_the_active_identity_fence() {
    let (mut f, target, _, sender, plan) = restricted("focus");
    held(&mut f, &target, &sender, "private-body");
    state(&f, &plan, "awake");
    let b = claim(&mut f, &target).unwrap();
    settle(&mut f, &b, DigestState::Delivered);
    let page = crate::digest::read(&mut f.storage, &target, Some(&b.id), 0, 128).unwrap();
    assert_eq!(page["projection"], "incoming_summary");
    assert_eq!(page["incomingCount"], 1);
    assert_eq!(page["urgentCount"], 0);
    assert_eq!(page["items"], serde_json::json!([]));
    let serialized = page.to_string();
    assert!(!serialized.contains("req_"));
    assert!(!serialized.contains("private-body"));
    Connection::open(&f.database)
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms=?1 WHERE id=?2",
            params![NOW_MS as i64, target],
        )
        .unwrap();
    assert!(matches!(
        crate::digest::read(&mut f.storage, &target, Some(&b.id), 0, 128),
        Err(tmt_core::request::RequestError::Digest(
            tmt_core::request::digest::DigestRejection::IdentityUnavailable
        ))
    ));
}
