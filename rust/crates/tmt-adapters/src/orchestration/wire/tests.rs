use super::*;
const PLAN: &str = "11111111-1111-4111-8111-111111111111";
fn document() -> Value {
    json!({"id":PLAN,"name":"Quiet","revision":1,"priority":1,"enabled":true,"targets":[{"kind":"all"}],"schedule":{"kind":"always"},"fields":{"state":"sleep"}})
}
#[test]
fn unset_and_explicit_unlimited_survive_the_wire_and_projection() {
    let mut v = document();
    v["fields"]["talkLimit"] = Value::Null;
    let p: OrchPlanDocument = serde_json::from_value(v).unwrap();
    let p = p.to_core().unwrap();
    assert_eq!(p.fields.talk_limit, Some(None));
    let round: OrchPlanDocument = serde_json::from_value(plan_value(&p).unwrap()).unwrap();
    assert_eq!(round.to_core().unwrap(), p);
    let absent: OrchPlanDocument = serde_json::from_value(document()).unwrap();
    assert_eq!(absent.to_core().unwrap().fields.talk_limit, None);
}
#[test]
fn malformed_partial_unknown_and_duplicate_fields_are_rejected() {
    for (key, value) in [
        ("state", Value::Null),
        ("receiving", json!({"mode":"batch","intervalMs":1})),
        ("sending", json!("all")),
        ("talkLimit", json!(0)),
        ("secret", json!("ignored")),
    ] {
        let mut v = document();
        v["fields"][key] = value;
        assert!(
            serde_json::from_value::<OrchPlanDocument>(v)
                .ok()
                .and_then(|p| p.to_core().ok())
                .is_none(),
            "{key}"
        );
    }
    let duplicate = format!("{{\"id\":\"{PLAN}\",\"id\":\"{PLAN}\"}}");
    assert!(serde_json::from_str::<OrchPlanDocument>(&duplicate).is_err());
    for body in [
        r#"{"view":"overview","rangeDays":2}"#,
        r#"{"view":"why","subject":{"kind":"request","id":"req_short"}}"#,
        r#"{"view":"overview","unknown":true}"#,
        r#"{"view":"overview","atMs":1,"atMs":2}"#,
    ] {
        assert!(decode_orch_query(body.as_bytes()).is_err());
    }
}
#[test]
fn weekly_admission_normalizes_anchor_and_rejects_empty_or_duplicate_days() {
    let mut v = document();
    v["schedule"] = json!({"kind":"weekly","anchorDate":"2026-12-31","intervalWeeks":2,"weekdays":[1,3],"start":"22:00","end":"06:00"});
    let p: OrchPlanDocument = serde_json::from_value(v.clone()).unwrap();
    let p = p.to_core().unwrap();
    assert_eq!(
        plan_value(&p).unwrap()["schedule"]["anchorDate"],
        "2026-12-28"
    );
    for days in [json!([]), json!([1, 1]), json!([0])] {
        v["schedule"]["weekdays"] = days;
        assert!(
            serde_json::from_value::<OrchPlanDocument>(v.clone())
                .unwrap()
                .to_core()
                .is_err()
        );
    }
}
#[test]
fn preview_file_is_bounded_regular_and_no_follow() {
    let d = crate::test_support::TestDirectory::new();
    let file = d.path.join("plan.json");
    std::fs::write(&file, document().to_string()).unwrap();
    assert!(orch_candidate_file(&file).is_ok());
    let link = d.path.join("linked.json");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(orch_candidate_file(&link).is_err());
    std::fs::write(&file, vec![b' '; ORCH_DOCUMENT_BYTES + 1]).unwrap();
    assert!(matches!(orch_candidate_file(&file), Err(OrchError::Limit)));
    assert!(orch_candidate_file(&d.path).is_err());
}
#[test]
fn stored_source_projection_rejects_unknown_content_and_invalid_ownership() {
    let source = OrchSource {
        kind: OrchSourceKind::Plan,
        plan_id: Some(PLAN.into()),
        identity_id: None,
        revision: 2,
        slot: None,
        defaulted: true,
    };
    let v = decision_source_value(&OrchDecisionSource {
        field: OrchPolicyField::Receiving,
        source,
    });
    assert_eq!(
        decode_sources(&json!([v.clone()]).to_string()).unwrap(),
        vec![v.clone()]
    );
    let mut extra = v.clone();
    extra["source"]["messageBody"] = json!("must not escape through journal reads");
    assert!(decode_sources(&json!([extra]).to_string()).is_err());
    let mut invalid = v;
    invalid["source"]["planId"] = json!("not-a-uuid");
    assert!(decode_sources(&json!([invalid]).to_string()).is_err());
}
#[test]
fn handwritten_contract_vectors_freeze_defaults_inheritance_and_sources() {
    let vectors: Value = serde_json::from_str(include_str!("tests/vectors.json")).unwrap();
    let zone = crate::orchestration::OrchZone::load(Some("UTC")).unwrap();
    for case in vectors["cases"].as_array().unwrap() {
        let mut plans = vectors["plans"].clone();
        plans[1]["fields"] = case["higherFields"].clone();
        let plans = plans
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                serde_json::from_value::<OrchPlanDocument>(p.clone())
                    .unwrap()
                    .to_core()
                    .unwrap()
            })
            .collect();
        let id = vectors["identity"]["id"].as_str().unwrap();
        let snapshot = OrchSnapshot {
            identities: vec![OrchIdentity {
                id: id.into(),
                name: vectors["identity"]["name"].as_str().unwrap().into(),
            }],
            plans,
            ..Default::default()
        };
        let p = effective(&snapshot, id, vectors["atMs"].as_u64().unwrap(), &zone).unwrap();
        let v = effective_value(&p);
        for field in ["state", "receiving", "sending", "routine", "talkLimit"] {
            assert_eq!(
                v[field]["value"], case["expected"][field],
                "{} {field}",
                case["name"]
            );
        }
        assert_eq!(
            v["receiving"]["source"]["defaulted"],
            case["receivingDefaulted"]
        );
        assert_eq!(v["sending"]["source"]["planId"], vectors["plans"][0]["id"]);
    }
}
#[test]
fn fieldless_wire_variants_reject_extra_content() {
    let mut receiving = document();
    receiving["fields"]["receiving"] = json!({"mode":"direct","unexpected":"discarded"});
    let mut target = document();
    target["targets"][0]["unexpected"] = json!(true);
    let mut schedule = document();
    schedule["schedule"]["unexpected"] = json!(true);
    let accepted: Vec<_> = [
        ("receiving", receiving),
        ("target", target),
        ("schedule", schedule),
    ]
    .into_iter()
    .filter_map(|(name, v)| {
        serde_json::from_value::<OrchPlanDocument>(v)
            .ok()
            .and_then(|p| p.to_core().ok())
            .map(|_| name)
    })
    .collect();
    assert_eq!(
        accepted,
        Vec::<&str>::new(),
        "fieldless variants must reject every unknown field"
    );
}
