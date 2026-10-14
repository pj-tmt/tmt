use super::*;
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const STREAM: &str = "20000000-0000-4000-8000-000000000001";
fn string(value: &str) -> Field {
    Field::String(value.into())
}
fn log() -> Fields {
    Fields::from_iter([
        ("space".into(), string(&"a".repeat(32))),
        ("page".into(), string(PAGE)),
        ("epoch".into(), Field::number(1).unwrap()),
        ("stream".into(), string(STREAM)),
        ("seq".into(), Field::number(1).unwrap()),
        ("uid".into(), string("owner")),
        ("hash".into(), string(&values::encode_binary(&[7; 32]))),
        ("envelope".into(), string("{\"signed\":\"original\"}")),
        (
            "expiresAt".into(),
            Field::Timestamp("2026-10-15T00:00:00Z".into()),
        ),
    ])
}
#[test]
fn log_roundtrip_keeps_exact_original_bytes_and_page_bound_id() {
    let fields = log();
    let doc = encode(Resource::Log, fields.clone()).unwrap();
    assert_eq!(
        doc.target.id,
        format!("{}_{}_1_{}_1", "a".repeat(32), PAGE, STREAM)
    );
    assert_eq!(
        doc.fields["envelope"],
        json!({"stringValue":"{\"signed\":\"original\"}"})
    );
    assert_eq!(decode(Resource::Log, &doc).unwrap(), fields);
    let mut forged = doc.clone();
    forged
        .fields
        .insert("envelope".into(), json!({"stringValue":"changed"}));
    assert!(decode(Resource::Log, &forged).is_err());
    let mut wrong_page = doc.clone();
    wrong_page.target.id.push('a');
    assert!(decode(Resource::Log, &wrong_page).is_err());
    assert!(decode(Resource::Authority, &doc).is_err());
}
#[test]
fn mappings_match_the_single_declaration_owner() {
    let declaration: Value =
        serde_json::from_str(include_str!("../../../../../firestore/declaration.json")).unwrap();
    for resource in [
        Resource::Spaces,
        Resource::Pages,
        Resource::Links,
        Resource::Members,
        Resource::Log,
        Resource::Checkpoints,
        Resource::Authority,
    ] {
        let spec = declaration["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == resource.name())
            .unwrap();
        let declared: std::collections::BTreeSet<_> = spec["sync"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(declared, resource.fields().iter().copied().collect());
    }
}
#[test]
fn log_refuses_missing_extra_wrong_type_and_noncanonical_numbers() {
    for (key, value) in [
        ("extra", string("not declared")),
        ("seq", Field::Integer("01".into())),
        ("seq", Field::Integer("9007199254740992".into())),
        ("seq", Field::Integer("0".into())),
        ("envelope", string("multibyte é")),
        ("hash", string("short")),
        ("expiresAt", string("2026-10-15T00:00:00Z")),
    ] {
        let mut fields = log();
        fields.insert(key.into(), value);
        assert!(encode(Resource::Log, fields).is_err(), "{key}");
    }
    let mut fields = log();
    fields.remove("uid");
    assert!(encode(Resource::Log, fields).is_err());
}
#[test]
fn full_encoded_envelope_boundary_is_exact() {
    let mut fields = log();
    fields.insert("envelope".into(), string(&"e".repeat(354_304)));
    let doc = encode(Resource::Log, fields.clone()).unwrap();
    assert_eq!(decode(Resource::Log, &doc).unwrap(), fields);
    fields.insert("envelope".into(), string(&"e".repeat(354_305)));
    assert!(encode(Resource::Log, fields).is_err());
}
#[test]
fn discovery_and_interest_do_not_carry_a_stream_or_cursor() {
    let scope = page_scope(&"a".repeat(32), PAGE, 1).unwrap();
    assert_eq!(scope.len(), 3);
    assert_eq!(scope["epoch"], json!({"integerValue":"1"}));
    let authority = authority_scope(&"a".repeat(32), PAGE, 1, "membership", "").unwrap();
    assert_eq!(authority.len(), 5);
    assert_eq!(authority["recipient"], json!({"stringValue":""}));
    assert!(authority_scope(&"a".repeat(32), PAGE, 1, "wrap", "").is_err());
    assert!(authority_scope(&"a".repeat(32), PAGE, 1, "chain", "someone").is_err());
    assert_eq!(
        authority_scope(&"a".repeat(32), PAGE, 1, "wrap", "member").unwrap()["recipient"],
        json!({"stringValue":"member"})
    );
}

fn checkpoint() -> Fields {
    let mut fields = log();
    for key in ["stream", "hash", "envelope"] {
        fields.remove(key);
    }
    fields.insert("object".into(), string(&"c".repeat(64)));
    fields.insert("index".into(), Field::number(0).unwrap());
    fields.insert("count".into(), Field::number(2).unwrap());
    fields.insert(
        "bytes".into(),
        string(&values::encode_binary(&[11; 32_768])),
    );
    fields
}
#[test]
fn checkpoint_chunks_use_index_ids_but_positive_stream_sequences() {
    let fields = checkpoint();
    let doc = encode(Resource::Checkpoints, fields.clone()).unwrap();
    assert!(doc.target.id.ends_with("_0"));
    assert_eq!(doc.fields["seq"], json!({"integerValue":"1"}));
    assert_eq!(decode(Resource::Checkpoints, &doc).unwrap(), fields);
    for (key, n) in [("seq", 2), ("count", 0)] {
        let mut wrong = fields.clone();
        wrong.insert(key.into(), Field::number(n).unwrap());
        assert!(encode(Resource::Checkpoints, wrong).is_err());
    }
    let mut oversized = fields;
    oversized.insert(
        "bytes".into(),
        string(&values::encode_binary(&[11; 32_769])),
    );
    assert!(encode(Resource::Checkpoints, oversized).is_err());
}
#[test]
fn authority_ids_and_wrap_discovery_preserve_recipient_boundaries() {
    let mut fields = checkpoint();
    fields.remove("object");
    fields.remove("index");
    fields.insert("subject".into(), string(&"c".repeat(64)));
    fields.insert("kind".into(), string("membership"));
    fields.insert("recipient".into(), string(""));
    fields.insert("bytes".into(), string("eA"));
    let doc = encode(Resource::Authority, fields.clone()).unwrap();
    assert!(
        doc.target
            .id
            .ends_with(&format!("_membership_{}_1", "c".repeat(64)))
    );
    assert_eq!(decode(Resource::Authority, &doc).unwrap(), fields);
    fields.insert("kind".into(), string("wrap"));
    assert!(encode(Resource::Authority, fields.clone()).is_err());
    fields.insert("recipient".into(), string("member"));
    assert!(
        encode(Resource::Authority, fields.clone()).is_err(),
        "wrap has exactly one chunk"
    );
    fields.insert("count".into(), Field::number(1).unwrap());
    assert!(encode(Resource::Authority, fields).is_ok());
}
#[test]
fn admission_documents_keep_the_six_field_member_shape() {
    let member = Fields::from_iter([
        ("space".into(), string(&"a".repeat(32))),
        ("page".into(), string(PAGE)),
        ("uid".into(), string("owner")),
        ("role".into(), string("editor")),
        ("link".into(), Field::Null(())),
        ("token".into(), Field::Null(())),
    ]);
    let doc = encode(Resource::Members, member.clone()).unwrap();
    assert_eq!(doc.fields["link"], json!({"nullValue":null}));
    assert_eq!(decode(Resource::Members, &doc).unwrap(), member);
    let mut legacy = member.clone();
    legacy.remove("link");
    legacy.remove("token");
    assert!(encode(Resource::Members, legacy).is_err());
    let mut mixed = member.clone();
    mixed.insert("token".into(), string("token"));
    assert!(encode(Resource::Members, mixed).is_err());
    let mut invalid = doc;
    invalid.fields.insert(
        "role".into(),
        json!({"stringValue":"editor","booleanValue":true}),
    );
    assert!(decode(Resource::Members, &invalid).is_err());
}
