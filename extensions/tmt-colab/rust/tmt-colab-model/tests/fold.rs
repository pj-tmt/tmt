//! The `fold` object (colab-v1, "Planned owner fold (#3983)"): header shape, sealing and opening,
//! the anchor check and the revoke payload's fold list. Who may sign for whom is the caller's
//! policy; these tests cover only what the model itself decides.
use ed25519_dalek::SigningKey;
use serde_json::json;
use tmt_colab_model::{
    framing,
    object::{self, Context, Envelope, Header},
    payload, values,
};

const SPACE: &str = "266znqpwg2rgvjfirvqfceh7h5v7b5gl";
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const SIGNER: &str = "50000000-0000-4000-8000-000000000001";
const COVERED: &str = "50000000-0000-4000-8000-000000000002";
const SECRET: [u8; 32] = [7; 32];

fn key(byte: u8) -> SigningKey {
    SigningKey::from_bytes(&[byte; 32])
}
fn context(kind: &str, author: &str, seq: u64, prev: [u8; 32]) -> Context {
    Context {
        space: SPACE.into(),
        page: PAGE.into(),
        epoch: "1".into(),
        kind: kind.into(),
        namespace: "content".into(),
        author_device: author.into(),
        membership_revision: "2".into(),
        stream_seq: seq.to_string(),
        prev_hash: prev,
    }
}
/// The covered device's update at seq 2 (its anchor) and a fold of 1..2 signed by another device.
fn pair() -> (Envelope, Envelope) {
    let first = object::seal(
        &context("update", COVERED, 1, [0; 32]),
        &SECRET,
        &key(2),
        b"first",
    )
    .unwrap();
    let anchor = object::seal(
        &context("update", COVERED, 2, first.hash().unwrap()),
        &SECRET,
        &key(2),
        b"second",
    )
    .unwrap();
    let fold = object::seal_fold(
        &context("fold", SIGNER, 2, anchor.hash().unwrap()),
        COVERED,
        &SECRET,
        &key(1),
        b"merged",
    )
    .unwrap();
    (anchor, fold)
}
fn public(byte: u8) -> [u8; 32] {
    key(byte).verifying_key().to_bytes()
}

#[test]
fn a_fold_header_has_one_trailing_field_and_other_kinds_keep_thirteen() {
    let (anchor, fold) = pair();
    let header = Header::decode(fold.header()).unwrap();
    assert_eq!(header.covered_device.as_deref(), Some(COVERED));
    assert_eq!(header.context.author_device, SIGNER);
    assert_eq!(framing::fields(fold.header(), 14, 1024).unwrap().len(), 14);
    assert!(
        Header::decode(anchor.header())
            .unwrap()
            .covered_device
            .is_none()
    );
    assert_eq!(
        framing::fields(anchor.header(), 13, 1024).unwrap().len(),
        13
    );
    // A reader that knows only the thirteen-field header refuses a fold by its field count.
    assert!(framing::fields(fold.header(), 13, 1024).is_err());
    // The envelope round-trips through its JSON form.
    assert_eq!(Envelope::from_json(&fold.to_json().unwrap()).unwrap(), fold);
}

#[test]
fn the_kind_and_the_covered_field_go_together() {
    let id = "0".repeat(64);
    let header = |kind: &str, covered: Option<&str>| Header {
        context: context(kind, SIGNER, 1, [0; 32]),
        object_id: id.clone(),
        covered_device: covered.map(Into::into),
    };
    assert!(header("fold", Some(COVERED)).encode().is_ok());
    // A fold may cover its signer's own stream.
    assert!(header("fold", Some(SIGNER)).encode().is_ok());
    assert!(header("fold", None).encode().is_err());
    assert!(header("checkpoint", Some(COVERED)).encode().is_err());
    assert!(header("update", Some(COVERED)).encode().is_err());
    assert!(header("fold", Some("not-a-device")).encode().is_err());
    // A fold covers a prefix: sequence zero is not one.
    let mut zero = header("fold", Some(COVERED));
    zero.context.stream_seq = "0".into();
    assert!(zero.encode().is_err());
}

