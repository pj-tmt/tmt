//! Native and browser admission of a bridge's own-namespace envelopes, from one frozen vector.
//! `bridge-own-v1.json` holds the owner-signed membership logs and sealed envelopes; this
//! test rebuilds each case in a real store and asserts that native admits exactly the cases
//! marked `admit`. The browser test (`bridge-own.test.ts`) replays the same bytes.
//!
//! Regenerate after a deliberate contract change (envelopes use random nonces, so every
//! byte changes): `cargo test --test bridge_vectors -- --ignored regenerate`.
mod support;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};
use tmt_colab::{
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page,
    store::{Envelope, Namespace, Store, StreamScope, owner::Mutation},
};
use tmt_colab_model::{object, statement, stream_cut, values, wrap};
use yrs::{Doc, ReadTxn, StateVector, Transact};

const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
const VECTOR: &str = include_str!("../../../contracts/vectors/bridge-own-v1.json");
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const OTHER_PAGE: &str = "10000000-0000-4000-8000-000000000002";
const BRIDGE: &str = "60000000-0000-4000-8000-000000000001";
const OTHER_BRIDGE: &str = "60000000-0000-4000-8000-000000000002";
const UNKNOWN: &str = "60000000-0000-4000-8000-000000000003";
const OWNER_SEED: [u8; 32] = [0x41; 32];
const EPOCH_SECRET: [u8; 32] = [11; 32];

