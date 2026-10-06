use super::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../../contracts/vectors/publication-content-v1.json"
    ))
    .unwrap()
}
fn fixture(index: usize) -> Value {
    vectors()["fixtures"][index].clone()
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn hex(value: &[u8]) -> String {
    value.iter().map(|b| format!("{b:02x}")).collect()
}
fn signer() -> SigningKey {
    // Public RFC8032 test-1 seed, shared with the independent Python oracle.
    SigningKey::from_bytes(&[
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ])
}
fn public() -> [u8; 32] {
    signer().verifying_key().to_bytes()
}
fn signed(manifest: Manifest) -> SignedJob {
    let signature = values::encode_binary(
        &signer()
            .sign(&manifest.signature_input().unwrap())
            .to_bytes(),
    );
    SignedJob {
        manifest,
        signature,
    }
}
fn input(index: usize) -> (SignedJob, Vec<u8>) {
    let f = fixture(index);
    (
        SignedJob::from_json(&bytes(&f["signedJob"])).unwrap(),
        values::binary(f["packet"].as_str().unwrap(), MAX_PACKET_BYTES).unwrap(),
    )
}
// Synthetic authenticated ciphertext is opaque here: this tests codec geometry, not Yjs or decryption.
fn envelope(header: Header, payload: usize, valid_signature: bool) -> Vec<u8> {
    let header = header.encode().unwrap();
    let ciphertext = vec![3; payload + 16];
    let signature = signer()
        .sign(
            &frame(&[
                b"tmt-colab-signature-v1",
                &header,
                &[0; 12],
                &crypto::digest(&ciphertext),
            ])
            .unwrap(),
        )
        .to_bytes();
    let mut signature = signature;
    if !valid_signature {
        signature[0] ^= 1;
    }
    bytes(
        &json!({"header":values::encode_binary(&header),"nonce":values::encode_binary(&[0;12]),"ciphertext":values::encode_binary(&ciphertext),"signature":values::encode_binary(&signature)}),
    )
}
fn build_packet(raw: &[Vec<u8>], template: &SignedJob) -> (SignedJob, Vec<u8>) {
    let packet: Vec<u8> = raw.iter().flatten().copied().collect();
    let mut manifest = template.manifest.clone();
    manifest.entries = raw
        .iter()
        .map(|bytes| {
            let e = Envelope::from_json(bytes).unwrap();
            PublicationEntry {
                namespace: PublicationKind::Content,
                seq: Header::decode(e.header()).unwrap().context.stream_seq,
                envelope_hash: values::encode_binary(&e.hash().unwrap()),
                envelope_bytes: bytes.len(),
            }
        })
        .collect();
    manifest.packet_bytes = packet.len();
    manifest.packet_hash = values::encode_binary(&crypto::digest(&packet));
    (signed(manifest), packet)
}
fn synthetic(count: usize, payload: usize) -> (SignedJob, Vec<u8>) {
    let (template, original) = input(0);
    let mut header = Header::decode(Envelope::from_json(&original).unwrap().header()).unwrap();
    let mut previous = [0; 32];
    let mut raw = Vec::new();
    for seq in 1..=count {
        header.context.stream_seq = seq.to_string();
        header.context.prev_hash = previous;
        header.object_id = format!("{seq:064x}");
        let item = envelope(header.clone(), payload, true);
        previous = Envelope::from_json(&item).unwrap().hash().unwrap();
        raw.push(item);
    }
    build_packet(&raw, &template)
}
// Inject exact JSON text at one nested boundary without a Value round-trip hiding duplicates.
fn raw_node(value: &Value, path: &[&str], raw: &str) -> Vec<u8> {
    let mut copy = value.clone();
    let mut node = &mut copy;
    for key in path {
        node = if node.is_array() {
            &mut node[key.parse::<usize>().unwrap()]
        } else {
            &mut node[*key]
        };
    }
    *node = Value::String("PUBLICATION_TEST_RAW_NODE".into());
    String::from_utf8(bytes(&copy))
        .unwrap()
        .replace("\"PUBLICATION_TEST_RAW_NODE\"", raw)
        .into_bytes()
}
fn at<'a>(value: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(value, |v, key| {
        if v.is_array() {
            &v[key.parse::<usize>().unwrap()]
        } else {
            &v[*key]
        }
    })
}
fn duplicate(value: &Value, path: &[&str], key: &str) -> Vec<u8> {
    let node = at(value, path);
    let raw = String::from_utf8(bytes(node)).unwrap();
    raw_node(
        value,
        path,
        &format!(
            "{{{}:{},{}",
            serde_json::to_string(key).unwrap(),
            node[key],
            &raw[1..]
        ),
    )
}

