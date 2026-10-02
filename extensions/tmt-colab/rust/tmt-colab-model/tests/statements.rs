use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use tmt_colab_model::{certificate, payload, statement, stream_cut, values};
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
fn admitted(op: &str, v: &Value) -> bool {
    payload::decode(op, &bytes(v)).is_ok()
}
#[test]
fn all_operation_schemas_and_bounded_sorted_lists() {
    let v = fixture();
    let p = &v["page"];
    let d = &v["device"];
    let pk = values::encode_binary(&hex(&v, "public"));
    let ek = values::encode_binary(&hex(&v, "recipientSeed"));
    let baseline = json!({"pageId":p,"epoch":"1","sourceDigest":ek,"baselineCommitment":ek,"title":"Page","objectEnvelopeHash":ek,"membershipRevision":"2"});
    let cases = [
        (
            "member.add",
            serde_json::from_str(v["payload"].as_str().unwrap()).unwrap(),
        ),
        ("member.remove", json!({"memberId":d,"cuts":[]})),
        (
            "member.role",
            json!({"memberId":d,"role":"viewer","cuts":[]}),
        ),
        (
            "link.add",
            json!({"linkId":d,"role":"editor","linkSignKey":pk,"linkEncKey":ek,"pages":[p]}),
        ),
        ("link.remove", json!({"linkId":d,"cuts":[]})),
        ("device.revoke", json!({"deviceId":d,"cuts":[]})),
        (
            "bridge.add",
            json!({"machineId":d,"machineSignKey":pk,"encKey":ek,"pages":[p]}),
        ),
        (
            "epoch.advance",
            json!({"pageId":p,"epoch":"1","cuts":[],"baseline":baseline,"wraps":[v["wrap"]]}),
        ),
        (
            "page.share",
            json!({"pageId":p,"epoch":"1","mode":"private"}),
        ),
        ("page.scripts", json!({"pageId":p,"mode":"static"})),
        ("retention.set", json!({"pageId":p,"days":null})),
        ("page.archive", json!({"pageId":p})),
        ("page.delete", json!({"pageId":p})),
    ];
    for (op, value) in cases {
        assert!(admitted(op, &value), "{op}");
        let mut extra = value;
        extra["extra"] = true.into();
        assert!(!admitted(op, &extra), "{op}");
    }
    let mut member: Value = serde_json::from_str(v["payload"].as_str().unwrap()).unwrap();
    for pages in [json!([p, p]), json!([d, p]), json!(vec![p; 257])] {
        member["pages"] = pages;
        assert!(!admitted("member.add", &member));
    }
    let cut = stream_cut::input(&stream_cut::StreamCut {
        stream_id: d.as_str().unwrap(),
        namespace: "content",
        checkpoint_hash: None,
        checkpoint_seq: "0",
        tail_head_seq: "0",
        tail_head_hash: &[0; 32],
    })
    .unwrap();
    let c = |epoch| json!({"pageId":p,"epoch":epoch,"namespace":"content","cut":values::encode_binary(&cut)});
    assert!(admitted(
        "device.revoke",
        &json!({"deviceId":d,"cuts":[c("2"),c("10")]})
    ));
    for cuts in [
        json!([c("10"), c("2")]),
        json!([c("2"), c("2")]),
        json!(vec![c("2"); 513]),
    ] {
        assert!(!admitted(
            "device.revoke",
            &json!({"deviceId":d,"cuts":cuts})
        ));
    }
    let mut bad = c("2");
    bad["namespace"] = "comment".into();
    assert!(!admitted(
        "device.revoke",
        &json!({"deviceId":d,"cuts":[bad]})
    ));
    let mut e =
        json!({"pageId":p,"epoch":"1","cuts":[],"baseline":baseline,"wraps":[v["wrap"],v["wrap"]]});
    assert!(!admitted("epoch.advance", &e));
    e["wraps"] = json!(vec![v["wrap"].clone(); 513]);
    assert!(!admitted("epoch.advance", &e));
}
#[test]
fn published_keys_require_current_epoch_and_numeric_unique_order() {
    let v = fixture();
    let p = &v["page"];
    let k = values::encode_binary(&hex(&v, "epochKey"));
    let key = |epoch| json!({"epoch":epoch,"key":k});
    assert!(admitted(
        "page.share",
        &json!({"pageId":p,"epoch":"10","mode":"public","publishedKeys":[key("2"),key("10")]})
    ));
    for keys in [
        json!(null),
        json!([]),
        json!([key("2")]),
        json!([key("10"), key("2")]),
        json!([key("10"), key("10")]),
        json!(vec![key("10"); 65]),
    ] {
        assert!(!admitted(
            "page.share",
            &json!({"pageId":p,"epoch":"10","mode":"public","publishedKeys":keys})
        ));
    }
    for mode in ["private", "link"] {
        assert!(!admitted(
            "page.share",
            &json!({"pageId":p,"epoch":"10","mode":mode,"publishedKeys":[key("10")]})
        ));
        assert!(admitted(
            "page.share",
            &json!({"pageId":p,"epoch":"10","mode":mode,"publishedKeys":[]})
        ));
        assert!(!admitted(
            "page.share",
            &json!({"pageId":p,"epoch":"10","mode":mode,"publishedKeys":null})
        ));
    }
}
#[test]
fn owner_chain_rejects_replay_forks_payload_substitution_and_wrong_root() {
    let v = fixture();
    let space = v["space"].as_str().unwrap();
    let owner = owner(&v);
    let first = statement::Envelope::from_json(&bytes(&v["statement"])).unwrap();
    let head = first
        .verify_next(space, &hex(&v, "public"), None)
        .unwrap()
        .head;
    assert!(
        first
            .verify_next(space, &hex(&v, "public"), Some(&head))
            .is_err()
    );
    assert!(first.verify_next(space, &[7; 32], None).is_err());
    let p = bytes(&json!({"pageId":v["page"],"mode":"static"}));
    let next = statement::sign(space, Some(&head), "page.scripts", &p, &owner).unwrap();
    assert!(
        next.verify_next(space, &hex(&v, "public"), Some(&head))
            .is_ok()
    );
    assert!(next.verify_next(space, &hex(&v, "public"), None).is_err());
    let mut fork = head.clone();
    fork.hash[0] ^= 1;
    assert!(
        next.verify_next(space, &hex(&v, "public"), Some(&fork))
            .is_err()
    );
    assert!(statement::sign(space, None, "page.scripts", &p, &owner).is_err());
    let mut wire = v["statement"].clone();
    wire["payload"] = values::encode_binary(&p).into();
    assert!(statement::Envelope::from_json(&bytes(&wire)).is_err());
    let chain = certificate::Chain::from_json(&bytes(&v["chain"])).unwrap();
    assert!(
        chain
            .verify(&[9; 32], &chain.certificate().unwrap(), &hex(&v, "public"))
            .is_err()
    );
    assert!(
        chain
            .verify(&head.hash, &chain.certificate().unwrap(), &[9; 32])
            .is_err()
    );
    let mut expected = chain.certificate().unwrap();
    expected.membership_revision = "2";
    assert!(
        chain
            .verify(&head.hash, &expected, &hex(&v, "public"))
            .is_err()
    );
}
#[test]
fn exact_payload_bytes_and_owner_statement_match_independent_vector() {
    let v = fixture();
    let s = statement::sign(
        v["space"].as_str().unwrap(),
        None,
        "member.add",
        v["payload"].as_str().unwrap().as_bytes(),
        &owner(&v),
    )
    .unwrap();
    let expected = statement::Envelope::from_json(&bytes(&v["statement"])).unwrap();
    assert_eq!(s.to_json().unwrap(), expected.to_json().unwrap());
    assert_eq!(s.hash().unwrap(), hex(&v, "statementHash"));
    let mut padded = v["payload"].as_str().unwrap().as_bytes().to_vec();
    padded.push(b' ');
    let different = statement::sign(
        v["space"].as_str().unwrap(),
        None,
        "member.add",
        &padded,
        &owner(&v),
    )
    .unwrap();
    assert_ne!(s.hash().unwrap(), different.hash().unwrap());
    let mut wire = v["statement"].clone();
    wire["signature"] = values::encode_binary(&[0; 64]).into();
    let altered = statement::Envelope::from_json(&bytes(&wire)).unwrap();
    assert!(
        altered
            .verify_next(v["space"].as_str().unwrap(), &hex(&v, "public"), None)
            .is_err()
    );
}
#[test]
fn payload_fields_require_strict_types_and_bounds_not_default_authority() {
    let v = fixture();
    let p = &v["page"];
    for days in [
        json!(0),
        json!(-1),
        json!(1.5),
        json!("1"),
        json!(9007199254740992u64),
    ] {
        assert!(!admitted("retention.set", &json!({"pageId":p,"days":days})));
    }
    assert!(!admitted("retention.set", &json!({"pageId":p})));
    assert!(admitted("retention.set", &json!({"pageId":p,"days":7})));
    assert!(payload::decode("page.delete", &vec![b' '; payload::MAX_BYTES + 1]).is_err());
    let mut member: Value = serde_json::from_str(v["payload"].as_str().unwrap()).unwrap();
    for role in ["owner", "admin", "EDITOR"] {
        member["role"] = role.into();
        assert!(!admitted("member.add", &member));
    }
    member["role"] = "editor".into();
    member["encKey"] = values::encode_binary(&[0; 31]).into();
    assert!(!admitted("member.add", &member));
    member["encKey"] = values::encode_binary(&[0; 32]).into();
    member["signKey"] = values::encode_binary(&[0; 32]).into();
    assert!(!admitted("member.add", &member));
    let duplicate = format!("{{\"pageId\":{},\"pageId\":{}}}", p, p);
    assert!(payload::decode("page.delete", duplicate.as_bytes()).is_err());
    assert!(payload::decode("unknown", b"{}").is_err());
    let mut wire = v["statement"].clone();
    wire["extra"] = true.into();
    assert!(statement::Envelope::from_json(&bytes(&wire)).is_err());
    let dup = format!(
        "{{\"payload\":{},{}",
        v["statement"]["payload"],
        std::str::from_utf8(&bytes(&v["statement"])[1..]).unwrap()
    );
    assert!(statement::Envelope::from_json(dup.as_bytes()).is_err());
}
#[test]
fn epoch_statement_binds_nested_wrap_owner_space_and_baseline_revision() {
    let v = fixture();
    let space = v["space"].as_str().unwrap();
    let owner = owner(&v);
    let pk = values::encode_binary(&hex(&v, "epochKey"));
    let mut payload = json!({"pageId":v["page"],"epoch":"1","cuts":[],"baseline":{"pageId":v["page"],"epoch":"1","sourceDigest":pk,"baselineCommitment":pk,"title":"Page","objectEnvelopeHash":pk,"membershipRevision":"2"},"wraps":[v["wrap"]]});
    let head = statement::Envelope::from_json(&bytes(&v["statement"]))
        .unwrap()
        .verify_next(space, &hex(&v, "public"), None)
        .unwrap()
        .head;
    let sign = |p: &Value| statement::sign(space, Some(&head), "epoch.advance", &bytes(p), &owner);
    assert!(sign(&payload).is_ok());
    payload["baseline"]["membershipRevision"] = "3".into();
    assert!(sign(&payload).is_err());
    payload["baseline"]["membershipRevision"] = "2".into();
    payload["wraps"][0]["signature"] = values::encode_binary(&[0; 64]).into();
    assert!(admitted("epoch.advance", &payload));
    assert!(sign(&payload).is_err());
    payload["wraps"][0] = v["wrap"].clone();
    payload["baseline"]["pageId"] = v["device"].clone();
    assert!(sign(&payload).is_err());
    payload["baseline"]["pageId"] = v["page"].clone();
    let cut = stream_cut::input(&stream_cut::StreamCut {
        stream_id: v["device"].as_str().unwrap(),
        namespace: "content",
        checkpoint_hash: None,
        checkpoint_seq: "0",
        tail_head_seq: "0",
        tail_head_hash: &[0; 32],
    })
    .unwrap();
    payload["cuts"] = json!([{"pageId":v["page"],"epoch":"1","namespace":"content","cut":values::encode_binary(&cut)}]);
    assert!(admitted("epoch.advance", &payload));
    assert!(sign(&payload).is_err());
    payload["epoch"] = "2".into();
    assert!(!admitted("epoch.advance", &payload));
}
#[test]
fn genesis_pins_editor_principal_and_successors_cannot_reuse_its_id_or_keys() {
    use ed25519_dalek::Signer;
    let v = fixture();
    let space = v["space"].as_str().unwrap();
    let owner = owner(&v);
    let first = statement::Envelope::from_json(&bytes(&v["statement"])).unwrap();
    let head = first
        .verify_next(space, &hex(&v, "public"), None)
        .unwrap()
        .head;
    let forged = |revision: &str, prev: &[u8; 32], operation: &str, p: &Value| {
        let payload = bytes(p);
        let header = statement::input(&statement::Header {
            space,
            revision,
            previous_hash: prev,
            operation,
            payload_digest: &tmt_colab_model::crypto::digest(&payload),
        })
        .unwrap();
        statement::Envelope::from_json(&bytes(&json!({"statement":values::encode_binary(&header),"payload":values::encode_binary(&payload),"signature":values::encode_binary(&owner.sign(&header).to_bytes())}))).unwrap()
    };
    let scripts = json!({"pageId":v["page"],"mode":"static"});
    assert!(
        forged("1", &[0; 32], "page.scripts", &scripts)
            .verify_next(space, &hex(&v, "public"), None)
            .is_err()
    );
    let mut genesis: Value = serde_json::from_str(v["payload"].as_str().unwrap()).unwrap();
    genesis["role"] = "viewer".into();
    assert!(statement::sign(space, None, "member.add", &bytes(&genesis), &owner).is_err());
    assert!(
        forged("1", &[0; 32], "member.add", &genesis)
            .verify_next(space, &hex(&v, "public"), None)
            .is_err()
    );
    let mut normal = genesis;
    normal["role"] = "editor".into();
    normal["memberId"] = v["page"].clone();
    normal["signKey"] =
        values::encode_binary(&SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes()).into();
    normal["encKey"] = values::encode_binary(&[4; 32]).into();
    let next = statement::sign(space, Some(&head), "member.add", &bytes(&normal), &owner).unwrap();
    assert_eq!(
        next.verify_next(space, &hex(&v, "public"), Some(&head))
            .unwrap()
            .head
            .owner_member,
        head.owner_member
    );
    let original: Value = serde_json::from_str(v["payload"].as_str().unwrap()).unwrap();
    for field in ["memberId", "signKey", "encKey"] {
        let mut reuse = normal.clone();
        reuse[field] = original[field].clone();
        assert!(
            statement::sign(space, Some(&head), "member.add", &bytes(&reuse), &owner).is_err(),
            "{field}"
        );
        assert!(
            forged("2", &head.hash, "member.add", &reuse)
                .verify_next(space, &hex(&v, "public"), Some(&head))
                .is_err(),
            "{field}"
        );
    }
}
#[test]
fn list_caps_reject_otherwise_valid_unique_ordered_entries() {
    let v = fixture();
    let ids: Vec<String> = (1..=257)
        .map(|n| format!("00000000-0000-4000-8000-{n:012x}"))
        .collect();
    let mut member: Value = serde_json::from_str(v["payload"].as_str().unwrap()).unwrap();
    member["pages"] = json!(&ids[..256]);
    assert!(admitted("member.add", &member));
    member["pages"] = json!(&ids);
    assert!(!admitted("member.add", &member));
    let key = values::encode_binary(&hex(&v, "epochKey"));
    let keys: Vec<Value> = (1..=65)
        .map(|n| json!({"epoch":n.to_string(),"key":key}))
        .collect();
    let mut share =
        json!({"pageId":v["page"],"epoch":"64","mode":"public","publishedKeys":&keys[..64]});
    assert!(admitted("page.share", &share));
    share["epoch"] = "65".into();
    share["publishedKeys"] = json!(&keys);
    assert!(!admitted("page.share", &share));
    let cuts: Vec<Value> = (1..=513).map(|n| {
        let id = format!("00000000-0000-4000-8000-{n:012x}");
        let cut = stream_cut::input(&stream_cut::StreamCut{stream_id:&id,namespace:"content",checkpoint_hash:None,checkpoint_seq:"0",tail_head_seq:"0",tail_head_hash:&[0;32]}).unwrap();
        json!({"pageId":v["page"],"epoch":"1","namespace":"content","cut":values::encode_binary(&cut)})
    }).collect();
    assert!(admitted(
        "device.revoke",
        &json!({"deviceId":v["device"],"cuts":&cuts[..512]})
    ));
    assert!(!admitted(
        "device.revoke",
        &json!({"deviceId":v["device"],"cuts":cuts})
    ));
    let mut exact = bytes(&json!({"pageId":v["page"]}));
    exact.resize(payload::MAX_BYTES, b' ');
    assert!(payload::decode("page.delete", &exact).is_ok());
    exact.push(b' ');
    assert!(payload::decode("page.delete", &exact).is_err());
}

