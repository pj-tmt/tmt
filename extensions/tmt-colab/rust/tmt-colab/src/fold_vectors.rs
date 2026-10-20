//! Frozen vectors for the owner fold (colab-v1, "Planned owner fold (#3983)"): the owner log,
//! the device chains and the sealed objects that the browser replays through `Admission` and
//! `Objects` (`app/test/fold-own.test.ts`). This module regenerates them and replays, against the
//! same bytes, the part of each case the model itself decides: the header shape, the anchor check,
//! what a revoke lists and that a reader of today's thirteen-field header refuses a fold. A store
//! replay joins them when native writes and reads folds.
//!
//! Regenerate after a deliberate contract change (objects use random IDs, so every byte changes):
//! `cargo test -p tmt-colab --lib fold_vectors::regenerate -- --ignored`.
use crate::keyring::{Keyring, Layout};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};
use tmt_colab_model::{
    certificate, framing,
    object::{self, Context},
    payload, statement, stream_cut, values, wrap,
};
use yrs::{Doc, ReadTxn, StateVector, Transact};

const VECTOR: &str = include_str!("../../../contracts/vectors/fold-v1.json");
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const SIGNER: &str = "50000000-0000-4000-8000-000000000001";
const COVERED: &str = "50000000-0000-4000-8000-000000000002";
const UNKNOWN: &str = "50000000-0000-4000-8000-000000000009";
const BRIDGE: &str = "60000000-0000-4000-8000-000000000001";
const OWNER_SEED: [u8; 32] = [0x41; 32];
const EPOCH_SECRET: [u8; 32] = [11; 32];
/// The revision every device chain and object cites: the log holds `member.add` and `bridge.add`.
const REVISION: &str = "2";

