use super::*;
use crate::office_block::BUILTIN_PROP_PACK_DIGEST;
use crate::office_world::PlacementError;

fn fixture() -> Value {
    let maps: Value = serde_json::from_str(include_str!(
        "../../../../../../../contracts/office/map-v1-vectors.json"
    ))
    .unwrap();
    json!({
        "version": 1,
        "map": maps["lobby"],
        "objects": [{
            "id": "30000000-0000-4000-8000-000000000001", "kind": "decoration",
            "extension": null,
            "surface": {"type": "floor"},
            "placement": {"prop": format!("{BUILTIN_PROP_PACK_DIGEST}/desk"), "footprint": {"width": 2, "height": 1},
                "x": 0, "y": 0, "rotation": 0, "customization": {"tint": "#bb7755", "text": "Design"}}
        }]
    })
}

fn decode(value: &Value) -> Result<WorldLayout, WorldCodecError> {
    decode_world(&serde_json::to_vec(value).unwrap())
}

#[test]
fn floor_base_roundtrips_without_rewriting_legacy_or_accepting_hostile_geometry() {
    let legacy = fixture();
    assert_eq!(world_value(&decode(&legacy).unwrap()), legacy);
    let mut value = legacy.clone();
    value["objects"][0]["surface"]["base"] = json!({"x": 0, "y": 0, "width": 1, "height": 1});
    assert_eq!(world_value(&decode(&value).unwrap()), value);
    for base in [
        json!(null),
        json!({}),
        json!({"x": 0, "y": 0, "width": 1, "height": 1, "extra": true}),
        json!({"x": 0.5, "y": 0, "width": 1, "height": 1}),
        json!({"x": -1, "y": 0, "width": 1, "height": 1}),
    ] {
        let mut invalid = value.clone();
        invalid["objects"][0]["surface"]["base"] = base;
        assert_eq!(decode(&invalid).unwrap_err(), WorldCodecError::InvalidJson);
    }
    for base in [
        json!({"x": 0, "y": 0, "width": 0, "height": 1}),
        json!({"x": 0, "y": 0, "width": 3, "height": 1}),
    ] {
        let mut invalid = value.clone();
        invalid["objects"][0]["surface"]["base"] = base;
        assert!(matches!(
            decode(&invalid),
            Err(WorldCodecError::Placement(_))
        ));
    }
    let text = serde_json::to_string(&value).unwrap();
    let duplicate = text.replace("\"base\":{", "\"base\":{\"x\":9,");
    assert_ne!(text, duplicate);
    assert_eq!(
        decode_world(duplicate.as_bytes()).unwrap_err(),
        WorldCodecError::InvalidJson
    );
}

#[test]
fn old_corridors_retain_readable_windows_until_explicit_validated_upgrade() {
    let mut old = fixture();
    old["map"] = serde_json::from_str::<Value>(include_str!(
        "../../../../../../../contracts/office/modules-v2-vectors.json"
    ))
    .unwrap()["starter"]
        .clone();
    old["objects"][0]["kind"] = json!("window");
    old["objects"][0]["surface"] =
        json!({"type": "wall", "axis": "horizontal", "face": "negative", "elevation": 4});
    old["objects"][0]["placement"]["x"] = json!(4);
    old["objects"][0]["placement"]["y"] = json!(-8);
    assert_eq!(world_value(&decode(&old).unwrap()), old);
    let mut candidate = old.clone();
    candidate["map"]["version"] = json!(3);
    assert!(
        matches!(decode(&candidate).unwrap_err(), WorldCodecError::Placement(WorldError::Placements(issues)) if issues[0].reason == PlacementError::WindowRequiresExterior)
    );
    assert_eq!(world_value(&decode(&old).unwrap()), old);
    candidate["objects"][0]["placement"]["y"] = json!(-48);
    candidate["objects"][0]["surface"]["face"] = json!("positive");
    assert_eq!(world_value(&decode(&candidate).unwrap()), candidate);
    assert_eq!(candidate["objects"][0]["id"], old["objects"][0]["id"]);
}

#[test]
fn codec_reuses_topology_and_artwork_owners_and_preserves_order_and_ids() {
    let mut value = fixture();
    let mut second = value["objects"][0].clone();
    second["id"] = json!("30000000-0000-4000-8000-000000000002");
    value["objects"].as_array_mut().unwrap().insert(0, second);
    let world = decode(&value).unwrap();
    assert_eq!(world_value(&world), value);
    assert_eq!(
        world.objects()[0].id,
        "30000000-0000-4000-8000-000000000002"
    );
}

#[test]
fn strict_nested_fields_and_duplicate_members_cannot_bypass_child_admission() {
    for path in [
        "",
        "/map",
        "/map/areas/0/binding",
        "/objects/0",
        "/objects/0/surface",
        "/objects/0/placement",
        "/objects/0/placement/footprint",
        "/objects/0/placement/customization",
    ] {
        let mut value = fixture();
        value
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(true));
        assert_eq!(
            decode(&value).unwrap_err(),
            WorldCodecError::InvalidJson,
            "{path}"
        );
    }
    let text = serde_json::to_string(&fixture()).unwrap();
    for (from, to) in [
        ("\"width\":2", "\"width\":2,\"width\":2"),
        ("\"version\":1", "\"version\":1,\"version\":1"),
        (
            "\"type\":\"floor\"",
            "\"type\":\"floor\",\"type\":\"floor\"",
        ),
    ] {
        assert_eq!(
            decode_world(text.replace(from, to).as_bytes()).unwrap_err(),
            WorldCodecError::InvalidJson
        );
    }
}

