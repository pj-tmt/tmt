//! Native and browser admission of a link device's own-stream envelopes, from one frozen vector.
//! `link-author-v1.json` holds the owner-signed membership logs, the link-issued device chains
//! and the sealed envelopes; this test rebuilds each case in a real store and asserts that native
//! admits exactly the cases marked `admit`. The browser test (`link-author.test.ts`) replays the
//! same bytes through `Admission` and `Objects`.
//!
//! Regenerate after a deliberate contract change (envelopes use random nonces, so every
//! byte changes): `cargo test --test link_author_vectors -- --ignored regenerate`.
mod support;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};
use tmt_colab::{
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page,
    store::{
        Envelope, Namespace, Store, StreamScope,
        owner::{Device, Mutation},
    },
};
use tmt_colab_model::{certificate, crypto, framing, link, object, statement, stream_cut, values};
use yrs::{Doc, ReadTxn, StateVector, Transact};

const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
const VECTOR: &str = include_str!("../../../contracts/vectors/link-author-v1.json");
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const OTHER_PAGE: &str = "10000000-0000-4000-8000-000000000002";
const EDITOR_LINK: &str = "50000000-0000-4000-8000-000000000001";
const COMMENTER_LINK: &str = "50000000-0000-4000-8000-000000000002";
const VIEWER_LINK: &str = "50000000-0000-4000-8000-000000000003";
const OTHER_LINK: &str = "50000000-0000-4000-8000-000000000004";
const OWNER_SEED: [u8; 32] = [0x41; 32];
const EPOCH_SECRET: [u8; 32] = [11; 32];

