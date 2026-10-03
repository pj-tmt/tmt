//! Explicit test-only state producer for native CLI-to-Chromium acceptance.
//! Seeds 9/10/17 and epoch key 8 are public fixture data, never product identities.
mod support;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use tmt_colab::{
    keyring::{Keyring, Layout},
    registration::Registration,
    store::{Envelope, Namespace, Store, StreamScope, owner::Mutation},
};
use tmt_colab_model::{framing, object, values, wrap};
use yrs::{Doc, Map, ReadTxn, StateVector, Text, Transact};
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
#[test]
#[ignore = "Invoked explicitly by the native browser acceptance fixture"]
fn seed_native_page() {
    let root = std::path::PathBuf::from(
        std::env::var_os("COLAB_PAGE_FIXTURE_ROOT").expect("fixture root"),
    );
    assert!(root.is_absolute() && root.is_dir());
    let layout = Layout::open(&root).unwrap();
    let key = Keyring::open(&layout).unwrap();
    let store = Store::open(&layout).unwrap();
    store.create_page(PAGE).unwrap();
    let remote = SigningKey::from_bytes(&[9; 32]);
    let sign = SigningKey::from_bytes(&[10; 32]).verifying_key().to_bytes();
    let enc = wrap::RecipientKey::from_seed(&[17; 32])
        .unwrap()
        .public_key();
    let now: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap();
    let proof = |purpose: &str, public: &[u8; 32]| {
        let input = framing::frame(&[
            b"tmt-ext-cert-v1",
            b"colab",
            purpose.as_bytes(),
            public,
            now.to_string().as_bytes(),
        ])
        .unwrap();
        json!({"publicKey":values::encode_binary(public),"issuedAtMs":now,
            "signature":values::encode_binary(&remote.sign(&input).to_bytes())})
    };
    let context = json!({"deviceId":DEVICE,"kind":"browser","origin":"http://127.0.0.1:4179",
        "name":"Native CLI fixture","publicKey":values::encode_binary(remote.verifying_key().as_bytes()),
        "owner":true,"grantRevision":1}).to_string();
    let body = serde_json::to_vec(
        &json!({"deviceId":DEVICE,"sign":proof("sign",&sign),"enc":proof("enc",&enc)}),
    )
    .unwrap();
    let mut registration = Registration::with_decoder_config(
        store,
        key,
        support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
    )
    .unwrap();
    registration.register(Some(&context), &body, now).unwrap();
    registration.close().unwrap();
    let key = Keyring::read(&layout).unwrap();
    let mut store = Store::write_existing(&layout).unwrap();
    store
        .owner_transaction(
            &key.space_id,
            &key.owner_public(),
            Mutation {
                operation_id: "40000000-0000-4000-8000-000000000001",
                digest: [1; 32],
                expected_revision: 1,
            },
            |tx| {
                tx.append_statement(&key.sign_statement(
                    tx.head(),
                    "page.share",
                    &serde_json::to_vec(&json!({"pageId":PAGE,"mode":"private","epoch":"1"}))?,
                )?)?;
                tx.put_epoch_secret(PAGE, 1, &[8; 32])?;
                tx.put_wrap(&key.seal_wrap(
                    &wrap::Header {
                        space: key.space_id.clone(),
                        page: PAGE.into(),
                        epoch: "1".into(),
                        recipient_kind: "device".into(),
                        recipient_id: DEVICE.into(),
                        recipient_key: enc,
                        signer_key: key.owner_public(),
                        membership_revision: "2".into(),
                    },
                    &[8; 32],
                )?)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    // Seed the browser's own stream. The CLI device must first appear while the
    // tab is already subscribed, so live chain admission is required to render it.
    let doc = Doc::new();
    doc.get_or_insert_text("html")
        .insert(&mut doc.transact_mut(), 0, "<h1>Before CLI</h1>");
    doc.get_or_insert_map("meta")
        .insert(&mut doc.transact_mut(), "title", "CLI page");
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let envelope = object::seal(
        &object::Context {
            space: key.space_id.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: "update".into(),
            namespace: "content".into(),
            author_device: DEVICE.into(),
            membership_revision: "2".into(),
            stream_seq: "1".into(),
            prev_hash: [0; 32],
        },
        &[8; 32],
        &SigningKey::from_bytes(&[10; 32]),
        &update,
    )
    .unwrap();
    store
        .append(&Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: DEVICE,
            },
            namespace: Namespace::Content,
            seq: 1,
            hash: envelope.hash().unwrap(),
            previous: [0; 32],
            bytes: &envelope.to_json().unwrap(),
        })
        .unwrap();
    store.close().unwrap();
    std::fs::write(
        root.join("browser-fixture.json"),
        serde_json::to_vec(&json!({
            "pageId":PAGE,"deviceId":DEVICE,"context":context,
            "signPublic":values::encode_binary(&sign),"encPublic":values::encode_binary(&enc),
        }))
        .unwrap(),
    )
    .unwrap();
}