#[test]
fn independent_vectors_pin_nested_lp_job_packet_envelope_signatures_and_outcomes() {
    let v = vectors();
    assert_eq!(values::encode_binary(&public()), v["publicKey"]);
    for f in v["fixtures"].as_array().unwrap() {
        let job = SignedJob::from_json(&bytes(&f["signedJob"])).unwrap();
        assert_eq!(
            hex(&job.manifest.signature_input().unwrap()),
            f["signatureInputHex"]
        );
        assert_eq!(job.manifest.job_digest().unwrap(), f["jobDigest"]);
        assert_eq!(
            job.key().unwrap(),
            serde_json::from_value(f["key"].clone()).unwrap()
        );
        let packet = values::binary(f["packet"].as_str().unwrap(), MAX_PACKET_BYTES).unwrap();
        assert_eq!(
            values::encode_binary(&crypto::digest(&packet)),
            f["packetHash"]
        );
        let parsed = job.verify_packet(&packet, &public()).unwrap();
        assert_eq!(parsed.len(), f["envelopes"].as_array().unwrap().len());
        for (entry, e) in parsed.iter().zip(f["envelopes"].as_array().unwrap()) {
            assert_eq!(entry.bytes, e["json"].as_str().unwrap().as_bytes());
            assert_eq!(hex(entry.envelope.header()), e["headerHex"]);
            assert_eq!(
                hex(&entry.envelope.signature_input().unwrap()),
                e["signatureInputHex"]
            );
            assert_eq!(
                values::encode_binary(&entry.envelope.hash().unwrap()),
                e["envelopeHash"]
            );
            assert_eq!(
                values::encode_binary(entry.envelope.signature()),
                e["signature"]
            );
        }
        for outcome in f["outcomes"].as_array().unwrap() {
            let out = Outcome::from_json(&bytes(outcome), &job.key().unwrap(), Some(&job)).unwrap();
            assert_eq!(
                Outcome::from_json(
                    &out.to_json(&job.key().unwrap(), Some(&job)).unwrap(),
                    &job.key().unwrap(),
                    Some(&job)
                )
                .unwrap(),
                out
            );
        }
        let status = LocalStatus::from_json(&bytes(&f["localStatus"])).unwrap();
        assert_eq!(
            LocalStatus::from_json(&status.to_json().unwrap()).unwrap(),
            status
        );
        if f.get("localWrite").is_some() {
            let write = LocalWrite::from_json(&bytes(&f["localWrite"]), &public()).unwrap();
            assert_eq!(
                LocalWrite::from_json(&write.to_json(&public()).unwrap(), &public()).unwrap(),
                write
            );
        }
        assert_eq!(SignedJob::from_json(&job.to_json().unwrap()).unwrap(), job);
    }
}

#[test]
fn exact_packet_bytes_are_bound_independently_of_envelope_json_order_and_whitespace() {
    let (job, raw) = input(0);
    let envelope = Envelope::from_json(&raw).unwrap();
    let mut changed = b" \n".to_vec();
    changed.extend(envelope.to_json().unwrap());
    changed.push(b' ');
    assert_eq!(
        envelope.hash().unwrap(),
        Envelope::from_json(&changed).unwrap().hash().unwrap()
    );
    assert!(job.verify_packet(&changed, &public()).is_err());
    let (resigned, packet) = build_packet(&[changed.clone()], &job);
    assert_eq!(
        resigned.verify_packet(&packet, &public()).unwrap()[0].bytes,
        changed
    );
    assert_ne!(resigned.manifest.packet_hash, job.manifest.packet_hash);
    assert_eq!(
        resigned.manifest.entries[0].envelope_hash,
        job.manifest.entries[0].envelope_hash
    );
}

