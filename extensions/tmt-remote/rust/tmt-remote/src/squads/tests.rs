use super::*;
use serde_json::{Value, json};
const LEAD: &str = "00000000-0000-4000-8000-000000000001";
const MEMBER: &str = "00000000-0000-4000-8000-000000000002";
fn row(id: &str, name: &str) -> Value {
    json!({"id":id,"name":name,"state":"working","presence":"active"})
}
fn board() -> Value {
    json!({"squads":[{"squad":{"name":"infra","lead":row(LEAD,"lead")},
        "sections":[{"rows":[row(MEMBER,"worker")]}]}]})
}
#[test]
fn only_the_approved_projection_crosses_the_machine_boundary() {
    let mut input = board();
    let member = &mut input["squads"][0]["sections"][0]["rows"][0];
    member["activity"] = json!({"activity":"PRIVATE-ACTIVITY", "state":"idle"});
    for key in [
        "mood",
        "idleAlert",
        "staleness",
        "colors",
        "attention",
        "task",
        "next",
        "pending",
        "usage",
        "timers",
        "history",
        "pane",
        "cwd",
    ] {
        member[key] = json!(format!("PRIVATE-{key}"));
    }
    input["squads"][0]["squad"]["attention"] = json!("PRIVATE-ATTENTION");
    input["unlisted"] = json!("PRIVATE-ROOT");
    let summary = Summary::from_ops("Studio", 1234, &input).unwrap();
    let bytes = summary.bytes().unwrap();
    let wire = String::from_utf8(bytes.clone()).unwrap();
    assert!(!wire.contains("PRIVATE"), "free text leaked: {wire}");
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({
            "version":1,"machineName":"Studio","observedAt":1234,
            "squads":[{"squad":"infra","lead":{"id":LEAD,"name":"lead","state":"working","presence":"active"},
            "members":[{"id":MEMBER,"name":"worker","state":"working","presence":"active"}]}]
        })
    );
}
#[test]
fn state_is_an_allowlist_over_the_public_field_not_an_activity_guess() {
    for state in [
        "working",
        "waiting",
        "waiting-ci",
        "idle",
        "resting",
        "blocked",
        "review",
        "testing",
        "hold",
    ] {
        let mut input = board();
        input["squads"][0]["sections"][0]["rows"][0]["state"] = json!(state);
        let result = Summary::from_ops("Studio", 1234, &input).unwrap();
        assert_eq!(
            serde_json::to_value(&result).unwrap()["squads"][0]["members"][0]["state"],
            state
        );
    }
    for state in [
        json!("SECRET-STATE"),
        Value::Null,
        json!(7),
        json!({"state":"idle"}),
    ] {
        let mut input = board();
        let row = &mut input["squads"][0]["sections"][0]["rows"][0];
        row["state"] = state;
        row["presence"] = json!("CUSTOM-PRESENCE");
        row["activity"] = json!({"activity":"PRIVATE-ACTIVITY", "state":"working"});
        row["fields"] = json!({"state":"idle"});
        let result = Summary::from_ops("Studio", 1234, &input).unwrap();
        assert_eq!(result.squads[0].members[0].state, State::Unknown);
        assert_eq!(
            result.squads[0].members[0].presence,
            MemberPresence::Unknown
        );
        let wire = String::from_utf8(result.bytes().unwrap()).unwrap();
        assert!(!wire.contains("SECRET") && !wire.contains("PRIVATE") && !wire.contains("CUSTOM"));
    }
}
#[test]
fn names_are_display_only_and_null_leads_stay_absent() {
    let mut input = board();
    input["squads"][0]["squad"]["lead"] = Value::Null;
    input["squads"][0]["sections"][0]["rows"] = json!([row(LEAD, "same"), row(MEMBER, "same")]);
    let first = Summary::from_ops("Studio", 1234, &input).unwrap();
    assert!(first.squads[0].lead.is_none());
    input["squads"][0]["sections"][0]["rows"][0]["name"] = json!("renamed");
    let second = Summary::from_ops("Studio", 1234, &input).unwrap();
    assert_eq!(
        first.squads[0].members[0].id,
        second.squads[0].members[0].id
    );
    assert_eq!(second.squads[0].members.len(), 2);
    assert!(
        serde_json::to_value(second).unwrap()["squads"][0]
            .get("lead")
            .is_none()
    );
}
#[test]
fn changed_shapes_and_duplicate_identity_keys_cannot_look_like_an_empty_roster() {
    for input in [
        json!({}),
        json!({"squads":{}}),
        json!({"squads":[{"squad":{"name":"infra","lead":null}}]}),
    ] {
        assert!(Summary::from_ops("Studio", 1234, &input).is_err());
    }
    let mut input = board();
    input["squads"][0]["sections"][0]["rows"]
        .as_array_mut()
        .unwrap()
        .push(row(MEMBER, "other name"));
    assert!(Summary::from_ops("Studio", 1234, &input).is_err());
    assert!(Summary::from_ops("Studio", 1234, &json!({"squads":[]})).is_ok());
}
#[test]
fn received_summaries_drop_unknown_fields_and_normalize_unknown_values() {
    let mut value =
        serde_json::to_value(Summary::from_ops("Studio", 1234, &board()).unwrap()).unwrap();
    value["secret"] = json!("PRIVATE");
    let row = &mut value["squads"][0]["members"][0];
    row["state"] = json!("custom-secret");
    row["presence"] = json!(77);
    row["activity"] = json!("PRIVATE-ACTIVITY");
    let parsed = Summary::read(value.to_string().as_bytes()).unwrap();
    assert_eq!(parsed.squads[0].members[0].state, State::Unknown);
    assert_eq!(
        parsed.squads[0].members[0].presence,
        MemberPresence::Unknown
    );
    let wire = String::from_utf8(parsed.bytes().unwrap()).unwrap();
    assert!(!wire.contains("PRIVATE") && !wire.contains("custom-secret"));
}
#[test]
fn summary_bounds_and_version_refuse_without_truncating_identity() {
    let mut summary = Summary::from_ops("Studio", 1234, &board()).unwrap();
    summary.version = 2;
    assert!(Summary::read(&serde_json::to_vec(&summary).unwrap()).is_err());
    summary.version = 1;
    summary.machine_name = "x".repeat(crate::limits::SQUAD_SUMMARY_NAME_BYTES + 1);
    assert!(summary.bytes().is_err());
    summary.machine_name = "Studio".into();
    summary.squads[0].members[0].id = "bad".into();
    assert!(summary.bytes().is_err());
}