#[test]
fn shared_owner_member_vectors_isolate_rejection_from_signature_and_payload_errors() {
    let v = fixture();
    let cases: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/owner-member-v1.json"
    ))
    .unwrap();
    let space = v["space"].as_str().unwrap();
    let key = hex(&v, "public");
    let genesis = statement::Envelope::from_json(&bytes(&v["statement"])).unwrap();
    let head = genesis.verify_next(space, &key, None).unwrap().head;
    for case in cases["cases"].as_array().unwrap() {
        let wire = &case["envelope"];
        let input = values::binary(wire["statement"].as_str().unwrap(), 1024).unwrap();
        let signature = values::binary(wire["signature"].as_str().unwrap(), 64).unwrap();
        tmt_colab_model::crypto::verify_signature(&key, &input, &signature).unwrap();
        let envelope = statement::Envelope::from_json(&bytes(wire)).unwrap();
        let accepted = case["accepted"].as_bool().unwrap();
        assert_eq!(
            envelope.verify_next(space, &key, Some(&head)).is_ok(),
            accepted,
            "{}",
            case["name"]
        );
        let raw = values::binary(wire["payload"].as_str().unwrap(), payload::MAX_BYTES).unwrap();
        assert_eq!(
            statement::sign(
                space,
                Some(&head),
                case["operation"].as_str().unwrap(),
                &raw,
                &owner(&v)
            )
            .is_ok(),
            accepted,
            "{}",
            case["name"]
        );
    }
}
