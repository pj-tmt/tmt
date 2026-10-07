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
    retain_diagnostic: bool,
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
            retain_diagnostic: false,
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
    fn prepare(
        &self,
        source: &str,
        revision: Option<&str>,
    ) -> tmt_colab::Result<page::FrozenPublication> {
        self.prepare_at(source, revision, NOW)
    }
    fn prepare_at(
        &self,
        source: &str,
        revision: Option<&str>,
        now: u64,
    ) -> tmt_colab::Result<page::FrozenPublication> {
        match page::prepare_publication(
            &self.store,
            &self.key,
            PAGE,
            tmt_colab::decoder::ContentEdit {
                source,
                publisher_agent: None,
            },
            revision,
            &mut self.decoder(),
            now,
        )? {
            page::PublicationPreparation::Write(frozen) => Ok(frozen),
            page::PublicationPreparation::Noop { .. } => {
                panic!("the fixture expected a write, but the source is unchanged")
            }
        }
    }
    fn commit(
        &mut self,
        frozen: &page::FrozenPublication,
        now: u64,
    ) -> tmt_colab::Result<page::PublicationCommitted> {
        page::commit_publication(
            &mut self.store,
            &self.key,
            frozen.job(),
            frozen.packet(),
            frozen.chain(),
            now,
        )
    }
    fn write(&mut self, source: &str) -> page::Receipt {
        let frozen = self.prepare(source, None).unwrap();
        let committed = self.commit(&frozen, NOW).unwrap();
        page::publication_receipt(frozen.job(), &committed.record).unwrap()
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
        if !self.retain_diagnostic {
            fs::remove_dir_all(&self.root).unwrap();
        }
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
        assert!(!String::from_utf8_lossy(p.packet()).contains("new 🐈"));
        let committed = f.commit(&p, NOW).unwrap();
        assert_eq!(committed.accepted, Accepted::New);
        let receipt = page::publication_receipt(p.job(), &committed.record).unwrap();
        let publication = receipt.publication.as_ref().unwrap();
        let result = f.read();
        assert_eq!(result.source, source);
        assert_eq!(result.title, "Exact title 🐈");
        assert_eq!(result.revision, receipt.revision);
        assert_eq!(result.membership_head.revision, "2");
        let chain = certificate::Chain::from_json(p.chain()).unwrap();
        let signing_key = chain.certificate().unwrap().signing_key;
        let entries = p.job().verify_packet(p.packet(), signing_key).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(publication.count, 1);
        let entry = &entries[0];
        assert_eq!(entry.header.context.author_device, publication.stream_id);
        assert_eq!(entry.header.context.namespace, "content");
        assert_eq!(
            entry.envelope.hash().unwrap().as_slice(),
            values::binary(&publication.envelope_hash, 32).unwrap()
        );
        assert!(
            !object::open(
                &entry.envelope,
                &entry.header.context,
                &[11; 32],
                signing_key
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
    use tmt_colab::publication::{Outcome, Rejection};
    let mut f = Fixture::new();
    let base = f.read().revision;
    // Both real children finish before either writer commits: deterministic race barrier.
    let first = f.prepare("one", Some(&base)).unwrap();
    let second = f.prepare("two", Some(&base)).unwrap();
    let accepted = f.commit(&first, NOW).unwrap();
    assert!(matches!(accepted.record.outcome, Outcome::Committed { .. }));
    // The loser records its own terminal refusal and changes nothing else.
    let before = f.read();
    let loser = f.commit(&second, NOW).unwrap();
    assert_eq!(
        loser.record.outcome,
        Outcome::Rejected {
            key: second.job().key().unwrap(),
            code: Rejection::StaleBase
        }
    );
    assert_eq!(
        page::publication_receipt(second.job(), &loser.record)
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::StaleBase)
    );
    assert_eq!(f.read().source, "one");
    assert_eq!(f.read().revision, before.revision);
    assert_eq!(
        f.prepare("three", Some(&base))
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::StaleBase)
    );
    f.write("later");
    let before = f.bytes();
    let replay = f.commit(&first, NOW).unwrap();
    assert_eq!(replay.accepted, Accepted::Replay);
    assert_eq!(replay.record.bytes, accepted.record.bytes);
    assert_eq!(f.bytes(), before);
    assert_eq!(f.read().source, "later");
    // Changed evidence is not the original operation: the signature no longer covers it.
    let mut changed = serde_json::to_value(first.job()).unwrap();
    changed["manifest"]["nativeEvidence"]["sourceSha256"] = Value::String("0".repeat(64));
    let changed =
        tmt_colab::publication::SignedJob::from_json(&serde_json::to_vec(&changed).unwrap());
    assert!(
        changed.is_err()
            || page::commit_publication(
                &mut f.store,
                &f.key,
                &changed.unwrap(),
                first.packet(),
                first.chain(),
                NOW
            )
            .is_err()
    );
    assert_eq!(f.bytes(), before);
}
#[test]
fn late_failure_rolls_back_device_append_and_receipt_and_revocation_denies_replay() {
    let mut f = Fixture::new();
    let p = f.prepare("draft", None).unwrap();
    f.sql().execute_batch("CREATE TRIGGER reject_page_receipt BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(ABORT,'fixture refusal'); END").unwrap();
    let before = f.bytes();
    assert!(f.commit(&p, NOW).is_err());
    assert_eq!(f.bytes(), before);
    assert_eq!(f.read().source, "old 🐈\r\n");
    f.sql()
        .execute_batch("DROP TRIGGER reject_page_receipt")
        .unwrap();
    let committed = f.commit(&p, NOW).unwrap();
    let r = page::publication_receipt(p.job(), &committed.record).unwrap();
    let stream_id = r.publication.unwrap().stream_id;
    let db = f.sql();
    let record: Vec<u8> = db
        .query_row(
            "SELECT record FROM devices WHERE id=?",
            [&stream_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut device: Device = serde_json::from_slice(&record).unwrap();
    device.revoked = true;
    db.execute(
        "UPDATE devices SET record=? WHERE id=?",
        rusqlite::params![serde_json::to_vec(&device).unwrap(), stream_id],
    )
    .unwrap();
    let before = f.bytes();
    assert_eq!(
        f.commit(&p, NOW).err().unwrap().downcast_ref::<Fault>(),
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
fn ciphertext_tampering_and_oversized_source_apply_nothing() {
    let mut f = Fixture::new();
    let p = f.prepare("test", None).unwrap();
    let before = f.bytes();
    // A flipped byte in the sealed packet no longer matches the signed packet hash.
    let mut packet = p.packet().to_vec();
    let middle = packet.len() / 2;
    packet[middle] ^= 1;
    assert!(
        page::commit_publication(&mut f.store, &f.key, p.job(), &packet, p.chain(), NOW).is_err()
    );
    assert_eq!(f.bytes(), before);
    let too_large = "x".repeat(tmt_colab::decoder::BASELINE_BYTES + 1);
    assert_eq!(
        f.prepare(&too_large, None)
            .err()
            .unwrap()
            .downcast_ref::<page::SourceTooLarge>(),
        Some(&page::SourceTooLarge::exact(
            tmt_colab::decoder::BASELINE_BYTES + 1,
            tmt_colab::decoder::BASELINE_BYTES
        ))
    );
    assert_eq!(f.bytes(), before);
}

#[test]
fn expired_local_certificate_renews_same_device_atomically() {
    let mut f = Fixture::new();
    let original = f.write("before expiry");
    let original = original.publication.unwrap();
    let later = NOW + tmt_colab::registration::CERTIFICATE_MS;
    let prepared = f.prepare_at("after expiry", None, later).unwrap();
    let renewed = certificate::Chain::from_json(prepared.chain()).unwrap();
    let cert = renewed.certificate().unwrap();
    assert_eq!(cert.device_id, original.stream_id);
    assert_eq!(cert.issued_at, later);
    assert_eq!(
        cert.expires_at,
        later + tmt_colab::registration::CERTIFICATE_MS
    );
    f.sql().execute_batch("CREATE TRIGGER reject_renewal BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(ABORT,'fixture refusal'); END").unwrap();
    let before = f.bytes();
    assert!(f.commit(&prepared, later).is_err());
    assert_eq!(f.bytes(), before);
    f.sql()
        .execute_batch("DROP TRIGGER reject_renewal")
        .unwrap();
    let committed = f.commit(&prepared, later).unwrap();
    let receipt = page::publication_receipt(prepared.job(), &committed.record).unwrap();
    let publication = receipt.publication.unwrap();
    assert_eq!(publication.stream_id, original.stream_id);
    assert_eq!(publication.seq, "2");
    assert_eq!(f.read().source, "after expiry");
    let record: Vec<u8> = f
        .sql()
        .query_row(
            "SELECT record FROM devices WHERE id=?",
            [&publication.stream_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut device: Device = serde_json::from_slice(&record).unwrap();
    assert_eq!(device.chain, prepared.chain());
    device.revoked = true;
    f.sql()
        .execute(
            "UPDATE devices SET record=? WHERE id=?",
            rusqlite::params![serde_json::to_vec(&device).unwrap(), publication.stream_id],
        )
        .unwrap();
    let before = f.bytes();
    assert_eq!(
        f.prepare_at("revoked", None, cert.expires_at)
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
    assert_eq!(
        f.prepare("CLI replacement", Some(&base))
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::StaleBase)
    );
    let stale = f.commit(&prepared, NOW).unwrap();
    assert!(matches!(
        stale.record.outcome,
        tmt_colab::publication::Outcome::Rejected {
            code: tmt_colab::publication::Rejection::StaleBase,
            ..
        }
    ));
    assert_eq!(f.read().source, "browser old 🐈\r\n");
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
            until: None,
        },
    )
    .unwrap()
}

#[test]
fn a_combine_past_its_deadline_publishes_nothing_so_the_page_never_moves_after_its_reply() {
    let mut f = Fixture::new();
    let mut source = String::from("old 🐈\r\n");
    for i in 0..3 {
        source.push_str(&format!("<p>{i}</p>"));
        f.write(&source);
    }
    let late = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(1))
        .unwrap();
    let revision = f.read().revision;
    let before = f.bytes();
    let mut decoder = f.decoder();
    let combine = |f: &mut Fixture, decoder: &mut Decoder, until| {
        page::compact::compact(
            &mut f.store,
            &f.key,
            PAGE,
            decoder,
            page::compact::Trigger {
                updates: 1,
                bytes: usize::MAX,
                until,
            },
        )
        .unwrap()
    };
    assert_eq!(combine(&mut f, &mut decoder, Some(late)), None);
    assert_eq!(f.bytes(), before, "a late combine changed the page state");
    assert_eq!(f.read().revision, revision);
    // Positive control: the same combine with time left does publish and moves the revision.
    assert!(combine(&mut f, &mut decoder, None).is_some());
    assert_ne!(f.read().revision, revision);
    assert_eq!(f.read().source, source);
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
                until: None,
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
    pub(super) fn signer(f: &Fixture, label: &[u8]) -> SigningKey {
        let seed = fs::read(f.layout.directory.join("owner.key")).unwrap();
        SigningKey::from_bytes(&crypto::derive_key(
            &seed,
            &[],
            &framing::frame(&[label, f.key.space_id.as_bytes()]).unwrap(),
        ))
    }
    pub(super) fn device(f: &Fixture) -> String {
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
    pub(super) fn chain(f: &Fixture, issued: u64) -> Vec<u8> {
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
    pub(super) fn revision(f: &Fixture) -> String {
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
    pub(super) fn effects(f: &Fixture) -> Vec<String> {
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
        f.sql()
            .execute("UPDATE pages SET last_update_at_ms=1000", [])
            .unwrap();
        f.store = Store::open(&f.layout).unwrap().with_clock(|| Ok(NOW + 7));
        let foreign: Vec<u8> = f
            .sql()
            .query_row(
                "SELECT payload FROM receipts WHERE stream=?",
                [DEVICE],
                |r| r.get(0),
            )
            .unwrap();
        let j = job(&f, 1);
        let result = commit(&mut f, &j).unwrap();
        assert_eq!(result.accepted, Accepted::New);
        assert_eq!(terminals(&f), 1);
        assert_eq!(
            f.sql()
                .query_row(
                    "SELECT last_update_at_ms FROM pages WHERE page=?",
                    [PAGE],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            (NOW + 7) as i64
        );
        assert_eq!(
            f.sql()
                .query_row(
                    "SELECT payload FROM receipts WHERE stream=?",
                    [DEVICE],
                    |r| r.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
            foreign
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
        f.sql()
            .execute("UPDATE pages SET last_update_at_ms=1000", [])
            .unwrap();
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
        f.sql()
            .execute_batch("ALTER TABLE owner_operations RENAME TO unavailable_original_operations")
            .unwrap();
        assert!(
            status(&f, &j).is_err(),
            "a failed SQL read is not UNKNOWN absence"
        );
        assert!(commit(&mut f, &j).is_err());
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

// Native snapshot-to-sealed-intent composition; production CLI/IPC remain unintegrated.
mod native_preparation {
    use super::native_publication::{chain, device, signer};
    use super::*;
    use rusqlite::{OptionalExtension, params};
    use tmt_colab::{
        decoder::ContentEdit,
        page::{FrozenPublication, PublicationPreparation},
        publication::Outcome,
    };
    use tmt_colab_model::crypto;
    use yrs::GetString;

    fn prepare(
        f: &Fixture,
        source: &str,
        publisher: Option<&str>,
        expected: Option<&str>,
    ) -> tmt_colab::Result<PublicationPreparation> {
        page::prepare_publication(
            &f.store,
            &f.key,
            PAGE,
            ContentEdit {
                source,
                publisher_agent: publisher,
            },
            expected,
            &mut Decoder::new(BINARY.into()).unwrap(),
            NOW,
        )
    }

    fn observation_custody<T>(result: &tmt_colab::Result<T>) -> &'static str {
        use tmt_colab::decoder::DecodeFault;
        match result {
            Ok(_) => "completed",
            Err(error) => match error.downcast_ref::<DecodeFault>() {
                Some(DecodeFault::Invoke(error)) => match error.cleanup {
                    tmt_invoke::Cleanup::NotStarted => "not-started",
                    tmt_invoke::Cleanup::Confirmed => "confirmed",
                    _ => "unknown",
                },
                // Mapped capacity and every other opaque error may have erased cleanup provenance.
                _ => "unknown",
            },
        }
    }

    #[test]
    fn observation_custody_requires_success_or_explicit_original_cleanup() {
        use tmt_colab::decoder::DecodeFault;
        use tmt_colab::store::owner::OwnerFault;
        use tmt_invoke::{Cleanup, FailureKind, InvokeError};
        assert_eq!(
            observation_custody(&Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())),
            "completed"
        );
        for (cleanup, expected) in [
            (Cleanup::NotStarted, "not-started"),
            (Cleanup::Confirmed, "confirmed"),
            (Cleanup::CallerOwned, "unknown"),
            (
                Cleanup::Unconfirmed(std::io::Error::other("original-cleanup")),
                "unknown",
            ),
        ] {
            let result: tmt_colab::Result<()> = Err(DecodeFault::Invoke(InvokeError {
                kind: FailureKind::Deadline,
                cause: Some(std::io::Error::other("original-cause")),
                cleanup,
            })
            .into());
            assert_eq!(observation_custody(&result), expected);
            // Classification borrows the original error; it cannot replace its cause or cleanup.
            let Some(DecodeFault::Invoke(original)) =
                result.as_ref().unwrap_err().downcast_ref::<DecodeFault>()
            else {
                panic!("original Invoke error replaced");
            };
            assert_eq!(original.kind, FailureKind::Deadline);
            assert_eq!(
                original.cause.as_ref().unwrap().to_string(),
                "original-cause"
            );
            if let Cleanup::Unconfirmed(cause) = &original.cleanup {
                assert_eq!(cause.to_string(), "original-cleanup");
            }
        }
        let mapped: tmt_colab::Result<()> = Err(OwnerFault::too_large(
            PAGE,
            "decoding its changes did not finish within 2 s".into(),
        )
        .into());
        assert_eq!(observation_custody(&mapped), "unknown");
        assert!(
            mapped
                .as_ref()
                .unwrap_err()
                .downcast_ref::<OwnerFault>()
                .is_some()
        );
        for error in [DecodeFault::InvalidOutput, DecodeFault::CleanupBlocked] {
            let result: tmt_colab::Result<()> = Err(error.into());
            assert_eq!(observation_custody(&result), "unknown");
        }
        let opaque: tmt_colab::Result<()> = Err(std::io::Error::other("opaque failure").into());
        assert_eq!(observation_custody(&opaque), "unknown");
        // This pure classifier performs no root reads, removals or runner reuse.
    }

    fn observation_metadata(path: &std::path::Path, value: &Value) {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(&serde_json::to_vec_pretty(value).unwrap())
            .unwrap();
    }

    fn observation_bytes(path: &std::path::Path, limit: usize) -> Option<Vec<u8>> {
        use std::io::Read;
        let mut bytes = Vec::new();
        fs::File::open(path)
            .ok()?
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() <= limit).then_some(bytes)
    }

    #[test]
    #[ignore = "supplementary child stage observation requires explicit compiler/native allocation"]
    fn native_preparation_observe_source_and_own_deadline_paths() {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        use tmt_colab::decoder::{Config, DecodeFault};
        let artifact_root = fs::canonicalize(PathBuf::from(
            std::env::var_os("TMT_COLAB_OBSERVER_ARTIFACT_ROOT")
                .expect("explicit observer artifact root"),
        ))
        .unwrap();
        let program = artifact_root.join("program");
        assert!(program.is_absolute());
        for own in [false, true] {
            let mut f = Fixture::new();
            // Choose custody before any helper launch. Panic or unknown cleanup still retains it.
            f.retain_diagnostic = true;
            fs::set_permissions(&f.root, fs::Permissions::from_mode(0o700)).unwrap();
            if own {
                own_checkpoint(&mut f, false);
            }
            let text = if own {
                "new".to_owned()
            } else {
                "x".repeat(tmt_colab::decoder::BASELINE_BYTES)
            };
            let before_state = f.bytes();
            let before: std::collections::BTreeSet<_> = fs::read_dir(&artifact_root)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            let mut decoder = Decoder::with_config(Config::new(program.clone())).unwrap();
            let result = page::prepare_publication(
                &f.store,
                &f.key,
                PAGE,
                ContentEdit {
                    source: &text,
                    publisher_agent: None,
                },
                None,
                &mut decoder,
                NOW,
            );
            let custody = observation_custody(&result);
            let deadline = matches!(result.as_ref().err().and_then(|error| error.downcast_ref::<DecodeFault>()),
                Some(DecodeFault::Invoke(error)) if error.kind == tmt_invoke::FailureKind::Deadline);
            let case = if own { "own-heavy" } else { "source-boundary" };
            // Do not inspect/reuse/delete roots while a helper may still own them.
            if custody == "unknown" {
                observation_metadata(
                    &f.root.join("observation-custody.json"),
                    &json!({
                        "case":case,"custody":"unknown","evidence_read":false,"retained":true
                    }),
                );
                panic!("observation custody unknown; isolated roots retained");
            }
            let mut records = Vec::new();
            for entry in fs::read_dir(&artifact_root).unwrap() {
                let root = entry.unwrap().path();
                if before.contains(&root) || !root.is_dir() {
                    continue;
                }
                let request =
                    observation_bytes(&root.join("request.json"), tmt_colab::decoder::STREAM_BYTES);
                let request_complete = root.join("request.complete").exists();
                let input_hash = request
                    .as_ref()
                    .filter(|bytes| {
                        request_complete && bytes.len() <= tmt_colab::decoder::STREAM_BYTES
                    })
                    .map(|bytes| {
                        base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .encode(Sha256::digest(bytes))
                    });
                let reply_complete = root.join("reply.complete").exists();
                let reply =
                    observation_bytes(&root.join("reply.json"), tmt_colab::decoder::STREAM_BYTES);
                let reply_value = reply
                    .as_ref()
                    .filter(|bytes| {
                        reply_complete && bytes.len() <= tmt_colab::decoder::STREAM_BYTES
                    })
                    .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());
                let correlation = reply_value
                    .as_ref()
                    .and_then(|reply| reply["input_hash"].as_str())
                    .zip(input_hash.as_deref())
                    .map(|(actual, expected)| actual == expected);
                if let Some(equal) = correlation {
                    assert!(equal, "actual original reply/input correlation");
                }
                let progress =
                    observation_bytes(&root.join("progress.txt"), 96 * 1024).unwrap_or_default();
                let mut reached = Vec::new();
                let mut rows_valid = progress.len() <= 96 * 1024
                    && progress.is_ascii()
                    && (progress.is_empty() || progress.last() == Some(&b'\n'));
                for (index, row) in progress.split_inclusive(|v| *v == b'\n').enumerate() {
                    if row.len() > 96 || index >= 1024 {
                        rows_valid = false;
                        break;
                    }
                    let line = std::str::from_utf8(row).unwrap_or("");
                    let fields: Vec<_> = line.split_whitespace().collect();
                    if fields.len() != 5
                        || fields[0] != format!("seq={}", index + 1)
                        || !matches!(fields[2], "edge=enter" | "edge=leave")
                        || fields[3]
                            .strip_prefix("n=")
                            .and_then(|v| v.parse::<u32>().ok())
                            .is_none()
                        || fields[4]
                            .strip_prefix("ns=")
                            .and_then(|v| v.parse::<u64>().ok())
                            .is_none()
                    {
                        rows_valid = false;
                        break;
                    }
                    let stage = fields[1].strip_prefix("stage=").unwrap_or("");
                    if ![
                        "memory",
                        "input",
                        "json",
                        "binary",
                        "decode",
                        "apply",
                        "own",
                        "project",
                        "merge",
                        "edit",
                        "generation",
                        "bounds",
                        "replay",
                        "prefix",
                        "reply-build",
                        "reply-json",
                        "reply-write",
                    ]
                    .contains(&stage)
                    {
                        rows_valid = false;
                        break;
                    }
                    reached.push(line.trim().to_owned());
                }
                let quality = observation_bytes(&root.join("quality.txt"), 32).unwrap_or_default();
                let classification_valid =
                    rows_valid && !reached.is_empty() && quality == b"terminal-valid\n";
                // Killed/torn/write-failed/capped helpers provide at most a reached prefix, never a stopping-phase classification.
                let command = observation_bytes(&root.join("command.txt"), 32);
                let command = match command.as_deref() {
                    Some(b"decode") => Some("decode"),
                    Some(b"prepare-content") => Some("prepare-content"),
                    _ => None,
                };
                let namespace = request
                    .as_ref()
                    .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
                    .and_then(|wire| match wire["namespace"].as_str() {
                        Some("own") => Some("own"),
                        Some("content") => Some("content"),
                        _ => None,
                    });
                let child_input_complete = rows_valid
                    && reached.iter().any(|row| {
                        row.contains("stage=input edge=leave ")
                            && request.as_ref().is_some_and(|bytes| {
                                row.split_whitespace()
                                    .any(|field| field == format!("n={}", bytes.len()))
                            })
                    });
                records.push(json!({"command":command,"namespace":namespace,
                    "helper_entered":root.join("progress.txt").exists(),"child_input_complete":child_input_complete,
                    "request_bytes":request.as_ref().map(Vec::len),"request_complete":request_complete,
                    "actual_input_hash":input_hash,"reply_complete":reply_complete,"reply_correlation":correlation,
                    "memory_initialized":rows_valid && reached.iter().any(|row| row.contains("stage=memory edge=leave ")),
                    "classification_valid":classification_valid,"progress_rows_valid":rows_valid,
                    "reached_prefix":if rows_valid { reached } else { Vec::new() },
                    "incomplete":!classification_valid,"historical_input_identity":false}));
            }
            assert_eq!(f.bytes(), before_state);
            if let Ok(PublicationPreparation::Write(write)) = &result {
                let reader = replay(&f, write);
                assert_eq!(
                    reader
                        .get_or_insert_text("html")
                        .get_string(&reader.transact()),
                    text
                );
            }
            observation_metadata(
                &f.root.join("observation-summary.json"),
                &json!({
                    "case":case,"custody":custody,"original_error_deadline":deadline,
                    "preparation_success":result.is_ok(),"retained":true,
                    "supplementary_only":true,"original_deadline_ms":2000,"records":records
                }),
            );
            // Only a safe synthetic fixture path is printed, never raw requests/replies or private causes.
            eprintln!(
                "supplementary observation fixture retained: {}",
                f.root.display()
            );
        }
    }

    fn write(result: PublicationPreparation) -> FrozenPublication {
        match result {
            PublicationPreparation::Write(w) => w,
            PublicationPreparation::Noop { .. } => panic!("expected frozen write"),
        }
    }
    fn state(f: &Fixture) -> Vec<String> {
        let db = f.sql();
        let tables = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let mut output = Vec::new();
        for table in tables {
            let mut q = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let n = q.column_count();
            let mut rows = q
                .query_map([], |r| {
                    Ok((0..n)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            rows.sort();
            output.push(format!("{table}:{rows:?}"));
        }
        output
    }
    fn put_chain(f: &mut Fixture, bytes: Vec<u8>, revoked: bool) {
        let id = certificate::Chain::from_json(&bytes)
            .unwrap()
            .certificate()
            .unwrap()
            .device_id
            .to_owned();
        f.sql().execute("INSERT INTO devices(id,record) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET record=excluded.record",params![id,serde_json::to_vec(&Device {chain:bytes,revoked}).unwrap()]).unwrap();
    }
    fn append(
        f: &mut Fixture,
        local: bool,
        namespace: Namespace,
        bytes: &[u8],
    ) -> object::Envelope {
        let id = if local { device(f) } else { DEVICE.into() };
        let prior: Option<(String, Vec<u8>)> = f
            .sql()
            .query_row(
                "SELECT seq,hash FROM receipts WHERE page=? AND stream=? ORDER BY seq DESC LIMIT 1",
                params![PAGE, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap();
        let (seq, previous) = prior.map_or((1, [0; 32]), |(s, h)| {
            (s.parse::<u64>().unwrap() + 1, h.try_into().unwrap())
        });
        let sign = if local {
            signer(f, b"tmt-colab-cli-signing-seed-v1")
        } else {
            SigningKey::from_bytes(&[9; 32])
        };
        let head = f
            .store
            .owner_head(&f.key.space_id, &f.key.owner_public())
            .unwrap()
            .unwrap();
        let envelope = object::seal(
            &object::Context {
                space: f.key.space_id.clone(),
                page: PAGE.into(),
                epoch: "1".into(),
                kind: "update".into(),
                namespace: match namespace {
                    Namespace::Content => "content",
                    Namespace::Own => "own",
                }
                .into(),
                author_device: id.clone(),
                membership_revision: head.revision.to_string(),
                stream_seq: seq.to_string(),
                prev_hash: previous,
            },
            &[11; 32],
            &sign,
            bytes,
        )
        .unwrap();
        f.store
            .append(&Envelope {
                scope: StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream: &id,
                },
                namespace,
                seq,
                hash: envelope.hash().unwrap(),
                previous,
                bytes: &envelope.to_json().unwrap(),
            })
            .unwrap();
        envelope
    }
    fn content(f: &Fixture) -> Doc {
        let doc = Doc::with_client_id(717);
        let rows = f
            .sql()
            .prepare("SELECT payload FROM receipts WHERE namespace='content' ORDER BY stream,seq")
            .unwrap()
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        for bytes in rows {
            let e = object::Envelope::from_json(&bytes).unwrap();
            let h = object::Header::decode(e.header()).unwrap();
            let sign = if h.context.author_device == DEVICE {
                SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes()
            } else {
                signer(f, b"tmt-colab-cli-signing-seed-v1")
                    .verifying_key()
                    .to_bytes()
            };
            let update = object::open(&e, &h.context, &[11; 32], &sign).unwrap();
            doc.transact_mut()
                .apply_update(Update::decode_v1(&update).unwrap())
                .unwrap();
        }
        doc
    }
    fn own(f: &mut Fixture, local: bool, value: &str) {
        if local {
            put_chain(f, chain(f, NOW), false);
        }
        let doc = Doc::with_client_id(if local { 818 } else { 919 });
        for root in ["threads", "messages", "intents", "replies"] {
            doc.get_or_insert_map(root);
        }
        doc.get_or_insert_map("threads")
            .insert(&mut doc.transact_mut(), "legacy", value);
        append(
            f,
            local,
            Namespace::Own,
            &doc.transact()
                .encode_state_as_update_v1(&StateVector::default()),
        );
    }
    fn replay(f: &Fixture, w: &FrozenPublication) -> Doc {
        let doc = content(f);
        let key = signer(f, b"tmt-colab-cli-signing-seed-v1")
            .verifying_key()
            .to_bytes();
        let verified = w.job().verify_packet(w.packet(), &key).unwrap();
        let mut offset = 0;
        for e in verified {
            assert_eq!(e.bytes, &w.packet()[offset..offset + e.bytes.len()]);
            offset += e.bytes.len();
            let raw = object::open(&e.envelope, &e.header.context, &[11; 32], &key).unwrap();
            doc.transact_mut()
                .apply_update(Update::decode_v1(&raw).unwrap())
                .unwrap();
            assert!(doc.transact().store().pending_update().is_none());
            assert!(doc.transact().store().pending_ds().is_none());
        }
        assert_eq!(offset, w.packet().len());
        doc
    }
    #[test]
    fn native_preparation_unicode_packet_is_causal_frozen_and_has_no_store_effect() {
        let mut f = Fixture::new();
        own(&mut f, false, "foreign own");
        own(&mut f, true, "local own");
        let before = state(&f);
        let original = content(&f);
        let clocks = original.transact().state_vector();
        let text = "🌱".repeat(393_216);
        assert_eq!(text.len(), 1_572_864);
        let w = write(prepare(&f, &text, Some("New publisher"), Some(&f.read().revision)).unwrap());
        assert_eq!(state(&f), before);
        assert!(w.job().manifest.entries.len() > 1);
        let m = &w.job().manifest;
        assert_eq!(m.packet_bytes, w.packet().len());
        assert_eq!(
            values::binary(&m.packet_hash, 32).unwrap(),
            crypto::digest(w.packet())
        );
        assert_eq!(
            m.entries[0].seq, "2",
            "shared stream follows local own sequence"
        );
        let r = replay(&f, &w);
        assert_eq!(r.get_or_insert_text("html").get_string(&r.transact()), text);
        assert_eq!(
            r.get_or_insert_map("meta")
                .get(&r.transact(), "title")
                .unwrap()
                .to_string(&r.transact()),
            "Exact title 🐈"
        );
        assert_eq!(
            r.get_or_insert_map("meta")
                .get(&r.transact(), "publisherAgent")
                .unwrap()
                .to_string(&r.transact()),
            "New publisher"
        );
        for (client, clock) in clocks.iter() {
            assert_eq!(r.transact().state_vector().get(client), *clock);
        }
        let e = m.native_evidence.as_ref().unwrap();
        assert_eq!(
            e.source_sha256,
            crypto::digest(text.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert_eq!(
            e.chain_hash,
            values::encode_binary(&crypto::digest(w.chain()))
        );
        assert_eq!(e.memory_limit, tmt_colab::decoder::memory_limit());
        let detached = Doc::new();
        for e in w
            .job()
            .verify_packet(
                w.packet(),
                signer(&f, b"tmt-colab-cli-signing-seed-v1")
                    .verifying_key()
                    .as_bytes(),
            )
            .unwrap()
        {
            let bytes = object::open(
                &e.envelope,
                &e.header.context,
                &[11; 32],
                signer(&f, b"tmt-colab-cli-signing-seed-v1")
                    .verifying_key()
                    .as_bytes(),
            )
            .unwrap();
            detached
                .transact_mut()
                .apply_update(Update::decode_v1(&bytes).unwrap())
                .unwrap();
        }
        assert!(
            detached.transact().store().pending_update().is_some()
                || detached.transact().store().pending_ds().is_some()
        );
        let frozen = w.job().key().unwrap();
        let bytes = w.packet().to_vec();
        let out =
            page::commit_publication(&mut f.store, &f.key, w.job(), w.packet(), w.chain(), NOW)
                .unwrap();
        assert!(matches!(out.record.outcome, Outcome::Committed { .. }));
        assert_eq!(frozen, w.job().key().unwrap());
        assert_eq!(w.packet(), bytes);
        assert_eq!(f.read().source, text);
    }
    #[test]
    fn native_preparation_noop_never_issues_or_renews_a_certificate() {
        for expiry in [None, Some(NOW - 86_400_001)] {
            let mut f = Fixture::new();
            if let Some(at) = expiry {
                let bytes = chain(&f, at);
                put_chain(&mut f, bytes, false);
            }
            let before = state(&f);
            let expected = f.read().revision;
            let p = prepare(&f, "old 🐈\r\n", None, Some(&expected)).unwrap();
            assert!(
                matches!(p,PublicationPreparation::Noop{base_revision,memory_limit,..} if base_revision==expected&&memory_limit==tmt_colab::decoder::memory_limit())
            );
            assert_eq!(state(&f), before);
            let w = write(prepare(&f, "changed", None, None).unwrap());
            assert!(
                certificate::Chain::from_json(w.chain())
                    .unwrap()
                    .certificate()
                    .unwrap()
                    .expires_at
                    > NOW
            );
            assert_eq!(state(&f), before);
        }
    }
    #[test]
    fn native_preparation_publisher_only_set_clear_preserves_metadata_and_own() {
        let mut f = Fixture::new();
        own(&mut f, false, "keep foreign");
        own(&mut f, true, "keep local");
        for label in [Some("Publisher"), None] {
            let source = f.read().source;
            let before = state(&f);
            let w = write(prepare(&f, &source, label, None).unwrap());
            assert_eq!(state(&f), before);
            let r = replay(&f, &w);
            assert_eq!(
                r.get_or_insert_text("html").get_string(&r.transact()),
                source
            );
            assert_eq!(
                r.get_or_insert_map("meta")
                    .get(&r.transact(), "title")
                    .unwrap()
                    .to_string(&r.transact()),
                "Exact title 🐈"
            );
            page::commit_publication(&mut f.store, &f.key, w.job(), w.packet(), w.chain(), NOW)
                .unwrap();
            let p = prepare(&f, &source, label, None).unwrap();
            assert!(matches!(p, PublicationPreparation::Noop { .. }));
            // Independent own records survive the content write byte-exact.
            let rows = f
                .sql()
                .prepare("SELECT payload FROM receipts WHERE namespace='own' ORDER BY stream,seq")
                .unwrap()
                .query_map([], |r| r.get::<_, Vec<u8>>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            assert_eq!(rows.len(), 2);
        }
    }
    fn signed_change(f: &mut Fixture, action: &str, payload: Value) {
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
                    operation_id: "90000000-0000-4000-8000-000000000001",
                    digest: [42; 32],
                    expected_revision: head.revision,
                },
                |tx| {
                    let statement = f.key.sign_statement(
                        tx.head(),
                        action,
                        &serde_json::to_vec(&payload).unwrap(),
                    )?;
                    tx.append_statement(&statement)?;
                    Ok(vec![])
                },
            )
            .unwrap();
    }
    #[test]
    fn native_preparation_stale_inactive_revoked_and_malformed_authority_fail_closed() {
        let f = Fixture::new();
        let before = state(&f);
        assert_eq!(
            prepare(&f, "old 🐈\r\n", None, Some("v1:stale"))
                .err()
                .unwrap()
                .downcast_ref::<Fault>(),
            Some(&Fault::StaleBase)
        );
        assert_eq!(state(&f), before);
        for action in ["page.archive", "page.delete"] {
            let mut f = Fixture::new();
            signed_change(&mut f, action, json!({"pageId":PAGE}));
            let before = state(&f);
            assert!(prepare(&f, "old 🐈\r\n", None, None).is_err());
            assert_eq!(state(&f), before);
        }
        for signed in [false, true] {
            let mut f = Fixture::new();
            let bytes = chain(&f, NOW);
            put_chain(&mut f, bytes, !signed);
            if signed {
                let id = device(&f);
                signed_change(&mut f, "device.revoke", json!({"deviceId":id,"cuts":[]}));
            }
            let before = state(&f);
            assert_eq!(
                prepare(&f, "old 🐈\r\n", None, None)
                    .err()
                    .unwrap()
                    .downcast_ref::<Fault>(),
                Some(&Fault::Denied)
            );
            assert_eq!(state(&f), before);
        }
        let f = Fixture::new();
        f.sql()
            .execute(
                "INSERT INTO devices VALUES(?,?)",
                params![
                    "40000000-0000-4000-8000-000000000001",
                    serde_json::to_vec(&Device {
                        chain: b"{}".to_vec(),
                        revoked: false
                    })
                    .unwrap()
                ],
            )
            .unwrap();
        let before = state(&f);
        assert!(prepare(&f, "changed", None, None).is_err());
        assert_eq!(state(&f), before);
    }
    #[test]
    fn native_preparation_reuses_exact_chain_and_frozen_job_is_not_refreshed_on_stale_commit() {
        let mut f = Fixture::new();
        let original = chain(&f, NOW);
        let original = format!(" {}\n", String::from_utf8(original).unwrap()).into_bytes();
        put_chain(&mut f, original.clone(), false);
        let before = state(&f);
        let w = write(prepare(&f, "new", None, None).unwrap());
        assert_eq!(w.chain(), original);
        assert_eq!(state(&f), before);
        let key = w.job().key().unwrap();
        let job_bytes = w.job().to_json().unwrap();
        // A different certified writer advances the captured shared cut after preparation.
        let doc = content(&f);
        let html = doc.get_or_insert_text("html");
        let mut tx = doc.transact_mut();
        html.insert(&mut tx, 0, "foreign ");
        let delta = tx.encode_update_v1();
        drop(tx);
        append(&mut f, false, Namespace::Content, &delta);
        let out =
            page::commit_publication(&mut f.store, &f.key, w.job(), w.packet(), w.chain(), NOW)
                .unwrap();
        assert!(matches!(
            out.record.outcome,
            Outcome::Rejected {
                code: tmt_colab::publication::Rejection::StaleBase,
                ..
            }
        ));
        assert_eq!(w.job().key().unwrap(), key);
        assert_eq!(w.job().to_json().unwrap(), job_bytes);
        let mut f = Fixture::new();
        let w = write(prepare(&f, "new", None, None).unwrap());
        let id = device(&f);
        signed_change(&mut f, "device.revoke", json!({"deviceId":id,"cuts":[]}));
        let before = state(&f);
        assert!(
            page::commit_publication(&mut f.store, &f.key, w.job(), w.packet(), w.chain(), NOW)
                .is_err()
        );
        assert_eq!(state(&f), before);
    }
    #[test]
    fn native_preparation_count_boundary_allows_noop_but_rejects_new_tail() {
        let mut f = Fixture::new();
        let bytes = chain(&f, NOW);
        put_chain(&mut f, bytes, false);
        let doc = Doc::with_client_id(1828);
        for root in ["threads", "messages", "intents", "replies"] {
            doc.get_or_insert_map(root);
        }
        let threads = doc.get_or_insert_map("threads");
        for i in 0..198 {
            let mut tx = doc.transact_mut();
            threads.insert(&mut tx, "legacy", i.to_string());
            let delta = tx.encode_update_v1();
            drop(tx);
            append(&mut f, true, Namespace::Own, &delta);
        }
        let w = write(prepare(&f, "new", None, None).unwrap());
        assert_eq!(
            w.job().manifest.entries.len(),
            1,
            "199 retained plus one new is exactly200"
        );
        let mut tx = doc.transact_mut();
        threads.insert(&mut tx, "legacy", "last");
        let delta = tx.encode_update_v1();
        drop(tx);
        append(&mut f, true, Namespace::Own, &delta);
        let before = state(&f);
        assert!(matches!(
            prepare(&f, "old 🐈\r\n", None, None).unwrap(),
            PublicationPreparation::Noop { .. }
        ));
        assert!(prepare(&f, "new", None, None).is_err());
        assert_eq!(state(&f), before);
    }
    #[test]
    fn native_preparation_source_boundary_and_exact_packet_tampering() {
        let f = Fixture::new();
        let text = "x".repeat(tmt_colab::decoder::BASELINE_BYTES);
        let w = write(prepare(&f, &text, None, None).unwrap());
        let reader = replay(&f, &w);
        assert_eq!(
            reader
                .get_or_insert_text("html")
                .get_string(&reader.transact()),
            text
        );
        let before = state(&f);
        assert_eq!(
            prepare(&f, &format!("{text}x"), None, None)
                .err()
                .unwrap()
                .downcast_ref::<page::SourceTooLarge>(),
            Some(&page::SourceTooLarge::exact(
                tmt_colab::decoder::BASELINE_BYTES + 1,
                tmt_colab::decoder::BASELINE_BYTES
            ))
        );
        assert_eq!(state(&f), before);
        let key = signer(&f, b"tmt-colab-cli-signing-seed-v1")
            .verifying_key()
            .to_bytes();
        let mut packet = w.packet().to_vec();
        packet[0] ^= 1;
        assert!(w.job().verify_packet(&packet, &key).is_err());
        let mut j = w.job().clone();
        j.manifest.packet_hash = values::encode_binary(&[0; 32]);
        assert!(j.verify_packet(w.packet(), &key).is_err());
        // Isolate structural guards on the actual composed manifest before signing.
        let mut envelope_limit = w.job().manifest.clone();
        let old = envelope_limit.entries[0].envelope_bytes;
        envelope_limit.entries[0].envelope_bytes = tmt_colab::limits::UPDATE_BYTES;
        envelope_limit.packet_bytes += tmt_colab::limits::UPDATE_BYTES - old;
        assert!(envelope_limit.signature_input().is_ok());
        envelope_limit.entries[0].envelope_bytes += 1;
        envelope_limit.packet_bytes += 1;
        assert!(envelope_limit.signature_input().is_err());
        let mut count_limit = w.job().manifest.clone();
        count_limit.entries = (1..=200)
            .map(|seq| {
                let mut e = count_limit.entries[0].clone();
                e.seq = seq.to_string();
                e.envelope_bytes = 100;
                e
            })
            .collect();
        count_limit.packet_bytes = 20_000;
        assert!(count_limit.signature_input().is_ok());
        let mut extra = count_limit.entries[0].clone();
        extra.seq = "201".into();
        count_limit.entries.push(extra);
        count_limit.packet_bytes += 100;
        assert!(count_limit.signature_input().is_err());
        let mut packet_limit = count_limit.clone();
        packet_limit.entries.pop();
        let cap = tmt_colab::publication::packet_limit(200).unwrap();
        for (i, e) in packet_limit.entries.iter_mut().enumerate() {
            e.envelope_bytes = cap / 200 + usize::from(i < cap % 200);
        }
        packet_limit.packet_bytes = cap;
        assert!(packet_limit.signature_input().is_ok());
        packet_limit.entries[199].envelope_bytes += 1;
        packet_limit.packet_bytes += 1;
        assert!(packet_limit.signature_input().is_err());
        // The existing model seals the exact raw cap and refuses +1 before packet creation.
        let context = w.job().verify_packet(w.packet(), &key).unwrap()[0]
            .header
            .context
            .clone();
        for length in [
            tmt_colab::decoder::UPDATE_BYTES,
            tmt_colab::decoder::UPDATE_BYTES + 1,
        ] {
            let sealed = object::seal(
                &context,
                &[11; 32],
                &signer(&f, b"tmt-colab-cli-signing-seed-v1"),
                &vec![0; length],
            );
            if length > tmt_colab::decoder::UPDATE_BYTES {
                assert!(sealed.is_err());
                continue;
            }
            let envelope = sealed.unwrap();
            let raw = envelope.to_json().unwrap();
            assert!(raw.len() < tmt_colab::limits::UPDATE_BYTES);
            let mut manifest = w.job().manifest.clone();
            manifest.entries = vec![tmt_colab::publication::PublicationEntry {
                namespace: tmt_colab::publication::PublicationKind::Content,
                seq: context.stream_seq.clone(),
                envelope_hash: values::encode_binary(&envelope.hash().unwrap()),
                envelope_bytes: raw.len(),
            }];
            manifest.packet_bytes = raw.len();
            manifest.packet_hash = values::encode_binary(&crypto::digest(&raw));
            let signature = values::encode_binary(
                &signer(&f, b"tmt-colab-cli-signing-seed-v1")
                    .sign(&manifest.signature_input().unwrap())
                    .to_bytes(),
            );
            let job = tmt_colab::publication::SignedJob {
                manifest,
                signature,
            };
            assert_eq!(
                job.verify_packet(&raw, &key).is_ok(),
                length == tmt_colab::decoder::UPDATE_BYTES
            );
        }
        let signed = w.job().to_json().unwrap();
        let mut exact = signed.clone();
        exact.resize(tmt_colab::publication::JSON_BYTES, b' ');
        assert_eq!(
            tmt_colab::publication::SignedJob::from_json(&exact).unwrap(),
            *w.job()
        );
        exact.push(b' ');
        assert!(tmt_colab::publication::SignedJob::from_json(&exact).is_err());
        let mut j = w.job().clone();
        j.signature = values::encode_binary(&[0; 64]);
        assert!(j.verify_packet(w.packet(), &key).is_err());
        let mut j = w.job().clone();
        j.manifest.native_evidence.as_mut().unwrap().chain_hash = values::encode_binary(&[0; 32]);
        assert!(j.verify_packet(w.packet(), &key).is_err());
        for e in &w.job().manifest.entries {
            assert!(e.envelope_bytes <= tmt_colab::limits::UPDATE_BYTES);
        }
        assert!(w.job().to_json().unwrap().len() <= tmt_colab::publication::JSON_BYTES);
        assert!(
            w.packet().len()
                <= tmt_colab::publication::packet_limit(w.job().manifest.entries.len()).unwrap()
        );
        assert!(w.chain().len() <= tmt_colab::publication::CHAIN_BYTES);
    }

    fn own_checkpoint(f: &mut Fixture, entropy: bool) -> Vec<u8> {
        let bytes = chain(f, NOW);
        put_chain(f, bytes, false);
        let doc = Doc::with_client_id(2929);
        let threads = doc.get_or_insert_map("threads");
        for root in ["messages", "intents", "replies"] {
            doc.get_or_insert_map(root);
        }
        let mut random = 0x123456789abcdef0u64;
        for i in 0..32 {
            let text = (0..224 * 1024)
                .map(|_| {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    if entropy {
                        char::from(32 + (random % 95) as u8)
                    } else {
                        'x'
                    }
                })
                .collect::<String>();
            let mut tx = doc.transact_mut();
            threads.insert(&mut tx, format!("legacy-{i}"), text);
            let delta = tx.encode_update_v1();
            drop(tx);
            assert!(delta.len() < tmt_colab::decoder::UPDATE_BYTES);
            append(f, true, Namespace::Own, &delta);
        }
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(update.len() > 5_000_000);
        let id = device(f);
        let (seq, hash): (String, Vec<u8>) = f
            .sql()
            .query_row(
                "SELECT seq,hash FROM receipts WHERE stream=? ORDER BY seq DESC LIMIT 1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let seq: u64 = seq.parse().unwrap();
        let previous: [u8; 32] = hash.try_into().unwrap();
        let cp = object::seal(
            &object::Context {
                space: f.key.space_id.clone(),
                page: PAGE.into(),
                epoch: "1".into(),
                kind: "checkpoint".into(),
                namespace: "own".into(),
                author_device: id.clone(),
                membership_revision: "2".into(),
                stream_seq: seq.to_string(),
                prev_hash: previous,
            },
            &[11; 32],
            &signer(f, b"tmt-colab-cli-signing-seed-v1"),
            &update,
        )
        .unwrap();
        f.store
            .checkpoint(&Envelope {
                scope: StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream: &id,
                },
                namespace: Namespace::Own,
                seq,
                hash: cp.hash().unwrap(),
                previous,
                bytes: &cp.to_json().unwrap(),
            })
            .unwrap();
        update
    }
    #[test]
    fn native_preparation_own_heavy_gzip_admission_also_fences_legacy_edit() {
        use flate2::{Compression, write::GzEncoder};
        for entropy in [false, true] {
            let mut f = Fixture::new();
            let own = own_checkpoint(&mut f, entropy);
            let mut gz = GzEncoder::new(Vec::new(), Compression::new(6));
            gz.write_all(&own).unwrap();
            let size = gz.finish().unwrap().len();
            assert_eq!(
                size > 5_000_000,
                entropy,
                "independent own-only stream alone proves over-budget; compressible same-size positive"
            );
            let before = f.bytes();
            let batch = prepare(&f, "new", None, None);
            let legacy = f.prepare("new", None);
            if entropy {
                assert!(batch.is_err());
                assert!(legacy.is_err());
                assert!(batch.err().unwrap().to_string().contains("compressed"));
                assert!(legacy.err().unwrap().to_string().contains("compressed"));
            } else {
                assert!(matches!(batch.unwrap(), PublicationPreparation::Write(_)));
                assert!(legacy.is_ok());
            }
            assert_eq!(f.bytes(), before);
        }
    }
    fn extreme_head(f: &mut Fixture, seq: u64) {
        let bytes = chain(f, NOW);
        put_chain(f, bytes, false);
        let id = device(f);
        let prior = [88; 32];
        let doc = Doc::new();
        for root in ["threads", "messages", "intents", "replies"] {
            doc.get_or_insert_map(root);
        }
        let raw = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let cp = object::seal(
            &object::Context {
                space: f.key.space_id.clone(),
                page: PAGE.into(),
                epoch: "1".into(),
                kind: "checkpoint".into(),
                namespace: "own".into(),
                author_device: id.clone(),
                membership_revision: "2".into(),
                stream_seq: seq.to_string(),
                prev_hash: prior,
            },
            &[11; 32],
            &signer(f, b"tmt-colab-cli-signing-seed-v1"),
            &raw,
        )
        .unwrap();
        // Independent retained/pruned receipt anchor: only authenticated checkpoint bytes are folded.
        let db = f.sql();
        db.execute(
            "INSERT INTO streams VALUES(?,?,?,0)",
            params![PAGE, "1", id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO receipts VALUES(?,?,?,?,?,?,?,NULL)",
            params![
                PAGE,
                "1",
                id,
                format!("{seq:020}"),
                "own",
                prior.as_slice(),
                [0u8; 32].as_slice()
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO checkpoints VALUES(?,?,?,?,?,?,?,?,?,0)",
            params![
                PAGE,
                "1",
                id,
                "own",
                format!("{seq:020}"),
                cp.hash().unwrap().as_slice(),
                [0u8; 32].as_slice(),
                prior.as_slice(),
                cp.to_json().unwrap()
            ],
        )
        .unwrap();
    }
    #[test]
    fn native_preparation_shared_sequence_exact_u64_boundary_and_chain_raw_cap() {
        let mut f = Fixture::new();
        extreme_head(&mut f, u64::MAX - 1);
        let w = write(prepare(&f, "new", None, None).unwrap());
        assert_eq!(w.job().manifest.entries[0].seq, u64::MAX.to_string());
        let mut f = Fixture::new();
        extreme_head(&mut f, u64::MAX);
        let before = state(&f);
        assert!(matches!(
            prepare(&f, "old 🐈\r\n", None, None).unwrap(),
            PublicationPreparation::Noop { .. }
        ));
        assert_eq!(
            prepare(&f, "new", None, None)
                .err()
                .unwrap()
                .downcast_ref::<Fault>(),
            Some(&Fault::Capacity)
        );
        assert_eq!(state(&f), before);
        for length in [16 * 1024, 16 * 1024 + 1] {
            let f = Fixture::new();
            let mut bytes = chain(&f, NOW);
            bytes.resize(length, b' ');
            let id = device(&f);
            f.sql()
                .execute(
                    "INSERT INTO devices VALUES(?,?)",
                    params![
                        id,
                        serde_json::to_vec(&Device {
                            chain: bytes.clone(),
                            revoked: false
                        })
                        .unwrap()
                    ],
                )
                .unwrap();
            let before = state(&f);
            let made = prepare(&f, "new", None, None);
            if length == 16 * 1024 {
                assert_eq!(write(made.unwrap()).chain(), bytes);
            } else {
                assert!(made.is_err());
            }
            assert_eq!(state(&f), before);
        }
    }

    fn own_delta(index: u64, length: usize) -> Vec<u8> {
        let doc = Doc::with_client_id(3000 + index);
        let threads = doc.get_or_insert_map("threads");
        for root in ["messages", "intents", "replies"] {
            doc.get_or_insert_map(root);
        }
        threads.insert(
            &mut doc.transact_mut(),
            format!("legacy-{index}"),
            "x".repeat(length),
        );
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default())
    }
    #[test]
    fn native_preparation_exact_retained_raw_limit_allows_noop_and_refuses_more_bytes() {
        let mut f = Fixture::new();
        let bytes = chain(&f, NOW);
        put_chain(&mut f, bytes, false);
        let bytes: Vec<u8> = f
            .sql()
            .query_row(
                "SELECT payload FROM receipts WHERE stream=?",
                [DEVICE],
                |r| r.get(0),
            )
            .unwrap();
        let envelope = object::Envelope::from_json(&bytes).unwrap();
        let context = object::Header::decode(envelope.header()).unwrap().context;
        let raw = object::open(
            &envelope,
            &context,
            &[11; 32],
            SigningKey::from_bytes(&[9; 32]).verifying_key().as_bytes(),
        )
        .unwrap();
        let mut total = raw.len();
        let mut index = 0;
        while total + 224 * 1024 + 1024 < tmt_colab::decoder::WRITE_TAIL_BYTES {
            let delta = own_delta(index, 224 * 1024);
            total += delta.len();
            append(&mut f, true, Namespace::Own, &delta);
            index += 1;
        }
        // Positive with legal retained raw bytes and room for the actual source edit.
        let w = write(prepare(&f, "new", None, None).unwrap());
        let key = signer(&f, b"tmt-colab-cli-signing-seed-v1")
            .verifying_key()
            .to_bytes();
        let added = w
            .job()
            .verify_packet(w.packet(), &key)
            .unwrap()
            .iter()
            .map(|e| e.envelope.ciphertext().len() - 16)
            .sum::<usize>();
        assert!(total + added < tmt_colab::decoder::WRITE_TAIL_BYTES);
        let target = tmt_colab::decoder::WRITE_TAIL_BYTES - total;
        let mut length = target - 64;
        let final_delta = loop {
            let delta = own_delta(index, length);
            if delta.len() == target {
                break delta;
            }
            if delta.len() < target {
                length += target - delta.len();
            } else {
                length -= delta.len() - target;
            }
        };
        assert!(final_delta.len() < tmt_colab::decoder::UPDATE_BYTES);
        total += final_delta.len();
        append(&mut f, true, Namespace::Own, &final_delta);
        assert_eq!(total, tmt_colab::decoder::WRITE_TAIL_BYTES);
        let before = state(&f);
        assert!(matches!(
            prepare(&f, "old 🐈\r\n", None, None).unwrap(),
            PublicationPreparation::Noop { .. }
        ));
        let error = prepare(&f, "new", None, None).err().unwrap();
        assert!(error.to_string().contains("changes"));
        assert_eq!(state(&f), before);
    }
    #[test]
    fn native_preparation_invalid_child_output_returns_no_intent_and_confirms_cleanup() {
        let f = Fixture::new();
        let program = f.root.join("bad-preparation");
        let script = format!(
            r#"#!/usr/bin/python3
import json, pathlib, subprocess, sys
marker=pathlib.Path({marker})
child=subprocess.run([{binary}]+sys.argv[1:],input=sys.stdin.buffer.read(),capture_output=True)
if child.returncode:
    sys.stderr.buffer.write(child.stderr);sys.exit(child.returncode)
reply=json.loads(child.stdout)
if 'prepare-content' in sys.argv and not marker.exists():
    marker.write_text('tampered once')
    reply['input_hash']='wrong'
sys.stdout.write(json.dumps(reply))
"#,
            binary = serde_json::to_string(BINARY).unwrap(),
            marker = serde_json::to_string(&f.root.join("tampered-once")).unwrap()
        );
        tmt_test_support::write_executable(&program, script.as_bytes(), 0o700).unwrap();
        let mut decoder = Decoder::new(program).unwrap();
        let before = state(&f);
        let result = page::prepare_publication(
            &f.store,
            &f.key,
            PAGE,
            ContentEdit {
                source: "new",
                publisher_agent: None,
            },
            None,
            &mut decoder,
            NOW,
        );
        assert!(matches!(
            result
                .err()
                .unwrap()
                .downcast_ref::<tmt_colab::decoder::DecodeFault>(),
            Some(tmt_colab::decoder::DecodeFault::InvalidOutput)
        ));
        assert_eq!(state(&f), before);
        // Confirmed child cleanup keeps this exact runner reusable (a no-op is not tampered).
        assert!(matches!(
            page::prepare_publication(
                &f.store,
                &f.key,
                PAGE,
                ContentEdit {
                    source: "old 🐈\r\n",
                    publisher_agent: None
                },
                None,
                &mut decoder,
                NOW
            )
            .unwrap(),
            PublicationPreparation::Noop { .. }
        ));
        assert_eq!(state(&f), before);
    }
    #[test]
    fn native_preparation_authenticated_malformed_base_returns_no_intent_and_runner_reuses() {
        let mut f = Fixture::new();
        append(&mut f, false, Namespace::Content, &[255]);
        let before = state(&f);
        let mut decoder = Decoder::new(BINARY.into()).unwrap();
        let result = page::prepare_publication(
            &f.store,
            &f.key,
            PAGE,
            ContentEdit {
                source: "new",
                publisher_agent: None,
            },
            None,
            &mut decoder,
            NOW,
        );
        assert!(matches!(
            result
                .err()
                .unwrap()
                .downcast_ref::<tmt_colab::decoder::DecodeFault>(),
            Some(tmt_colab::decoder::DecodeFault::Rejected)
        ));
        assert_eq!(state(&f), before);
        // Remove only the task fixture's malformed signed tail, then reuse the same runner.
        f.sql()
            .execute(
                "DELETE FROM receipts WHERE stream=? AND seq='00000000000000000002'",
                [DEVICE],
            )
            .unwrap();
        assert!(matches!(
            page::prepare_publication(
                &f.store,
                &f.key,
                PAGE,
                ContentEdit {
                    source: "old 🐈\r\n",
                    publisher_agent: None
                },
                None,
                &mut decoder,
                NOW
            )
            .unwrap(),
            PublicationPreparation::Noop { .. }
        ));
    }
}
