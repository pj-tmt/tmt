use super::*;
use serde_json::json;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../contracts/office/map-v1-vectors.json"
    ))
    .unwrap()
}

#[test]
fn budgets_conform_to_the_versioned_literal_contract() {
    use crate::office_map::{COORDINATE_LIMIT, MAX_AREAS, MAX_DOORS, MAX_SPANS, MAX_TILES};
    assert_eq!(
        fixture()["limits"],
        json!({
            "documentBytes": MAP_DOCUMENT_LIMIT,
            "tiles": MAX_TILES, "spans": MAX_SPANS, "areas": MAX_AREAS,
            "doors": MAX_DOORS, "coordinate": COORDINATE_LIMIT
        })
    );
}

#[test]
fn whole_number_notation_matches_browser_values_without_relaxing_shape() {
    let base = serde_json::to_string(&fixture()["lobby"]).unwrap();
    let expected = map_value(&decode_map(base.as_bytes()).unwrap());
    for notation in ["1.0", "1e0", "1.00e+0"] {
        let changed = base.replace("\"version\":1", &format!("\"version\":{notation}"));
        assert_eq!(
            map_value(&decode_map(changed.as_bytes()).unwrap()),
            expected
        );
    }
    let changed = base
        .replace("\"y\":0", "\"y\":-1e0")
        .replace("\"y\":1", "\"y\":0.0");
    assert_eq!(
        decode_map(changed.as_bytes()).unwrap().draft().floor[0].y,
        -1
    );
    for notation in ["1.5", "1e99", "-1", "\"1\"", "null"] {
        let changed = base.replace("\"version\":1", &format!("\"version\":{notation}"));
        assert!(decode_map(changed.as_bytes()).is_err(), "{notation}");
    }
}

#[test]
fn literal_vectors_verify_admission_geometry_and_canonical_roundtrip() {
    let corpus = fixture();
    let base = &corpus["lobby"];
    for case in corpus["cases"].as_array().unwrap() {
        let mut input = base.clone();
        for (key, value) in case["replace"].as_object().unwrap() {
            input[key] = value.clone();
        }
        let result = decode_map(&serde_json::to_vec(&input).unwrap());
        if let Some(tiles) = case["tiles"].as_u64() {
            let map = result.unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
            assert_eq!(
                map.geometry().tile_count() as u64,
                tiles,
                "{}",
                case["name"]
            );
            assert_eq!(
                map.geometry().boundaries().count() as u64,
                case["walls"].as_u64().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                map.geometry().boundaries().filter(|edge| edge.open).count() as u64,
                case["openings"].as_u64().unwrap_or(0),
                "{}",
                case["name"]
            );
            let encoded = map_value(&map);
            assert_eq!(
                map_value(&decode_map(&serde_json::to_vec(&encoded).unwrap()).unwrap()),
                encoded
            );
        } else {
            let code = match result.unwrap_err() {
                MapCodecError::TooLarge => "tooLarge".into(),
                MapCodecError::InvalidJson => "invalidJson".into(),
                MapCodecError::UnsupportedVersion => "unsupportedVersion".into(),
                MapCodecError::Geometry(error) => format!("{error:?}"),
            };
            assert_eq!(code, case["error"].as_str().unwrap(), "{}", case["name"]);
        }
    }
}

#[test]
fn nested_unknown_fields_and_duplicate_members_are_not_silently_ignored() {
    let original = fixture()["lobby"].clone();
    decode_map(&serde_json::to_vec(&original).unwrap()).unwrap();
    for path in ["", "/areas/0", "/areas/0/binding", "/floor/0"] {
        let mut input = original.clone();
        input
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), json!(true));
        assert_eq!(
            decode_map(&serde_json::to_vec(&input).unwrap()).unwrap_err(),
            MapCodecError::InvalidJson
        );
    }
    let text = serde_json::to_string(&original).unwrap();
    let duplicate = text.replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
    assert_ne!(duplicate, text);
    assert_eq!(
        decode_map(duplicate.as_bytes()).unwrap_err(),
        MapCodecError::InvalidJson
    );
}

#[test]
fn byte_limit_and_invalid_numeric_values_fail_before_geometry() {
    assert_eq!(
        decode_map(&vec![b' '; MAP_DOCUMENT_LIMIT + 1]).unwrap_err(),
        MapCodecError::TooLarge
    );
    for value in [json!(1.5), json!(2147483648_u64), json!("0"), Value::Null] {
        let mut input = fixture()["lobby"].clone();
        input["floor"][0]["start"] = value;
        assert_eq!(
            decode_map(&serde_json::to_vec(&input).unwrap()).unwrap_err(),
            MapCodecError::InvalidJson
        );
    }
}

#[test]
fn common_floor_must_be_explicit_not_an_omitted_field() {
    let mut input = fixture()["lobby"].clone();
    input["floor"][0].as_object_mut().unwrap().remove("areaId");
    assert_eq!(
        decode_map(&serde_json::to_vec(&input).unwrap()).unwrap_err(),
        MapCodecError::InvalidJson
    );
}
