use serde_json::json;
use tmt_colab_model::{auth, framing, payload, values};
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../contracts/vectors/model-v1.json")).unwrap()
}
fn hex(v: &serde_json::Value, key: &str) -> Vec<u8> {
    let s = v[key].as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
#[test]
fn management_decoder_binds_exact_payload_and_rejects_single_field_mutations() {
    let v = fixture();
    let input = hex(&v, "management");
    let payload = hex(&v, "payload");
    let decoded = auth::decode_management(&input, &payload).unwrap();
    assert_eq!(auth::management_input(&decoded).unwrap(), input);
    assert!(auth::decode_management(&input, b"{}").is_err());
    let mut trailing = input.clone();
    trailing.push(0);
    assert!(auth::decode_management(&trailing, &payload).is_err());
    for (index, value) in [
        (1, b"2".as_slice()),
        (4, b"01"),
        (6, b"unknown"),
        (9, b"01790860000000"),
        (10, b"1790860000000"),
    ] {
        let fields = framing::fields(&input, 11, 1024).unwrap();
        let mut changed = fields;
        changed[index] = value;
        assert!(auth::decode_management(&framing::frame(&changed).unwrap(), &payload).is_err());
    }
}
#[test]
fn shared_baseline_decoder_has_the_same_strict_schema_as_epoch_payloads() {
    let key = values::encode_binary(&[3; 32]);
    let baseline = json!({"pageId":"00000000-0000-4000-8000-000000000002","epoch":"1","sourceDigest":key,"baselineCommitment":key,"title":"Page","objectEnvelopeHash":key,"membershipRevision":"2"});
    let bytes = serde_json::to_vec(&baseline).unwrap();
    let decoded = payload::decode_baseline(&bytes).unwrap();
    payload::validate_baseline(&decoded).unwrap();
    for (field, value) in [
        ("epoch", json!("01")),
        ("membershipRevision", json!("0")),
        ("sourceDigest", json!(values::encode_binary(&[0; 31]))),
        ("pageId", json!("invalid")),
        ("extra", json!(true)),
    ] {
        let mut changed = baseline.clone();
        changed[field] = value;
        assert!(payload::decode_baseline(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    let duplicate = format!(
        "{{\"title\":\"Page\",{}",
        std::str::from_utf8(&bytes[1..]).unwrap()
    );
    assert!(payload::decode_baseline(duplicate.as_bytes()).is_err());
}
