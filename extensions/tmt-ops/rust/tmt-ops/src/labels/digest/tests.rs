use super::*;
use serde_json::json;

fn document() -> Value {
    super::super::tests::vectors()["status"][0]["response"].clone()
}

fn parse(document: &Value) -> Option<Status> {
    let labels = super::super::parse("digest", document)?;
    Status::from_source("digest", document, &labels)
}

#[test]
fn supplier_vectors_preserve_settings_null_evidence_and_source_times() {
    for vector in super::super::tests::vectors()["status"].as_array().unwrap() {
        let document = &vector["response"];
        let status = parse(document).expect("supplier status vector");
        assert_eq!(
            status.observed_at_ms,
            document["observedAtMs"].as_u64().unwrap()
        );
        assert_eq!(
            status.members.len(),
            document["members"].as_array().unwrap().len()
        );
        for supplied in document["members"].as_array().unwrap() {
            let member = &status.members[supplied["identityId"].as_str().unwrap()];
            assert_eq!(member.name, supplied["name"]);
            assert_eq!(member.held_count, supplied["heldCount"].as_u64());
            assert_eq!(
                member.stats_observed_at_ms,
                supplied["statsObservedAtMs"].as_u64()
            );
            assert_eq!(member.next_send_at_ms, supplied["nextSendAtMs"].as_u64());
            assert_eq!(member.time_to_send_ms, supplied["timeToSendMs"].as_u64());
            assert_eq!(member.setting_value, supplied["settingValue"]);
            assert_eq!(member.set_at_ms, supplied["setAtMs"].as_u64());
            assert_eq!(
                member.set_by_identity_id.as_deref(),
                supplied["setByIdentityId"].as_str()
            );
        }
    }
}

#[test]
fn auto_default_and_override_are_distinct_without_invented_deadlines() {
    let status = parse(&document()).unwrap();
    let inherited = &status.members["10000000-0000-4000-8000-000000000001"];
    let explicit = &status.members["10000000-0000-4000-8000-000000000002"];
    assert_eq!(inherited.mode, Mode::Auto);
    assert_eq!(inherited.setting_source, SettingSource::Default);
    assert_eq!(inherited.held_count, Some(0));
    assert_eq!(explicit.mode, Mode::Auto);
    assert_eq!(explicit.setting_source, SettingSource::Member);
    assert_eq!(explicit.held_count, Some(3));
    assert_eq!(explicit.next_send_at_ms, None);
    assert_eq!(explicit.time_to_send_ms, None);
}

#[test]
fn zero_count_and_unavailable_count_remain_distinct() {
    let mut document = document();
    let id = document["members"][0]["identityId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(parse(&document).unwrap().members[&id].held_count, Some(0));
    document["members"][0]["heldCount"] = Value::Null;
    document["members"][0]["statsObservedAtMs"] = Value::Null;
    let unknown = parse(&document).unwrap();
    assert_eq!(unknown.members[&id].held_count, None);
    assert_eq!(unknown.members[&id].stats_observed_at_ms, None);
    document["members"][0]
        .as_object_mut()
        .unwrap()
        .remove("heldCount");
    assert!(
        parse(&document).is_none(),
        "missing required evidence is not null or zero"
    );
}

#[test]
fn required_shapes_safe_numbers_and_canonical_identity_fail_closed() {
    for (field, value) in [
        ("identityId", json!("alias")),
        ("name", json!(false)),
        ("identityId", json!("AAAAAAAA-0000-4000-8000-000000000001")),
        ("mode", json!("idle")),
        ("intervalMs", json!(0)),
        ("settingSource", json!("global")),
        ("settingValue", json!("auto")),
        ("heldCount", json!(-1)),
        ("heldCount", json!(1.5)),
        ("statsObservedAtMs", json!(SAFE + 1)),
        ("nextSendAtMs", json!(1)),
        ("timeToSendMs", json!("0")),
        ("setAtMs", json!(0)),
        ("setByIdentityId", json!("not-an-identity")),
    ] {
        let mut document = document();
        document["members"][0][field] = value;
        assert!(
            parse(&document).is_none(),
            "accepted {field}: {}",
            document["members"][0][field]
        );
    }
    for field in ["version", "observedAtMs", "members"] {
        let mut document = document();
        document.as_object_mut().unwrap().remove(field);
        assert!(parse(&document).is_none());
    }
    let mut document = document();
    document["version"] = json!(2);
    assert!(parse(&document).is_none());
}

#[test]
fn duplicate_identity_is_not_silently_overwritten_and_namespace_is_external() {
    let mut document = document();
    let labels = super::super::parse("digest", &document).unwrap();
    assert!(Status::from_source("other", &document, &labels).is_none());
    let duplicate = document["members"][0].clone();
    document["members"].as_array_mut().unwrap().push(duplicate);
    assert!(parse(&document).is_none());
}

#[test]
fn threshold_zero_retains_scheduled_deadline_and_additive_fields_are_ignored() {
    let mut document = document();
    let id = document["members"][2]["identityId"]
        .as_str()
        .unwrap()
        .to_owned();
    document["members"][2]["timeToSendMs"] = json!(0);
    let status = parse(&document).unwrap();
    assert_eq!(status.members[&id].mode, Mode::Interval);
    assert_eq!(status.members[&id].time_to_send_ms, Some(0));
    assert!(status.members[&id].next_send_at_ms.unwrap() > status.observed_at_ms);
    document["future"] = json!({"unknown": true});
    document["members"][2]["future"] = json!([1, 2]);
    // Savings has no published numeric contract in this phase.
    document["members"][2]["savings"] = json!({"future": true});
    assert_eq!(parse(&document), Some(status));
}
