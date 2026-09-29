//! Office prop pack file acquisition used by the CLI, with shared pack tests.

#[cfg(test)]
use serde_json::{Value, json};
#[cfg(test)]
use std::collections::HashSet;
use std::path::Path;
use tmt_office_model::codec::office_prop::*;

pub fn read_pack_file(path: &Path) -> Result<ValidatedPropPack, PropPackError> {
    let bytes =
        tmt_adapters::bounded_file::read_no_follow(path, PACK_INPUT_LIMIT).map_err(|error| {
            match error {
                tmt_adapters::bounded_file::FileReadError::TooLarge => PropPackError::TooLarge,
                tmt_adapters::bounded_file::FileReadError::Io(error) => PropPackError::Io(error),
            }
        })?;
    validate_pack(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIRECTIONAL_SAMPLE: &[u8] =
        include_bytes!("../../../contracts/prop-pack-v2-sample.tmtprop.json");

    #[test]
    fn directional_frames_have_independent_identity_and_summary_dimensions() {
        let pack = validate_pack(DIRECTIONAL_SAMPLE).unwrap();
        assert_eq!(
            pack.digest(),
            "sha256:429603feb9648005f3d16191a281433455b550fbdc67494b8f3bb7f4cd118af3"
        );
        assert_eq!(pack.pixel_count(), 32);
        let summary = pack_projection(&pack);
        assert_eq!(
            summary["props"][0]["frames"],
            json!([
                {"width":4,"height":2}, {"width":2,"height":4},
                {"width":4,"height":2}, {"width":2,"height":4}
            ])
        );
        assert!(summary["props"][0].get("raster").is_none());
        assert!(
            !String::from_utf8(serde_json::to_vec(&summary).unwrap())
                .unwrap()
                .contains("00100101")
        );
    }

    #[test]
    fn directional_furniture_preserves_legacy_identity_and_footprints() {
        for (old_digest, new_digest) in [
            (MODULAR_WORKSTATION_DIGEST, DIRECTIONAL_WORKSTATION_DIGEST),
            (MODULAR_LOUNGE_DIGEST, DIRECTIONAL_LOUNGE_DIGEST),
            (MODULAR_RECEPTION_DIGEST, DIRECTIONAL_RECEPTION_DIGEST),
            (MODULAR_FACILITIES_DIGEST, DIRECTIONAL_FACILITIES_DIGEST),
        ] {
            let old = builtin_by_digest(old_digest).unwrap();
            let new = builtin_by_digest(new_digest).unwrap();
            for prop in &new.pack().props {
                let retained = old.pack().props.iter().find(|p| p.key == prop.key).unwrap();
                assert_eq!(prop.footprint, retained.footprint);
                let frames = prop.frames.as_ref().unwrap();
                let unique: HashSet<_> = frames.iter().collect();
                assert_eq!(unique.len(), 4, "{}", prop.key);
                assert!(
                    retained
                        .frames
                        .as_ref()
                        .unwrap()
                        .windows(2)
                        .all(|pair| pair[0] == pair[1])
                );
            }
        }
    }

    #[test]
    fn directional_admission_rejects_invalid_and_mixed_frame_encodings() {
        for kind in [
            "missing",
            "extra",
            "mixed",
            "null",
            "empty",
            "odd",
            "index",
            "uppercase",
        ] {
            let mut value: Value = serde_json::from_slice(DIRECTIONAL_SAMPLE).unwrap();
            let prop = &mut value["props"][0];
            match kind {
                "missing" => {
                    prop["frames"].as_array_mut().unwrap().pop();
                }
                "extra" => {
                    prop["frames"].as_array_mut().unwrap().push(json!(["01"]));
                }
                "mixed" => prop["pixels"] = json!(["01"]),
                "null" => prop["pixels"] = Value::Null,
                "empty" => prop["frames"][0] = json!(["0000"]),
                "odd" => prop["frames"][0] = json!(["001"]),
                "index" => prop["frames"][0] = json!(["ff"]),
                "uppercase" => prop["frames"][0] = json!(["0A"]),
                _ => unreachable!(),
            }
            assert!(
                validate_pack(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{kind}"
            );
        }
    }

    #[test]
    fn directional_total_cell_budget_accepts_the_boundary_and_rejects_one_extra_row() {
        let mut value: Value = serde_json::from_slice(DIRECTIONAL_SAMPLE).unwrap();
        let sample = value["props"][0].clone();
        value["props"] = Value::Array(
            (0..16)
                .map(|index| {
                    let mut prop = sample.clone();
                    prop["key"] = json!(format!("capacity-{index}"));
                    prop["frames"] = json!(vec![vec!["01".repeat(128); 16]; 4]);
                    prop
                })
                .collect(),
        );
        assert!(validate_pack(&serde_json::to_vec(&value).unwrap()).is_ok());
        // All individual rasters remain within their side/cell limits; only
        // the 131,072-cell aggregate budget is crossed by this extra row.
        value["props"][0]["frames"][0]
            .as_array_mut()
            .unwrap()
            .push(json!("01".repeat(128)));
        assert_eq!(
            validate_pack(&serde_json::to_vec(&value).unwrap()),
            Err(PropPackError::Invalid)
        );
    }

    #[test]
    fn embedded_pack_has_independently_frozen_identity_and_legacy_geometry() {
        let pack = builtin_pack();
        assert_eq!(pack.digest(), BUILTIN_DIGEST);
        assert_eq!(pack.bytes().len(), 1_563);
        assert_eq!(
            pack.pack()
                .props
                .iter()
                .map(|prop| (
                    prop.key.as_str(),
                    prop.footprint.width,
                    prop.footprint.height
                ))
                .collect::<Vec<_>>(),
            [
                ("desk", 4, 2),
                ("chair", 2, 2),
                ("plant", 2, 2),
                ("rug", 6, 4)
            ]
        );
    }

    #[test]
    fn shared_vectors_cover_values_and_full_capacity() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../../../contracts/prop-block-vectors.json"))
                .unwrap();
        for case in vectors["packCases"].as_array().unwrap() {
            let bytes = serde_json::to_vec(&case["value"]).unwrap();
            assert_eq!(
                validate_pack(&bytes).is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
        let capacity = &vectors["capacity"];
        let props = (0..capacity["count"].as_u64().unwrap())
            .map(|index| {
                let mut prop = capacity["prop"].clone();
                prop["key"] = serde_json::json!(format!(
                    "{}{:02}",
                    capacity["keyPrefix"].as_str().unwrap(),
                    index
                ));
                prop["label"] = capacity["label"].clone();
                prop["pixels"] = serde_json::json!(vec![
                    capacity["rasterRow"].as_str().unwrap();
                    capacity["rasterRows"].as_u64().unwrap()
                        as usize
                ]);
                prop
            })
            .collect::<Vec<_>>();
        let mut pack = capacity["pack"].clone();
        pack["props"] = serde_json::json!(props);
        let validated = validate_pack(&serde_json::to_vec(&pack).unwrap()).unwrap();
        assert_eq!(validated.pack().props.len(), PACK_PROP_LIMIT);
        assert_eq!(validated.pixel_count(), PACK_PIXEL_LIMIT);
    }

    #[test]
    fn exact_bytes_and_framing_change_identity() {
        let original = validate_pack(BUILTIN_BYTES).unwrap();
        let mut changed = BUILTIN_BYTES.to_vec();
        changed.push(b' ');
        let changed = validate_pack(&changed).unwrap();
        assert_ne!(original.digest(), changed.digest());
        assert_ne!(
            original.digest().strip_prefix("sha256:").unwrap(),
            tmt_core::content_digest::sha256(BUILTIN_BYTES)
        );
    }

    #[test]
    fn typed_decode_rejects_duplicate_and_unknown_fields() {
        for bytes in [
            br##"{"formatVersion":1,"formatVersion":1,"label":"x","credit":"y","license":"MIT","palette":["#00000000","#ffffffff"],"props":[{"key":"lamp","label":"Lamp","footprint":{"width":1,"height":1},"pixels":["1"]}]}"##.as_slice(),
            br##"{"formatVersion":1,"label":"x","credit":"y","license":"MIT","palette":["#00000000","#ffffffff"],"props":[{"key":"lamp","label":"Lamp","footprint":{"width":1,"height":1},"pixels":["1"]}],"extra":true}"##,
            br##"{"formatVersion":1,"label":"x","credit":"y","license":"MIT","palette":["#00000000","#ffffffff"],"props":[{"key":"lamp","key":"lamp","label":"Lamp","footprint":{"width":1,"height":1},"pixels":["1"]}]}"##,
            br##"{"formatVersion":1,"label":"x","credit":"y","license":"MIT","palette":["#00000000","#ffffffff"],"props":[{"key":"lamp","label":"Lamp","footprint":{"width":1,"width":1,"height":1},"pixels":["1"]}]}"##,
        ] {
            assert_eq!(validate_pack(bytes), Err(PropPackError::Invalid));
        }
    }

    #[test]
    fn file_acquisition_is_bounded_regular_and_no_follow() {
        let directory = crate::test_support::TestDirectory::new();
        let regular = directory.path.join("pack.json");
        std::fs::write(&regular, BUILTIN_BYTES).unwrap();
        assert_eq!(read_pack_file(&regular).unwrap().digest(), BUILTIN_DIGEST);
        assert!(matches!(
            read_pack_file(&directory.path.join("missing.json")),
            Err(PropPackError::Io(_))
        ));
        let oversized = directory.path.join("oversized.json");
        std::fs::write(&oversized, vec![b' '; PACK_INPUT_LIMIT + 1]).unwrap();
        assert!(matches!(
            read_pack_file(&oversized),
            Err(PropPackError::TooLarge)
        ));
        let link = directory.path.join("link.json");
        std::os::unix::fs::symlink(&regular, &link).unwrap();
        assert!(matches!(read_pack_file(&link), Err(PropPackError::Io(_))));
    }

    #[test]
    fn invalid_palette_pixels_keys_and_bounds_fail_closed() {
        let valid = br##"{"formatVersion":1,"label":"x","credit":"y","license":"MIT","palette":["#00000000","#ffffffff"],"props":[{"key":"lamp","label":"Lamp","footprint":{"width":1,"height":1},"pixels":["1"]}]}"##;
        assert!(validate_pack(valid).is_ok());
        for (from, to) in [
            ("#ffffffff", "#fffffffe"),
            ("\"1\"]", "\"2\"]"),
            ("\"lamp\"", "\"Lamp\""),
            ("\"width\":1", "\"width\":0"),
        ] {
            let changed = String::from_utf8(valid.to_vec()).unwrap().replace(from, to);
            assert_eq!(
                validate_pack(changed.as_bytes()),
                Err(PropPackError::Invalid)
            );
        }
        assert_eq!(
            validate_pack(&vec![b' '; PACK_INPUT_LIMIT + 1]),
            Err(PropPackError::TooLarge)
        );
    }

    #[test]
    fn references_are_exact() {
        let reference = format!("{BUILTIN_DIGEST}/desk");
        assert_eq!(
            parse_prop_reference(&reference),
            Some((BUILTIN_DIGEST, "desk"))
        );
        assert!(parse_prop_reference(&reference.to_uppercase()).is_none());
        assert!(parse_prop_reference(&format!("{BUILTIN_DIGEST}/Desk")).is_none());
    }
}