#[test]
fn every_manifest_nested_boundary_rejects_unknown_duplicate_null_and_missing_keys() {
    let value = fixture(3)["signedJob"].clone();
    for path in [
        vec![],
        vec!["manifest"],
        vec!["manifest", "membershipHead"],
        vec!["manifest", "entries", "0"],
        vec!["manifest", "nativeEvidence"],
    ] {
        let node = at(&value, &path).as_object().unwrap();
        let mut extra = node.clone();
        extra.insert("extra".into(), json!(true));
        assert!(
            SignedJob::from_json(&raw_node(
                &value,
                &path,
                &serde_json::to_string(&extra).unwrap()
            ))
            .is_err()
        );
        for key in node.keys() {
            assert!(
                SignedJob::from_json(&duplicate(&value, &path, key)).is_err(),
                "duplicate {path:?}/{key}"
            );
            let mut missing = node.clone();
            missing.remove(key);
            // Evidence alone is optional, but explicit null is forbidden.
            if key != "nativeEvidence" {
                assert!(
                    SignedJob::from_json(&raw_node(
                        &value,
                        &path,
                        &serde_json::to_string(&missing).unwrap()
                    ))
                    .is_err(),
                    "missing {path:?}/{key}"
                );
            }
            let mut null = node.clone();
            null.insert(key.clone(), Value::Null);
            assert!(
                SignedJob::from_json(&raw_node(
                    &value,
                    &path,
                    &serde_json::to_string(&null).unwrap()
                ))
                .is_err(),
                "null {path:?}/{key}"
            );
        }
    }
    assert!(SignedJob::from_json(&bytes(&value)).is_ok());
}

#[test]
fn canonical_value_grammar_is_enforced_before_encoding_or_packet_verification() {
    let valid = fixture(2)["signedJob"].clone();
    for (field, value) in [
        ("version", json!(2)),
        ("operationId", json!("00000000-0000-4000-8000-00000000000A")),
        ("pageId", json!("00000000-0000-1000-8000-000000000002")),
        ("streamId", json!("00000000-0000-4000-7000-000000000012")),
        ("spaceId", json!("A".repeat(32))),
        ("epoch", json!("01")),
        ("epoch", json!("0")),
        ("epoch", json!("18446744073709551616")),
        ("kind", json!("checkpoint")),
        ("baseRevision", json!(format!("v1:{}", "A".repeat(64)))),
        ("packetHash", json!("AA==")),
        ("packetHash", json!(values::encode_binary(&[0; 31]))),
        ("packetBytes", json!(0)),
        ("packetBytes", json!(-1)),
        ("packetBytes", json!(1.0)),
    ] {
        let mut bad = valid.clone();
        bad["manifest"][field] = value;
        assert!(SignedJob::from_json(&bytes(&bad)).is_err(), "{field}");
    }
    for (field, value) in [
        ("seq", json!("01")),
        ("envelopeHash", json!("AA==")),
        ("envelopeBytes", json!(0)),
        ("envelopeBytes", json!(1.0)),
        ("namespace", json!("own")),
    ] {
        let mut bad = valid.clone();
        bad["manifest"]["entries"][0][field] = value;
        assert!(SignedJob::from_json(&bytes(&bad)).is_err());
    }
    for (field, value) in [
        ("revision", json!("0")),
        ("statementHash", json!("FF".repeat(32))),
    ] {
        let mut bad = valid.clone();
        bad["manifest"]["membershipHead"][field] = value;
        assert!(SignedJob::from_json(&bytes(&bad)).is_err());
    }
    for (field, value) in [
        ("sourceSha256", json!("AA".repeat(32))),
        ("memoryLimit", json!("512 MiB")),
        ("chainHash", json!("AA==")),
    ] {
        let mut bad = valid.clone();
        bad["manifest"]["nativeEvidence"][field] = value;
        assert!(SignedJob::from_json(&bytes(&bad)).is_err());
    }
    let (mut job, packet) = input(0);
    job.manifest.operation_id = "bad".into();
    assert!(job.to_json().is_err());
    assert!(job.key().is_err());
    assert!(job.manifest.signature_input().is_err());
    assert!(job.verify_packet(&packet, &public()).is_err());
}