#[test]
fn directory_filters_the_local_machine_and_never_turns_bad_storage_into_an_empty_list() {
    let record = |machine: &str, remote: &str| Record {
        machine_id: machine.into(),
        remote: remote.into(),
        summary: Summary::from_ops("Studio", 1234, &board()).unwrap(),
    };
    let list = project_records(LEAD, vec![record(LEAD, "here"), record(MEMBER, "other")]).unwrap();
    assert_eq!(list.machines.len(), 1);
    assert_eq!(list.machines[0].remote, "other");
    assert!(project_records(LEAD, vec![record(MEMBER, "other"), record(MEMBER, "other")]).is_err());
    assert!(project_records(LEAD, vec![record("invalid", "other")]).is_err());
    assert_eq!(list.machines[0].summary.age_ms(2000), Some(766));
    assert_eq!(list.machines[0].summary.age_ms(1000), None);
}
#[test]
fn delivery_uses_carrier_or_original_receipts_and_never_serializes_remote_text() {
    use crate::cloud_inbox::DeliveryEvidence;
    let original = MEMBER;
    assert_eq!(
        project_delivery(original, DeliveryEvidence::Unknown).state,
        DeliveryPhase::Unknown
    );
    assert_eq!(
        project_delivery(original, DeliveryEvidence::CarrierStored).state,
        DeliveryPhase::Queued
    );
    for (phase, expected) in [
        ("held", DeliveryPhase::Held),
        ("uncertain", DeliveryPhase::Unconfirmed),
        ("refused", DeliveryPhase::Failed),
        ("cancelled", DeliveryPhase::Failed),
        ("accepted", DeliveryPhase::Unknown),
        ("strange", DeliveryPhase::Unknown),
    ] {
        let answer = project_delivery(
            original,
            DeliveryEvidence::Journal(json!({"state":phase,"reason":"PRIVATE plaintext"})),
        );
        assert_eq!(answer.state, expected);
        assert!(!serde_json::to_string(&answer).unwrap().contains("PRIVATE"));
    }
    for (phase, expected) in [
        ("sent", DeliveryPhase::Queued),
        ("batched", DeliveryPhase::Held),
        ("delivered", DeliveryPhase::Delivered),
        ("working", DeliveryPhase::Delivered),
        ("unconfirmed", DeliveryPhase::Unconfirmed),
        ("not_sent", DeliveryPhase::Failed),
        ("unknown-new-state", DeliveryPhase::Unknown),
    ] {
        let answer = project_delivery(
            original,
            DeliveryEvidence::Request(
                json!({"messageState":{"state":phase,"reason":{"text":"PRIVATE","code":"SAFE_CODE"}},"final":{"status":"not_submitted","response":"PRIVATE"}}),
            ),
        );
        assert_eq!(answer.state, expected);
        assert!(!serde_json::to_string(&answer).unwrap().contains("PRIVATE"));
    }
    let answer = project_delivery(
        original,
        DeliveryEvidence::Request(
            json!({"messageState":{"state":"answered"},"final":{"status":"retained","response":"PRIVATE"}}),
        ),
    );
    assert_eq!(answer.state, DeliveryPhase::Answered);
    assert!(!serde_json::to_string(&answer).unwrap().contains("PRIVATE"));
    let answer = project_delivery(
        original,
        DeliveryEvidence::Request(
            json!({"messageState":{"state":"answered"},"final":{"status":"unavailable"}}),
        ),
    );
    assert_eq!(answer.state, DeliveryPhase::Unknown);
}

#[test]
fn received_scalar_and_changed_container_shapes_are_refused_without_panicking() {
    for value in [
        json!(7),
        json!("invalid"),
        Value::Null,
        json!({"squads":[7]}),
        json!({"squads":[null]}),
        json!({"squads":[{"members":7}]}),
        json!({"squads":[{"members":[7]}]}),
    ] {
        assert!(
            Summary::read(value.to_string().as_bytes()).is_err(),
            "{value}"
        );
    }
}
