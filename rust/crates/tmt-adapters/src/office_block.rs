//! Bounded filesystem input for Office block layouts.
#[cfg(test)]
use serde_json::{Value, json};
use std::path::Path;
use tmt_office_model::codec::office_block::*;
#[cfg(test)]
use tmt_office_model::office_block::OBJECT_LIMIT;
use tmt_office_model::office_block::{BlockLayout, INPUT_LIMIT};
use tmt_office_model::office_protocol::OfficeError;

pub fn read_layout_file(path: &Path) -> Result<BlockLayout, OfficeError> {
    let bytes =
        crate::bounded_file::read(path, INPUT_LIMIT).map_err(|_| OfficeError::LayoutInvalid)?;
    decode_layout(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_customization_values_match_shared_vectors_and_require_v3() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../extensions/tmt-office/contracts/prop-customization-vectors.json"
        ))
        .unwrap();
        for case in vectors["placementCases"].as_array().unwrap() {
            let mut input = json!({"version":3,"objects":[{
                "prop":format!("sha256:{}/rug", "1".repeat(64)), "footprint":{"width":2,"height":1},
                "x":0,"y":0,"rotation":0,"customization":case["value"]
            }]});
            let decoded = decode_local_layout(&serde_json::to_vec(&input).unwrap());
            assert_eq!(
                decoded.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if let Ok(layout) = decoded {
                assert_eq!(local_layout_value(&layout), input);
            }
            input["version"] = json!(2);
            assert!(decode_local_layout(&serde_json::to_vec(&input).unwrap()).is_err());
        }
    }

    #[test]
    fn readable_wire_round_trips_without_exposing_storage_tokens() {
        let layout =
            decode_layout(br#"{"objects":[{"asset":"desk","x":30,"y":28,"rotation":1}]}"#).unwrap();
        assert_eq!(layout.encode(), ["d1us"]);
        let snapshot = BlockSnapshot {
            block_id: "11111111-1111-4111-8111-111111111111".into(),
            revision: 1,
            layout,
        };
        let bytes = serde_json::to_vec(&snapshot.wire_value()).unwrap();
        assert_eq!(BlockSnapshot::decode(&bytes).unwrap(), snapshot);
        assert_eq!(snapshot.public_value()["objects"][0]["asset"], "desk");
        assert_eq!(
            snapshot.public_value()["catalog"].as_array().unwrap().len(),
            4
        );
    }

    #[test]
    fn actual_readable_inputs_conform_to_shared_literal_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../extensions/tmt-office/contracts/block-v1.vectors.json"
        ))
        .unwrap();
        for vector in vectors {
            let result =
                decode_layout(&serde_json::to_vec(&json!({"objects":[vector["item"]]})).unwrap());
            assert_eq!(
                result.is_ok(),
                vector["valid"].as_bool().unwrap(),
                "{}",
                vector["name"]
            );
        }
    }

    #[test]
    fn local_v2_inputs_conform_to_shared_prop_vectors() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../extensions/tmt-office/contracts/prop-block-vectors.json"
        ))
        .unwrap();
        for case in vectors["layoutCases"].as_array().unwrap() {
            let decoded = decode_local_layout(&serde_json::to_vec(&case["value"]).unwrap());
            assert_eq!(
                decoded.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
        let capacity = &vectors["capacity"];
        let mut layout = capacity["layout"].clone();
        layout["objects"] = serde_json::json!(vec![
            capacity["placement"].clone();
            capacity["count"].as_u64().unwrap() as usize
        ]);
        assert_eq!(
            decode_local_layout(&serde_json::to_vec(&layout).unwrap())
                .unwrap()
                .objects()
                .len(),
            OBJECT_LIMIT
        );
        layout["objects"]
            .as_array_mut()
            .unwrap()
            .push(capacity["placement"].clone());
        assert_eq!(
            decode_local_layout(&serde_json::to_vec(&layout).unwrap()),
            Err(OfficeError::LayoutInvalid)
        );
    }

    #[test]
    fn malformed_envelopes_and_oversized_files_are_rejected() {
        for bytes in [
            b"{}".as_slice(),
            br#"{"objects":[],"extra":true}"#,
            br#"{"objects":["d000"]}"#,
            br#"{"objects":[],"objects":[]}"#,
        ] {
            assert_eq!(decode_layout(bytes), Err(OfficeError::LayoutInvalid));
        }
        assert_eq!(
            decode_layout(&vec![b' '; INPUT_LIMIT + 1]),
            Err(OfficeError::LayoutInvalid)
        );
        let directory = crate::test_support::TestDirectory::new();
        let file = directory.path.join("layout.json");
        std::fs::write(&file, br#"{"objects":[]}"#).unwrap();
        assert!(read_layout_file(&file).unwrap().objects().is_empty());
        std::fs::write(&file, vec![b' '; INPUT_LIMIT + 1]).unwrap();
        assert_eq!(read_layout_file(&file), Err(OfficeError::LayoutInvalid));
        assert_eq!(
            read_layout_file(&directory.path),
            Err(OfficeError::LayoutInvalid)
        );
    }
}
