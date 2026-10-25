use super::*;
use crate::{
    orchestration::{decode_orch_query, orchestration_read},
    test_support::TestDirectory,
};
use tmt_core::identity::{Lifetime, create_or_resolve};
const NOW: u64 = 1_790_000_000_000;
const PLAN: &str = "11111111-1111-4111-8111-111111111111";
fn configured(s: &Storage) {
    s.connection()
        .unwrap()
        .execute("INSERT INTO orchestration_settings VALUES(1,1,NULL)", [])
        .unwrap();
}
fn event(id: &str, at: u64) -> OrchDecision {
    OrchDecision {
        recorded_at_ms: at,
        subject_kind: OrchSubjectKind::Identity,
        subject_id: id.into(),
        actor_id: None,
        recipient_id: Some(id.into()),
        action: OrchDecisionAction::Delay,
        outcome: OrchDecisionOutcome::Held,
        reason: "slot_receiving_hold".into(),
        configuration_revision: 1,
        time_zone: "Asia/Tokyo".into(),
        sources: vec![OrchDecisionSource {
            field: OrchPolicyField::Receiving,
            source: OrchSource {
                kind: OrchSourceKind::Plan,
                plan_id: Some(PLAN.into()),
                identity_id: None,
                revision: 3,
                slot: None,
                defaulted: true,
            },
        }],
    }
}
fn subject(id: &str) -> OrchSubjectDocument {
    OrchSubjectDocument {
        kind: "identity".into(),
        id: id.into(),
    }
}
fn identity(s: &mut Storage, name: &str) -> String {
    create_or_resolve(s, name, Lifetime::Saved)
        .unwrap()
        .identity
        .id
}
#[test]
fn canonical_filter_is_case_insensitive_prefix_first_and_snapshot_does_not_write() {
    let d = TestDirectory::new();
    let mut s = Storage::open(d.path.join("state.db")).unwrap();
    for name in ["a-night-3", "Night-2", "night-1"] {
        identity(&mut s, name);
    }
    let changes = s.connection().unwrap().total_changes();
    let snapshot = s.orchestration_snapshot(Some("NIGHT")).unwrap().snapshot;
    assert_eq!(
        snapshot
            .identities
            .iter()
            .map(|i| i.name.to_ascii_lowercase())
            .collect::<Vec<_>>(),
        ["night-1", "night-2", "a-night-3"]
    );
    assert_eq!(changes, s.connection().unwrap().total_changes());
    assert!(snapshot.plans.is_empty());
}
#[test]
fn decision_and_callers_state_roll_back_together() {
    let d = TestDirectory::new();
    let mut s = Storage::open(d.path.join("state.db")).unwrap();
    let id = identity(&mut s, "seat");
    configured(&s);
    {
        let tx = s
            .connection_mut()
            .unwrap()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute(
            "UPDATE orchestration_settings SET revision=2 WHERE id=1",
            [],
        )
        .unwrap();
        append(&tx, &event(&id, NOW)).unwrap();
    }
    assert_eq!(s.orchestration_snapshot(None).unwrap().snapshot.revision, 1);
    assert!(
        s.orchestration_journal(&subject(&id), None, 50, NOW)
            .unwrap()
            .items
            .is_empty()
    );
}
#[test]
fn journal_order_is_commit_order_and_sources_are_frozen_after_plan_edits() {
    let d = TestDirectory::new();
    let mut s = Storage::open(d.path.join("state.db")).unwrap();
    let id = identity(&mut s, "seat");
    configured(&s);
    seed_plan(&s, r#"{"state":"sleep"}"#);
    s.connection()
        .unwrap()
        .execute("UPDATE orchestration_plans SET revision=3", [])
        .unwrap();
    let first = s.append_orchestration_decision(&event(&id, NOW)).unwrap();
    s.append_orchestration_decision(&event(&id, NOW - 1))
        .unwrap();
    let last = s
        .append_orchestration_decision(&event(&id, NOW + 1))
        .unwrap();
    s.connection()
        .unwrap()
        .execute(
            "UPDATE orchestration_plans SET revision=4,fields='{\"state\":\"awake\"}'",
            [],
        )
        .unwrap();
    s.connection()
        .unwrap()
        .execute(
            "UPDATE orchestration_settings SET revision=9 WHERE id=1",
            [],
        )
        .unwrap();
    let page = s
        .orchestration_journal(&subject(&id), None, 2, NOW + 2)
        .unwrap();
    assert_eq!(page.items[0]["id"], last);
    assert_eq!(page.items[1]["recordedAtMs"], NOW - 1);
    assert_eq!(page.items[0]["sources"][0]["source"]["revision"], 3);
    let final_page = s
        .orchestration_journal(&subject(&id), page.next_before, 2, NOW + 2)
        .unwrap();
    assert_eq!(final_page.items.len(), 1);
    assert_eq!(final_page.items[0]["id"], first);
    assert_eq!(final_page.next_before, None);
}
#[test]
fn journal_append_and_writer_pruning_do_not_wake_boards_and_reads_only_filter_expiry() {
    let d = TestDirectory::new();
    let mut s = Storage::open(d.path.join("state.db")).unwrap();
    let id = identity(&mut s, "seat");
    // The on-demand journal pages by event ID, independently of the board cursor.
    let cursor = s.change_cursor().unwrap();
    s.append_orchestration_decision(&event(&id, NOW)).unwrap();
    assert_eq!(s.change_cursor().unwrap(), cursor, "journal append");
    let expired = NOW + ORCH_JOURNAL_RETENTION_MS;
    let changes = s.connection().unwrap().total_changes();
    assert!(
        s.orchestration_journal(&subject(&id), None, 50, expired)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(changes, s.connection().unwrap().total_changes());
    let count: i64 = s
        .connection()
        .unwrap()
        .query_row("SELECT count(*) FROM orchestration_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    s.append_orchestration_decision(&event(&id, expired))
        .unwrap();
    assert_eq!(
        s.change_cursor().unwrap(),
        cursor,
        "journal append and prune"
    );
    let count: i64 = s
        .connection()
        .unwrap()
        .query_row("SELECT count(*) FROM orchestration_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    let page = s
        .orchestration_journal(&subject(&id), None, 50, expired)
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0]["recordedAtMs"], expired);
}
#[test]
fn preview_uses_real_store_and_has_no_policy_journal_or_cursor_effect() {
    let d = TestDirectory::new();
    let mut s = Storage::open(d.path.join("state.db")).unwrap();
    let id = identity(&mut s, "seat");
    s.connection()
        .unwrap()
        .execute(
            "INSERT INTO orchestration_settings VALUES(1,1,'Asia/Tokyo')",
            [],
        )
        .unwrap();
    let candidate = json!({"id":PLAN,"name":"Quiet","revision":1,"priority":1,"enabled":true,"targets":[{"kind":"all"}],"schedule":{"kind":"always"},"fields":{"state":"sleep"}});
    let query = decode_orch_query(
        json!({"view":"preview","atMs":NOW,"candidate":candidate})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let changes = s.connection().unwrap().total_changes();
    let report: Value =
        serde_json::from_slice(&orchestration_read(&s, query, NOW).unwrap()).unwrap();
    assert_eq!(
        report["before"]["members"][0]["effective"]["state"]["value"],
        "awake"
    );
    assert_eq!(
        report["after"]["members"][0]["effective"]["receiving"]["proposedValue"]["mode"],
        "hold"
    );
    assert_eq!(
        report["after"]["members"][0]["effective"]["receiving"]["available"],
        false
    );
    assert!(report["after"]["members"][0]["effective"]["receiving"]["value"].is_null());
    assert_eq!(report["changes"][0]["identityId"], id);
    assert!(
        s.orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .plans
            .is_empty()
    );
    assert!(
        s.orchestration_journal(&subject(&id), None, 50, NOW)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(changes, s.connection().unwrap().total_changes());
}
fn seed_plan(s: &Storage, fields: &str) {
    s.connection().unwrap().execute("INSERT INTO orchestration_plans(id,name,revision,priority,enabled,schedule,fields) VALUES(?1,'Quiet',1,1,1,'{\"kind\":\"always\"}',?2)",params![PLAN,fields]).unwrap();
    s.connection()
        .unwrap()
        .execute(
            "INSERT INTO orchestration_plan_targets VALUES(?1,0,'{\"kind\":\"all\"}')",
            [PLAN],
        )
        .unwrap();
}
#[test]
fn a_writer_commit_cannot_mix_plan_and_settings_in_the_pinned_read() {
    let d = TestDirectory::new();
    let path = d.path.join("state.db");
    let mut s = Storage::open(&path).unwrap();
    identity(&mut s, "seat");
    configured(&s);
    seed_plan(&s, r#"{"state":"sleep"}"#);
    let tx = s.connection().unwrap().unchecked_transaction().unwrap();
    let pinned: i64 = tx
        .query_row(
            "SELECT revision FROM orchestration_settings WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pinned, 1);
    let mut writer = Connection::open(&path).unwrap();
    let write = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    write
        .execute(
            "UPDATE orchestration_settings SET revision=2 WHERE id=1",
            [],
        )
        .unwrap();
    write
        .execute(
            "UPDATE orchestration_plans SET fields='{\"state\":\"awake\"}',revision=2",
            [],
        )
        .unwrap();
    write.commit().unwrap();
    let before = read_snapshot(&tx, None, None).unwrap();
    assert_eq!(before.snapshot.revision, 1);
    assert_eq!(
        before.snapshot.plans[0].fields.state,
        Some(OrchState::Sleep)
    );
    tx.commit().unwrap();
    let after = s.orchestration_snapshot(None).unwrap();
    assert_eq!(after.snapshot.revision, 2);
    assert_eq!(after.snapshot.plans[0].fields.state, Some(OrchState::Awake));
}
#[test]
fn duplicate_stored_policy_fields_refuse_without_losing_the_evidence() {
    let d = TestDirectory::new();
    let mut s = Storage::open(d.path.join("state.db")).unwrap();
    identity(&mut s, "seat");
    seed_plan(&s, r#"{"state":"sleep","state":"awake"}"#);
    let changes = s.connection().unwrap().total_changes();
    assert_eq!(
        s.orchestration_snapshot(None).err().unwrap().code,
        StorageErrorCode::Corrupt
    );
    assert_eq!(changes, s.connection().unwrap().total_changes());
    let text: String = s
        .connection()
        .unwrap()
        .query_row("SELECT fields FROM orchestration_plans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(text, r#"{"state":"sleep","state":"awake"}"#);
}
