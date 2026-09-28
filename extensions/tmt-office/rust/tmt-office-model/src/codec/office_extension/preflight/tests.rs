use super::*;
use serde_json::{Value, json};

const DEFINITION: &str =
    include_str!("../../../../../../../../contracts/office/discussion-extension-v1.json");
const INSTANCE: &str =
    include_str!("../../../../../../../../contracts/office/lobby-extension-v1.json");

#[test]
fn shared_pair_vectors_validate_identity_binding_and_rotated_edges() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../../../../../contracts/office/extension-pair-vectors.json"
    ))
    .unwrap();
    let definition = serde_json::to_vec(&vectors["definition"]).unwrap();
    for case in vectors["cases"].as_array().unwrap() {
        let result = validate_pair(&definition, &serde_json::to_vec(&case["instance"]).unwrap());
        let problem = result
            .err()
            .map(|error| serde_json::to_value(error).unwrap())
            .unwrap_or(Value::Null);
        assert_eq!(problem, case["problem"], "{}", case["name"]);
    }
}

#[test]
fn every_bundled_binding_preflights_without_storage_or_catalog_resolution() {
    for (definition, instance) in [
        (DEFINITION, INSTANCE),
        (
            include_str!("../../../../../../../../contracts/office/whiteboard-extension-v1.json"),
            include_str!("../../../../../../../../contracts/office/lobby-whiteboard-v1.json"),
        ),
        (
            include_str!("../../../../../../../../contracts/office/broadcaster-extension-v1.json"),
            include_str!("../../../../../../../../contracts/office/lobby-broadcaster-v1.json"),
        ),
    ] {
        let input = ValidationInput {
            definition: definition.into(),
            instance: instance.into(),
        };
        let result: ValidationReport =
            serde_json::from_slice(&execute(&serde_json::to_vec(&input).unwrap())).unwrap();
        assert!(matches!(
            result,
            ValidationReport::Valid {
                scope: ValidationScope::StructureOnly,
                ..
            }
        ));
    }
    let mut unknown_art: Value = serde_json::from_str(DEFINITION).unwrap();
    unknown_art["appearance"]["prop"] = json!(format!("sha256:{}/not-installed", "0".repeat(64)));
    assert!(
        validate_pair(
            &serde_json::to_vec(&unknown_art).unwrap(),
            INSTANCE.as_bytes()
        )
        .is_ok()
    );
}

#[test]
fn raw_documents_keep_duplicate_field_and_size_checks_across_transport() {
    let repeated = DEFINITION.replacen(
        "\"formatVersion\": 1",
        "\"formatVersion\": 1, \"formatVersion\": 1",
        1,
    );
    for (definition, instance, problem) in [
        (repeated, INSTANCE.into(), ExtensionError::InvalidDefinition),
        (
            DEFINITION.into(),
            INSTANCE.replacen("\"x\": 1", "\"x\": 1, \"x\": 1", 1),
            ExtensionError::InvalidInstance,
        ),
        (
            " ".repeat(INPUT_LIMIT + 1),
            INSTANCE.into(),
            ExtensionError::InvalidDefinition,
        ),
        (
            DEFINITION.into(),
            " ".repeat(INPUT_LIMIT + 1),
            ExtensionError::InvalidInstance,
        ),
    ] {
        let input = serde_json::to_vec(&ValidationInput {
            definition,
            instance,
        })
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<ValidationReport>(&execute(&input)).unwrap(),
            ValidationReport::Invalid { problem }
        );
    }
    for input in [
        b"{}".to_vec(),
        b"{\"definition\":\"a\",\"definition\":\"b\",\"instance\":\"c\"}".to_vec(),
        serde_json::to_vec(
            &json!({"definition": DEFINITION, "instance": INSTANCE, "execute": true}),
        )
        .unwrap(),
        vec![b' '; PROTOCOL_INPUT_LIMIT + 1],
    ] {
        assert_eq!(
            serde_json::from_slice::<ValidationReport>(&execute(&input)).unwrap(),
            ValidationReport::Invalid {
                problem: ExtensionError::InvalidInput
            }
        );
    }
}