#[test]
fn signatures_digest_scope_and_causal_order_require_the_original_frozen_binding() {
    let (job, raw) = input(1);
    assert!(
        job.verify_packet(
            &raw,
            &SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes()
        )
        .is_err()
    );
    assert!(job.verify_packet(&raw, &[0; 32]).is_err());
    assert!(job.verify_packet(&raw, &[1; 31]).is_err());
    let mut changed = job.clone();
    changed.signature = values::encode_binary(&[0; 64]);
    assert!(changed.verify_packet(&raw, &public()).is_err());
    let mut bad = raw.clone();
    bad.push(0);
    assert!(job.verify_packet(&bad, &public()).is_err());
    for change in 0..7 {
        let mut m = job.manifest.clone();
        match change {
            0 => m.operation_id = "00000000-0000-4000-8000-000000000099".into(),
            1 => m.space_id = "a".repeat(32),
            2 => m.page_id = "00000000-0000-4000-8000-000000000099".into(),
            3 => m.epoch = "8".into(),
            4 => m.stream_id = "00000000-0000-4000-8000-000000000099".into(),
            5 => m.membership_head.statement_hash = "dd".repeat(32),
            _ => m.base_revision = format!("v1:{}", "dd".repeat(32)),
        }
        let stale = SignedJob {
            manifest: m,
            signature: job.signature.clone(),
        };
        assert!(stale.verify_packet(&raw, &public()).is_err());
    }
    let (native, native_packet) = input(3);
    for change in 0..3 {
        let mut altered = native.clone();
        let evidence = altered.manifest.native_evidence.as_mut().unwrap();
        match change {
            0 => evidence.source_sha256 = "dd".repeat(32),
            1 => evidence.memory_limit = decoder::MemoryLimit::Enforced,
            _ => evidence.chain_hash = values::encode_binary(&[9; 32]),
        }
        assert!(altered.verify_packet(&native_packet, &public()).is_err());
    }
    let entries = job.verify_packet(&raw, &public()).unwrap();
    let mut reverse: Vec<Vec<u8>> = entries.iter().rev().map(|e| e.bytes.to_vec()).collect();
    let mut reordered = job.clone();
    reordered.manifest.entries.reverse();
    assert!(reordered.to_json().is_err());
    for change in 0..9 {
        let mut header = entries[1].header.clone();
        match change {
            0 => header.context.space = "a".repeat(32),
            1 => header.context.page = "00000000-0000-4000-8000-000000000099".into(),
            2 => header.context.epoch = "8".into(),
            3 => header.context.author_device = "00000000-0000-4000-8000-000000000099".into(),
            4 => header.context.membership_revision = "4".into(),
            5 => header.context.prev_hash = [9; 32],
            6 => header.context.kind = "checkpoint".into(),
            7 => header.context.namespace = "own".into(),
            _ => {}
        }
        let late = envelope(header, 1, change != 8);
        reverse[0] = entries[0].bytes.to_vec();
        reverse[1] = late;
        let (mutated, packet) = build_packet(&reverse, &job);
        assert!(
            mutated.verify_packet(&packet, &public()).is_err(),
            "binding {change}"
        );
    }
    // A later device-head transaction must admit this first link; this pure codec checks syntax only.
    let mut later_header = entries[0].header.clone();
    later_header.context.stream_seq = "5".into();
    later_header.context.prev_hash = [8; 32];
    let later = envelope(later_header, 1, true);
    let (later_job, later_packet) = build_packet(&[later], &job);
    assert!(later_job.verify_packet(&later_packet, &public()).is_ok());
    let mut empty = job.clone();
    empty.manifest.entries.clear();
    empty.manifest.packet_bytes = 0;
    assert!(empty.to_json().is_err());
    let mut gap = job.clone();
    gap.manifest.entries[1].seq = "3".into();
    assert!(gap.to_json().is_err());
    let mut overflow = job.clone();
    overflow.manifest.entries[0].seq = u64::MAX.to_string();
    overflow.manifest.entries[1].seq = "1".into();
    assert!(overflow.to_json().is_err());
    let mut wrong_hash = job.clone();
    wrong_hash.manifest.entries[0].envelope_hash = values::encode_binary(&[9; 32]);
    let wrong_hash = signed(wrong_hash.manifest);
    assert!(wrong_hash.verify_packet(&raw, &public()).is_err());
    let mut length = job.clone();
    length.manifest.entries[0].envelope_bytes += 1;
    assert!(length.to_json().is_err());
    let mut digest = job.clone();
    digest.manifest.packet_hash = values::encode_binary(&[9; 32]);
    let digest = signed(digest.manifest);
    assert!(digest.verify_packet(&raw, &public()).is_err());
}

