//! Office prop file and command adapters.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
#[cfg(test)]
use std::collections::HashSet;
use std::path::Path;
use tmt_office_model::codec::office_prop::*;
use tmt_office_model::office_protocol::{OfficeError, OfficeInvocation};

pub fn read_pack_file(path: &Path) -> Result<ValidatedPropPack, PropPackError> {
    let bytes =
        crate::bounded_file::read_no_follow(path, PACK_INPUT_LIMIT).map_err(
            |error| match error {
                crate::bounded_file::FileReadError::TooLarge => PropPackError::TooLarge,
                crate::bounded_file::FileReadError::Io(error) => PropPackError::Io(error),
            },
        )?;
    validate_pack(&bytes)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PropCommandInput {
    #[serde(default)]
    bytes: Option<String>,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    expected_revision: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

pub fn execute(operation: OfficeInvocation, input: &[u8]) -> Vec<u8> {
    serde_json::to_vec(
        &execute_inner(operation, input).unwrap_or_else(|error| json!({"error":error.code()})),
    )
    .unwrap_or_else(|_| br#"{"error":"OFFICE_PROP_CORRUPT"}"#.to_vec())
}

fn execute_inner(operation: OfficeInvocation, input: &[u8]) -> Result<Value, OfficeError> {
    if input.len() > PROTOCOL_INPUT_LIMIT {
        return Err(OfficeError::PropInvalid);
    }
    let input: PropCommandInput =
        serde_json::from_slice(input).map_err(|_| OfficeError::PropInvalid)?;
    if operation == OfficeInvocation::LocalPropValidate {
        let candidate = command_pack(&input)?;
        return Ok(pack_projection(&candidate));
    }
    let paths =
        crate::config::ConfigPaths::discover().map_err(|_| OfficeError::CredentialsUnavailable)?;
    let mut storage = crate::storage::Storage::open(paths.database).map_err(storage_prop_error)?;
    let result = match operation {
        OfficeInvocation::LocalPropInstall => {
            let candidate = command_pack(&input)?;
            let revision = input.expected_revision.ok_or(OfficeError::PropInvalid)?;
            storage
                .install_local_prop_pack(revision, &candidate)
                .map(|mutation| mutation_projection(mutation, true))
                .map_err(catalog_error)
        }
        OfficeInvocation::LocalPropRemove => {
            let digest = input.digest.as_deref().ok_or(OfficeError::PropInvalid)?;
            let revision = input.expected_revision.ok_or(OfficeError::PropInvalid)?;
            storage
                .remove_local_prop_pack(revision, digest)
                .map(|mutation| mutation_projection(mutation, false))
                .map_err(catalog_error)
        }
        OfficeInvocation::LocalPropList => storage
            .list_local_prop_packs(input.limit.unwrap_or(20), input.cursor.as_deref())
            .map(list_projection)
            .map_err(catalog_error),
        OfficeInvocation::LocalPropShow => storage
            .show_local_prop_pack(input.digest.as_deref().ok_or(OfficeError::PropInvalid)?)
            .map(snapshot_projection)
            .map_err(catalog_error),
        _ => Err(OfficeError::CredentialsInvalid),
    };
    let close = storage.close().map_err(storage_prop_error);
    match (result, close) {
        (Err(error), _) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn command_pack(input: &PropCommandInput) -> Result<ValidatedPropPack, OfficeError> {
    let encoded = input.bytes.as_deref().ok_or(OfficeError::PropInvalid)?;
    if encoded.len() > ENCODED_INPUT_LIMIT {
        return Err(OfficeError::PropInvalid);
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| OfficeError::PropInvalid)?;
    validate_pack(&bytes).map_err(|_| OfficeError::PropInvalid)
}

fn snapshot_projection(snapshot: crate::storage::LocalPropSnapshot) -> Value {
    let mut value = pack_projection(&snapshot.pack);
    value["builtin"] = json!(snapshot.builtin);
    value["catalogRevision"] = json!(snapshot.catalog_revision);
    value["installedAtMs"] = snapshot
        .installed_at_ms
        .map_or(Value::Null, |value| json!(value));
    value
}

fn mutation_projection(mutation: crate::storage::LocalPropMutation, include_pack: bool) -> Value {
    if include_pack {
        let mut value = snapshot_projection(mutation.snapshot.expect("install returns a snapshot"));
        value["changed"] = json!(mutation.changed);
        value
    } else {
        json!({
            "digest":mutation.digest,
            "catalogRevision":mutation.catalog_revision,
            "changed":mutation.changed
        })
    }
}

fn list_projection(list: crate::storage::LocalPropCatalogList) -> Value {
    json!({
        "catalogRevision":list.catalog_revision,
        "builtins":list.builtins.into_iter().map(snapshot_projection).collect::<Vec<_>>(),
        "packs":list.packs.into_iter().map(snapshot_projection).collect::<Vec<_>>(),
        "excluded":list.excluded.into_iter().map(|excluded| json!({
            "digest":excluded.digest,
            "reason":excluded.reason.code()
        })).collect::<Vec<_>>(),
        "nextCursor":list.next_cursor
    })
}

/// Stable catalog errors shared by CLI/companion and local browser adapters.
pub fn catalog_error(error: crate::storage::LocalPropCatalogError) -> OfficeError {
    use crate::storage::LocalPropCatalogError as Error;
    match error {
        Error::Invalid => OfficeError::PropInvalid,
        Error::Corrupt => OfficeError::PropCorrupt,
        Error::NotFound => OfficeError::PropNotFound,
        Error::Limit => OfficeError::PropLimit,
        Error::Builtin => OfficeError::PropBuiltin,
        Error::RevisionConflict | Error::RevisionExhausted => OfficeError::CatalogRevisionConflict,
        Error::CursorInvalid => OfficeError::CatalogCursorInvalid,
        Error::CursorStale => OfficeError::CatalogCursorStale,
        Error::Storage(error) => storage_prop_error(error),
    }
}

fn storage_prop_error(error: impl std::error::Error) -> OfficeError {
    let _ = error;
    OfficeError::CredentialsUnavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIRECTIONAL_SAMPLE: &[u8] =
        include_bytes!("../../../../contracts/office/prop-pack-v2-sample.tmtprop.json");

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
    fn version_specific_bytes_and_base64_envelope_are_bounded() {
        let mut bytes = DIRECTIONAL_SAMPLE.to_vec();
        bytes.resize(PACK_INPUT_LIMIT, b' ');
        let pack = validate_pack(&bytes).unwrap();
        let input = command_pack_input(&pack);
        assert_eq!(input["bytes"].as_str().unwrap().len(), ENCODED_INPUT_LIMIT);
        let envelope = serde_json::to_vec(&input).unwrap();
        assert!(envelope.len() < PROTOCOL_INPUT_LIMIT);
        assert!(execute_inner(OfficeInvocation::LocalPropValidate, &envelope).is_ok());
        bytes.push(b' ');
        assert_eq!(validate_pack(&bytes), Err(PropPackError::TooLarge));
        let mut legacy = BUILTIN_BYTES.to_vec();
        legacy.resize(V1_PACK_INPUT_LIMIT + 1, b' ');
        assert_eq!(validate_pack(&legacy), Err(PropPackError::TooLarge));
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
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/office/prop-block-vectors.json"
        ))
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