fn seed(byte: u8) -> SigningKey {
    SigningKey::from_bytes(&[byte; 32])
}
fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}
fn unb64(value: &Value) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(value.as_str().unwrap()).unwrap()
}
fn enc_key(byte: u8) -> [u8; 32] {
    wrap::RecipientKey::from_seed(&[byte; 32])
        .unwrap()
        .public_key()
}
/// A layout whose owner key is the public test seed, so every statement signature is reproducible.
fn layout_with_owner() -> (PathBuf, Layout, Keyring) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "bv-{}-{}",
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
    let key = Keyring::open(&layout).unwrap();
    (root, layout, key)
}
fn own_update() -> Vec<u8> {
    let doc = Doc::new();
    for root in ["threads", "messages", "intents", "replies"] {
        doc.get_or_insert_map(root);
    }
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
}
struct Sealed {
    json: Value,
    hash: [u8; 32],
}
#[allow(clippy::too_many_arguments)]
fn seal(
    space: &str,
    stream: &str,
    namespace: &str,
    revision: &str,
    seq: u64,
    previous: [u8; 32],
    signer: &SigningKey,
    update: &[u8],
) -> Sealed {
    let envelope = object::seal(
        &object::Context {
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
        update,
    )
    .unwrap();
    let hash = envelope.hash().unwrap();
    Sealed {
        json: json!({
            "stream": stream, "namespace": namespace, "seq": seq.to_string(),
            "previous": b64(&previous), "hash": b64(&hash), "envelope": b64(&envelope.to_json().unwrap()),
        }),
        hash,
    }
}

#[test]
#[ignore = "regenerates the frozen vector"]
fn regenerate() {
    let (root, _layout, key) = layout_with_owner();
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
    let bridge_add = |id: &str, byte: u8, page: &str| {
        json!({"machineId": id, "machineSignKey": b64(seed(byte).verifying_key().as_bytes()),
            "encKey": b64(&enc_key(byte + 1)), "pages": [page]})
    };
    push(
        &mut log,
        &mut head,
        "bridge.add",
        bridge_add(BRIDGE, 13, PAGE),
    );
    push(
        &mut log,
        &mut head,
        "bridge.add",
        bridge_add(OTHER_BRIDGE, 16, OTHER_PAGE),
    );
    let base_len = log.len();
    let update = own_update();
    let bridge = seed(13);
    let wrong = seed(15);
    let first = seal(&space, BRIDGE, "own", "2", 1, [0; 32], &bridge, &update);
    let second = seal(&space, BRIDGE, "own", "2", 2, first.hash, &bridge, &update);
    // A bridge's signed cut ends its stream at `tail`; `late` claims the revoke's own revision.
    let late = seal(&space, BRIDGE, "own", "4", 2, first.hash, &bridge, &update);
    let cut = |tail: &Sealed, seq: &str| {
        values::encode_binary(
            &stream_cut::input(&stream_cut::StreamCut {
                stream_id: BRIDGE,
                namespace: "own",
                checkpoint_hash: None,
                checkpoint_seq: "0",
                tail_head_seq: seq,
                tail_head_hash: &tail.hash,
            })
            .unwrap(),
        )
    };
    let encoded = |log: &[statement::Envelope]| -> Vec<String> {
        log.iter().map(|s| b64(&s.to_json().unwrap())).collect()
    };
    let base = encoded(&log);
    let base_head = head.clone();
    let revoke = |cut: String| json!({"deviceId": BRIDGE, "cuts": [{"pageId": PAGE, "epoch": "1", "namespace": "own", "cut": cut}]});
    push(
        &mut log,
        &mut head,
        "device.revoke",
        revoke(cut(&first, "1")),
    );
    let revoked = encoded(&log);
    log.truncate(base_len);
    head = base_head;
    push(
        &mut log,
        &mut head,
        "device.revoke",
        revoke(cut(&late, "2")),
    );
    let revoked_late = encoded(&log);
    assert_eq!((base.len(), revoked.len()), (base_len, base_len + 1));
    let case = |name: &str, log: &str, envelopes: Vec<&Sealed>, expect: &str| json!({"name": name, "log": log, "envelopes": envelopes.iter().map(|s| s.json.clone()).collect::<Vec<_>>(), "expect": expect});
    let at = |stream: &str, namespace: &str, revision: &str, signer: &SigningKey| {
        seal(
            &space, stream, namespace, revision, 1, [0; 32], signer, &update,
        )
    };
    let later = at(BRIDGE, "own", "3", &bridge);
    let below = at(BRIDGE, "own", "1", &bridge);
    let in_content = at(BRIDGE, "content", "3", &bridge);
    let not_in_pages = at(OTHER_BRIDGE, "own", "3", &seed(16));
    let wrong_key = at(BRIDGE, "own", "3", &wrong);
    let unknown = at(UNKNOWN, "own", "3", &seed(18));
    let vector = json!({
        "version": 1,
        "producer": "tmt-colab native tests/bridge_vectors.rs; public test seeds; owner seed 0x41 x32",
        "spaceId": space, "ownerKey": b64(&key.owner_public()), "pageId": PAGE, "epoch": "1",
        "epochSecret": b64(&EPOCH_SECRET),
        "logs": {"base": base, "revoked": revoked, "revokedLate": revoked_late},
        "bridge": {"id": BRIDGE, "signKey": b64(bridge.verifying_key().as_bytes())},
        "cases": [
            case("bridge own envelope at its bridge.add revision", "base", vec![&first], "admit"),
            case("bridge own envelope at a later revision", "base", vec![&later], "admit"),
            case("bridge stream of two own envelopes", "base", vec![&first, &second], "admit"),
            case("bridge envelope below its bridge.add revision", "base", vec![&below], "reject"),
            case("bridge envelope in the content namespace", "base", vec![&in_content], "reject"),
            case("bridge granted another page only", "base", vec![&not_in_pages], "reject"),
            case("bridge envelope signed by another key", "base", vec![&wrong_key], "reject"),
            case("author with neither device chain nor bridge.add", "base", vec![&unknown], "reject"),
            case("revoked bridge: envelope inside the signed cut", "revoked", vec![&first], "admit"),
            case("revoked bridge: envelope beyond the signed cut", "revoked", vec![&first, &second], "reject"),
            case(
                "revoked bridge: envelope claiming the revoke's own revision",
                "revokedLate",
                vec![&first, &late],
                "reject",
            ),
        ],
    });
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/vectors/bridge-own-v1.json");
    fs::write(&path, serde_json::to_string_pretty(&vector).unwrap() + "\n").unwrap();
    let _ = fs::remove_dir_all(root);
}

fn read_case(v: &Value, case: &Value) -> tmt_colab::Result<page::Page> {
    let (root, layout, key) = layout_with_owner();
    assert_eq!(key.space_id, v["spaceId"].as_str().unwrap());
    assert_eq!(b64(&key.owner_public()), v["ownerKey"].as_str().unwrap());
    let mut store = Store::open(&layout).unwrap();
    store.create_page(PAGE).unwrap();
    let log = v["logs"][case["log"].as_str().unwrap()].as_array().unwrap();
    store
        .owner_transaction(
            &key.space_id,
            &key.owner_public(),
            Mutation {
                operation_id: "90000000-0000-4000-8000-000000000001",
                digest: [1; 32],
                expected_revision: 0,
            },
            |tx| {
                for raw in log {
                    tx.append_statement(&statement::Envelope::from_json(&unb64(raw))?)?;
                }
                tx.put_epoch_secret(PAGE, 1, &EPOCH_SECRET)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    let envelopes = case["envelopes"].as_array().unwrap();
    for e in envelopes {
        let hash: [u8; 32] = unb64(&e["hash"]).try_into().unwrap();
        let previous: [u8; 32] = unb64(&e["previous"]).try_into().unwrap();
        let stream = e["stream"].as_str().unwrap();
        store
            .append(&Envelope {
                scope: StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream,
                },
                namespace: if e["namespace"] == "own" {
                    Namespace::Own
                } else {
                    Namespace::Content
                },
                seq: e["seq"].as_str().unwrap().parse().unwrap(),
                hash,
                previous,
                bytes: &unb64(&e["envelope"]),
            })
            .unwrap();
    }
    let mut decoder = Decoder::with_config(support::decoder_config(BINARY.into())).unwrap();
    let read = page::read(&store, &key, PAGE, &mut decoder);
    let _ = fs::remove_dir_all(root);
    read
}

#[test]
fn native_admits_exactly_the_bridge_own_envelopes_the_vector_marks_admit() {
    let v: Value = serde_json::from_str(VECTOR).unwrap();
    assert_eq!(v["pageId"], PAGE);
    let cases = v["cases"].as_array().unwrap();
    assert!(cases.iter().any(|c| c["expect"] == "admit"));
    assert!(cases.iter().any(|c| c["expect"] == "reject"));
    for case in cases {
        let read = read_case(&v, case);
        match case["expect"].as_str().unwrap() {
            "admit" => {
                read.unwrap_or_else(|e| panic!("{}: native rejected: {e:?}", case["name"]));
            }
            // Invalid authority, not a decoder or storage failure.
            _ => assert_eq!(
                format!("{:?}", read.err()),
                "Some(Invalid)",
                "{}",
                case["name"]
            ),
        }
    }
}
