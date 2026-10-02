use ed25519_dalek::{Signer, SigningKey};
use serde_json::Value;
use tmt_colab_model::{certificate, crypto, link, values, wrap};
fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../contracts/vectors/authority-v1.json")).unwrap()
}
fn hex(v: &Value, field: &str) -> [u8; 32] {
    let s = v[field].as_str().unwrap();
    std::array::from_fn(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
}
fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}
fn owner(v: &Value) -> SigningKey {
    SigningKey::from_bytes(&hex(v, "seed"))
}
fn frozen_wrap(v: &Value) -> wrap::Envelope {
    wrap::Envelope::from_json(&bytes(&v["wrap"])).unwrap()
}
#[test]
fn independent_wrap_chain_and_link_known_answers() {
    let v = fixture();
    let recipient = wrap::RecipientKey::from_seed(&hex(&v, "recipientSeed")).unwrap();
    let w = frozen_wrap(&v);
    let h = w.header().unwrap();
    assert_eq!(
        wrap::open(&w, &h, &recipient, &hex(&v, "public")).unwrap(),
        hex(&v, "epochKey")
    );
    let chain = certificate::Chain::from_json(&bytes(&v["chain"])).unwrap();
    chain
        .verify(
            &hex(&v, "statementHash"),
            &chain.certificate().unwrap(),
            &hex(&v, "public"),
        )
        .unwrap();
    assert_eq!(chain.digest().unwrap(), hex(&v, "chainDigest"));
    let keys = link::Keys::derive(
        &hex(&v, "linkSeed"),
        v["space"].as_str().unwrap(),
        v["linkId"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(keys.signing_public(), hex(&v, "linkSigningPublic"));
    assert_eq!(
        keys.recipient().public_key(),
        hex(&v, "linkEncryptionPublic")
    );
    assert_eq!(*keys.join_proof(), hex(&v, "joinProof"));
}
#[test]
fn fresh_wraps_open_and_low_order_dh_is_rejected_after_valid_owner_authentication() {
    let v = fixture();
    let owner = owner(&v);
    let recipient = wrap::RecipientKey::from_seed(&hex(&v, "recipientSeed")).unwrap();
    let h = frozen_wrap(&v).header().unwrap();
    let a = wrap::seal(&h, &hex(&v, "epochKey"), &owner).unwrap();
    let b = wrap::seal(&h, &hex(&v, "epochKey"), &owner).unwrap();
    assert_ne!(a, b);
    for w in [&a, &b] {
        assert_eq!(
            wrap::open(w, &h, &recipient, &hex(&v, "public")).unwrap(),
            hex(&v, "epochKey")
        );
    }
    for first in [0, 1] {
        let mut low = [0; 32];
        low[0] = first;
        let mut bad_h = h.clone();
        bad_h.recipient_key = low;
        assert!(wrap::seal(&bad_h, &hex(&v, "epochKey"), &owner).is_err());
        let mut wire: Value = serde_json::from_slice(&a.to_json().unwrap()).unwrap();
        wire["enc"] = values::encode_binary(&low).into();
        let altered = wrap::Envelope::from_json(&bytes(&wire)).unwrap();
        wire["signature"] =
            values::encode_binary(&owner.sign(&altered.signature_input().unwrap()).to_bytes())
                .into();
        let altered = wrap::Envelope::from_json(&bytes(&wire)).unwrap();
        altered.verify_owner(&hex(&v, "public")).unwrap();
        assert!(wrap::open(&altered, &h, &recipient, &hex(&v, "public")).is_err());
    }
    let mut other = h.clone();
    other.epoch = "2".into();
    assert!(wrap::open(&a, &other, &recipient, &hex(&v, "public")).is_err());
    assert!(
        wrap::open(
            &a,
            &h,
            &wrap::RecipientKey::from_seed(&[9; 32]).unwrap(),
            &hex(&v, "public")
        )
        .is_err()
    );
    assert!(wrap::open(&a, &h, &recipient, &[9; 32]).is_err());
    for field in ["enc", "ciphertext", "signature"] {
        let mut wire: Value = serde_json::from_slice(&a.to_json().unwrap()).unwrap();
        let mut raw = values::binary(wire[field].as_str().unwrap(), 64).unwrap();
        raw[0] ^= 1;
        wire[field] = values::encode_binary(&raw).into();
        let altered = wrap::Envelope::from_json(&bytes(&wire)).unwrap();
        assert!(wrap::open(&altered, &h, &recipient, &hex(&v, "public")).is_err());
    }
}
#[test]
fn retained_link_seed_remains_a_capability_until_new_link_identity_and_key() {
    let v = fixture();
    let space = v["space"].as_str().unwrap();
    let keys =
        link::Keys::derive(&hex(&v, "linkSeed"), space, v["linkId"].as_str().unwrap()).unwrap();
    let mut h = frozen_wrap(&v).header().unwrap();
    h.recipient_kind = "link".into();
    h.recipient_id = v["linkId"].as_str().unwrap().into();
    h.recipient_key = keys.recipient().public_key();
    h.epoch = "2".into();
    let w = wrap::seal(&h, &hex(&v, "epochKey"), &owner(&v)).unwrap();
    let retained = link::Keys::derive(&hex(&v, "linkSeed"), space, &h.recipient_id).unwrap();
    assert_eq!(
        wrap::open(&w, &h, retained.recipient(), &hex(&v, "public")).unwrap(),
        hex(&v, "epochKey")
    );
    let reset = link::Keys::derive(&[7; 32], space, v["page"].as_str().unwrap()).unwrap();
    h.recipient_id = v["page"].as_str().unwrap().into();
    h.recipient_key = reset.recipient().public_key();
    h.epoch = "3".into();
    let w = wrap::seal(&h, &hex(&v, "epochKey"), &owner(&v)).unwrap();
    assert!(wrap::open(&w, &h, retained.recipient(), &hex(&v, "public")).is_err());
    assert!(wrap::open(&w, &h, reset.recipient(), &hex(&v, "public")).is_ok());
    let cert = certificate::Certificate {
        space,
        issuer_kind: "link",
        issuer_id: v["linkId"].as_str().unwrap(),
        device_id: v["device"].as_str().unwrap(),
        signing_key: &hex(&v, "public"),
        encryption_key: &hex(&v, "recipientSeed"),
        membership_revision: "2",
        issued_at: 0,
        expires_at: 100,
    };
    crypto::verify_signature(
        &keys.signing_public(),
        &certificate::input(&cert).unwrap(),
        &retained.certify(&cert).unwrap(),
    )
    .unwrap();
    // Revoking a device does not erase the link seed; live-log recipient filtering is a caller gate.
    assert!(reset.certify(&cert).is_err());
}
#[test]
fn strict_json_rejects_duplicate_unknown_missing_and_wrong_width_fields() {
    let v = fixture();
    for (field, n) in [("enc", 31), ("ciphertext", 47), ("signature", 63)] {
        let mut w = v["wrap"].clone();
        w[field] = values::encode_binary(&vec![0; n]).into();
        assert!(wrap::Envelope::from_json(&bytes(&w)).is_err());
    }
    for kind in ["wrap", "chain"] {
        let original = bytes(&v[kind]);
        let field = if kind == "wrap" { "enc" } else { "version" };
        let duplicate = format!(
            "{{\"{}\":{},{}",
            field,
            v[kind][field],
            std::str::from_utf8(&original[1..]).unwrap()
        );
        let mut unknown = v[kind].clone();
        unknown["extra"] = true.into();
        let reject = |b: &[u8]| match kind {
            "wrap" => wrap::Envelope::from_json(b).is_err(),
            "chain" => certificate::Chain::from_json(b).is_err(),
            _ => unreachable!(),
        };
        assert!(!reject(&original));
        assert!(reject(duplicate.as_bytes()));
        assert!(reject(&bytes(&unknown)));
    }
}

#[test]
fn certificates_require_exact_live_log_bindings_supplied_by_caller() {
    let v = fixture();
    let c = certificate::Chain::from_json(&bytes(&v["chain"])).unwrap();
    let expected = c.certificate().unwrap();
    assert!(c.verify(&[9; 32], &expected, &hex(&v, "public")).is_err());
    assert!(
        c.verify(&hex(&v, "statementHash"), &expected, &[9; 32])
            .is_err()
    );
    let mut wrong = c.certificate().unwrap();
    wrong.membership_revision = "2";
    assert!(
        c.verify(&hex(&v, "statementHash"), &wrong, &hex(&v, "public"))
            .is_err()
    );
    wrong = c.certificate().unwrap();
    wrong.expires_at = wrong.issued_at;
    assert!(certificate::input(&wrong).is_err());
    let mut wire = v["chain"].clone();
    wire["version"] = 2.into();
    assert!(certificate::Chain::from_json(&bytes(&wire)).is_err());
    let mut raw = values::binary(v["chain"]["deviceCertificate"].as_str().unwrap(), 1024).unwrap();
    raw.push(0);
    assert!(certificate::decode(&raw).is_err());
    wire = v["chain"].clone();
    let mut sig = values::binary(wire["issuerSignature"].as_str().unwrap(), 64).unwrap();
    sig[0] ^= 1;
    wire["issuerSignature"] = values::encode_binary(&sig).into();
    let c = certificate::Chain::from_json(&bytes(&wire)).unwrap();
    assert!(
        c.verify(&hex(&v, "statementHash"), &expected, &hex(&v, "public"))
            .is_err()
    );
}

#[test]
fn earlier_epoch_wrap_and_multi_list_join_use_existing_wrap_grammar() {
    let v = fixture();
    let j = &v["historyJoin"];
    let root = hex(&v, "public");
    let recipient = wrap::RecipientKey::from_seed(&hex(j, "recipientSeed")).unwrap();
    let genesis = tmt_colab_model::statement::Envelope::from_json(&bytes(&v["statement"])).unwrap();
    let head = genesis
        .verify_next(v["space"].as_str().unwrap(), &root, None)
        .unwrap()
        .head;
    let join = tmt_colab_model::statement::Envelope::from_json(&bytes(&j["memberAdd"])).unwrap();
    let verified = join
        .verify_next(v["space"].as_str().unwrap(), &root, Some(&head))
        .unwrap();
    assert_eq!(verified.head.revision, 2);
    let forward = wrap::Envelope::from_json(&bytes(&v["forwardWrap"])).unwrap();
    let h = forward.header().unwrap();
    assert_eq!(h.epoch, "63");
    assert_eq!(h.membership_revision, "2");
    assert_eq!(
        wrap::open(&forward, &h, &recipient, &root).unwrap(),
        hex(&v, "epochKey")
    );
    // These grouping/history checks are caller policy; the model parses each existing wrap.
    let lists = j["wrapLists"].as_array().unwrap();
    assert_eq!(
        lists
            .iter()
            .map(|l| l.as_array().unwrap().len())
            .collect::<Vec<_>>(),
        [512, 64]
    );
    let mut prior = None;
    let mut pages = std::collections::BTreeMap::<String, usize>::new();
    for list in lists {
        for raw in list.as_array().unwrap() {
            let w = wrap::Envelope::from_json(raw.as_str().unwrap().as_bytes()).unwrap();
            let h = w.header().unwrap();
            let epoch = values::decimal(&h.epoch, false).unwrap();
            assert!(epoch <= 64);
            assert_eq!(h.membership_revision, "2");
            assert_eq!(h.recipient_kind, "member");
            assert_eq!(h.recipient_id, "00000000-0000-4000-8000-000000000051");
            let order = (
                h.page.clone(),
                epoch,
                h.recipient_kind.clone(),
                h.recipient_id.clone(),
            );
            assert!(prior.as_ref().is_none_or(|p| p < &order));
            prior = Some(order);
            *pages.entry(h.page.clone()).or_default() += 1;
            assert_eq!(
                wrap::open(&w, &h, &recipient, &root).unwrap(),
                hex(&v, "epochKey")
            );
        }
    }
    assert_eq!(pages.len(), 9);
    assert!(pages.values().all(|n| *n == 64));
}
