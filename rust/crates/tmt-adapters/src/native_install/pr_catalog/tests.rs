use super::*;
use serde_json::Value;

const FIXTURE: &[u8] = include_bytes!("../fixtures/pr-rc-catalog-v2.json");
const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const WORKFLOW_ID: u64 = 701;
const NOW: u64 = 1_791_291_600_010;

fn parse(bytes: &[u8]) -> io::Result<Catalog> {
    Catalog::parse(
        bytes,
        PrNumber::parse("234").unwrap(),
        HEAD,
        NOW,
        WORKFLOW_ID,
    )
}

#[test]
fn frozen_wire_revision_two_parses_without_registering_synthetic_authority() {
    let catalog = parse(FIXTURE).unwrap();
    assert!(parse(include_bytes!("../fixtures/pr-rc-catalog-v1.json")).is_err());
    assert_eq!(catalog.candidates.len(), 4);
    assert_eq!(catalog.producer.run_id, 9001);
    assert_ne!(catalog.head_sha, catalog.producer.tooling_sha);
    assert_eq!(
        catalog.candidates[0].application_schema.databases[0].version,
        48
    );
    assert_eq!(
        artifact::digest(FIXTURE),
        "3e1683828ba6c49dba7ecf40f6da49f52e612e8be6192bc3383e7de8cec6f71e"
    );
    assert!(Catalog::parse(FIXTURE, PrNumber::parse("234").unwrap(), HEAD, NOW, 0).is_err());
}

#[test]
fn catalog_refuses_wrong_source_trust_schema_and_incomplete_generations() {
    let original: Value = serde_json::from_slice(FIXTURE).unwrap();
    for (pointer, replacement) in [
        ("/schema_version", Value::from(1)),
        ("/kind", Value::from("other")),
        ("/repository", Value::from("fork/tmt")),
        ("/pr", Value::from(235)),
        ("/head_sha", Value::from("d".repeat(40))),
        ("/producer/workflow_id", Value::from(702)),
        (
            "/producer/workflow_path",
            Value::from(".github/workflows/ci.yml"),
        ),
        ("/producer/tooling_sha", Value::from("bad")),
        ("/producer/workflow_sha256", Value::from("bad")),
        ("/producer/run_id", Value::from(0)),
        ("/producer/run_attempt", Value::from(101)),
        ("/expires_at_ms", Value::from(NOW)),
        ("/published_at_ms", Value::from(NOW + 1)),
        ("/expires_at_ms", Value::from(NOW + TTL_MS + 1)),
        ("/candidates/0/product", Value::from("office")),
        ("/candidates/0/version", Value::from("not-semver")),
        ("/candidates/0/target", Value::from("unsupported")),
        ("/candidates/0/payload_artifact/id", Value::from(0)),
        (
            "/candidates/0/payload_artifact/zip_sha256",
            Value::from("sha256:bad"),
        ),
        (
            "/candidates/0/payload_artifact/zip_bytes",
            Value::from(PAYLOAD_ZIP_LIMIT + 1),
        ),
        (
            "/candidates/0/archive/name",
            Value::from("../payload.tar.gz"),
        ),
        (
            "/candidates/0/dist_manifest/name",
            Value::from("alternate.json"),
        ),
        (
            "/candidates/0/dist_manifest/bytes",
            Value::from(artifact::MANIFEST_LIMIT + 1),
        ),
        (
            "/candidates/0/application_schema/source_sha",
            Value::from("d".repeat(40)),
        ),
        (
            "/candidates/0/application_schema/product",
            Value::from("remote"),
        ),
        (
            "/candidates/0/application_schema/databases/0/domain",
            Value::from("unknown-db"),
        ),
        (
            "/candidates/0/application_schema/source_files/0/path",
            Value::from("rust/../secret"),
        ),
        (
            "/candidates/0/verification/source_sha",
            Value::from("d".repeat(40)),
        ),
        ("/candidates/0/verification/complete", Value::from(false)),
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let mut missing = original.clone();
    missing["candidates"].as_array_mut().unwrap().pop();
    assert!(parse(&serde_json::to_vec(&missing).unwrap()).is_err());
    for pointer in [
        "/candidates",
        "/candidates/0/application_schema/databases",
        "/candidates/0/application_schema/source_files",
    ] {
        let mut duplicate = original.clone();
        let array = duplicate
            .pointer_mut(pointer)
            .unwrap()
            .as_array_mut()
            .unwrap();
        array.push(array[0].clone());
        assert!(
            parse(&serde_json::to_vec(&duplicate).unwrap()).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn catalog_refuses_duplicate_unknown_missing_and_null_fields() {
    let text = std::str::from_utf8(FIXTURE).unwrap();
    assert!(
        parse(
            text.replacen(
                "\"schema_version\": 1",
                "\"schema_version\": 1, \"schema_version\": 1",
                1
            )
            .as_bytes()
        )
        .is_err()
    );
    for pointer in [
        "",
        "/producer",
        "/candidates/0",
        "/candidates/0/application_schema",
        "/candidates/0/payload_artifact",
    ] {
        let mut value: Value = serde_json::from_slice(FIXTURE).unwrap();
        value
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), Value::from(true));
        assert!(
            parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{pointer}"
        );
    }
    for replacement in [Value::Null, Value::from("9001"), Value::from(9001.0)] {
        let mut value: Value = serde_json::from_slice(FIXTURE).unwrap();
        value["producer"]["run_id"] = replacement;
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn digest_catalog_requires_one_consistent_generation_for_all_targets() {
    // Adapt the frozen CLI wire fixture without changing its bytes or trusting a new producer.
    let text = std::str::from_utf8(FIXTURE)
        .unwrap()
        .replace("cli", "digest");
    let mut catalog: Value = serde_json::from_str(&text).unwrap();
    assert!(parse(&serde_json::to_vec(&catalog).unwrap()).is_ok());
    let mut missing = catalog.clone();
    missing["candidates"].as_array_mut().unwrap().pop();
    assert!(parse(&serde_json::to_vec(&missing).unwrap()).is_err());
    catalog["candidates"][1]["version"] = Value::from("0.1.0-alpha.2");
    assert!(parse(&serde_json::to_vec(&catalog).unwrap()).is_err());
}