#[test]
fn floors_windows_and_wall_lights_roundtrip_without_materializing_resources() {
    for kind in ["window", "wallLight", "decoration"] {
        let mut value = fixture();
        value["objects"][0]["kind"] = json!(kind);
        value["objects"][0]["surface"] =
            json!({"type": "wall", "axis": "horizontal", "face": "positive", "elevation": 4});
        assert_eq!(world_value(&decode(&value).unwrap()), value);
    }
    let mut invalid = fixture();
    invalid["objects"][0]["kind"] = json!("window");
    assert!(
        matches!(decode(&invalid).unwrap_err(), WorldCodecError::Placement(WorldError::Placements(issues)) if issues[0].reason == PlacementError::InvalidSurface)
    );
}

#[test]
fn invalid_child_geometry_is_rejected_as_one_candidate_not_filtered_objects() {
    let mut value = fixture();
    value["objects"][0]["placement"]["x"] = json!(2);
    assert!(
        matches!(decode(&value).unwrap_err(), WorldCodecError::Placement(WorldError::Placements(issues)) if issues.len() == 1 && issues[0].reason == PlacementError::OutsideFloor)
    );
    value["map"]["doors"] = json!([{"x": 0, "y": 0, "axis": "horizontal"}]);
    assert!(matches!(
        decode(&value).unwrap_err(),
        WorldCodecError::Map(_)
    ));
}

#[test]
fn wire_budgets_versions_and_numeric_values_are_explicit() {
    assert_eq!(
        decode_world(&vec![b' '; WORLD_DOCUMENT_LIMIT + 1]).unwrap_err(),
        WorldCodecError::TooLarge
    );
    let mut value = fixture();
    value["version"] = json!(2);
    assert_eq!(
        decode(&value).unwrap_err(),
        WorldCodecError::UnsupportedVersion
    );
    let text = serde_json::to_string(&fixture()).unwrap();
    let floats = text
        .replace("\"x\":0", "\"x\":-0.0")
        .replace("\"width\":2", "\"width\":2e0")
        .replace("\"rotation\":0", "\"rotation\":0.0");
    assert_eq!(
        world_value(&decode_world(floats.as_bytes()).unwrap()),
        fixture()
    );
    let nested = text
        .replace("\"version\":1", "\"version\":1e0")
        .replace("\"y\":0", "\"y\":-0.0")
        .replace("\"start\":0", "\"start\":0e0")
        .replace("\"end\":3", "\"end\":3.0");
    assert_eq!(
        world_value(&decode_world(nested.as_bytes()).unwrap()),
        fixture()
    );
    assert_eq!(
        decode_world(text.replace("\"width\":2", "\"width\":2.5").as_bytes()).unwrap_err(),
        WorldCodecError::InvalidJson
    );
}

#[test]
fn shared_attachment_vectors_preserve_references_without_admitting_executable_fields() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../../../../contracts/office/world-v1-vectors.json"
    ))
    .unwrap();
    assert_eq!(vectors["limits"]["documentBytes"], WORLD_DOCUMENT_LIMIT);
    assert_eq!(
        vectors["limits"]["objects"],
        crate::office_world::MAX_OBJECTS
    );
    assert_eq!(
        vectors["limits"]["wallHeight"],
        crate::office_world::WALL_HEIGHT
    );
    for case in vectors["attachments"].as_array().unwrap() {
        let mut value = fixture();
        value["objects"][0]["extension"] = case["value"].clone();
        let result = decode(&value);
        assert_eq!(
            result.is_ok(),
            case["nativeValid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Ok(world) = result {
            assert_eq!(world_value(&world), value);
        }
    }
    let mut absent = fixture();
    absent["objects"][0]
        .as_object_mut()
        .unwrap()
        .remove("extension");
    assert_eq!(decode(&absent).unwrap_err(), WorldCodecError::InvalidJson);
    let text = serde_json::to_string(&fixture()).unwrap();
    assert_eq!(
        decode_world(
            text.replace(
                "\"extension\":null",
                "\"extension\":null,\"extension\":null"
            )
            .as_bytes()
        )
        .unwrap_err(),
        WorldCodecError::InvalidJson
    );
}

#[test]
fn save_envelope_preserves_raw_nested_admission_and_requires_the_correct_migration_fence_shape() {
    let input = json!({"expectedRevision":0,"legacyBasis":"a".repeat(64),"layout":fixture()});
    let bytes = serde_json::to_vec(&input).unwrap();
    let admitted = decode_save(&bytes).unwrap();
    assert_eq!(world_value(&admitted.layout), fixture());
    assert_eq!(admitted.expected_revision, 0);
    assert_eq!(
        admitted.legacy_basis.as_deref(),
        Some("a".repeat(64).as_str())
    );
    for (field, value) in [
        ("legacyBasis", Value::Null),
        ("expectedRevision", json!(1)),
        ("actor", json!("owner")),
    ] {
        let mut invalid = input.clone();
        invalid[field] = value;
        assert!(decode_save(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    let text = String::from_utf8(bytes).unwrap();
    for (from, to) in [
        (
            "\"expectedRevision\":0",
            "\"expectedRevision\":0,\"expectedRevision\":0",
        ),
        (
            "\"extension\":null",
            "\"extension\":null,\"extension\":null",
        ),
    ] {
        assert!(text.contains(from));
        assert!(decode_save(text.replace(from, to).as_bytes()).is_err());
    }
    assert!(matches!(
        decode_save(&vec![b' '; WORLD_ENVELOPE_LIMIT + 1]),
        Err(WorldCodecError::TooLarge)
    ));
}
