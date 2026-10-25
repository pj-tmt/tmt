use super::*;
use crate::test_support::TestDirectory;
use tmt_core::identity::{Lifetime, create_or_resolve};
const NOW: u64 = 1_790_000_000_000;
const PLAN: &str = "11111111-1111-4111-8111-111111111111";
fn fixture() -> (TestDirectory, Storage, String) {
    let d = TestDirectory::new();
    let mut s = Storage::open(d.path.join("state.db")).unwrap();
    let id = create_or_resolve(&mut s, "seat-1", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    rusqlite::Connection::open(d.path.join("state.db"))
        .unwrap()
        .execute("INSERT INTO orchestration_settings VALUES(1,1,'UTC')", [])
        .unwrap();
    s.sync_digest_receiving(
        0,
        super::super::DigestReceivingSettings {
            enabled: true,
            default_receiving: super::super::DigestReceiving::Direct {},
            members: vec![],
        },
    )
    .unwrap();
    (d, s, id)
}
#[test]
fn missing_digest_input_is_visible_and_gates_only_receiving_changes() {
    let (d, mut s, _) = fixture();
    rusqlite::Connection::open(d.path.join("state.db"))
        .unwrap()
        .execute("DELETE FROM digest_receiving_settings", [])
        .unwrap();
    let p = preview_for(&mut s, json!({"kind":"plan.add","candidate":plan()}));
    let receiving = &p["before"]["members"][0]["effective"]["receiving"];
    assert_eq!(receiving["available"], false);
    assert!(receiving["value"].is_null());
    assert_eq!(receiving["proposedValue"]["mode"], "direct");
    assert!(matches!(
        orchestration_write(&mut s, apply_query(&p), || NOW),
        Err(OrchReadFailure::Policy(OrchError::ReceivingUnavailable))
    ));
    let mut sending = plan();
    sending["fields"] = json!({"sending":"paused","talkLimit":3});
    let p = preview_for(&mut s, json!({"kind":"plan.add","candidate":sending}));
    orchestration_write(&mut s, apply_query(&p), || NOW).unwrap();
    assert_eq!(
        s.orchestration_snapshot(None).unwrap().snapshot.plans.len(),
        1
    );
}
fn plan() -> Value {
    json!({"id":PLAN,"name":"Quiet","revision":1,"priority":1,"enabled":true,"targets":[{"kind":"all"}],"schedule":{"kind":"always"},"fields":{"state":"sleep"}})
}
fn query(v: Value) -> OrchWriteQuery {
    decode_orch_write(&serde_json::to_vec(&v).unwrap()).unwrap()
}
fn preview_for(s: &mut Storage, mutation: Value) -> Value {
    serde_json::from_slice(
        &orchestration_write(
            s,
            query(json!({"phase":"preview","mutation":mutation})),
            || NOW,
        )
        .unwrap(),
    )
    .unwrap()
}
fn apply_query(p: &Value) -> OrchWriteQuery {
    query(
        json!({"phase":"apply","mutation":p["mutation"],"expectedConfigurationRevision":p["configurationRevision"],"previewAtMs":p["previewAtMs"],"previewDigest":p["previewDigest"],"consent":true}),
    )
}
fn cursor(d: &TestDirectory) -> i64 {
    rusqlite::Connection::open(d.path.join("state.db"))
        .unwrap()
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn preview_is_read_only_and_apply_persists_one_revision_and_no_journal() {
    let (d, mut s, id) = fixture();
    let initial = cursor(&d);
    let p = preview_for(&mut s, json!({"kind":"plan.add","candidate":plan()}));
    assert_eq!(
        p["before"]["members"][0]["effective"]["receiving"]["value"]["mode"],
        "direct"
    );
    assert_eq!(
        p["after"]["members"][0]["effective"]["receiving"]["value"]["mode"],
        "hold"
    );
    assert!(
        s.orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .plans
            .is_empty()
    );
    assert_eq!(
        p["changes"],
        json!([{"identityId":id,"fields":["state","receiving","routine"]}])
    );
    assert_eq!(cursor(&d), initial);
    let applied: Value =
        serde_json::from_slice(&orchestration_write(&mut s, apply_query(&p), || NOW + 1).unwrap())
            .unwrap();
    assert_eq!(applied["configurationRevision"], 2);
    assert_eq!(applied["changed"], true);
    assert_eq!(
        s.orchestration_snapshot(None).unwrap().snapshot.plans[0].id,
        PLAN
    );
    assert!(cursor(&d) > initial);
    let page = s
        .orchestration_journal(
            &wire::OrchSubjectDocument {
                kind: "identity".into(),
                id,
            },
            None,
            50,
            NOW,
        )
        .unwrap();
    assert!(page.items.is_empty());
}
#[test]
fn consent_stale_snapshot_and_corrupt_digest_cannot_save() {
    let (d, mut s, _) = fixture();
    let p = preview_for(&mut s, json!({"kind":"plan.add","candidate":plan()}));
    let initial = cursor(&d);
    let mut no = apply_query(&p);
    if let OrchWriteQuery::Apply {
        ref mut consent, ..
    } = no
    {
        *consent = false;
    }
    assert!(orchestration_write(&mut s, no, || NOW).is_err());
    let mut bad = apply_query(&p);
    if let OrchWriteQuery::Apply {
        ref mut preview_digest,
        ..
    } = bad
    {
        *preview_digest = "0".repeat(64);
    }
    assert!(matches!(
        orchestration_write(&mut s, bad, || NOW),
        Err(OrchReadFailure::Policy(OrchError::Conflict))
    ));
    assert_eq!(cursor(&d), initial);
    create_or_resolve(&mut s, "new-seat", Lifetime::Saved).unwrap();
    let after_roster = cursor(&d);
    assert!(matches!(
        orchestration_write(&mut s, apply_query(&p), || NOW),
        Err(OrchReadFailure::Policy(OrchError::Conflict))
    ));
    assert!(
        s.orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .plans
            .is_empty()
    );
    assert_eq!(cursor(&d), after_roster);
}
#[test]
fn focus_set_clear_and_plan_replace_remove_use_the_same_write_owner() {
    let (_d, mut s, id) = fixture();
    let p = preview_for(&mut s, json!({"kind":"plan.add","candidate":plan()}));
    orchestration_write(&mut s, apply_query(&p), || NOW).unwrap();
    let mut edited = plan();
    edited["enabled"] = json!(false);
    let p = preview_for(
        &mut s,
        json!({"kind":"plan.set","plan":"Quiet","candidate":edited}),
    );
    assert_eq!(p["mutation"]["plan"], PLAN);
    orchestration_write(&mut s, apply_query(&p), || NOW).unwrap();
    let state = s.orchestration_snapshot(None).unwrap().snapshot;
    assert_eq!(state.plans[0].revision, 2);
    assert!(!state.plans[0].enabled);
    let p = preview_for(
        &mut s,
        json!({"kind":"focus.set","identityId":id,"startMs":NOW,"endMs":NOW+60_000}),
    );
    orchestration_write(&mut s, apply_query(&p), || NOW + 1).unwrap();
    assert_eq!(
        s.orchestration_snapshot(None).unwrap().snapshot.focus.len(),
        1
    );
    let p = preview_for(&mut s, json!({"kind":"focus.clear","identityId":id}));
    orchestration_write(&mut s, apply_query(&p), || NOW + 1).unwrap();
    assert!(
        s.orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .focus
            .is_empty()
    );
    let p = preview_for(&mut s, json!({"kind":"plan.rm","plan":"Quiet"}));
    orchestration_write(&mut s, apply_query(&p), || NOW + 1).unwrap();
    assert!(
        s.orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .plans
            .is_empty()
    );
}
#[test]
fn crossing_a_relevant_slot_boundary_requires_another_preview() {
    let (_d, mut s, _) = fixture();
    let mut candidate = plan();
    candidate["schedule"] = json!({"kind":"once","startMs":NOW,"endMs":NOW+60_000});
    let p = preview_for(&mut s, json!({"kind":"plan.add","candidate":candidate}));
    assert_eq!(p["validUntilMs"], NOW + 60_000);
    assert!(matches!(
        orchestration_write(&mut s, apply_query(&p), || NOW + 60_000),
        Err(OrchReadFailure::Policy(OrchError::Conflict))
    ));
    assert!(
        s.orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .plans
            .is_empty()
    );
}
#[test]
fn writer_admission_rechecks_the_snapshot_and_an_error_rolls_back() {
    let (_d, mut s, id) = fixture();
    let before = s.orchestration_snapshot(None).unwrap().snapshot;
    let result: Result<(), OrchReadFailure> =
        s.update_orchestration(|_| Err(OrchError::Conflict.into()));
    assert!(result.is_err());
    assert_eq!(s.orchestration_snapshot(None).unwrap().snapshot, before);
    let p = preview_for(
        &mut s,
        json!({"kind":"focus.set","identityId":id,"startMs":NOW,"endMs":NOW+60_000}),
    );
    orchestration_write(&mut s, apply_query(&p), || NOW).unwrap();
    assert!(matches!(
        orchestration_write(&mut s, apply_query(&p), || NOW),
        Err(OrchReadFailure::Policy(OrchError::Conflict))
    ));
}
#[test]
fn unknown_duplicate_and_partial_mutation_fields_are_refused() {
    for text in [
        r#"{"phase":"preview","mutation":{"kind":"focus.clear","identityId":"10000000-0000-4000-8000-000000000001","extra":true}}"#,
        r#"{"phase":"preview","mutation":{"kind":"plan.rm","plan":"x","plan":"y"}}"#,
        r#"{"phase":"apply","mutation":{"kind":"plan.rm","plan":"x"},"consent":true}"#,
    ] {
        assert!(decode_orch_write(text.as_bytes()).is_err());
    }
}

#[test]
fn a_base_input_hidden_by_a_plan_still_invalidates_its_preview_fence() {
    let (_d, mut s, id) = fixture();
    let p = preview_for(&mut s, json!({"kind":"plan.add","candidate":plan()}));
    orchestration_write(&mut s, apply_query(&p), || NOW).unwrap();
    let mut state = s.orchestration_snapshot(None).unwrap();
    state.snapshot.base_receiving.insert(
        id.clone(),
        OrchBaseReceiving {
            revision: 7,
            receiving: OrchReceiving::Auto,
        },
    );
    let zone = OrchZone::load(Some("UTC")).unwrap();
    let mutation = OrchMutationDocument::ClearFocus {
        identity_id: id.clone(),
    };
    let before = preview(&state, &zone, &mutation, NOW).unwrap();
    state.snapshot.base_receiving.get_mut(&id).unwrap().revision = 8;
    let after = preview(&state, &zone, &mutation, NOW).unwrap();
    assert_eq!(before.value["before"], after.value["before"]);
    assert_eq!(before.value["after"], after.value["after"]);
    assert_ne!(before.digest, after.digest);
}
