use super::*;
use serde_json::{Value, json};

fn preview() -> Value {
    let maps: Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/map-v1-vectors.json"
    ))
    .unwrap();
    json!({
        "worldId": null, "revision": 0, "legacyBasis": "a".repeat(64),
        "layout": {"version": 1, "map": maps["lobby"], "objects": []},
        "updatedAtMs": 0, "changed": false,
    })
}

fn decode(value: &Value) -> Result<Result<LocalWorldSnapshot, WorldFailure>, WorldCodecError> {
    decode_reply(&serde_json::to_vec(value).unwrap())
}

#[test]
fn preview_and_saved_replies_share_the_same_world_codec() {
    let source = preview();
    let snapshot = decode(&source).unwrap().unwrap();
    assert!(snapshot.world_id.is_none());
    assert_eq!(
        crate::codec::office_world::snapshot_value(&snapshot),
        source
    );

    let mut saved = source;
    saved["worldId"] = json!("10000000-0000-4000-8000-000000000001");
    saved["revision"] = json!(1);
    saved["legacyBasis"] = Value::Null;
    saved["updatedAtMs"] = json!(1234);
    saved["changed"] = json!(true);
    let snapshot = decode(&saved).unwrap().unwrap();
    assert_eq!(crate::codec::office_world::snapshot_value(&snapshot), saved);
}

#[test]
fn malformed_snapshot_metadata_cannot_be_published_as_success() {
    for (key, value) in [
        ("extra", json!(true)),
        ("worldId", json!("not-a-world")),
        ("worldId", json!("00000000-0000-0000-0000-000000000000")),
        ("revision", json!(1)),
        ("revision", json!(0.5)),
        ("updatedAtMs", json!(-1)),
        ("updatedAtMs", json!(1)),
        ("updatedAtMs", json!(MAX_JS_SAFE_INTEGER + 1)),
        ("legacyBasis", Value::Null),
        ("legacyBasis", json!("not-a-digest")),
        ("changed", json!("false")),
        ("changed", json!(true)),
    ] {
        let mut value_under_test = preview();
        value_under_test[key] = value;
        assert!(
            decode(&value_under_test).is_err(),
            "{key}: {value_under_test}"
        );
    }
    for key in [
        "worldId",
        "revision",
        "legacyBasis",
        "layout",
        "updatedAtMs",
        "changed",
    ] {
        let mut value = preview();
        value.as_object_mut().unwrap().remove(key);
        assert!(decode(&value).is_err(), "missing {key}");
    }
    let duplicate = preview()
        .to_string()
        .replace("\"revision\":0", "\"revision\":0,\"revision\":0");
    assert!(decode_reply(duplicate.as_bytes()).is_err());
    let mut invalid_layout = preview();
    invalid_layout["layout"]["version"] = json!(999);
    assert!(matches!(
        decode(&invalid_layout),
        Err(WorldCodecError::UnsupportedVersion)
    ));
}

#[test]
fn public_diagnostics_are_closed_and_bounded_not_arbitrary_companion_output() {
    for code in [
        WorldFailureCode::WorldInvalid,
        WorldFailureCode::WorldStoredInvalid,
        WorldFailureCode::WorldMigrationInvalid,
        WorldFailureCode::WorldRevisionConflict,
        WorldFailureCode::WorldRevisionExhausted,
        WorldFailureCode::WorldIdentityIneligible,
        WorldFailureCode::WorldRoomMissing,
        WorldFailureCode::WorldPropUnavailable,
        WorldFailureCode::StorageUnavailable,
    ] {
        let failure = decode(&json!({"error": code.code()})).unwrap().unwrap_err();
        assert_eq!(failure.code, code);
    }
    let valid = json!({"error": "WORLD_INVALID", "message": "Invalid placement.",
        "issues": [{"objectId": null, "reason": "invalidId"}]});
    assert!(decode(&valid).unwrap().is_err());
    for invalid in [
        json!({"error": "UNKNOWN_ERROR"}),
        json!({"error": "WORLD_INVALID", "extra": true}),
        json!({"error": "WORLD_INVALID", "message": "x".repeat(1025)}),
        json!({"error": "WORLD_INVALID", "issues": [{"reason": "outsideFloor"}]}),
        json!({"error": "WORLD_INVALID", "issues": [{"objectId": "not-an-id", "reason": "outsideFloor"}]}),
        json!({"error": "WORLD_INVALID", "issues": [{"objectId": null, "reason": "invented"}]}),
        json!({"error": "WORLD_INVALID", "issues": [{"objectId": null, "reason": "outsideFloor", "extra": true}]}),
        json!({"error": "WORLD_INVALID", "issues": vec![json!({"objectId": null, "reason": "invalidId"}); crate::office_world::MAX_OBJECTS + 1]}),
    ] {
        assert!(decode(&invalid).is_err());
    }
    assert!(matches!(
        decode_reply(&vec![b' '; WORLD_REPLY_LIMIT + 1]),
        Err(WorldCodecError::TooLarge)
    ));
}
