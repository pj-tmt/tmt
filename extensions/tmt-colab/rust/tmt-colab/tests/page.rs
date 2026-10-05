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