#[test]
fn a_fold_opens_only_with_its_covered_device_and_only_through_open_fold() {
    let (_, fold) = pair();
    let expected = Header::decode(fold.header()).unwrap().context;
    assert_eq!(
        object::open_fold(&fold, &expected, COVERED, &SECRET, &public(1)).unwrap(),
        b"merged"
    );
    // The signature is the signer's, and the secret and the covered device must match.
    assert!(object::open_fold(&fold, &expected, COVERED, &SECRET, &public(2)).is_err());
    assert!(object::open_fold(&fold, &expected, SIGNER, &SECRET, &public(1)).is_err());
    assert!(object::open_fold(&fold, &expected, COVERED, &[8; 32], &public(1)).is_err());
    // `open` never opens a fold, so a caller cannot forget the covered device.
    assert!(object::open(&fold, &expected, &SECRET, &public(1)).is_err());
    // And `open_fold` opens nothing else.
    let (anchor, _) = pair();
    let update = Header::decode(anchor.header()).unwrap().context;
    assert!(object::open_fold(&anchor, &update, COVERED, &SECRET, &public(2)).is_err());
}

#[test]
fn the_anchor_must_be_the_covered_devices_own_update_at_the_folds_head() {
    let (anchor, fold) = pair();
    let header = Header::decode(fold.header()).unwrap();
    object::check_anchor(&header, &anchor, &public(2)).unwrap();
    // Signed by someone else: the fold would name a head the covered device never wrote.
    assert!(object::check_anchor(&header, &anchor, &public(1)).is_err());
    // Not the head the fold binds.
    let other = object::seal(
        &context("update", COVERED, 2, [9; 32]),
        &SECRET,
        &key(2),
        b"second",
    )
    .unwrap();
    assert!(object::check_anchor(&header, &other, &public(2)).is_err());
    // A different sequence, author, kind or page.
    let sealed = |c: Context, signer: u8| object::seal(&c, &SECRET, &key(signer), b"x").unwrap();
    let mut wrong_author = context("update", SIGNER, 2, [0; 32]);
    wrong_author.prev_hash = header.context.prev_hash;
    for envelope in [
        sealed(wrong_author, 1),
        sealed(context("checkpoint", COVERED, 2, [0; 32]), 2),
    ] {
        assert!(object::check_anchor(&header, &envelope, &public(2)).is_err());
    }
    // Only a fold has an anchor.
    let plain = Header::decode(anchor.header()).unwrap();
    assert!(object::check_anchor(&plain, &anchor, &public(2)).is_err());
}

fn revoke(folds: serde_json::Value) -> Result<payload::Payload, tmt_colab_model::Invalid> {
    let body = json!({"deviceId": SIGNER, "cuts": [], "folds": folds});
    payload::decode("device.revoke", &serde_json::to_vec(&body).unwrap())
}
fn hash(byte: u8) -> String {
    values::encode_binary(&[byte; 32])
}

#[test]
fn a_revoke_lists_retained_folds_as_a_sorted_unique_hash_list() {
    let payload::Payload::DeviceRevoke(listed) = revoke(json!([hash(1), hash(2)])).unwrap() else {
        panic!("a device.revoke");
    };
    assert!(listed.lists_fold(&[1; 32]) && listed.lists_fold(&[2; 32]));
    assert!(!listed.lists_fold(&[3; 32]));
    // The list is optional: an older statement means none.
    let bare = json!({"deviceId": SIGNER, "cuts": []});
    let payload::Payload::DeviceRevoke(none) =
        payload::decode("device.revoke", &serde_json::to_vec(&bare).unwrap()).unwrap()
    else {
        panic!("a device.revoke");
    };
    assert!(!none.lists_fold(&[1; 32]));
    // Sorted and unique, 32 bytes each, at most 512.
    assert!(revoke(json!([hash(2), hash(1)])).is_err());
    assert!(revoke(json!([hash(1), hash(1)])).is_err());
    assert!(revoke(json!([values::encode_binary(&[1; 31])])).is_err());
    assert!(revoke(json!(["!"])).is_err());
    let many: Vec<String> = (0..513u16)
        .map(|n| values::encode_binary(&[&n.to_be_bytes()[..], &[0; 30]].concat()))
        .collect();
    assert!(revoke(json!(many[..512])).is_ok());
    assert!(revoke(json!(many)).is_err());
}

#[test]
fn only_a_device_revoke_carries_folds() {
    let body = json!({"memberId": SIGNER, "cuts": [], "folds": []});
    assert!(payload::decode("member.remove", &serde_json::to_vec(&body).unwrap()).is_err());
}