fn seed(byte: u8) -> SigningKey {
    SigningKey::from_bytes(&[byte; 32])
}
fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}
fn unb64(value: &Value) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(value.as_str().unwrap()).unwrap()
}
fn layout_with_owner() -> (PathBuf, Keyring) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "fv-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let layout = Layout::open(&root).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(layout.directory.join("owner.key"))
        .unwrap()
        .write_all(&OWNER_SEED)
        .unwrap();
    (root, Keyring::open(&layout).unwrap())
}
fn update_bytes() -> Vec<u8> {
    let doc = Doc::new();
    for root in ["threads", "messages", "intents", "replies"] {
        doc.get_or_insert_map(root);
    }
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
}
struct Sealed {
    entry: Value,
    hash: [u8; 32],
    envelope: object::Envelope,
}
fn entry(seq: u64, hash: &[u8; 32], envelope: &object::Envelope) -> Value {
    json!({"seq": seq.to_string(), "envelopeHash": b64(hash), "envelope": b64(&envelope.to_json().unwrap())})
}
#[allow(clippy::too_many_arguments)]
fn update(
    space: &str,
    stream: &str,
    namespace: &str,
    revision: &str,
    seq: u64,
    previous: [u8; 32],
    signer: &SigningKey,
) -> Sealed {
    let envelope = object::seal(
        &Context {
            space: space.into(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: "update".into(),
            namespace: namespace.into(),
            author_device: stream.into(),
            membership_revision: revision.into(),
            stream_seq: seq.to_string(),
            prev_hash: previous,
        },
        &EPOCH_SECRET,
        signer,
        &update_bytes(),
    )
    .unwrap();
    let hash = envelope.hash().unwrap();
    Sealed {
        entry: entry(seq, &hash, &envelope),
        hash,
        envelope,
    }
}
#[allow(clippy::too_many_arguments)]
fn fold(
    space: &str,
    signer_device: &str,
    covered: &str,
    namespace: &str,
    revision: &str,
    seq: u64,
    anchor: [u8; 32],
    signer: &SigningKey,
) -> Sealed {
    let envelope = object::seal_fold(
        &Context {
            space: space.into(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: object::FOLD.into(),
            namespace: namespace.into(),
            author_device: signer_device.into(),
            membership_revision: revision.into(),
            stream_seq: seq.to_string(),
            prev_hash: anchor,
        },
        covered,
        &EPOCH_SECRET,
        signer,
        &update_bytes(),
    )
    .unwrap();
    let hash = envelope.hash().unwrap();
    Sealed {
        entry: entry(seq, &hash, &envelope),
        hash,
        envelope,
    }
}
fn frame_of(stream: &str, namespace: &str, slot: &str, sealed: &Sealed) -> Value {
    match slot {
        "anchor" => json!({"streamId": stream, "namespace": namespace, "checkpoint": null,
            "anchor": sealed.entry, "tail": []}),
        _ => {
            json!({"streamId": stream, "namespace": namespace, "checkpoint": sealed.entry, "tail": []})
        }
    }
}

#[test]
#[ignore = "regenerates the frozen vector"]
fn regenerate() {
    let (root, key) = layout_with_owner();
    let space = key.space_id.clone();
    let owner_member = key.management_member().unwrap();
    let mut log: Vec<statement::Envelope> = Vec::new();
    let mut head: Option<statement::Head> = None;
    let push = |log: &mut Vec<statement::Envelope>,
                head: &mut Option<statement::Head>,
                op: &str,
                payload: Value| {
        let envelope = key
            .sign_statement(head.as_ref(), op, &serde_json::to_vec(&payload).unwrap())
            .unwrap();
        *head = Some(
            envelope
                .verify_next(&space, &key.owner_public(), head.as_ref())
                .unwrap()
                .head,
        );
        log.push(envelope);
    };
    push(
        &mut log,
        &mut head,
        "member.add",
        json!({"memberId": owner_member.id, "role": "editor",
            "signKey": b64(&owner_member.signing_key), "encKey": b64(&owner_member.encryption_key), "pages": []}),
    );
    let member_statement = head.as_ref().unwrap().hash;
    push(
        &mut log,
        &mut head,
        "bridge.add",
        json!({"machineId": BRIDGE, "machineSignKey": b64(seed(13).verifying_key().as_bytes()),
            "encKey": b64(&wrap::RecipientKey::from_seed(&[14; 32]).unwrap().public_key()), "pages": [PAGE]}),
    );
    let base_len = log.len();
    // An owner-member device chain, issued at the member.add revision.
    let chain = |device: &str, byte: u8| -> String {
        let signing = seed(byte).verifying_key().to_bytes();
        let enc = wrap::RecipientKey::from_seed(&[byte + 1; 32])
            .unwrap()
            .public_key();
        let certificate = certificate::Certificate {
            space: &space,
            issuer_kind: "member",
            issuer_id: &owner_member.id,
            device_id: device,
            signing_key: &signing,
            encryption_key: &enc,
            membership_revision: "1",
            issued_at: 1,
            expires_at: 100,
        };
        let signature = key.sign_device_certificate(&certificate).unwrap();
        b64(&serde_json::to_vec(&json!({"version": 1,
            "issuerStatement": values::encode_binary(&member_statement),
            "deviceCertificate": values::encode_binary(&certificate::input(&certificate).unwrap()),
            "issuerSignature": values::encode_binary(&signature)}))
        .unwrap())
    };
    let (signer_key, covered_key, bridge_key) = (seed(21), seed(22), seed(13));
    // The covered device wrote two content updates; the second is the anchor, the head that a fold
    // of 1..2 binds in both namespaces (a stream has one sequence across its namespaces).
    let content_first = update(
        &space,
        COVERED,
        "content",
        REVISION,
        1,
        [0; 32],
        &covered_key,
    );
    let anchor = update(
        &space,
        COVERED,
        "content",
        REVISION,
        2,
        content_first.hash,
        &covered_key,
    );
    let fold_of = |namespace: &str, revision: &str| {
        fold(
            &space,
            SIGNER,
            COVERED,
            namespace,
            revision,
            2,
            anchor.hash,
            &signer_key,
        )
    };
    let content = fold_of("content", REVISION);
    let own = fold_of("own", REVISION);
    let late = fold_of("content", "3");
    let encoded = |log: &[statement::Envelope]| -> Vec<String> {
        log.iter().map(|s| b64(&s.to_json().unwrap())).collect()
    };
    let base = encoded(&log);
    let base_head = head.clone();
    // The base log plus one `device.revoke`, which is revision 3.
    let revoked =
        |log: &mut Vec<statement::Envelope>, head: &mut Option<statement::Head>, payload: Value| {
            push(log, head, "device.revoke", payload);
            let out = encoded(log);
            log.truncate(base_len);
            *head = base_head.clone();
            out
        };
    let listing = |hashes: &[[u8; 32]]| -> Vec<String> {
        let mut sorted = hashes.to_vec();
        sorted.sort();
        sorted.iter().map(|h| values::encode_binary(h)).collect()
    };
    let signer_revoke =
        |listed: &[[u8; 32]]| json!({"deviceId": SIGNER, "cuts": [], "folds": listing(listed)});
    let cut = |namespace: &str, seq: &str, tail: &Sealed| {
        json!({"pageId": PAGE, "epoch": "1", "namespace": namespace, "cut": values::encode_binary(
            &stream_cut::input(&stream_cut::StreamCut {
                stream_id: COVERED, namespace, checkpoint_hash: None, checkpoint_seq: "0",
                tail_head_seq: seq, tail_head_hash: &tail.hash,
            }).unwrap())})
    };
    let covered_revoke = |seq: &str, tail: &Sealed| json!({"deviceId": COVERED, "cuts": [cut("content", seq, tail)]});
    let logs = json!({
        "base": base,
        "signerListed": revoked(&mut log, &mut head, signer_revoke(&[content.hash, own.hash])),
        "signerListedLate": revoked(&mut log, &mut head, signer_revoke(&[late.hash])),
        "signerUnlisted": revoked(&mut log, &mut head, signer_revoke(&[])),
        "signerOtherListed": revoked(&mut log, &mut head, signer_revoke(&[content_first.hash])),
        "coveredAtN": revoked(&mut log, &mut head, covered_revoke("2", &anchor)),
        "coveredBelowN": revoked(&mut log, &mut head, covered_revoke("1", &content_first)),
    });
    // An anchor frame precedes its fold; each is one catch-up object.
    let anchor_frame = |stream: &str, s: &Sealed| frame_of(stream, &namespace_of(s), "anchor", s);
    let fold_frame = |stream: &str, s: &Sealed| frame_of(stream, &namespace_of(s), "checkpoint", s);
    fn namespace_of(s: &Sealed) -> String {
        object::Header::decode(s.envelope.header())
            .unwrap()
            .context
            .namespace
    }
    // What the model alone decides, which the native replay checks: the anchor against the fold
    // (`ok`, `missing` or `refused`) and whether the signer's revoke lists the fold.
    let case = |name: &str,
                log: &str,
                frames: Vec<Value>,
                expect: &str,
                anchor: &str,
                listed: Option<bool>| {
        json!({"name": name, "log": log, "frames": frames, "expect": expect,
            "model": {"anchor": anchor, "listed": listed}})
    };
    let pair = |anchor: &Sealed, fold: &Sealed| {
        vec![anchor_frame(COVERED, anchor), fold_frame(COVERED, fold)]
    };
    // A second update at seq 2 with its own random nonce: its hash is not the head the folds bind.
    let other_anchor = update(
        &space,
        COVERED,
        "content",
        REVISION,
        2,
        content_first.hash,
        &covered_key,
    );
    // An update that names the covered device but that the signer, not the covered device, signed.
    let forged_anchor = update(
        &space,
        COVERED,
        "content",
        REVISION,
        2,
        content_first.hash,
        &signer_key,
    );
    let forged_fold = fold(
        &space,
        SIGNER,
        COVERED,
        "content",
        REVISION,
        2,
        forged_anchor.hash,
        &signer_key,
    );
    let bridge_anchor = update(&space, BRIDGE, "own", REVISION, 2, [1; 32], &bridge_key);
    let bridge_fold = fold(
        &space,
        SIGNER,
        BRIDGE,
        "own",
        REVISION,
        2,
        bridge_anchor.hash,
        &signer_key,
    );
    let no_chain = fold(
        &space,
        UNKNOWN,
        COVERED,
        "content",
        REVISION,
        2,
        anchor.hash,
        &seed(30),
    );
    let wrong_key = fold(
        &space,
        SIGNER,
        COVERED,
        "content",
        REVISION,
        2,
        anchor.hash,
        &seed(30),
    );
    let mut both = pair(&anchor, &content);
    both.push(fold_frame(COVERED, &own));
    let vector = json!({
        "version": 1,
        "producer": "tmt-colab native src/fold_vectors.rs; public test seeds; owner seed 0x41 x32",
        "spaceId": space, "ownerKey": b64(&key.owner_public()), "pageId": PAGE, "epoch": "1",
        "epochSecret": b64(&EPOCH_SECRET),
        "logs": logs,
        "chains": [
            {"deviceId": SIGNER, "chain": chain(SIGNER, 21)},
            {"deviceId": COVERED, "chain": chain(COVERED, 22)},
        ],
        "keys": {SIGNER: b64(signer_key.verifying_key().as_bytes()),
            COVERED: b64(covered_key.verifying_key().as_bytes()),
            BRIDGE: b64(bridge_key.verifying_key().as_bytes())},
        "oldReader": {"header": b64(content.envelope.header()), "fields": 14, "readerFields": 13},
        "cases": [
            case("a fold of 1..2 in both namespaces, with the one anchor", "base", both.clone(), "admit", "ok", None),
            case("a fold in the content namespace only", "base", pair(&anchor, &content), "admit", "ok", None),
            case("a fold without its anchor", "base", vec![fold_frame(COVERED, &content)], "reject", "missing", None),
            case("a fold whose anchor is not the head it binds", "base", pair(&other_anchor, &content), "reject", "refused", None),
            case("an anchor the signer signed instead of the covered device", "base", pair(&forged_anchor, &forged_fold), "reject", "refused", None),
            case("a fold covering a bridge's stream", "base",
                vec![anchor_frame(BRIDGE, &bridge_anchor), fold_frame(BRIDGE, &bridge_fold)], "reject", "ok", None),
            case("a fold signed by a device with no chain", "base", pair(&anchor, &no_chain), "reject", "ok", None),
            case("a fold signed with another key than its signer's", "base", pair(&anchor, &wrong_key), "reject", "ok", None),
            case("a fold that cites a revision past the log's head", "base", pair(&anchor, &late), "reject", "ok", None),
            case("a revoked signer: folds the revoke lists", "signerListed", both.clone(), "admit", "ok", Some(true)),
            case("a revoked signer: an unlisted fold that predates the revoke", "signerUnlisted", pair(&anchor, &content), "reject", "ok", Some(false)),
            case("a revoked signer: a revoke that lists another hash", "signerOtherListed", pair(&anchor, &content), "reject", "ok", Some(false)),
            case("a revoked signer: a listed fold claiming the revoke's own revision", "signerListedLate", pair(&anchor, &late), "reject", "ok", Some(true)),
            case("a revoked covered device: a fold at the cut's tail head", "coveredAtN", pair(&anchor, &content), "admit", "ok", None),
            case("a revoked covered device: a fold in the namespace its revoke did not cut", "coveredAtN",
                vec![anchor_frame(COVERED, &anchor), fold_frame(COVERED, &own)], "reject", "ok", None),
            case("a revoked covered device: a fold past the cut's tail head", "coveredBelowN", pair(&anchor, &content), "reject", "ok", None),
        ],
    });
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/vectors/fold-v1.json");
    fs::write(&path, serde_json::to_string_pretty(&vector).unwrap() + "\n").unwrap();
    let _ = fs::remove_dir_all(root);
}

fn envelope(value: &Value) -> object::Envelope {
    object::Envelope::from_json(&unb64(&value["envelope"])).unwrap()
}
fn key_of(v: &Value, device: &str) -> [u8; 32] {
    unb64(&v["keys"][device]).try_into().unwrap()
}

/// What the model alone decides about each fold frame: the header kind and covered device, the
/// anchor check and whether the signer's revoke lists the fold.
#[test]
fn the_model_decides_each_case_as_the_vector_records() {
    let v: Value = serde_json::from_str(VECTOR).unwrap();
    let owner: [u8; 32] = unb64(&v["ownerKey"]).try_into().unwrap();
    let cases = v["cases"].as_array().unwrap();
    assert!(cases.iter().any(|c| c["expect"] == "admit"));
    assert!(cases.iter().any(|c| c["expect"] == "reject"));
    for case in cases {
        let name = case["name"].as_str().unwrap();
        // The signer's revoke, if the case's log holds one.
        let mut head: Option<statement::Head> = None;
        let mut revoke: Option<payload::DeviceRevoke> = None;
        for raw in v["logs"][case["log"].as_str().unwrap()].as_array().unwrap() {
            let statement = statement::Envelope::from_json(&unb64(raw)).unwrap();
            let verified = statement
                .verify_next(v["spaceId"].as_str().unwrap(), &owner, head.as_ref())
                .unwrap();
            head = Some(verified.head);
            if let payload::Payload::DeviceRevoke(r) = verified.payload
                && r.device_id == SIGNER
            {
                revoke = Some(r);
            }
        }
        let frames = case["frames"].as_array().unwrap();
        let anchors: Vec<&Value> = frames
            .iter()
            .filter(|f| f.get("anchor").is_some())
            .collect();
        let mut folds = 0;
        for frame in frames.iter().filter(|f| !f["checkpoint"].is_null()) {
            folds += 1;
            let stream = frame["streamId"].as_str().unwrap();
            let fold = envelope(&frame["checkpoint"]);
            let header = object::Header::decode(fold.header()).unwrap();
            assert_eq!(header.context.kind, object::FOLD, "{name}");
            assert_eq!(header.covered_device.as_deref(), Some(stream), "{name}");
            let anchor = anchors.iter().find(|a| a["streamId"] == frame["streamId"]);
            let outcome = match anchor {
                None => "missing",
                Some(a) => match object::check_anchor(
                    &header,
                    &envelope(&a["anchor"]),
                    &key_of(&v, stream),
                ) {
                    Ok(()) => "ok",
                    Err(_) => "refused",
                },
            };
            assert_eq!(outcome, case["model"]["anchor"], "{name}");
            if let Some(expected) = case["model"]["listed"].as_bool() {
                let hash: [u8; 32] = unb64(&frame["checkpoint"]["envelopeHash"])
                    .try_into()
                    .unwrap();
                assert_eq!(fold.hash().unwrap(), hash, "{name}");
                assert_eq!(
                    revoke.as_ref().unwrap().lists_fold(&hash),
                    expected,
                    "{name}"
                );
            }
        }
        assert!(folds > 0, "{name}");
    }
}

/// A reader that predates folds decodes thirteen-field headers only, so it refuses a fold by its
/// field count before it reads any trailing field, and an old object still decodes as before.
#[test]
fn a_reader_of_today_refuses_a_fold_header_by_its_field_count() {
    let v: Value = serde_json::from_str(VECTOR).unwrap();
    let header = unb64(&v["oldReader"]["header"]);
    assert_eq!(v["oldReader"]["fields"], 14);
    assert_eq!(v["oldReader"]["readerFields"], 13);
    assert!(framing::fields(&header, 13, 1024).is_err());
    assert_eq!(framing::fields(&header, 14, 1024).unwrap().len(), 14);
    let decoded = object::Header::decode(&header).unwrap();
    assert_eq!(decoded.context.kind, object::FOLD);
    // The same fold, read as the thirteen-field header an old reader expects, is not a header.
    assert!(
        object::open(
            &object::Envelope::from_json(&unb64(
                &v["cases"][0]["frames"][1]["checkpoint"]["envelope"]
            ))
            .unwrap(),
            &decoded.context,
            &unb64(&v["epochSecret"]).try_into().unwrap(),
            &key_of(&v, SIGNER),
        )
        .is_err()
    );
}