#[test]
fn geometry_bounds_have_authenticated_positive_controls_and_checked_arithmetic() {
    assert_eq!(
        packet_limit(decoder::WRITE_TAIL_UPDATES).unwrap(),
        MAX_PACKET_BYTES
    );
    assert!(packet_limit(usize::MAX).is_err());
    let (count, packet) = synthetic(decoder::WRITE_TAIL_UPDATES, 1);
    assert_eq!(count.verify_packet(&packet, &public()).unwrap().len(), 200);
    let mut excess = count.clone();
    excess
        .manifest
        .entries
        .push(count.manifest.entries[0].clone());
    assert!(excess.to_json().is_err());
    let mut raw: Value = serde_json::from_slice(&count.to_json().unwrap()).unwrap();
    let extra = raw["manifest"]["entries"][0].clone();
    raw["manifest"]["entries"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    assert!(SignedJob::from_json(&bytes(&raw)).is_err());
    let (full, packet) = synthetic(16, decoder::UPDATE_BYTES);
    assert!(full.verify_packet(&packet, &public()).is_ok());
    let decoded = full.verify_packet(&packet, &public()).unwrap();
    let mut raw: Vec<Vec<u8>> = decoded.iter().map(|e| e.bytes.to_vec()).collect();
    let mut h = decoded.last().unwrap().header.clone();
    h.context.stream_seq = "17".into();
    h.context.prev_hash = decoded.last().unwrap().envelope.hash().unwrap();
    h.object_id = "ff".repeat(32);
    raw.push(envelope(h, 1, true));
    let (over, packet) = build_packet(&raw, &full);
    assert!(over.verify_packet(&packet, &public()).is_err());
    let (one, raw) = synthetic(1, decoder::UPDATE_BYTES);
    assert!(one.verify_packet(&raw, &public()).is_ok());
    let mut padding = raw.clone();
    padding.resize(limits::UPDATE_BYTES, b' ');
    let (max_json, p) = build_packet(&[padding], &one);
    assert!(max_json.verify_packet(&p, &public()).is_ok());
    let mut over_json = max_json.clone();
    over_json.manifest.entries[0].envelope_bytes += 1;
    over_json.manifest.packet_bytes += 1;
    assert!(over_json.to_json().is_err());
    let header = Header::decode(Envelope::from_json(&raw).unwrap().header()).unwrap();
    assert!(Envelope::from_json(&envelope(header, decoder::UPDATE_BYTES + 1, true)).is_err());
}

#[test]
fn outcomes_bind_every_original_key_field_and_terminal_fields_to_the_expected_job() {
    let f = fixture(3);
    let job = SignedJob::from_json(&bytes(&f["signedJob"])).unwrap();
    let key = job.key().unwrap();
    for value in f["outcomes"].as_array().unwrap() {
        for (field, new) in [
            ("operationId", json!("00000000-0000-4000-8000-000000000099")),
            ("jobDigest", json!(values::encode_binary(&[9; 32]))),
            ("spaceId", json!("a".repeat(32))),
            ("pageId", json!("00000000-0000-4000-8000-000000000099")),
            ("originalEpoch", json!("8")),
            ("streamId", json!("00000000-0000-4000-8000-000000000099")),
        ] {
            let mut bad = value.clone();
            bad["key"][field] = new;
            assert!(Outcome::from_json(&bytes(&bad), &key, Some(&job)).is_err());
        }
        for path in [vec![], vec!["key"]] {
            for field in at(value, &path).as_object().unwrap().keys() {
                assert!(
                    Outcome::from_json(&duplicate(value, &path, field), &key, Some(&job)).is_err(),
                    "duplicate {field}"
                );
            }
        }
        let mut extra = value.clone();
        extra["extra"] = json!(true);
        assert!(Outcome::from_json(&bytes(&extra), &key, Some(&job)).is_err());
    }
    let committed = &f["outcomes"][0];
    for (field, value) in [
        ("count", json!(1)),
        ("count", json!(201)),
        ("count", json!(0)),
        ("committedRevision", json!("v1:bad")),
        ("nativeEvidence", Value::Null),
        (
            "finalPosition",
            json!({"seq":"1","envelopeHash":job.manifest.entries[0].envelope_hash}),
        ),
    ] {
        let mut bad = committed.clone();
        bad[field] = value;
        assert!(Outcome::from_json(&bytes(&bad), &key, Some(&job)).is_err());
    }
    for field in ["sourceSha256", "chainHash", "memoryLimit"] {
        let mut bad = committed.clone();
        bad["nativeEvidence"][field] = match field {
            "sourceSha256" => json!("dd".repeat(32)),
            "chainHash" => json!(values::encode_binary(&[9; 32])),
            _ => json!("512 MiB address-space limit"),
        };
        assert!(Outcome::from_json(&bytes(&bad), &key, Some(&job)).is_err());
    }
    for path in [vec!["finalPosition"], vec!["nativeEvidence"]] {
        for field in at(committed, &path).as_object().unwrap().keys() {
            assert!(
                Outcome::from_json(&duplicate(committed, &path, field), &key, Some(&job)).is_err()
            );
        }
    }
    for i in [1, 2] {
        for field in [
            "count",
            "finalPosition",
            "committedRevision",
            "nativeEvidence",
        ] {
            let mut bad = f["outcomes"][i].clone();
            bad[field] = committed[field].clone();
            assert!(Outcome::from_json(&bytes(&bad), &key, Some(&job)).is_err());
        }
    }
    let mut bad = f["outcomes"][2].clone();
    bad["code"] = json!("COLAB_CAPACITY");
    assert!(Outcome::from_json(&bytes(&bad), &key, Some(&job)).is_err());
    for code in [
        "COLAB_STALE_BASE",
        "COLAB_CAPACITY",
        "COLAB_PAGE_INACTIVE",
        "COLAB_STATE_MISSING",
        "COLAB_STREAM_GAP",
    ] {
        let mut out = f["outcomes"][1].clone();
        out["code"] = json!(code);
        assert!(Outcome::from_json(&bytes(&out), &key, Some(&job)).is_ok());
    }
    let mut bad = f["outcomes"][1].clone();
    bad["code"] = json!("COLAB_DENIED");
    assert!(Outcome::from_json(&bytes(&bad), &key, Some(&job)).is_err());
}

#[test]
fn local_dtos_do_not_accept_v1_browser_jobs_or_substituted_chain_and_write_fields() {
    let f = fixture(3);
    let valid = &f["localWrite"];
    for field in valid.as_object().unwrap().keys() {
        assert!(LocalWrite::from_json(&duplicate(valid, &[], field), &public()).is_err());
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(LocalWrite::from_json(&bytes(&missing), &public()).is_err());
    }
    for (field, value) in [
        ("version", json!(1)),
        ("action", json!("status")),
        ("extra", json!(true)),
        (
            "packet",
            json!(format!("{}=", valid["packet"].as_str().unwrap())),
        ),
        ("chain", json!(values::encode_binary(b"substituted chain"))),
    ] {
        let mut bad = valid.clone();
        bad[field] = value;
        assert!(LocalWrite::from_json(&bytes(&bad), &public()).is_err());
    }
    let browser = fixture(0);
    let native_missing = json!({"version":2,"action":"write","signedJob":browser["signedJob"],"packet":browser["packet"],"chain":valid["chain"]});
    assert!(LocalWrite::from_json(&bytes(&native_missing), &public()).is_err());
    let mut write = LocalWrite::from_json(&bytes(valid), &public()).unwrap();
    let chain = vec![5; CHAIN_BYTES];
    write.chain = values::encode_binary(&chain);
    write
        .signed_job
        .manifest
        .native_evidence
        .as_mut()
        .unwrap()
        .chain_hash = values::encode_binary(&crypto::digest(&chain));
    write.signed_job = signed(write.signed_job.manifest);
    assert!(write.to_json(&public()).is_ok());
    write.chain = values::encode_binary(&vec![5; CHAIN_BYTES + 1]);
    assert!(write.to_json(&public()).is_err());
    let status = &f["localStatus"];
    for field in ["signedJob", "packet", "chain"] {
        let mut bad = status.clone();
        bad[field] = valid[field].clone();
        assert!(LocalStatus::from_json(&bytes(&bad)).is_err());
    }
    assert!(LocalStatus::from_json(&duplicate(status, &[], "action")).is_err());
    let mut bad = status.clone();
    bad["version"] = json!(1);
    assert!(LocalStatus::from_json(&bytes(&bad)).is_err());
    let old =
        json!({"version":1,"operationId":"00000000-0000-4000-8000-000000000041","envelope":"AQ"});
    assert!(LocalWrite::from_json(&bytes(&old), &public()).is_err());
    assert!(LocalStatus::from_json(&bytes(&old)).is_err());
}

#[test]
fn pre_parse_json_caps_have_valid_exact_boundary_controls() {
    let f = fixture(3);
    let job = SignedJob::from_json(&bytes(&f["signedJob"])).unwrap();
    let key = job.key().unwrap();
    let padded = |value: &Value, cap: usize| {
        let mut raw = bytes(value);
        raw.resize(cap, b' ');
        raw
    };
    let mut raw = padded(&f["signedJob"], JSON_BYTES);
    assert!(SignedJob::from_json(&raw).is_ok());
    raw.push(b' ');
    assert!(SignedJob::from_json(&raw).is_err());
    let mut raw = padded(&f["outcomes"][0], JSON_BYTES);
    assert!(Outcome::from_json(&raw, &key, Some(&job)).is_ok());
    raw.push(b' ');
    assert!(Outcome::from_json(&raw, &key, Some(&job)).is_err());
    let mut raw = padded(&f["localStatus"], JSON_BYTES);
    assert!(LocalStatus::from_json(&raw).is_ok());
    raw.push(b' ');
    assert!(LocalStatus::from_json(&raw).is_err());
    let mut raw = padded(&f["localWrite"], LOCAL_WRITE_BYTES);
    assert!(LocalWrite::from_json(&raw, &public()).is_ok());
    raw.push(b' ');
    assert!(LocalWrite::from_json(&raw, &public()).is_err());
}

#[test]
fn outcome_and_local_dto_nested_objects_reject_structural_ambiguity() {
    let f = fixture(3);
    let job = SignedJob::from_json(&bytes(&f["signedJob"])).unwrap();
    let key = job.key().unwrap();
    let check = |value: &Value, paths: &[Vec<&str>], accept: &dyn Fn(&[u8]) -> bool| {
        assert!(accept(&bytes(value)));
        for path in paths {
            let node = at(value, path).as_object().unwrap();
            let mut extra = node.clone();
            extra.insert("extra".into(), json!(true));
            assert!(!accept(&raw_node(
                value,
                path,
                &serde_json::to_string(&extra).unwrap()
            )));
            for field in node.keys() {
                assert!(
                    !accept(&duplicate(value, path, field)),
                    "duplicate {path:?}/{field}"
                );
                let mut missing = node.clone();
                missing.remove(field);
                if field != "nativeEvidence" {
                    assert!(
                        !accept(&raw_node(
                            value,
                            path,
                            &serde_json::to_string(&missing).unwrap()
                        )),
                        "missing {path:?}/{field}"
                    );
                }
                let mut null = node.clone();
                null.insert(field.clone(), Value::Null);
                assert!(
                    !accept(&raw_node(
                        value,
                        path,
                        &serde_json::to_string(&null).unwrap()
                    )),
                    "null {path:?}/{field}"
                );
            }
        }
    };
    for outcome in f["outcomes"].as_array().unwrap() {
        let mut paths = vec![vec![], vec!["key"]];
        if outcome["status"] == "committed" {
            paths.extend([vec!["finalPosition"], vec!["nativeEvidence"]]);
        }
        check(outcome, &paths, &|raw| {
            Outcome::from_json(raw, &key, Some(&job)).is_ok()
        });
    }
    check(&f["localStatus"], &[vec![], vec!["key"]], &|raw| {
        LocalStatus::from_json(raw).is_ok()
    });
    check(
        &f["localWrite"],
        &[
            vec![],
            vec!["signedJob"],
            vec!["signedJob", "manifest"],
            vec!["signedJob", "manifest", "membershipHead"],
            vec!["signedJob", "manifest", "entries", "0"],
            vec!["signedJob", "manifest", "nativeEvidence"],
        ],
        &|raw| {
            let admitted = LocalWrite::from_json(raw, &public()).is_ok();
            assert_eq!(admitted, serde_json::from_slice::<LocalWrite>(raw).is_ok());
            admitted
        },
    );
}

// Padding is INSIDE the nested object, so it belongs to SignedJob's raw byte budget.
fn nested_job_input(size: usize) -> Vec<u8> {
    let f = fixture(3);
    let job = bytes(&f["signedJob"]);
    let mut raw = vec![b'{'];
    raw.resize(1 + size - job.len(), b' ');
    raw.extend_from_slice(&job[1..]);
    assert_eq!(raw.len(), size);
    assert_eq!(
        serde_json::from_slice::<Value>(&raw).unwrap(),
        f["signedJob"]
    );
    assert_eq!(SignedJob::from_json(&raw).is_ok(), size <= JSON_BYTES);
    let input = raw_node(
        &f["localWrite"],
        &["signedJob"],
        std::str::from_utf8(&raw).unwrap(),
    );
    assert!(input.len() < LOCAL_WRITE_BYTES);
    eprintln!(
        "nested-job bytes={size}, outer bytes={}, input SHA256={}",
        input.len(),
        hex(&crypto::digest(&input))
    );
    input
}

#[test]
fn local_write_from_json_bounds_the_original_nested_signed_job_bytes() {
    assert!(LocalWrite::from_json(&nested_job_input(JSON_BYTES), &public()).is_ok());
    assert!(LocalWrite::from_json(&nested_job_input(JSON_BYTES + 1), &public()).is_err());
}

#[test]
fn local_write_derived_deserialize_bounds_the_original_nested_signed_job_bytes() {
    assert!(serde_json::from_slice::<LocalWrite>(&nested_job_input(JSON_BYTES)).is_ok());
    assert!(serde_json::from_slice::<LocalWrite>(&nested_job_input(JSON_BYTES + 1)).is_err());
}

#[test]
fn primary_oversized_nested_job_counterexample_is_rejected_without_changing_its_fields() {
    let normalized = bytes(&fixture(3)["signedJob"]);
    let input = nested_job_input(normalized.len() + JSON_BYTES + 1);
    // Exact source-traced input retained by the primary; only internal object whitespace differs.
    assert_eq!(
        hex(&crypto::digest(&input)),
        "067ec5632f1f19b0aed91024215b12bfedee4d379eab73a7186c0901cbd1ccb3"
    );
    assert!(LocalWrite::from_json(&input, &public()).is_err());
    assert!(serde_json::from_slice::<LocalWrite>(&input).is_err());
}