fn seed(byte: u8) -> SigningKey {
    SigningKey::from_bytes(&[byte; 32])
}
fn device_id(n: u8) -> String {
    format!("70000000-0000-4000-8000-{n:012}")
}
fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}
fn unb64(value: &Value) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(value.as_str().unwrap()).unwrap()
}
/// A layout whose owner key is the public test seed, so every statement signature is reproducible.
fn layout_with_owner() -> (PathBuf, Layout, Keyring) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "lav-{}-{}",
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
/// The link's keys: the seed is public for the vector. The signer is derived exactly as the model
/// derives it, which the generator asserts against the model's own public key.
struct LinkKeys {
    id: &'static str,
    signer: SigningKey,
    keys: link::Keys,
}
fn link_keys(space: &str, id: &'static str, byte: u8) -> LinkKeys {
    let seed = [byte; 32];
    let signer = SigningKey::from_bytes(&crypto::derive_key(
        &seed,
        &[],
        &framing::frame(&[
            b"tmt-colab-link-signing-seed-v1",
            space.as_bytes(),
            id.as_bytes(),
        ])
        .unwrap(),
    ));
    let keys = link::Keys::derive(&seed, space, id).unwrap();
    assert_eq!(signer.verifying_key().to_bytes(), keys.signing_public());
    LinkKeys { id, signer, keys }
}
/// A device chain: the certificate and the signature over it are the caller's, so a chain can
/// be forged, can cite another statement, or can claim the other issuer kind.
#[allow(clippy::too_many_arguments)]
fn chain(
    space: &str,
    kind: &str,
    issuer: &str,
    device: &str,
    device_key: &SigningKey,
    revision: &str,
    statement_hash: &[u8; 32],
    signer: &SigningKey,
) -> String {
    let public = device_key.verifying_key().to_bytes();
    let input = certificate::input(&certificate::Certificate {
        space,
        issuer_kind: kind,
        issuer_id: issuer,
        device_id: device,
        signing_key: &public,
        encryption_key: &[7; 32],
        membership_revision: revision,
        issued_at: 0,
        expires_at: 9_007_199_254_740_991,
    })
    .unwrap();
    b64(&serde_json::to_vec(&json!({
        "version": 1,
        "issuerStatement": b64(statement_hash),
        "deviceCertificate": b64(&input),
        "issuerSignature": b64(&signer.sign(&input).to_bytes()),
    }))
    .unwrap())
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
    let editor = link_keys(&space, EDITOR_LINK, 0x51);
    let commenter = link_keys(&space, COMMENTER_LINK, 0x52);
    let viewer = link_keys(&space, VIEWER_LINK, 0x53);
    let other = link_keys(&space, OTHER_LINK, 0x54);
    push(
        &mut log,
        &mut head,
        "member.add",
        json!({"memberId": owner_member.id, "role": "editor",
            "signKey": b64(&owner_member.signing_key), "encKey": b64(&owner_member.encryption_key), "pages": []}),
    );
    push(
        &mut log,
        &mut head,
        "page.share",
        json!({"pageId": PAGE, "mode": "link", "epoch": "1"}),
    );
    for (link, role, page) in [
        (&editor, "editor", PAGE),
        (&commenter, "commenter", PAGE),
        (&viewer, "viewer", PAGE),
        // The page this link covers is not the one under test.
        (&other, "editor", OTHER_PAGE),
    ] {
        push(
            &mut log,
            &mut head,
            "link.add",
            json!({"linkId": link.id, "role": role,
                "linkSignKey": b64(&link.keys.signing_public()),
                "linkEncKey": b64(&link.keys.recipient().public_key()), "pages": [page]}),
        );
    }
    let base_len = log.len();
    assert_eq!(base_len, 6);
    let hash_at = |revision: usize| -> [u8; 32] { log[revision - 1].hash().unwrap() };
    // Device chains. Revision 3..6 are the four `link.add` statements.
    let (d_editor, d_commenter, d_viewer, d_other) = (
        (device_id(1), seed(0x61)),
        (device_id(2), seed(0x62)),
        (device_id(3), seed(0x63)),
        (device_id(4), seed(0x64)),
    );
    let (d_forged, d_wrong_hash, d_below, d_member, d_reverse) = (
        (device_id(5), seed(0x65)),
        (device_id(6), seed(0x66)),
        (device_id(7), seed(0x67)),
        (device_id(8), seed(0x68)),
        (device_id(9), seed(0x69)),
    );
    let mut chains = serde_json::Map::new();
    let mut add = |name: &str, device: &(String, SigningKey), value: String| {
        chains.insert(name.into(), json!({"deviceId": device.0, "chain": value}));
    };
    let issue = |link: &LinkKeys, device: &(String, SigningKey), revision: usize| {
        chain(
            &space,
            "link",
            link.id,
            &device.0,
            &device.1,
            &revision.to_string(),
            &hash_at(revision),
            &link.signer,
        )
    };
    add("editor", &d_editor, issue(&editor, &d_editor, 3));
    add(
        "commenter",
        &d_commenter,
        issue(&commenter, &d_commenter, 4),
    );
    add("viewer", &d_viewer, issue(&viewer, &d_viewer, 5));
    add("other", &d_other, issue(&other, &d_other, 6));
    // The right statement and revision, signed by a key that is not the link's.
    add(
        "forged",
        &d_forged,
        chain(
            &space,
            "link",
            EDITOR_LINK,
            &d_forged.0,
            &d_forged.1,
            "3",
            &hash_at(3),
            &seed(0x77),
        ),
    );
    // The link's own signature, over a chain that cites the `page.share` statement instead.
    add(
        "wrongHash",
        &d_wrong_hash,
        chain(
            &space,
            "link",
            EDITOR_LINK,
            &d_wrong_hash.0,
            &d_wrong_hash.1,
            "3",
            &hash_at(2),
            &editor.signer,
        ),
    );
    // The right statement and signature, at a revision before the link existed.
    add(
        "belowAdd",
        &d_below,
        chain(
            &space,
            "link",
            EDITOR_LINK,
            &d_below.0,
            &d_below.1,
            "2",
            &hash_at(3),
            &editor.signer,
        ),
    );
    // An owner-member chain that says a link issued it...
    add(
        "memberClaimingLink",
        &d_member,
        chain(
            &space,
            "link",
            &owner_member.id,
            &d_member.0,
            &d_member.1,
            "1",
            &hash_at(1),
            &seed(0x41),
        ),
    );
    // ...and a link chain that says the owner member issued it.
    add(
        "linkClaimingMember",
        &d_reverse,
        chain(
            &space,
            "member",
            EDITOR_LINK,
            &d_reverse.0,
            &d_reverse.1,
            "3",
            &hash_at(3),
            &editor.signer,
        ),
    );

    // The removal of the editor link ends its device's stream at the signed cut. `late` claims
    // the removal's own revision, 7.
    let update = own_update();
    let first = seal(
        &space,
        &d_editor.0,
        "own",
        "3",
        1,
        [0; 32],
        &d_editor.1,
        &update,
    );
    let second = seal(
        &space,
        &d_editor.0,
        "own",
        "3",
        2,
        first.hash,
        &d_editor.1,
        &update,
    );
    let late = seal(
        &space,
        &d_editor.0,
        "own",
        "7",
        2,
        first.hash,
        &d_editor.1,
        &update,
    );
    let cut = |tail: &Sealed, seq: &str| {
        values::encode_binary(
            &stream_cut::input(&stream_cut::StreamCut {
                stream_id: &d_editor.0,
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
    let remove = |cut: String| json!({"linkId": EDITOR_LINK, "cuts": [{"pageId": PAGE, "epoch": "1", "namespace": "own", "cut": cut}]});
    push(&mut log, &mut head, "link.remove", remove(cut(&first, "1")));
    let removed = encoded(&log);
    log.truncate(base_len);
    head = base_head.clone();
    push(&mut log, &mut head, "link.remove", remove(cut(&late, "2")));
    let removed_late = encoded(&log);
    log.truncate(base_len);
    head = base_head;
    push(
        &mut log,
        &mut head,
        "page.share",
        json!({"pageId": PAGE, "mode": "private", "epoch": "1"}),
    );
    let unshared = encoded(&log);
    assert_eq!(
        (base.len(), removed.len(), unshared.len()),
        (base_len, base_len + 1, base_len + 1)
    );

    let case = |name: &str, log: &str, chains: Vec<&str>, envelopes: Vec<&Sealed>, expect: &str| {
        json!({"name": name, "log": log, "chains": chains,
            "envelopes": envelopes.iter().map(|s| s.json.clone()).collect::<Vec<_>>(), "expect": expect})
    };
    let at = |device: &(String, SigningKey), namespace: &str, revision: &str| {
        seal(
            &space, &device.0, namespace, revision, 1, [0; 32], &device.1, &update,
        )
    };
    let editor_later = at(&d_editor, "own", "6");
    let editor_below = at(&d_editor, "own", "2");
    let editor_unshared = at(&d_editor, "own", "7");
    let editor_shared = at(&d_editor, "own", "6");
    let commenter_own = at(&d_commenter, "own", "4");
    let commenter_content = at(&d_commenter, "content", "4");
    let viewer_own = at(&d_viewer, "own", "5");
    let other_own = at(&d_other, "own", "6");
    let forged_own = at(&d_forged, "own", "3");
    let wrong_hash_own = at(&d_wrong_hash, "own", "3");
    let below_own = at(&d_below, "own", "3");
    let member_own = at(&d_member, "own", "3");
    let reverse_own = at(&d_reverse, "own", "3");
    let vector = json!({
        "version": 1,
        "producer": "tmt-colab native tests/link_author_vectors.rs; public test seeds; owner seed 0x41 x32",
        "spaceId": space, "ownerKey": b64(&key.owner_public()), "pageId": PAGE, "epoch": "1",
        "epochSecret": b64(&EPOCH_SECRET),
        "ownerMemberId": owner_member.id,
        "logs": {"base": base, "removed": removed, "removedLate": removed_late, "unshared": unshared},
        "chains": chains,
        "cases": [
            case("editor link: own envelope at its link.add revision", "base", vec!["editor"], vec![&first], "admit"),
            case("editor link: own envelope at a later revision", "base", vec!["editor"], vec![&editor_later], "admit"),
            case("editor link: stream of two own envelopes", "base", vec!["editor"], vec![&first, &second], "admit"),
            case("commenter link: own envelope", "base", vec!["commenter"], vec![&commenter_own], "admit"),
            case("viewer link is never an author", "base", vec!["viewer"], vec![&viewer_own], "reject"),
            case("commenter link: content envelope", "base", vec!["commenter"], vec![&commenter_content], "reject"),
            case("link that does not cover the page", "base", vec!["other"], vec![&other_own], "reject"),
            case("forged issuer signature", "base", vec!["forged"], vec![&forged_own], "reject"),
            case("chain citing another statement", "base", vec!["wrongHash"], vec![&wrong_hash_own], "reject"),
            case("chain revision below the link.add revision", "base", vec!["belowAdd"], vec![&below_own], "reject"),
            case("envelope below its link.add revision", "base", vec!["editor"], vec![&editor_below], "reject"),
            case("member chain claiming link issuance", "base", vec!["memberClaimingLink"], vec![&member_own], "reject"),
            case("link chain claiming member issuance", "base", vec!["linkClaimingMember"], vec![&reverse_own], "reject"),
            case("page still link-shared at the envelope's revision", "unshared", vec!["editor"], vec![&editor_shared], "admit"),
            case("page no longer link-shared at the envelope's revision", "unshared", vec!["editor"], vec![&editor_unshared], "reject"),
            case("removed link: envelope inside the signed cut", "removed", vec!["editor"], vec![&first], "admit"),
            case("removed link: envelope beyond the signed cut", "removed", vec!["editor"], vec![&first, &second], "reject"),
            case(
                "removed link: envelope claiming the removal's own revision",
                "removedLate",
                vec!["editor"],
                vec![&first, &late],
                "reject",
            ),
        ],
    });
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/vectors/link-author-v1.json");
    fs::write(&path, serde_json::to_string_pretty(&vector).unwrap() + "\n").unwrap();
    let _ = fs::remove_dir_all(root);
}

fn read_case(v: &Value, case: &Value) -> tmt_colab::Result<page::Page> {
    let (root, layout, key) = layout_with_owner();
    assert_eq!(key.space_id, v["spaceId"].as_str().unwrap());
    assert_eq!(b64(&key.owner_public()), v["ownerKey"].as_str().unwrap());
    assert_eq!(key.management_member().unwrap().id, v["ownerMemberId"]);
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
                for name in case["chains"].as_array().unwrap() {
                    tx.put_device(&Device {
                        chain: unb64(&v["chains"][name.as_str().unwrap()]["chain"]),
                        revoked: false,
                    })?;
                }
                tx.put_epoch_secret(PAGE, 1, &EPOCH_SECRET)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    for e in case["envelopes"].as_array().unwrap() {
        let hash: [u8; 32] = unb64(&e["hash"]).try_into().unwrap();
        let previous: [u8; 32] = unb64(&e["previous"]).try_into().unwrap();
        store
            .append(&Envelope {
                scope: StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream: e["stream"].as_str().unwrap(),
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
fn native_admits_exactly_the_link_authors_the_vector_marks_admit() {
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
