//! A page whose update tail grew far past the old 256 KiB / 200-object read caps (browser saves
//! and many small appends) must read back byte-exact through every root-local reader, and a page
//! that cannot be read must not hide the others (#1627).
mod support;

use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    decoder::Decoder,
    export::Bundle,
    keyring::{Keyring, Layout},
    store::{
        Envelope, Namespace, Store, StreamScope,
        owner::{Device, Mutation, Recipient},
    },
};
use tmt_colab_model::{certificate, object, values, wrap};
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact};

const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const OTHER: &str = "10000000-0000-4000-8000-000000000002";
const HUGE: &str = "10000000-0000-4000-8000-000000000003";
const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");

fn signer(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}
struct Fixture {
    root: PathBuf,
    key: Keyring,
    store: Store,
    /// Next stream position and previous hash, per page.
    next: std::collections::BTreeMap<&'static str, (u64, [u8; 32])>,
}
impl Fixture {
    fn new(pages: &[&'static str]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-large-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let mut store = Store::open(&layout).unwrap();
        for page in pages {
            store.create_page(page).unwrap();
        }
        store.owner_transaction(&key.space_id, &key.owner_public(), Mutation {
            operation_id: "40000000-0000-4000-8000-000000000001", digest: [7;32], expected_revision: 0,
        }, |tx| {
            let owner = key.management_member()?;
            let recipient = Recipient { kind: "member".into(), id: owner.id.clone(), role: Some("editor".into()), signing_key: owner.signing_key, encryption_key: owner.encryption_key, pages: vec![], revoked: false };
            let issuer = key.sign_statement(tx.head(), "member.add", &serde_json::to_vec(&json!({"memberId": owner.id, "role":"editor", "signKey":values::encode_binary(&owner.signing_key), "encKey":values::encode_binary(&owner.encryption_key), "pages":[]}))?)?;
            tx.append_statement(&issuer)?;
            tx.put_recipient(&recipient)?;
            let member = "20000000-0000-4000-8000-000000000001";
            let recipient = Recipient { kind: "member".into(), id: member.into(), role: Some("editor".into()), signing_key: signer(7).verifying_key().to_bytes(), encryption_key: wrap::RecipientKey::from_seed(&[8;32])?.public_key(), pages: pages.iter().map(|p| (*p).into()).collect(), revoked: false };
            let issuer = key.sign_statement(tx.head(), "member.add", &serde_json::to_vec(&json!({"memberId": member,"role":"editor","signKey":values::encode_binary(&recipient.signing_key),"encKey":values::encode_binary(&recipient.encryption_key),"pages":pages}))?)?;
            tx.append_statement(&issuer)?;
            tx.put_recipient(&recipient)?;
            let cert = certificate::input(&certificate::Certificate { space:&key.space_id, issuer_kind:"member", issuer_id:member, device_id:DEVICE, signing_key:&signer(9).verifying_key().to_bytes(), encryption_key:&wrap::RecipientKey::from_seed(&[10;32])?.public_key(), membership_revision:"2", issued_at:1, expires_at:100000 })?;
            tx.put_device(&Device {revoked:false, chain:serde_json::to_vec(&json!({"version":1,"issuerStatement":values::encode_binary(&issuer.hash()?),"deviceCertificate":values::encode_binary(&cert),"issuerSignature":values::encode_binary(&signer(7).sign(&cert).to_bytes())}))?})?;
            for page in pages {
                tx.put_epoch_secret(page, 1, &[11; 32])?;
            }
            Ok(b"genesis".to_vec())
        }).unwrap();
        Self {
            root,
            key,
            store,
            next: Default::default(),
        }
    }
    /// Seals and stores one content update as the fixture device's next stream object.
    fn append(&mut self, page: &'static str, update: &[u8]) {
        let (seq, previous) = self.next.get(page).copied().unwrap_or((1, [0; 32]));
        let envelope = object::seal(
            &object::Context {
                space: self.key.space_id.clone(),
                page: page.into(),
                epoch: "1".into(),
                kind: "update".into(),
                namespace: "content".into(),
                author_device: DEVICE.into(),
                membership_revision: "2".into(),
                stream_seq: seq.to_string(),
                prev_hash: previous,
            },
            &[11; 32],
            &signer(9),
            update,
        )
        .unwrap();
        let bytes = envelope.to_json().unwrap();
        let hash = envelope.hash().unwrap();
        self.store
            .append(&Envelope {
                scope: StreamScope {
                    page,
                    epoch: 1,
                    stream: DEVICE,
                },
                namespace: Namespace::Content,
                seq,
                hash,
                previous,
                bytes: &bytes,
            })
            .unwrap();
        self.next.insert(page, (seq + 1, hash));
    }
    fn cli(&self, args: &[&str]) -> (bool, Value, String) {
        let core = self.root.join("core");
        if !core.exists() {
            fs::write(
                &core,
                format!(
                    "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{}'\n",
                    json!({"dataRoot":self.root})
                ),
            )
            .unwrap();
            fs::set_permissions(&core, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let output = Command::new(BINARY)
            .env("TMT_EXECUTABLE", core)
            .current_dir(&self.root)
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        (
            output.status.success(),
            serde_json::from_str(stdout.trim()).unwrap_or(Value::Null),
            stdout,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Browser-style saves then many small appends, as one author's incremental updates.
/// Returns the final source and the stream's object count and plaintext bytes.
fn grow(f: &mut Fixture, page: &'static str, appends: usize) -> (String, usize, usize) {
    let doc = Doc::with_client_id(57);
    let html = doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    let mut sizes = Vec::new();
    let mut step = |f: &mut Fixture, update: Vec<u8>| {
        sizes.push(update.len());
        f.append(page, &update);
    };
    let filler = |n: usize, tag: char| -> String {
        (0..n)
            .map(|i| if i % 61 == 0 { tag } else { 'a' })
            .collect()
    };
    let mut txn = doc.transact_mut();
    meta.insert(&mut txn, "title", "Large page");
    html.insert(&mut txn, 0, &filler(1024, 'x'));
    let first = txn.encode_update_v1();
    drop(txn);
    step(f, first);
    // Whole-source browser saves: 200 KiB, then 60 KiB more, past the old 256 KiB tail cap.
    let mut txn = doc.transact_mut();
    let before = html.len(&txn);
    html.remove_range(&mut txn, 0, before);
    html.insert(&mut txn, 0, &filler(200 * 1024, 'y'));
    let second = txn.encode_update_v1();
    drop(txn);
    step(f, second);
    let mut txn = doc.transact_mut();
    let end = html.len(&txn);
    html.insert(&mut txn, end, &filler(60 * 1024, 'w'));
    let third = txn.encode_update_v1();
    drop(txn);
    step(f, third);
    // Many small CLI-style appends, past the old 200-object cap.
    for i in 0..appends {
        let mut txn = doc.transact_mut();
        let end = html.len(&txn);
        html.insert(&mut txn, end, &format!("{i:04}|"));
        let update = txn.encode_update_v1();
        drop(txn);
        step(f, update);
    }
    let source = html.get_string(&doc.transact());
    let _ = doc.transact().state_vector();
    (source, sizes.len(), sizes.iter().sum())
}
use yrs::Map;

#[test]
fn a_tail_past_the_old_caps_reads_back_byte_exact_through_the_library_readers() {
    let mut f = Fixture::new(&[PAGE, OTHER]);
    let (source, objects, tail) = grow(&mut f, PAGE, 210);
    // Ben's state: past 256 KiB of tail and 200 objects.
    assert!(
        tail > 256 * 1024 && objects > 200,
        "{tail} B over {objects} objects"
    );
    // The library path runs the decoder with test headroom: a debug child is far slower than
    // the shipped one. The shipped binary on the same shape is covered by the real-binary
    // acceptance (acceptance/page-size.spec.ts).
    let mut decoder = Decoder::with_config(support::decoder_config(BINARY.into())).unwrap();
    let read = tmt_colab::page::read(&f.store, &f.key, PAGE, &mut decoder).unwrap();
    assert!(
        read.source == source,
        "library read differs: {} vs {} chars",
        read.source.len(),
        source.len()
    );
    let bundle = Bundle::capture(&f.store, &f.key, PAGE, &mut decoder, 1234).unwrap();
    let published = bundle.publish(&f.root).unwrap();
    assert!(
        fs::read(published.directory.join("page.html")).unwrap() == source.as_bytes(),
        "export differs"
    );
}

/// Plaintext objects past the page-state cap, without caring what they decode to: the size check
/// runs before any decoding.
fn overfill(f: &mut Fixture, page: &'static str) {
    let chunk = vec![b'x'; 255 * 1024];
    for _ in 0..(tmt_colab::decoder::STATE_BYTES / chunk.len() + 2) {
        f.append(page, &chunk);
    }
}

#[test]
fn a_page_that_cannot_open_names_itself_and_does_not_hide_the_others() {
    let mut f = Fixture::new(&[OTHER, HUGE]);
    let small = Doc::with_client_id(58);
    small
        .get_or_insert_text("html")
        .insert(&mut small.transact_mut(), 0, "<p>other</p>");
    small
        .get_or_insert_map("meta")
        .insert(&mut small.transact_mut(), "title", "Other");
    f.append(
        OTHER,
        &small
            .transact()
            .encode_state_as_update_v1(&StateVector::default()),
    );
    overfill(&mut f, HUGE);

    let (ok, json, _) = f.cli(&["ls", "--json"]);
    assert!(ok, "ls must list the readable pages: {json}");
    let rows = json["pages"].as_array().unwrap();
    let find = |id: &str| rows.iter().find(|p| p["pageId"] == id).unwrap();
    assert_eq!(find(OTHER)["title"], "Other");
    assert!(find(OTHER).get("error").is_none());
    let huge = find(HUGE);
    assert_eq!(huge["title"], Value::Null);
    assert_eq!(huge["error"]["code"], "COLAB_CAPACITY");
    let message = huge["error"]["message"].as_str().unwrap();
    for part in [
        HUGE,
        "too large to open",
        "bytes (limit 25165824)",
        "copy its source",
        "#1627",
    ] {
        assert!(message.contains(part), "{part} missing from {message}");
    }
    assert!(
        huge["warnings"]
            .as_array()
            .unwrap()
            .contains(&json!("page-unavailable"))
    );

    // show prints what it can plus the error; the readable page is untouched.
    let (ok, json, _) = f.cli(&["show", HUGE, "--json"]);
    assert!(
        ok && json["page"]["error"]["code"] == "COLAB_CAPACITY",
        "{json}"
    );
    let (ok, json, _) = f.cli(&["show", OTHER, "--json"]);
    assert!(ok && json["page"]["title"] == "Other");

    // Reading the page itself fails with the same named, coded error (not 'Owner store: Capacity').
    let (ok, json, _) = f.cli(&["page", "read", HUGE, "--json"]);
    assert!(!ok);
    assert_eq!(json["error"]["code"], "COLAB_CAPACITY");
    assert!(json["error"]["message"].as_str().unwrap().contains(HUGE));
    // Human output keeps the list and explains the page on stderr.
    let human = Command::new(BINARY)
        .env("TMT_EXECUTABLE", f.root.join("core"))
        .current_dir(&f.root)
        .arg("ls")
        .output()
        .unwrap();
    assert!(human.status.success());
    assert!(String::from_utf8_lossy(&human.stdout).contains("unavailable (COLAB_CAPACITY)"));
    assert!(String::from_utf8_lossy(&human.stderr).contains("too large to open"));
}
