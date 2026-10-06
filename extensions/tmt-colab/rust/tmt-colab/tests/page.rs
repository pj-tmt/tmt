mod support;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page::{self, Fault},
    store::{
        Accepted, Envelope, Namespace, Store, StreamScope,
        owner::{Device, Mutation, Recipient},
    },
    transitions::{Engine, EpochAdvance},
};
use tmt_colab_model::{certificate, object, values, wrap};
use yrs::{Doc, Map, ReadTxn, StateVector, Text, Transact, Update, updates::decoder::Decode};
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
const NOW: u64 = 1_791_025_000_000;
struct Fixture {
    root: PathBuf,
    layout: Layout,
    key: Keyring,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let mut store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        store.owner_transaction(
            &key.space_id,
            &key.owner_public(),
            Mutation { operation_id: PAGE, digest: [1; 32], expected_revision: 0 },
            |tx| {
                let m = key.management_member()?;
                let genesis = key.sign_statement(tx.head(), "member.add", &serde_json::to_vec(&json!({
                    "memberId": m.id, "role": "editor", "signKey": values::encode_binary(&m.signing_key),
                    "encKey": values::encode_binary(&m.encryption_key), "pages": []
                }))?)?;
                tx.append_statement(&genesis)?;
                tx.put_recipient(&Recipient {
                    kind: "member".into(), id: m.id, role: Some("editor".into()),
                    signing_key: m.signing_key, encryption_key: m.encryption_key,
                    pages: vec![], revoked: false,
                })?;
                let member = "20000000-0000-4000-8000-000000000001";
                let sign = SigningKey::from_bytes(&[7; 32]);
                let enc = wrap::RecipientKey::from_seed(&[8; 32])?.public_key();
                let issuer = key.sign_statement(tx.head(), "member.add", &serde_json::to_vec(&json!({
                    "memberId": member, "role": "editor", "signKey": values::encode_binary(sign.verifying_key().as_bytes()),
                    "encKey": values::encode_binary(&enc), "pages": [PAGE]
                }))?)?;
                tx.append_statement(&issuer)?;
                tx.put_recipient(&Recipient {
                    kind: "member".into(), id: member.into(), role: Some("editor".into()),
                    signing_key: sign.verifying_key().to_bytes(), encryption_key: enc,
                    pages: vec![PAGE.into()], revoked: false,
                })?;
                let cert = certificate::input(&certificate::Certificate {
                    space: &key.space_id, issuer_kind: "member", issuer_id: member, device_id: DEVICE,
                    signing_key: SigningKey::from_bytes(&[9; 32]).verifying_key().as_bytes(),
                    encryption_key: &enc, membership_revision: "2", issued_at: 1,
                    expires_at: NOW + 86_400_000,
                })?;
                tx.put_device(&Device {
                    revoked: false,
                    chain: serde_json::to_vec(&json!({
                        "version": 1, "issuerStatement": values::encode_binary(&issuer.hash()?),
                        "deviceCertificate": values::encode_binary(&cert),
                        "issuerSignature": values::encode_binary(&sign.sign(&cert).to_bytes())
                    }))?,
                })?;
                tx.put_epoch_secret(PAGE, 1, &[11; 32])?;
                Ok(Vec::new())
            },
        ).unwrap();
        let doc = Doc::new();
        doc.get_or_insert_text("html")
            .insert(&mut doc.transact_mut(), 0, "old 🐈\r\n");
        doc.get_or_insert_map("meta")
            .insert(&mut doc.transact_mut(), "title", "Exact title 🐈");
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
            &[11; 32],
            &SigningKey::from_bytes(&[9; 32]),
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
        Self {
            root,
            layout,
            key,
            store,
        }
    }
    fn decoder(&self) -> Decoder {
        Decoder::with_config(support::decoder_config(BINARY.into())).unwrap()
    }
    fn read(&self) -> page::Page {
        page::read(&self.store, &self.key, PAGE, &mut self.decoder()).unwrap()
    }
    fn prepare(&self, source: &str, revision: Option<&str>) -> tmt_colab::Result<page::Prepared> {
        page::prepare(
            &self.store,
            &self.key,
            PAGE,
            tmt_colab::decoder::ContentEdit {
                source,
                publisher_agent: None,
            },
            revision,
            &mut self.decoder(),
            NOW,
        )
    }
    fn write(&mut self, source: &str) -> page::Receipt {
        let p = self.prepare(source, None).unwrap();
        page::commit(&mut self.store, &self.key, &p, NOW)
            .unwrap()
            .receipt
    }
    fn bytes(&self) -> Vec<u8> {
        fs::read(self.layout.directory.join("space.db")).unwrap()
    }
    fn sql(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.layout.directory.join("space.db")).unwrap()
    }
    fn command(&self) -> Command {
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
        let mut c = Command::new(BINARY);
        c.env("TMT_EXECUTABLE", core).current_dir(&self.root);
        c
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn exact_source_title_authenticated_receipts_and_reopening() {
    let mut f = Fixture::new();
    let before = f.bytes();
    let read = Store::read(&f.layout).unwrap();
    let initial = page::read(&read, &f.key, PAGE, &mut f.decoder()).unwrap();
    read.close().unwrap();
    assert_eq!(initial.source, "old 🐈\r\n");
    assert_eq!(f.bytes(), before);
    for source in ["new 🐈\r\n\0雪", "", "old 🦊\r\n"] {
        let p = f.prepare(source, Some(&f.read().revision)).unwrap();
        assert!(!serde_json::to_string(&p).unwrap().contains("new 🐈"));
        let receipt = page::commit(&mut f.store, &f.key, &p, NOW).unwrap();
        assert_eq!(receipt.accepted, Accepted::New);
        let result = f.read();
        assert_eq!(result.source, source);
        assert_eq!(result.title, "Exact title 🐈");
        assert_eq!(result.revision, receipt.receipt.revision);
        assert_eq!(result.membership_head.revision, "2");
        let envelope = object::Envelope::from_json(
            &values::binary(&p.envelope, tmt_colab::limits::UPDATE_BYTES).unwrap(),
        )
        .unwrap();
        let chain =
            certificate::Chain::from_json(&values::binary(&p.chain, 16 * 1024).unwrap()).unwrap();
        let header = object::Header::decode(envelope.header()).unwrap();
        assert_eq!(header.context.author_device, receipt.receipt.stream_id);
        assert_eq!(header.context.namespace, "content");
        assert_eq!(
            envelope.hash().unwrap().as_slice(),
            values::binary(&receipt.receipt.envelope_hash, 32).unwrap()
        );
        assert!(
            !object::open(
                &envelope,
                &header.context,
                &[11; 32],
                chain.certificate().unwrap().signing_key
            )
            .unwrap()
            .is_empty()
        );
        let reopened = Store::read(&f.layout).unwrap();
        assert_eq!(
            page::read(&reopened, &f.key, PAGE, &mut f.decoder())
                .unwrap()
                .source,
            source
        );
    }
    Engine::with_decoder_config(support::decoder_config(BINARY.into()))
        .unwrap()
        .advance_epoch(
            &mut f.store,
            &f.key,
            EpochAdvance {
                page: PAGE,
                operation_id: "40000000-0000-4000-8000-000000000001",
                expected_revision: 2,
            },
            NOW,
        )
        .unwrap();
    let r = f.write("after rotation 🦊");
    assert_eq!(r.epoch, "2");
    assert_eq!(f.read().source, "after rotation 🦊");
    assert_eq!(f.read().title, "Exact title 🐈");
}
#[test]
fn stale_concurrent_bases_and_exact_replay_never_overwrite_or_reseal() {
    let mut f = Fixture::new();
    let base = f.read().revision;
    // Both real children finish before either writer commits: deterministic race barrier.
    let first = f.prepare("one", Some(&base)).unwrap();
    let second = f.prepare("two", Some(&base)).unwrap();
    let accepted = page::commit(&mut f.store, &f.key, &first, NOW)
        .unwrap()
        .receipt;
    let before = f.bytes();
    assert_eq!(
        page::commit(&mut f.store, &f.key, &second, NOW)
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::StaleBase)
    );
    assert_eq!(f.bytes(), before);
    assert_eq!(f.read().source, "one");
    assert_eq!(
        f.prepare("three", Some(&base))
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::StaleBase)
    );
    f.write("later");
    let before = f.bytes();
    let replay = page::commit(&mut f.store, &f.key, &first, NOW).unwrap();
    assert_eq!(replay.accepted, Accepted::Replay);
    assert_eq!(
        serde_json::to_vec(&replay.receipt).unwrap(),
        serde_json::to_vec(&accepted).unwrap()
    );
    assert_eq!(f.bytes(), before);
    assert_eq!(f.read().source, "later");
    let mut changed = serde_json::to_value(&first).unwrap();
    changed["sourceSha256"] = Value::String("0".repeat(64));
    let changed: page::Prepared = serde_json::from_value(changed).unwrap();
    assert!(page::commit(&mut f.store, &f.key, &changed, NOW).is_err());
    assert_eq!(f.bytes(), before);
}
#[test]
fn late_failure_rolls_back_device_append_and_receipt_and_revocation_denies_replay() {
    let mut f = Fixture::new();
    let p = f.prepare("draft", None).unwrap();
    f.sql().execute_batch("CREATE TRIGGER reject_page_receipt BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(ABORT,'fixture refusal'); END").unwrap();
    let before = f.bytes();
    assert!(page::commit(&mut f.store, &f.key, &p, NOW).is_err());
    assert_eq!(f.bytes(), before);
    assert_eq!(f.read().source, "old 🐈\r\n");
    f.sql()
        .execute_batch("DROP TRIGGER reject_page_receipt")
        .unwrap();
    let r = page::commit(&mut f.store, &f.key, &p, NOW).unwrap().receipt;
    let db = f.sql();
    let record: Vec<u8> = db
        .query_row(
            "SELECT record FROM devices WHERE id=?",
            [&r.stream_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut device: Device = serde_json::from_slice(&record).unwrap();
    device.revoked = true;
    db.execute(
        "UPDATE devices SET record=? WHERE id=?",
        rusqlite::params![serde_json::to_vec(&device).unwrap(), r.stream_id],
    )
    .unwrap();
    let before = f.bytes();
    assert_eq!(
        page::commit(&mut f.store, &f.key, &p, NOW)
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::Denied)
    );
    assert_eq!(
        f.prepare("again", None)
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::Denied)
    );
    assert_eq!(f.bytes(), before);
}
#[test]
fn cli_raw_json_stdin_invalid_capacity_and_lifecycle_refusal() {
    let f = Fixture::new();
    let out = f.command().args(["page", "read", PAGE]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"old \xf0\x9f\x90\x88\r\n");
    assert!(String::from_utf8(out.stderr).unwrap().contains("epoch"));
    let read = f
        .command()
        .args(["page", "read", PAGE, "--json"])
        .output()
        .unwrap();
    let read: Value = serde_json::from_slice(&read.stdout).unwrap();
    assert_eq!(read["membershipHead"]["revision"], "2");
    let mut child = f
        .command()
        .args([
            "page",
            "write",
            PAGE,
            "--file",
            "-",
            "--expected-revision",
            read["revision"].as_str().unwrap(),
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let source = format!("{}\r\n\0", "stdin".repeat(4096));
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let r: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_ne!(r["revision"], read["revision"]);
    assert_eq!(f.read().source, source);
    let input = f.root.join("source");
    for (bytes, code) in [
        (vec![0xff], "COLAB_INPUT_INVALID"),
        (
            vec![b'x'; tmt_colab::decoder::BASELINE_BYTES + 1],
            "COLAB_CAPACITY",
        ),
    ] {
        fs::write(&input, bytes).unwrap();
        let before = f.bytes();
        let out = f
            .command()
            .args([
                "page",
                "write",
                PAGE,
                "--file",
                input.to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let error: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(error["error"]["code"], code);
        assert_eq!(f.bytes(), before);
    }
    fs::write(&input, b"new").unwrap();
    let out = f
        .command()
        .args([
            "page",
            "write",
            PAGE,
            "--file",
            input.to_str().unwrap(),
            "--expected-revision",
            read["revision"].as_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["error"]["code"],
        "COLAB_STALE_BASE"
    );
    let _lock = f.layout.serve_lock().unwrap();
    let before = f.bytes();
    let out = f
        .command()
        .args([
            "page",
            "write",
            PAGE,
            "--file",
            input.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(e["error"]["code"], "COLAB_UNAVAILABLE");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no offline fallback")
    );
    assert_eq!(f.bytes(), before);
}
#[test]
fn ciphertext_tampering_and_oversized_delta_apply_nothing() {
    let mut f = Fixture::new();
    let p = f.prepare("test", None).unwrap();
    let before = f.bytes();
    let mut encoded = serde_json::to_value(&p).unwrap();
    let mut envelope: Value = serde_json::from_slice(
        &values::binary(&p.envelope, tmt_colab::limits::UPDATE_BYTES).unwrap(),
    )
    .unwrap();
    envelope["signature"] = values::encode_binary(&[0; 64]).into();
    encoded["envelope"] = values::encode_binary(&serde_json::to_vec(&envelope).unwrap()).into();
    let bad: page::Prepared = serde_json::from_value(encoded).unwrap();
    assert!(page::commit(&mut f.store, &f.key, &bad, NOW).is_err());
    assert_eq!(f.bytes(), before);
    assert!(
        f.prepare(&"x".repeat(tmt_colab::decoder::UPDATE_BYTES + 1), None)
            .is_err()
    );
    assert_eq!(f.bytes(), before);
}

#[test]
fn expired_local_certificate_renews_same_device_atomically() {
    let mut f = Fixture::new();
    let original = f.write("before expiry");
    let later = NOW + tmt_colab::registration::CERTIFICATE_MS;
    let prepared = page::prepare(
        &f.store,
        &f.key,
        PAGE,
        tmt_colab::decoder::ContentEdit {
            source: "after expiry",
            publisher_agent: None,
        },
        None,
        &mut f.decoder(),
        later,
    )
    .unwrap();
    let renewed_bytes = values::binary(&prepared.chain, 16 * 1024).unwrap();
    let renewed = certificate::Chain::from_json(&renewed_bytes).unwrap();
    let cert = renewed.certificate().unwrap();
    assert_eq!(cert.device_id, original.stream_id);
    assert_eq!(cert.issued_at, later);
    assert_eq!(
        cert.expires_at,
        later + tmt_colab::registration::CERTIFICATE_MS
    );
    f.sql().execute_batch("CREATE TRIGGER reject_renewal BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(ABORT,'fixture refusal'); END").unwrap();
    let before = f.bytes();
    assert!(page::commit(&mut f.store, &f.key, &prepared, later).is_err());
    assert_eq!(f.bytes(), before);
    f.sql()
        .execute_batch("DROP TRIGGER reject_renewal")
        .unwrap();
    let receipt = page::commit(&mut f.store, &f.key, &prepared, later)
        .unwrap()
        .receipt;
    assert_eq!(receipt.stream_id, original.stream_id);
    assert_eq!(receipt.seq, "2");
    assert_eq!(f.read().source, "after expiry");
    let record: Vec<u8> = f
        .sql()
        .query_row(
            "SELECT record FROM devices WHERE id=?",
            [&receipt.stream_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut device: Device = serde_json::from_slice(&record).unwrap();
    assert_eq!(device.chain, renewed_bytes);
    device.revoked = true;
    f.sql()
        .execute(
            "UPDATE devices SET record=? WHERE id=?",
            rusqlite::params![serde_json::to_vec(&device).unwrap(), receipt.stream_id],
        )
        .unwrap();
    let before = f.bytes();
    assert_eq!(
        page::prepare(
            &f.store,
            &f.key,
            PAGE,
            tmt_colab::decoder::ContentEdit {
                source: "revoked",
                publisher_agent: None
            },
            None,
            &mut f.decoder(),
            cert.expires_at
        )
        .err()
        .unwrap()
        .downcast_ref::<Fault>(),
        Some(&Fault::Denied)
    );
    assert_eq!(f.bytes(), before);
}
#[test]
fn browser_author_append_invalidates_read_and_prepared_cli_bases() {
    let mut f = Fixture::new();
    let base = f.read().revision;
    let prepared = f.prepare("CLI replacement", Some(&base)).unwrap();
    // The existing certified browser author appends its own signed stream delta.
    let bytes: Vec<u8> = f
        .sql()
        .query_row(
            "SELECT payload FROM receipts WHERE stream=? ORDER BY seq DESC LIMIT 1",
            [DEVICE],
            |row| row.get(0),
        )
        .unwrap();
    let previous = object::Envelope::from_json(&bytes).unwrap();
    let header = object::Header::decode(previous.header()).unwrap();
    let plain = object::open(
        &previous,
        &header.context,
        &[11; 32],
        SigningKey::from_bytes(&[9; 32]).verifying_key().as_bytes(),
    )
    .unwrap();
    let doc = Doc::new();
    doc.transact_mut()
        .apply_update(Update::decode_v1(&plain).unwrap())
        .unwrap();
    let vector = doc.transact().state_vector();
    doc.get_or_insert_text("html")
        .insert(&mut doc.transact_mut(), 0, "browser ");
    let update = doc.transact().encode_state_as_update_v1(&vector);
    let context = object::Context {
        stream_seq: "2".into(),
        prev_hash: previous.hash().unwrap(),
        ..header.context
    };
    let envelope = object::seal(
        &context,
        &[11; 32],
        &SigningKey::from_bytes(&[9; 32]),
        &update,
    )
    .unwrap();
    f.store
        .append(&Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: DEVICE,
            },
            namespace: Namespace::Content,
            seq: 2,
            hash: envelope.hash().unwrap(),
            previous: context.prev_hash,
            bytes: &envelope.to_json().unwrap(),
        })
        .unwrap();
    assert_eq!(f.read().source, "browser old 🐈\r\n");
    let before = f.bytes();
    assert_eq!(
        f.prepare("CLI replacement", Some(&base))
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::StaleBase)
    );
    assert_eq!(
        page::commit(&mut f.store, &f.key, &prepared, NOW)
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::StaleBase)
    );
    assert_eq!(f.bytes(), before);
    assert_eq!(f.read().membership_head.revision, "2");
}

fn compact_now(f: &mut Fixture, updates: usize) -> Option<tmt_colab::page::compact::Compacted> {
    let mut decoder = f.decoder();
    tmt_colab::page::compact::compact(
        &mut f.store,
        &f.key,
        PAGE,
        &mut decoder,
        tmt_colab::page::compact::Trigger {
            updates,
            bytes: usize::MAX,
        },
    )
    .unwrap()
}

#[test]
fn a_changed_merge_cannot_publish_a_checkpoint_or_prune_its_updates() {
    let mut f = Fixture::new();
    let mut source = String::from("old 🐈\r\n");
    for i in 0..12 {
        source.push_str(&format!("<p>{i}</p>"));
        f.write(&source);
    }
    let receipts = |f: &Fixture| {
        let connection = f.sql();
        let mut query = connection
            .prepare("SELECT hex(payload) FROM receipts WHERE payload IS NOT NULL ORDER BY rowid")
            .unwrap();
        query
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let before = receipts(&f);
    let program = f.root.join("changed-merge");
    let script = format!(
        r#"#!/usr/bin/python3
import json, subprocess, sys
wire = sys.stdin.buffer.read()
child = subprocess.run([{binary}, "__decoder"], input=wire, capture_output=True)
if child.returncode:
    sys.stderr.buffer.write(child.stderr)
    sys.exit(child.returncode)
reply = json.loads(child.stdout)
request = json.loads(wire)
if request.get("merge_only"):
    reply["merged"] = request["updates"][0]
sys.stdout.write(json.dumps(reply))
"#,
        binary = serde_json::to_string(BINARY).unwrap()
    );
    tmt_test_support::write_executable(&program, script.as_bytes(), 0o700).unwrap();
    let mut decoder = Decoder::with_config(support::decoder_config(program)).unwrap();
    assert_eq!(
        page::compact::compact(
            &mut f.store,
            &f.key,
            PAGE,
            &mut decoder,
            page::compact::Trigger {
                updates: 10,
                bytes: usize::MAX,
            },
        )
        .unwrap(),
        None
    );
    let checkpoints: i64 = f
        .sql()
        .query_row(
            "SELECT count(*) FROM checkpoints WHERE payload IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(checkpoints, 0);
    assert_eq!(receipts(&f), before, "all update payloads remain available");
    assert_eq!(f.read().source, source);
}

#[test]
fn a_device_combines_its_own_tail_and_the_page_still_reads_back_exact() {
    let mut f = Fixture::new();
    let mut source = String::from("old 🐈\r\n");
    for i in 0..12 {
        source.push_str(&format!("<p>{i}</p>"));
        f.write(&source);
    }
    // Below the trigger nothing happens.
    assert_eq!(compact_now(&mut f, 100), None);
    let objects_before: i64 = f
        .sql()
        .query_row(
            "SELECT count(*) FROM receipts WHERE payload IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let done = compact_now(&mut f, 10).expect("the tail passed the trigger");
    assert_eq!(done.namespaces, 1);
    assert_eq!(done.merged_objects, 12);
    let objects_after: i64 = f
        .sql()
        .query_row(
            "SELECT count(*) FROM receipts WHERE payload IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // The device's twelve updates became one checkpoint; the first device's update is untouched.
    assert_eq!(objects_before - objects_after, 12);
    let checkpoints: i64 = f
        .sql()
        .query_row(
            "SELECT count(*) FROM checkpoints WHERE payload IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(checkpoints, 1);
    assert_eq!(f.read().source, source);
    // Nothing left to combine, and repeating is harmless.
    assert_eq!(compact_now(&mut f, 1), None);
    // Writes continue on top of the checkpoint and read back exact.
    source.push_str("<p>after</p>");
    f.write(&source);
    assert_eq!(f.read().source, source);
}

#[test]
fn a_page_takes_more_than_two_hundred_changes_when_its_device_combines() {
    let mut f = Fixture::new();
    let mut source = String::from("old 🐈\r\n");
    for i in 0..230 {
        source.push_str(&format!("<i>{i}</i>"));
        f.write(&source);
        compact_now(&mut f, 20);
    }
    assert_eq!(f.read().source, source);
    let live: i64 = f
        .sql()
        .query_row(
            "SELECT count(*) FROM receipts WHERE payload IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(live <= 22, "{live} updates still stored");
}

// Native publication fixtures are pure library/SQLite checks: no decoder or CLI process.
mod native_publication {
    use super::*;
    use rusqlite::{OptionalExtension, params};
    use tmt_colab::{publication::*, store::owner::Cut};
    use tmt_colab_model::{crypto, framing};
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    fn signer(f: &Fixture, label: &[u8]) -> SigningKey {
        let seed = fs::read(f.layout.directory.join("owner.key")).unwrap();
        SigningKey::from_bytes(&crypto::derive_key(
            &seed,
            &[],
            &framing::frame(&[label, f.key.space_id.as_bytes()]).unwrap(),
        ))
    }
    fn device(f: &Fixture) -> String {
        let seed = fs::read(f.layout.directory.join("owner.key")).unwrap();
        let mut id = crypto::derive_key(
            &seed,
            &[],
            &framing::frame(&[b"tmt-colab-cli-device-id-v1", f.key.space_id.as_bytes()]).unwrap(),
        );
        id[6] = (id[6] & 15) | 64;
        id[8] = (id[8] & 63) | 128;
        let h = hex(&id[..16]);
        format!(
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        )
    }
    fn chain(f: &Fixture, issued: u64) -> Vec<u8> {
        let member = f.key.management_member().unwrap();
        let seed = fs::read(f.layout.directory.join("owner.key")).unwrap();
        let enc = crypto::derive_key(
            &seed,
            &[],
            &framing::frame(&[
                b"tmt-colab-cli-encryption-seed-v1",
                f.key.space_id.as_bytes(),
            ])
            .unwrap(),
        );
        let enc = wrap::RecipientKey::from_seed(&enc).unwrap().public_key();
        let local = signer(f, b"tmt-colab-cli-signing-seed-v1");
        let input = certificate::input(&certificate::Certificate {
            space: &f.key.space_id,
            issuer_kind: "member",
            issuer_id: &member.id,
            device_id: &device(f),
            signing_key: local.verifying_key().as_bytes(),
            encryption_key: &enc,
            membership_revision: "1",
            issued_at: issued,
            expires_at: issued + 86_400_000,
        })
        .unwrap();
        let issuer: Vec<u8> = f
            .sql()
            .query_row(
                "SELECT hash FROM membership_log WHERE revision='00000000000000000001'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::to_vec(&json!({"version":1,"issuerStatement":values::encode_binary(&issuer),
            "deviceCertificate":values::encode_binary(&input),"issuerSignature":values::encode_binary(&signer(f,b"tmt-colab-management-signing-seed-v1").sign(&input).to_bytes())})).unwrap()
    }
    // Independent token oracle from public Cut framing and durable SQL state; never calls page::read.
    fn revision(f: &Fixture) -> String {
        let db = f.sql();
        let head = f
            .store
            .owner_head(&f.key.space_id, &f.key.owner_public())
            .unwrap()
            .unwrap();
        let epoch: String = db
            .query_row("SELECT epoch FROM pages WHERE page=?", [PAGE], |r| r.get(0))
            .unwrap();
        let streams = db
            .prepare("SELECT stream FROM streams WHERE page=? AND epoch=? ORDER BY stream")
            .unwrap()
            .query_map(params![PAGE, epoch], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>();
        let mut cuts = vec![];
        for namespace in ["content", "own"] {
            for stream in &streams {
                let tail: Option<(String,Vec<u8>)> = db.query_row("SELECT seq,hash FROM receipts WHERE page=? AND epoch=? AND stream=? AND namespace=? ORDER BY seq DESC LIMIT 1",
                params![PAGE,epoch,stream,namespace],|r|Ok((r.get(0)?,r.get(1)?))).optional().unwrap();
                let (seq, hash) = tail.map_or((0, [0; 32]), |(s, h)| {
                    (s.parse().unwrap(), h.try_into().unwrap())
                });
                cuts.push(
                    Cut {
                        page: PAGE.into(),
                        epoch: epoch.parse().unwrap(),
                        stream: stream.clone(),
                        namespace: namespace.into(),
                        checkpoint_seq: 0,
                        checkpoint_hash: None,
                        tail_seq: seq,
                        tail_hash: hash,
                    }
                    .payload()
                    .unwrap(),
                );
            }
        }
        format!(
            "v1:{}",
            hex(&crypto::digest(
                &framing::frame(&[
                    b"tmt-colab-page-revision-v1",
                    f.key.space_id.as_bytes(),
                    PAGE.as_bytes(),
                    head.revision.to_string().as_bytes(),
                    &head.hash,
                    epoch.as_bytes(),
                    &serde_json::to_vec(&cuts).unwrap()
                ])
                .unwrap()
            ))
        )
    }
    struct Job {
        signed: SignedJob,
        packet: Vec<u8>,
        chain: Vec<u8>,
    }
    fn resign(f: &Fixture, manifest: Manifest) -> SignedJob {
        let signature = values::encode_binary(
            &signer(f, b"tmt-colab-cli-signing-seed-v1")
                .sign(&manifest.signature_input().unwrap())
                .to_bytes(),
        );
        SignedJob {
            manifest,
            signature,
        }
    }
    fn job(f: &Fixture, operation: u32) -> Job {
        sealed_job(f, operation, &[vec![1; 8], vec![2; 8]])
    }
    fn sealed_job(f: &Fixture, operation: u32, updates: &[Vec<u8>]) -> Job {
        let chain = chain(f, NOW);
        let head = f
            .store
            .owner_head(&f.key.space_id, &f.key.owner_public())
            .unwrap()
            .unwrap();
        let stream = device(f);
        let mut packet = vec![];
        let mut entries = vec![];
        let mut previous = [0; 32];
        for (index, update) in updates.iter().enumerate() {
            let seq = index + 1;
            let envelope = object::seal(
                &object::Context {
                    space: f.key.space_id.clone(),
                    page: PAGE.into(),
                    epoch: "1".into(),
                    kind: "update".into(),
                    namespace: "content".into(),
                    author_device: stream.clone(),
                    membership_revision: head.revision.to_string(),
                    stream_seq: seq.to_string(),
                    prev_hash: previous,
                },
                &[11; 32],
                &signer(f, b"tmt-colab-cli-signing-seed-v1"),
                update,
            )
            .unwrap();
            previous = envelope.hash().unwrap();
            // Preserve legal unusual whitespace: the stored payload must be these bytes, not to_json().
            let bytes = format!(
                " {}\n",
                String::from_utf8(envelope.to_json().unwrap()).unwrap()
            )
            .into_bytes();
            entries.push(PublicationEntry {
                namespace: PublicationKind::Content,
                seq: seq.to_string(),
                envelope_hash: values::encode_binary(&previous),
                envelope_bytes: bytes.len(),
            });
            packet.extend(bytes);
        }
        let manifest = Manifest {
            version: 1,
            operation_id: format!("40000000-0000-4000-8000-{operation:012}"),
            space_id: f.key.space_id.clone(),
            page_id: PAGE.into(),
            epoch: "1".into(),
            stream_id: stream,
            kind: PublicationKind::Content,
            membership_head: MembershipHead {
                revision: head.revision.to_string(),
                statement_hash: hex(&head.hash),
            },
            base_revision: revision(f),
            entries,
            packet_bytes: packet.len(),
            packet_hash: values::encode_binary(&crypto::digest(&packet)),
            native_evidence: Some(NativeEvidence {
                source_sha256: hex(&crypto::digest(b"fixture opaque content")),
                memory_limit: tmt_colab::decoder::MemoryLimit::Unavailable,
                chain_hash: values::encode_binary(&crypto::digest(&chain)),
            }),
        };
        Job {
            signed: resign(f, manifest),
            packet,
            chain,
        }
    }
    fn commit(f: &mut Fixture, j: &Job) -> tmt_colab::Result<page::PublicationCommitted> {
        page::commit_publication(&mut f.store, &f.key, &j.signed, &j.packet, &j.chain, NOW)
    }
    fn status(f: &Fixture, j: &Job) -> tmt_colab::Result<page::PublicationRecord> {
        page::publication_status(&f.store, &f.key, &j.signed.key().unwrap(), &j.chain, NOW)
    }
    fn effects(f: &Fixture) -> Vec<String> {
        let db = f.sql();
        ["pages", "streams", "receipts", "devices", "checkpoints"]
            .into_iter()
            .map(|t| {
                let mut q = db
                    .prepare(&format!("SELECT * FROM {t} ORDER BY 1,2"))
                    .unwrap();
                let n = q.column_count();
                q.query_map([], |r| {
                    Ok((0..n)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(|r| r.unwrap())
                .collect::<Vec<_>>()
                .join("\n")
            })
            .collect()
    }
    fn terminals(f: &Fixture) -> i64 {
        f.sql()
            .query_row(
                "SELECT count(*) FROM owner_operations WHERE publication_kind IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn rejected(record: &page::PublicationRecord, code: Rejection) {
        assert!(matches!(&record.outcome,Outcome::Rejected {code:actual,..} if *actual==code));
    }
    #[test]
    fn native_publication_commits_exact_bytes_and_reopens_original_replay() {
        let mut f = Fixture::new();
        let j = job(&f, 1);
        let result = commit(&mut f, &j).unwrap();
        assert_eq!(result.accepted, Accepted::New);
        assert_eq!(terminals(&f), 1);
        assert!(
            f.sql()
                .query_row(
                    "SELECT last_update_at_ms FROM pages WHERE page=?",
                    [PAGE],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap()
                > 0
        );
        assert!(
            matches!(&result.record.outcome,Outcome::Committed{final_position,native_evidence,..}
            if final_position.seq=="2" && final_position.envelope_hash==j.signed.manifest.entries[1].envelope_hash
            && native_evidence==&j.signed.manifest.native_evidence)
        );
        let bytes = f
            .sql()
            .prepare("SELECT payload FROM receipts WHERE stream=? ORDER BY seq")
            .unwrap()
            .query_map([device(&f)], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .flat_map(|r| r.unwrap())
            .collect::<Vec<_>>();
        assert_eq!(bytes, j.packet);
        assert!(
            matches!(&result.record.outcome,Outcome::Committed {count:2,committed_revision,..} if *committed_revision==revision(&f))
        );
        let before = effects(&f);
        f.store = Store::open(&f.layout)
            .unwrap()
            .with_clock(|| panic!("replay sampled clock"));
        let replay = commit(&mut f, &j).unwrap();
        assert_eq!(replay.accepted, Accepted::Replay);
        assert_eq!(replay.record.bytes, result.record.bytes);
        assert_eq!(status(&f, &j).unwrap().bytes, result.record.bytes);
        assert_eq!(effects(&f), before);
        assert_eq!(terminals(&f), 1);
        // A renewed supplied authority is observational; original evidence remains byte-identical.
        assert_eq!(
            page::publication_status(
                &f.store,
                &f.key,
                &j.signed.key().unwrap(),
                &chain(&f, NOW + 1),
                NOW + 1
            )
            .unwrap()
            .bytes,
            result.record.bytes
        );
    }
    #[test]
    fn native_publication_late_capacity_rolls_back_measured_first_append() {
        let mut f = Fixture::new();
        let j = job(&f, 2);
        let before = effects(&f);
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let clock = calls.clone();
        f.store = Store::open(&f.layout).unwrap().with_clock(move || {
            clock.fetch_add(1, Ordering::SeqCst);
            Ok(NOW + 7)
        });
        // Controlled storage perturbation after the first insert, not a weakened preflight or invalid packet.
        f.sql().execute_batch(&format!("CREATE TRIGGER late_capacity AFTER INSERT ON receipts WHEN NEW.stream='{}' AND NEW.seq='00000000000000000001'
            BEGIN INSERT INTO checkpoints(page,epoch,stream,namespace,seq,hash,digest,head,payload,pinned)
            VALUES(NEW.page,NEW.epoch,NEW.stream,'own','00000000000000000000',X'01',X'02',X'03',zeroblob({}),0); END;",device(&f),tmt_colab::limits::PAGE_BYTES)).unwrap();
        let result = commit(&mut f, &j).unwrap();
        rejected(&result.record, Rejection::Capacity);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "must reach a verified first append before the second admission fails"
        );
        assert_eq!(effects(&f), before);
        assert_eq!(terminals(&f), 1);
        f.store = Store::open(&f.layout).unwrap();
        assert_eq!(
            commit(&mut f, &j).unwrap().record.bytes,
            result.record.bytes
        );
        assert_eq!(effects(&f), before);
    }
    #[test]
    fn native_publication_unexpected_sql_and_clock_errors_leave_unknown() {
        for failure in [
            "content",
            "outcome",
            "clock_invalid",
            "clock_capacity",
            "clock_gap",
        ] {
            let mut f = Fixture::new();
            let j = job(&f, 3);
            let before = effects(&f);
            match failure {
                "content" => f.sql().execute_batch("CREATE TRIGGER fail_content BEFORE INSERT ON receipts BEGIN SELECT RAISE(ABORT,'content fixture'); END").unwrap(),
                "outcome" => f.sql().execute_batch("CREATE TRIGGER fail_outcome BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(ABORT,'outcome fixture'); END").unwrap(),
                "clock_invalid" => f.store=Store::open(&f.layout).unwrap().with_clock(||Err(tmt_colab::store::Fault::Invalid)),
                "clock_capacity" => f.store=Store::open(&f.layout).unwrap().with_clock(||Err(tmt_colab::store::Fault::Capacity)),
                _ => f.store=Store::open(&f.layout).unwrap().with_clock(||Err(tmt_colab::store::Fault::Gap)),
            }
            let error = commit(&mut f, &j).err().expect(failure);
            if failure.starts_with("clock") {
                let tmt_colab::store::Fault::Clock(cause) =
                    error.downcast_ref::<tmt_colab::store::Fault>().unwrap()
                else {
                    panic!("lost clock provenance")
                };
                assert!(matches!(
                    (failure, cause.as_ref()),
                    ("clock_invalid", tmt_colab::store::Fault::Invalid)
                        | ("clock_capacity", tmt_colab::store::Fault::Capacity)
                        | ("clock_gap", tmt_colab::store::Fault::Gap)
                ));
                assert!(std::error::Error::source(error.as_ref()).is_some());
            }
            assert_eq!(effects(&f), before);
            assert_eq!(terminals(&f), 0);
            assert!(matches!(
                status(&f, &j).unwrap().outcome,
                Outcome::Unknown { .. }
            ));
        }
    }
    #[test]
    fn native_publication_stale_rejection_replay_and_changed_keys() {
        let mut f = Fixture::new();
        let mut j = job(&f, 4);
        j.signed.manifest.base_revision = format!("v1:{}", "0".repeat(64));
        j.signed = resign(&f, j.signed.manifest);
        let before = effects(&f);
        let first = commit(&mut f, &j).unwrap();
        rejected(&first.record, Rejection::StaleBase);
        assert_eq!(effects(&f), before);
        f.store = Store::open(&f.layout).unwrap();
        assert_eq!(commit(&mut f, &j).unwrap().record.bytes, first.record.bytes);
        for field in ["digest", "space", "page", "epoch", "stream"] {
            let mut changed = j.signed.key().unwrap();
            match field {
                "digest" => changed.job_digest = values::encode_binary(&[9; 32]),
                "space" => {
                    changed.space_id = crypto::space_id(
                        SigningKey::from_bytes(&[6; 32]).verifying_key().as_bytes(),
                    )
                    .unwrap()
                }
                "page" => changed.page_id = "10000000-0000-4000-8000-000000000002".into(),
                "epoch" => changed.original_epoch = "2".into(),
                _ => changed.stream_id = DEVICE.into(),
            }
            assert!(
                page::publication_status(&f.store, &f.key, &changed, &j.chain, NOW).is_err(),
                "{field}"
            );
        }
        assert_eq!(terminals(&f), 1);
        assert_eq!(effects(&f), before);
    }
    #[test]
    fn native_publication_legacy_collision_and_legacy_refusal_of_scoped_identity() {
        let mut f = Fixture::new();
        let j = job(&f, 5);
        let original = j.signed.key().unwrap();
        f.sql()
            .execute(
                "INSERT INTO owner_operations(id,digest,outcome) VALUES(?,?,?)",
                params![
                    original.operation_id,
                    values::binary(&original.job_digest, 32).unwrap(),
                    b"legacy".as_slice()
                ],
            )
            .unwrap();
        let before = effects(&f);
        assert!(commit(&mut f, &j).is_err());
        assert!(status(&f, &j).is_err());
        assert_eq!(effects(&f), before);
        f.sql()
            .execute(
                "DELETE FROM owner_operations WHERE id=?",
                [&original.operation_id],
            )
            .unwrap();
        commit(&mut f, &j).unwrap();
        let head = f
            .store
            .owner_head(&f.key.space_id, &f.key.owner_public())
            .unwrap()
            .unwrap();
        assert!(
            f.store
                .owner_transaction(
                    &f.key.space_id,
                    &f.key.owner_public(),
                    Mutation {
                        operation_id: &original.operation_id,
                        digest: values::binary(&original.job_digest, 32)
                            .unwrap()
                            .try_into()
                            .unwrap(),
                        expected_revision: head.revision
                    },
                    |_| panic!("scoped collision entered legacy apply")
                )
                .is_err()
        );
    }
    #[test]
    fn native_publication_status_refuses_expired_revoked_and_malformed_storage() {
        let mut f = Fixture::new();
        let j = job(&f, 6);
        commit(&mut f, &j).unwrap();
        assert!(
            page::publication_status(
                &f.store,
                &f.key,
                &j.signed.key().unwrap(),
                &j.chain,
                NOW + 86_400_000
            )
            .is_err()
        );
        let before = f.bytes();
        assert!(
            page::publication_status(&f.store, &f.key, &j.signed.key().unwrap(), b"{}", NOW)
                .is_err()
        );
        assert_eq!(f.bytes(), before);
        let record: Vec<u8> = f
            .sql()
            .query_row("SELECT record FROM devices WHERE id=?", [device(&f)], |r| {
                r.get(0)
            })
            .unwrap();
        let mut d: Device = serde_json::from_slice(&record).unwrap();
        d.revoked = true;
        f.sql()
            .execute(
                "UPDATE devices SET record=? WHERE id=?",
                params![serde_json::to_vec(&d).unwrap(), device(&f)],
            )
            .unwrap();
        assert!(status(&f, &j).is_err());
        assert!(commit(&mut f, &j).is_err());
        d.revoked = false;
        f.sql()
            .execute(
                "UPDATE devices SET record=? WHERE id=?",
                params![serde_json::to_vec(&d).unwrap(), device(&f)],
            )
            .unwrap();
        let mut missing_evidence = status(&f, &j).unwrap().outcome;
        if let Outcome::Committed {
            native_evidence, ..
        } = &mut missing_evidence
        {
            *native_evidence = None;
        }
        let missing_evidence = missing_evidence
            .to_json(&j.signed.key().unwrap(), None)
            .unwrap();
        for malformed in [
            missing_evidence,
            b"{}".to_vec(),
            Outcome::Unknown {
                key: j.signed.key().unwrap(),
            }
            .to_json(&j.signed.key().unwrap(), None)
            .unwrap(),
            vec![b' '; JSON_BYTES + 1],
        ] {
            f.sql()
                .execute(
                    "UPDATE owner_operations SET outcome=? WHERE id=?",
                    params![malformed, j.signed.manifest.operation_id],
                )
                .unwrap();
            assert!(status(&f, &j).is_err());
            assert!(commit(&mut f, &j).is_err());
        }
    }
    #[test]
    fn native_publication_count_reservation_and_lookup_at_capacity() {
        let mut f = Fixture::new();
        // 99,999 retained receipts leave one terminal identity, but no room for a two-entry batch.
        f.sql().execute_batch(&format!("WITH RECURSIVE n(x) AS(SELECT 2 UNION ALL SELECT x+1 FROM n WHERE x<{})
            INSERT INTO receipts(page,epoch,stream,seq,namespace,hash,digest,payload)
            SELECT '{}','1','{}',printf('%020d',x),'content',zeroblob(32),zeroblob(32),NULL FROM n;",tmt_colab::limits::PAGE_RECEIPTS-1,PAGE,DEVICE)).unwrap();
        let j = job(&f, 7);
        let before = effects(&f);
        let rejected_result = commit(&mut f, &j).unwrap();
        rejected(&rejected_result.record, Rejection::Capacity);
        assert_eq!(effects(&f), before);
        assert_eq!(status(&f, &j).unwrap().bytes, rejected_result.record.bytes);
        assert_eq!(commit(&mut f, &j).unwrap().accepted, Accepted::Replay);
        let fresh = job(&f, 8);
        assert!(commit(&mut f, &fresh).is_err());
        assert!(matches!(
            status(&f, &fresh).unwrap().outcome,
            Outcome::Unknown { .. }
        ));
        assert_eq!(terminals(&f), 1);
    }
    #[test]
    fn native_publication_byte_reservation_uses_actual_outcome_charge() {
        let mut f = Fixture::new();
        let j = job(&f, 9);
        let k = j.signed.key().unwrap();
        let receipt_bytes: i64 = f
            .sql()
            .query_row("SELECT sum(length(payload)) FROM receipts", [], |r| {
                r.get(0)
            })
            .unwrap();
        let overhead = k.operation_id.len()
            + 32
            + 7
            + k.space_id.len()
            + k.page_id.len()
            + k.original_epoch.len()
            + k.stream_id.len();
        let filler = tmt_colab::limits::PAGE_BYTES - receipt_bytes as usize - overhead - JSON_BYTES;
        f.sql()
            .execute(
                "INSERT INTO baselines VALUES(?,'00000000000000000001',X'01',zeroblob(?))",
                params![PAGE, filler as i64],
            )
            .unwrap();
        let result = commit(&mut f, &j).unwrap();
        rejected(&result.record, Rejection::Capacity);
        let charged:i64=f.sql().query_row("SELECT length(outcome)+length(digest)+length(CAST(id AS BLOB))+length(CAST(publication_kind AS BLOB))+length(CAST(space AS BLOB))+length(CAST(page AS BLOB))+length(CAST(original_epoch AS BLOB))+length(CAST(original_stream AS BLOB)) FROM owner_operations WHERE publication_kind IS NOT NULL",[],|r|r.get(0)).unwrap();
        assert_eq!(charged as usize, overhead + result.record.bytes.len());
        assert!(result.record.bytes.len() < JSON_BYTES);
        let before = effects(&f);
        assert_eq!(
            commit(&mut f, &j).unwrap().record.bytes,
            result.record.bytes
        );
        assert_eq!(effects(&f), before);
        f.sql()
            .execute(
                "UPDATE baselines SET envelope=zeroblob(?)",
                [
                    (tmt_colab::limits::PAGE_BYTES - receipt_bytes as usize - charged as usize)
                        as i64,
                ],
            )
            .unwrap();
        let fresh = job(&f, 10);
        assert!(commit(&mut f, &fresh).is_err());
        assert!(matches!(
            status(&f, &fresh).unwrap().outcome,
            Outcome::Unknown { .. }
        ));
        assert_eq!(status(&f, &j).unwrap().bytes, result.record.bytes);
    }
    fn policy(f: &mut Fixture, action: &str, operation: &str) {
        let head = f
            .store
            .owner_head(&f.key.space_id, &f.key.owner_public())
            .unwrap()
            .unwrap();
        f.store
            .owner_transaction(
                &f.key.space_id,
                &f.key.owner_public(),
                Mutation {
                    operation_id: operation,
                    digest: [7; 32],
                    expected_revision: head.revision,
                },
                |tx| {
                    let statement = f.key.sign_statement(
                        tx.head(),
                        action,
                        &serde_json::to_vec(&json!({"pageId":PAGE})).unwrap(),
                    )?;
                    tx.append_statement(&statement)?;
                    Ok(b"signed policy fixture".to_vec())
                },
            )
            .unwrap();
    }
    #[test]
    fn native_publication_retains_original_after_signed_archive_and_deleted_data() {
        let mut f = Fixture::new();
        let j = job(&f, 11);
        let original = commit(&mut f, &j).unwrap().record.bytes;
        policy(
            &mut f,
            "page.archive",
            "50000000-0000-4000-8000-000000000001",
        );
        assert_eq!(commit(&mut f, &j).unwrap().record.bytes, original);
        let fresh = job(&f, 12);
        let before = effects(&f);
        rejected(
            &commit(&mut f, &fresh).unwrap().record,
            Rejection::PageInactive,
        );
        assert_eq!(effects(&f), before);
        policy(
            &mut f,
            "page.delete",
            "50000000-0000-4000-8000-000000000002",
        );
        // Independent deleted-data fixture matching the existing deletion owner's table order.
        f.sql()
            .execute_batch(
                "DELETE FROM receipts; DELETE FROM checkpoints; DELETE FROM streams;
            DELETE FROM baselines; DELETE FROM wraps; DELETE FROM epoch_secrets;",
            )
            .unwrap();
        let before = effects(&f);
        assert_eq!(commit(&mut f, &j).unwrap().accepted, Accepted::Replay);
        assert_eq!(status(&f, &j).unwrap().bytes, original);
        assert_eq!(effects(&f), before);
        assert_eq!(terminals(&f), 2);
        assert_eq!(
            f.sql()
                .query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn native_publication_absence_is_read_only_and_bad_authentication_is_not_rejection() {
        for failure in ["packet", "signature", "evidence", "chain"] {
            let mut f = Fixture::new();
            let mut j = job(&f, 13);
            let before = f.bytes();
            assert!(matches!(
                status(&f, &j).unwrap().outcome,
                Outcome::Unknown { .. }
            ));
            assert_eq!(f.bytes(), before, "status issued or persisted authority");
            match failure {
                "packet" => j.packet[0] ^= 1,
                "signature" => j.signed.signature = values::encode_binary(&[0; 64]),
                "evidence" => {
                    j.signed.manifest.native_evidence = None;
                    j.signed = resign(&f, j.signed.manifest);
                }
                _ => j.chain.push(b' '),
            }
            let before = effects(&f);
            let error = commit(&mut f, &j).err().expect(failure);
            if failure.starts_with("clock") {
                let tmt_colab::store::Fault::Clock(cause) =
                    error.downcast_ref::<tmt_colab::store::Fault>().unwrap()
                else {
                    panic!("lost clock provenance")
                };
                assert!(matches!(
                    (failure, cause.as_ref()),
                    ("clock_invalid", tmt_colab::store::Fault::Invalid)
                        | ("clock_capacity", tmt_colab::store::Fault::Capacity)
                        | ("clock_gap", tmt_colab::store::Fault::Gap)
                ));
                assert!(std::error::Error::source(error.as_ref()).is_some());
            }
            assert_eq!(effects(&f), before);
            assert_eq!(terminals(&f), 0);
        }
    }
    #[test]
    fn native_publication_stored_scope_and_noncanonical_terminal_bytes() {
        let mut f = Fixture::new();
        let j = job(&f, 14);
        let first = commit(&mut f, &j).unwrap();
        let exact = format!(" \n{}\n", String::from_utf8(first.record.bytes).unwrap()).into_bytes();
        f.sql()
            .execute(
                "UPDATE owner_operations SET outcome=? WHERE id=?",
                params![exact, j.signed.manifest.operation_id],
            )
            .unwrap();
        assert_eq!(commit(&mut f, &j).unwrap().record.bytes, exact);
        assert_eq!(status(&f, &j).unwrap().bytes, exact);
        f.sql()
            .execute(
                "UPDATE owner_operations SET original_epoch='01' WHERE id=?",
                [&j.signed.manifest.operation_id],
            )
            .unwrap();
        assert!(status(&f, &j).is_err());
        assert!(commit(&mut f, &j).is_err());
    }
    #[test]
    fn native_publication_large_causal_packet_preserves_exact_borrowed_updates() {
        let mut f = Fixture::new();
        let doc = Doc::new();
        let html = doc.get_or_insert_text("html");
        let mut updates = vec![];
        let text = "🌱".repeat(48 * 1024); // 192KiB UTF-8 per causal edit; existing preparation geometry.
        let mut offset = 0;
        for _ in 0..8 {
            let mut tx = doc.transact_mut();
            let prior = tx.state_vector();
            html.insert(&mut tx, offset, &text);
            offset += 96 * 1024;
            updates.push(tx.encode_state_as_update_v1(&prior));
        }
        assert!(updates.iter().map(Vec::len).sum::<usize>() >= 1536 * 1024);
        let j = sealed_job(&f, 15, &updates);
        let result = commit(&mut f, &j).unwrap();
        assert!(matches!(
            result.record.outcome,
            Outcome::Committed { count: 8, .. }
        ));
        let stored = f
            .sql()
            .prepare("SELECT payload FROM receipts WHERE stream=? ORDER BY seq")
            .unwrap()
            .query_map([device(&f)], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>();
        assert_eq!(stored.concat(), j.packet);
        let reader = Doc::new();
        reader.get_or_insert_text("html");
        let public = signer(&f, b"tmt-colab-cli-signing-seed-v1")
            .verifying_key()
            .to_bytes();
        for (bytes, update) in stored.iter().zip(&updates) {
            let envelope = object::Envelope::from_json(bytes).unwrap();
            let header = object::Header::decode(envelope.header()).unwrap();
            let raw = object::open(&envelope, &header.context, &[11; 32], &public).unwrap();
            assert_eq!(&raw, update);
            reader
                .transact_mut()
                .apply_update(Update::decode_v1(&raw).unwrap())
                .unwrap();
        }
        use yrs::GetString;
        assert_eq!(
            reader
                .get_or_insert_text("html")
                .get_string(&reader.transact()),
            text.repeat(8)
        );
    }
    #[test]
    fn native_publication_new_identity_cannot_adopt_receipt_replay() {
        let mut f = Fixture::new();
        let j = job(&f, 16);
        commit(&mut f, &j).unwrap();
        let mut manifest = j.signed.manifest.clone();
        manifest.operation_id = "40000000-0000-4000-8000-000000000017".into();
        manifest.base_revision = revision(&f);
        let changed = Job {
            signed: resign(&f, manifest),
            packet: j.packet.clone(),
            chain: j.chain.clone(),
        };
        let before = effects(&f);
        rejected(
            &commit(&mut f, &changed).unwrap().record,
            Rejection::StreamGap,
        );
        assert_eq!(effects(&f), before);
        assert_eq!(terminals(&f), 2);
    }
    #[test]
    fn native_publication_original_epoch_can_be_stale_only_for_exact_replay() {
        let mut f = Fixture::new();
        let j = job(&f, 18);
        let original = commit(&mut f, &j).unwrap().record.bytes;
        f.sql()
            .execute("UPDATE pages SET epoch='2' WHERE page=?", [PAGE])
            .unwrap();
        let before = effects(&f);
        assert_eq!(commit(&mut f, &j).unwrap().record.bytes, original);
        assert_eq!(status(&f, &j).unwrap().bytes, original);
        let mut manifest = j.signed.manifest.clone();
        manifest.operation_id = "40000000-0000-4000-8000-000000000019".into();
        let changed = Job {
            signed: resign(&f, manifest),
            packet: j.packet.clone(),
            chain: j.chain.clone(),
        };
        rejected(
            &commit(&mut f, &changed).unwrap().record,
            Rejection::StaleBase,
        );
        assert_eq!(effects(&f), before);
    }
    #[test]
    fn native_publication_missing_effect_state_is_durable_only_after_admission() {
        let mut f = Fixture::new();
        let j = job(&f, 20);
        // Independent missing-content-state fixture; retained owner authority still admits this writer.
        f.sql().execute_batch("DELETE FROM receipts; DELETE FROM streams; DELETE FROM wraps; DELETE FROM epoch_secrets; DELETE FROM pages").unwrap();
        let before = effects(&f);
        let result = commit(&mut f, &j).unwrap();
        rejected(&result.record, Rejection::StateMissing);
        assert_eq!(effects(&f), before);
        assert_eq!(terminals(&f), 1);
        assert_eq!(status(&f, &j).unwrap().bytes, result.record.bytes);
    }
}
