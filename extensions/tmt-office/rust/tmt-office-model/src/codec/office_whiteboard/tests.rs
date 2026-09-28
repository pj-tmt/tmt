use super::*;
use serde_json::{Value, json};

const EXAMPLE: &[u8] = include_bytes!("../../../../../contracts/whiteboard-scene-v1.json");

fn example() -> Value {
    serde_json::from_slice(EXAMPLE).unwrap()
}

#[test]
fn whole_number_notation_matches_browser_values_without_rounding_fractions() {
    let source = serde_json::to_string(&example()).unwrap();
    for (before, after) in [
        ("\"formatVersion\":1", "\"formatVersion\":1.0"),
        ("\"width\":1600", "\"width\":1.6e3"),
        ("[650,450]", "[650.0,4.5e2]"),
    ] {
        assert!(source.contains(before));
        let changed = source.replacen(before, after, 1);
        let scene = decode_scene(changed.as_bytes()).unwrap_or_else(|_| {
            panic!(
                "Failed notation {after}: {:?}",
                serde_json::from_str::<SceneWire>(&changed)
            )
        });
        assert_eq!(
            serde_json::from_slice::<Value>(&encode_scene(&scene).unwrap()).unwrap(),
            example()
        );
    }
    assert!(
        decode_scene(
            source
                .replacen("\"width\":1600", "\"width\":1600.1", 1)
                .as_bytes()
        )
        .is_err()
    );
}
fn admits(value: &Value) -> bool {
    decode_scene(&serde_json::to_vec(value).unwrap()).is_ok()
}

#[test]
fn shared_vectors_and_all_element_kinds_round_trip_without_reordering() {
    let scene = decode_scene(EXAMPLE).unwrap();
    assert_eq!(scene.elements.len(), 6);
    assert_eq!(
        serde_json::from_slice::<Value>(&encode_scene(&scene).unwrap()).unwrap(),
        example()
    );
    let vectors: Value = serde_json::from_slice(include_bytes!(
        "../../../../../contracts/whiteboard-vectors.json"
    ))
    .unwrap();
    for case in vectors.as_array().unwrap() {
        let result = decode_scene(&serde_json::to_vec(&case["value"]).unwrap());
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Ok(scene) = result {
            assert_eq!(
                serde_json::from_slice::<Value>(&encode_scene(&scene).unwrap()).unwrap(),
                case["value"]
            );
        }
    }
}

#[test]
fn raw_duplicate_fields_unknown_fields_and_unpaired_surrogates_reject() {
    let source = serde_json::to_string(&example()).unwrap();
    for (original, replacement) in [
        (
            "\"formatVersion\":1",
            "\"formatVersion\":1,\"formatVersion\":1",
        ),
        ("\"fontSize\":24", "\"fontSize\":24,\"fontSize\":24"),
        ("\"kind\":\"note\"", "\"kind\":\"note\",\"kind\":\"note\""),
    ] {
        let bytes = source.replacen(original, replacement, 1);
        assert_ne!(bytes, source, "fixture marker must exist");
        assert!(decode_scene(bytes.as_bytes()).is_err());
    }
    let invalid_text = source.replacen("What should we build next?", "\\ud800", 1);
    assert!(decode_scene(invalid_text.as_bytes()).is_err());
}

#[test]
fn element_ids_are_unique_canonical_rfc_uuids() {
    let mut value = example();
    let valid = value["elements"][0]["id"].clone();
    for id in [
        json!("not-an-id"),
        json!("11111111-1111-4111-0111-111111111111"),
        json!("11111111-1111-7111-8111-111111111111"),
    ] {
        value["elements"][0]["id"] = id;
        assert!(!admits(&value));
    }
    value["elements"][0]["id"] = valid.clone();
    assert!(admits(&value));
    value["elements"][1]["id"] = valid;
    assert!(!admits(&value));
}

#[test]
fn utf8_and_aggregate_budgets_have_valid_boundary_controls() {
    let mut value = example();
    let mut note = value["elements"][0].clone();
    note["text"] = json!("🙂".repeat(policy::TEXT_BYTES / 4));
    value["elements"] = json!([note]);
    assert!(admits(&value));
    value["elements"][0]["text"] = json!(format!("{}x", "🙂".repeat(policy::TEXT_BYTES / 4)));
    assert!(!admits(&value));
    let notes = (0..4)
        .map(|index| {
            let mut note = note.clone();
            note["id"] = json!(format!("{index:08x}-1111-4111-8111-111111111111"));
            note
        })
        .collect::<Vec<_>>();
    value["elements"] = json!(notes);
    assert!(admits(&value));
    let mut extra = note.clone();
    extra["text"] = json!("x");
    value["elements"].as_array_mut().unwrap().push(extra);
    assert!(!admits(&value));

    let mut path = example()["elements"][3].clone();
    path["points"] = json!(vec![[0, 0]; policy::POINT_LIMIT]);
    value["elements"] = json!([path]);
    assert!(admits(&value));
    value["elements"][0]["points"]
        .as_array_mut()
        .unwrap()
        .push(json!([1, 1]));
    assert!(!admits(&value));
}

#[test]
fn bounded_raw_input_and_element_count_do_not_truncate_content() {
    let mut value = example();
    let shape = value["elements"][1].clone();
    value["elements"] = json!(
        (0..policy::ELEMENT_LIMIT)
            .map(|index| {
                let mut item = shape.clone();
                item["id"] = json!(format!("{index:08x}-1111-4111-8111-111111111111"));
                item
            })
            .collect::<Vec<_>>()
    );
    assert!(admits(&value));
    value["elements"].as_array_mut().unwrap().push(shape);
    assert!(!admits(&value));
    value["elements"] = json!([]);
    let mut source = serde_json::to_vec(&value).unwrap();
    source.resize(policy::DOCUMENT_BYTES, b' ');
    assert!(decode_scene(&source).is_ok());
    source.push(b' ');
    assert!(decode_scene(&source).is_err());
}
