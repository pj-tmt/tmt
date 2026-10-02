use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use tmt_colab_model::{
    auth, crypto, framing,
    object::{self, Context, Envelope, Header},
    stream_cut, values,
};

fn bytes(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../contracts/vectors/model-v1.json")).unwrap()
}
fn field(v: &Value, key: &str) -> Vec<u8> {
    bytes(v[key].as_str().unwrap())
}
fn context(v: &Value) -> Context {
    Header::decode(&field(v, "header")).unwrap().context
}
fn envelope(v: &Value) -> Envelope {
    Envelope::from_json(&serde_json::to_vec(&json!({"header":values::encode_binary(&field(v,"header")),"nonce":values::encode_binary(&[0;12]),"ciphertext":values::encode_binary(&field(v,"ciphertext")),"signature":values::encode_binary(&field(v,"signature"))})).unwrap()).unwrap()
}

#[test]
fn namespace_object_matches_independent_python_ciphertext_signature_and_hash() {
    let v = fixture();
    let e = envelope(&v);
    let header = Header::decode(e.header()).unwrap();
    assert_eq!(header.encode().unwrap(), field(&v, "header"));
    assert_eq!(
        crypto::derive_key(
            &field(&v, "master"),
            &[],
            &framing::frame(&[b"tmt-colab-object-key-v1", e.header()]).unwrap()
        )
        .as_slice(),
        field(&v, "key")
    );
    assert_eq!(
        object::open(
            &e,
            &header.context,
            &field(&v, "master").try_into().unwrap(),
            &field(&v, "public").try_into().unwrap()
        )
        .unwrap(),
        field(&v, "plaintext")
    );
    assert_eq!(e.hash().unwrap().as_slice(), field(&v, "envelopeHash"));
    assert_eq!(Envelope::from_json(&e.to_json().unwrap()).unwrap(), e);
}
#[test]
fn signin_and_management_match_independent_inputs_and_proofs() {
    let v = fixture();
    let input = field(&v, "signin");
    let signin = auth::decode_signin(&input).unwrap();
    let code = field(&v, "code").try_into().unwrap();
    assert_eq!(auth::signin_input(&signin).unwrap(), input);
    assert_eq!(
        auth::signin_proof(&code, &signin).unwrap().as_slice(),
        field(&v, "proof")
    );
    assert_eq!(
        auth::signin_possession_input(&signin).unwrap(),
        field(&v, "possession")
    );
    auth::verify_signin(
        &code,
        &signin,
        &field(&v, "proof"),
        &field(&v, "possessionSignature"),
    )
    .unwrap();
    assert!(
        auth::verify_signin(
            &[9; 16],
            &signin,
            &field(&v, "proof"),
            &field(&v, "possessionSignature")
        )
        .is_err()
    );
    let other = SigningKey::from_bytes(&[9; 32])
        .sign(&field(&v, "possession"))
        .to_bytes();
    assert!(auth::verify_signin(&code, &signin, &field(&v, "proof"), &other).is_err());
    let payload = field(&v, "payload");
    let ctx = context(&v);
    let mut request = auth::Management {
        space: &ctx.space,
        page: &ctx.page,
        expected_revision: "1",
        operation_id: signin.code_id,
        operation: "page.scripts",
        payload: &payload,
        sender_device: signin.device,
        issued_at: 1_790_860_000_000,
        expires_at: 1_790_860_600_000,
    };
    assert_eq!(
        auth::management_input(&request).unwrap(),
        field(&v, "management")
    );
    request.expires_at += 1;
    assert!(auth::management_input(&request).is_err());
    request.expires_at -= 1;
    request.expected_revision = "01";
    assert!(auth::management_input(&request).is_err());
}
#[test]
fn cuts_bind_namespace_and_reject_impossible_boundaries() {
    let v = fixture();
    let raw = field(&v, "cut");
    let mut cut = stream_cut::decode(&raw).unwrap();
    assert_eq!(stream_cut::input(&cut).unwrap(), raw);
    cut.namespace = "own";
    assert_ne!(stream_cut::input(&cut).unwrap(), raw);
    cut.namespace = "invalid";
    assert!(stream_cut::input(&cut).is_err());
    cut.namespace = "content";
    cut.checkpoint_seq = "4";
    assert!(stream_cut::input(&cut).is_err());
    cut.checkpoint_seq = "0";
    assert!(stream_cut::input(&cut).is_err());
    cut.checkpoint_hash = None;
    cut.tail_head_seq = "0";
    assert!(stream_cut::input(&cut).is_err());
    cut.tail_head_hash = &[0; 32];
    assert!(stream_cut::input(&cut).is_ok());
    let mut trailing = raw.clone();
    trailing.push(0);
    assert!(stream_cut::decode(&trailing).is_err());
}
#[test]
fn every_829_signature_corpus_row_obeys_strict_policy_including_mixed_order() {
    let corpus = include_str!("../../../contracts/vectors/ed25519-829.jsonl");
    let rows: Vec<Value> = corpus
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 148);
    assert_eq!(rows.iter().filter(|v| v["nativePolicy"] == true).count(), 9);
    for v in rows {
        assert_eq!(
            crypto::verify_signature(
                &field(&v, "public"),
                &field(&v, "message"),
                &field(&v, "signature")
            )
            .is_ok(),
            v["nativePolicy"].as_bool().unwrap(),
            "{}",
            v["name"]
        );
    }
}
#[test]
fn retained_829_canonical_signatures_and_hmacs_are_known_answers() {
    for row in include_str!("../../../contracts/vectors/encoding-829.jsonl").lines() {
        let v: Value = serde_json::from_str(row).unwrap();
        let raw = field(&v, "hex");
        let parts: Vec<Vec<u8>> = v["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| bytes(v.as_str().unwrap()))
            .collect();
        let mut fields = vec![v["label"].as_str().unwrap().as_bytes(), b"1"];
        fields.extend(parts.iter().map(Vec::as_slice));
        assert_eq!(framing::frame(&fields).unwrap(), raw);
        if v.get("signature").is_some() {
            crypto::verify_signature(&field(&v, "public"), &raw, &field(&v, "signature")).unwrap();
            assert_eq!(crypto::digest(&raw).as_slice(), field(&v, "digest"));
        } else {
            crypto::verify_mac(&field(&v, "key"), &raw, &field(&v, "tag")).unwrap();
        }
    }
}
#[test]
fn frozen_retries_and_fresh_seals_have_distinct_ids_and_keys() {
    let v = fixture();
    let ctx = context(&v);
    let key = SigningKey::from_bytes(&field(&v, "seed").try_into().unwrap());
    let master = field(&v, "master").try_into().unwrap();
    let a = object::seal(&ctx, &master, &key, b"same text").unwrap();
    let b = object::seal(&ctx, &master, &key, b"same text").unwrap();
    assert_ne!(a.header(), b.header());
    assert_ne!(a.ciphertext(), b.ciphertext());
    assert_eq!(
        Envelope::from_json(&a.to_json().unwrap())
            .unwrap()
            .hash()
            .unwrap(),
        a.hash().unwrap()
    );
    assert_eq!(
        object::open(&a, &ctx, &master, key.verifying_key().as_bytes()).unwrap(),
        b"same text"
    );
    assert_eq!(
        object::open(&b, &ctx, &master, key.verifying_key().as_bytes()).unwrap(),
        b"same text"
    );
    assert!(object::seal(&ctx, &master, &key, &vec![0; 256 * 1024 + 1]).is_err());
}
#[test]
fn open_rejects_context_key_ciphertext_signature_and_namespace_substitution() {
    let v = fixture();
    let e = envelope(&v);
    let ctx = context(&v);
    let master = field(&v, "master").try_into().unwrap();
    let public = field(&v, "public").try_into().unwrap();
    let mut other = ctx.clone();
    other.epoch = "2".into();
    assert!(object::open(&e, &other, &master, &public).is_err());
    other = ctx.clone();
    other.namespace = "own".into();
    assert!(object::open(&e, &other, &master, &public).is_err());
    assert!(object::open(&e, &ctx, &[9; 32], &public).is_err());
    assert!(
        object::open(
            &e,
            &ctx,
            &master,
            SigningKey::from_bytes(&[9; 32]).verifying_key().as_bytes()
        )
        .is_err()
    );
    for name in ["ciphertext", "signature"] {
        let mut wire: Value = serde_json::from_slice(&e.to_json().unwrap()).unwrap();
        let mut raw =
            values::binary(wire[name].as_str().unwrap(), object::MAX_PLAINTEXT + 16).unwrap();
        raw[0] ^= 1;
        wire[name] = json!(values::encode_binary(&raw));
        let changed = Envelope::from_json(&serde_json::to_vec(&wire).unwrap()).unwrap();
        assert!(object::open(&changed, &ctx, &master, &public).is_err());
    }
}
#[test]
fn strict_wire_rejects_duplicate_unknown_noncanonical_and_trailing_fields() {
    let v = fixture();
    let e = envelope(&v);
    let raw = e.to_json().unwrap();
    let text = String::from_utf8(raw).unwrap();
    let duplicate = text.replacen('{', "{\"nonce\":\"AAAAAAAAAAAAAAAA\",", 1);
    assert!(Envelope::from_json(duplicate.as_bytes()).is_err());
    let unknown = text.replacen('{', "{\"unknown\":true,", 1);
    assert!(Envelope::from_json(unknown.as_bytes()).is_err());
    assert!(Envelope::from_json(format!("{text}x").as_bytes()).is_err());
    let mut wire: Value = serde_json::from_str(&text).unwrap();
    wire["nonce"] = json!("AAAAAAAAAAAAAAAB");
    assert!(Envelope::from_json(&serde_json::to_vec(&wire).unwrap()).is_err());
    for key in ["header", "ciphertext", "signature"] {
        let mut wire: Value = serde_json::from_str(&text).unwrap();
        wire[key] = json!(format!("{}=", wire[key].as_str().unwrap()));
        assert!(Envelope::from_json(&serde_json::to_vec(&wire).unwrap()).is_err());
    }
    assert!(values::binary("AB", 1).is_err());
    assert!(values::binary("AA", 1).is_ok());
    assert!(values::decimal("18446744073709551616", false).is_err());
    assert!(values::decimal("18446744073709551615", false).is_ok());
    assert!(values::generated_id("00000000-0000-0000-0000-000000000000").is_err());
    assert!(values::time(9_007_199_254_740_992).is_err());
    assert!(framing::id_list(&["00000000-0000-4000-8000-000000000002"; 2], false).is_err());
    let mut header = field(&v, "header");
    header.push(0);
    assert!(Header::decode(&header).is_err());
    header = field(&v, "header");
    header[0..4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(Header::decode(&header).is_err());
}

#[test]
fn namespace_relabeling_with_a_valid_signature_still_fails_aead() {
    let v = fixture();
    let e = envelope(&v);
    let mut h = Header::decode(e.header()).unwrap();
    h.context.namespace = "own".into();
    let header = h.encode().unwrap();
    let signer = SigningKey::from_bytes(&field(&v, "seed").try_into().unwrap());
    let input = framing::frame(&[
        b"tmt-colab-signature-v1",
        &header,
        &[0; 12],
        &crypto::digest(e.ciphertext()),
    ])
    .unwrap();
    let signature = signer.sign(&input).to_bytes();
    crypto::verify_signature(signer.verifying_key().as_bytes(), &input, &signature).unwrap();
    let relabeled = Envelope::from_json(&serde_json::to_vec(&json!({"header":values::encode_binary(&header),"nonce":values::encode_binary(&[0;12]),"ciphertext":values::encode_binary(e.ciphertext()),"signature":values::encode_binary(&signature)})).unwrap()).unwrap();
    assert!(
        object::open(
            &relabeled,
            &h.context,
            &field(&v, "master").try_into().unwrap(),
            signer.verifying_key().as_bytes()
        )
        .is_err()
    );
    let old: Vec<&[u8]> = framing::fields(e.header(), 13, 1024)
        .unwrap()
        .into_iter()
        .enumerate()
        .filter_map(|(i, f)| (i != 7).then_some(f))
        .collect();
    assert!(Header::decode(&framing::frame(&old).unwrap()).is_err());
    let owner = bytes("03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8")
        .try_into()
        .unwrap();
    assert_eq!(crypto::space_id(&owner).unwrap(), h.context.space);
}

#[test]
fn deterministic_keys_match_rfc_and_829_public_fixtures() {
    for row in include_str!("../../../contracts/vectors/keys.jsonl").lines() {
        let v: Value = serde_json::from_str(row).unwrap();
        let seed = field(&v, "seed").try_into().unwrap();
        let public = if v["kind"] == "ed25519" {
            tmt_colab_model::keys::ed25519_public(&seed)
        } else {
            tmt_colab_model::keys::x25519_public(&seed)
        };
        assert_eq!(public.as_slice(), field(&v, "public"), "{}", v["name"]);
        if v["kind"] == "x25519" {
            let mut clamped = seed;
            clamped[0] &= 248;
            clamped[31] &= 127;
            clamped[31] |= 64;
            assert_eq!(tmt_colab_model::keys::x25519_public(&clamped), public);
        }
    }
}
