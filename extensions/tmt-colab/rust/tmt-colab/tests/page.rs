#[path = "support/core_fixture.rs"]
mod core_fixture;
#[path = "support/process.rs"]
// This consumer owns stdin writers; the CLI consumer also uses serving wait methods.
#[allow(dead_code)]
mod fixture_process;
mod support;
use ed25519_dalek::{Signer, SigningKey};
use fixture_process::FixtureCommand;
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
        Accepted, Bootstrapped, Envelope, Namespace, Store, StreamScope,
        owner::{Device, Mutation, Recipient},
    },
    transitions::{DeviceRevoke, Engine, EpochAdvance},
};
use tmt_colab_model::{certificate, object, values, wrap};
use yrs::{Doc, Map, ReadTxn, StateVector, Text, Transact, Update, updates::decoder::Decode};
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
const NOW: u64 = 1_791_025_000_000;

#[test]
fn review_owner_publication_keeps_legacy_questions_and_refuses_new_blocks_atomically() {
    let mut f = Fixture::new();
    let mut device = f.owner_device("30000000-0000-4000-8000-000000000099", 99);
    let question = "<tmt-question data-id=\"00000000-0000-0000-0000-000000000001\" data-agent=\"legacy\" data-kind=\"short\"><p data-question>Choose</p></tmt-question>";
    // A verified pre-registry editor stream, not a newly admitted source write.
    device.type_text(&mut f, question);
    let before = f.read().source;
    let changed = before.replace("old", "Updated");
    f.write(&changed);
    assert_eq!(f.read().source, changed);
    for source in [
        format!("{changed}{question}"),
        changed.replace("data-agent=\"legacy\"", "data-agent=\"another\""),
    ] {
        let error = f
            .prepare(&source, None)
            .err()
            .expect("new question requires registration");
        let refused = error
            .downcast_ref::<tmt_colab::question_registry::Refused>()
            .expect("typed refusal");
        assert_eq!(refused.code(), "COLAB_QUESTION_UNREGISTERED");
        assert_eq!(f.read().source, changed);
    }
}
/// An owner-member device with a document of its own, whose edits become its stream's updates.
struct OwnerDevice {
    id: &'static str,
    signing: SigningKey,
    doc: Doc,
}
impl OwnerDevice {
    /// The device types `text` into the page's html and stores the update as its next content
    /// update; the stream's new head is its sequence and hash.
    fn type_text(&mut self, f: &mut Fixture, text: &str) -> (u64, [u8; 32]) {
        let before = self.doc.transact().state_vector();
        let html = self.doc.get_or_insert_text("html");
        html.push(&mut self.doc.transact_mut(), text);
        let update = self.doc.transact().encode_state_as_update_v1(&before);
        let scope = StreamScope {
            page: PAGE,
            epoch: 1,
            stream: self.id,
        };
        let (seq, previous) = f
            .store
            .stream_head(scope)
            .unwrap()
            .map_or((1, [0; 32]), |(seq, hash)| (seq + 1, hash));
        let envelope = object::seal(
            &object::Context {
                space: f.key.space_id.clone(),
                page: PAGE.into(),
                epoch: "1".into(),
                kind: "update".into(),
                namespace: "content".into(),
                author_device: self.id.into(),
                membership_revision: "2".into(),
                stream_seq: seq.to_string(),
                prev_hash: previous,
            },
            &[11; 32],
            &self.signing,
            &update,
        )
        .unwrap();
        let hash = envelope.hash().unwrap();
        f.store
            .append(&Envelope {
                scope,
                namespace: Namespace::Content,
                seq,
                hash,
                previous,
                bytes: &envelope.to_json().unwrap(),
            })
            .unwrap();
        (seq, hash)
    }
    /// The checkpoint pair this device would publish after compacting its stream through `seq`
    /// (whose head is `head`): everything it typed in the content namespace, nothing in its own.
    fn checkpoint_pair(&self, f: &Fixture, seq: u64, head: [u8; 32]) -> Vec<Held> {
        let state = |doc: &Doc| {
            doc.transact()
                .encode_state_as_update_v1(&StateVector::default())
        };
        [
            (Namespace::Content, "content", state(&self.doc)),
            (Namespace::Own, "own", state(&Doc::new())),
        ]
        .into_iter()
        .map(|(namespace, name, plaintext)| {
            let envelope = object::seal(
                &object::Context {
                    space: f.key.space_id.clone(),
                    page: PAGE.into(),
                    epoch: "1".into(),
                    kind: "checkpoint".into(),
                    namespace: name.into(),
                    author_device: self.id.into(),
                    membership_revision: "2".into(),
                    stream_seq: seq.to_string(),
                    prev_hash: head,
                },
                &[11; 32],
                &self.signing,
                &plaintext,
            )
            .unwrap();
            Held {
                namespace,
                hash: envelope.hash().unwrap(),
                previous: head,
                bytes: envelope.to_json().unwrap(),
                signer: None,
            }
        })
        .collect()
    }
}
struct Fixture {
    root: PathBuf,
    retain_diagnostic: bool,
    layout: Layout,
    key: Keyring,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        Self::with_metadata(None, None)
    }
    fn with_author(author: Option<&str>) -> Self {
        Self::with_metadata(author, None)
    }
    fn with_creation_recipient(recipient: Option<&tmt_colab::decoder::CreationRecipient>) -> Self {
        Self::with_metadata(None, recipient)
    }
    fn with_metadata(
        author: Option<&str>,
        recipient: Option<&tmt_colab::decoder::CreationRecipient>,
    ) -> Self {
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
        if let Some(author) = author {
            doc.get_or_insert_map("meta")
                .insert(&mut doc.transact_mut(), "originalAuthor", author);
        }
        if let Some(recipient) = recipient {
            let fields = std::collections::HashMap::from([
                (
                    "machineId".into(),
                    yrs::Any::String(recipient.machine_id.as_str().into()),
                ),
                (
                    "agentId".into(),
                    yrs::Any::String(recipient.agent_id.as_str().into()),
                ),
            ]);
            doc.get_or_insert_map("meta").insert(
                &mut doc.transact_mut(),
                "creationRecipient",
                yrs::Any::Map(fields.into()),
            );
        }
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
    /// Another machine of the same owner, started from what this one holds now: the same keys
    /// and authority, this machine's own stream so far, and nothing it stores afterwards.
    fn copy(&self) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cp-{}-copy-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        for name in ["owner.key", "space.db"] {
            let target = layout.directory.join(name);
            fs::write(&target, fs::read(self.layout.directory.join(name)).unwrap()).unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let key = Keyring::open(&layout).unwrap();
        let store = Store::open(&layout).unwrap();
        Self {
            root,
            retain_diagnostic: false,
            layout,
            key,
            store,
        }
    }
    /// An owner-member device other than the local writer, as a browser paired through Remote or
    /// another machine's agent becomes one: its chain is issued by the owner's management member.
    fn owner_device(&mut self, id: &'static str, seed: u8) -> OwnerDevice {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let enc = wrap::RecipientKey::from_seed(&[seed.wrapping_add(1); 32])
            .unwrap()
            .public_key();
        let key = &self.key;
        let head = self
            .store
            .owner_head(&key.space_id, &key.owner_public())
            .unwrap()
            .unwrap();
        self.store
            .owner_transaction(
                &key.space_id,
                &key.owner_public(),
                Mutation {
                    operation_id: &format!("40000000-0000-4000-8000-0000000000{seed:02x}"),
                    digest: [seed; 32],
                    expected_revision: head.revision,
                },
                |tx| {
                    let public = signing.verifying_key();
                    let cert = certificate::Certificate {
                        space: &key.space_id,
                        issuer_kind: "member",
                        issuer_id: &head.owner_member.id,
                        device_id: id,
                        signing_key: public.as_bytes(),
                        encryption_key: &enc,
                        membership_revision: "1",
                        issued_at: 1,
                        expires_at: NOW + 86_400_000,
                    };
                    tx.put_device(&Device {
                        revoked: false,
                        chain: serde_json::to_vec(&json!({
                            "version": 1,
                            "issuerStatement": values::encode_binary(&tx.statement(1)?.unwrap().hash()?),
                            "deviceCertificate": values::encode_binary(&certificate::input(&cert)?),
                            "issuerSignature": values::encode_binary(&key.sign_device_certificate(&cert)?)
                        }))?,
                    })?;
                    // An owner transaction must move the log; a device needs no statement of its own.
                    tx.append_statement(&key.sign_statement(
                        tx.head(),
                        "retention.set",
                        &serde_json::to_vec(&json!({"pageId": PAGE, "days": null}))?,
                    )?)?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        OwnerDevice {
            id,
            signing,
            doc: Doc::new(),
        }
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
                layout: None,
                source,
                publisher_agent: None,
                attachments: None,
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
    fn patch_command(&self) -> Command {
        use sha2::{Digest, Sha256};
        let source = self.read().source;
        let lines = source.split_inclusive('\n').count().max(1);
        let hash: String = Sha256::digest(source.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let mut command = self.command();
        command.args([
            "page",
            "patch",
            PAGE,
            "--lines",
            &format!("1:{lines}"),
            "--range-sha256",
            &hash,
        ]);
        command
    }
    fn command(&self) -> Command {
        let core = core_fixture::link(&self.root, "core", core_fixture::Program::DataRoot);
        let mut c = Command::new(BINARY);
        // These read/source-publication scenarios exercise the non-agent path, with no
        // identity in their data-root-only Core. Identity-aware agent lock lifecycles live
        // in cli.rs. The opener guard still makes a host browser unreachable.
        c.env("TMT_EXECUTABLE", core)
            .env(
                "TMT_COLAB_TEST_REPLY",
                json!({"dataRoot":self.root}).to_string(),
            )
            .env_remove("TMT_AGENT")
            .env("PATH", core_fixture::opener_guard(&self.root))
            .current_dir(&self.root);
        c
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fixture_process::clean_root(&self.root);
        if !std::thread::panicking() {
            core_fixture::assert_no_open(&self.root);
        }
        if !self.retain_diagnostic {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}
/// What a status action's retained original outcome reports, as the CLI would print it.
struct StatusCommit {
    accepted: Accepted,
    receipt: StatusReceipt,
}
struct StatusReceipt {
    seq: String,
    envelope_hash: String,
    stream_id: String,
}
fn commit_status(
    f: &mut Fixture,
    frozen: &page::FrozenPublication,
) -> tmt_colab::Result<StatusCommit> {
    let committed = f.commit(frozen, NOW)?;
    let receipt = page::publication_receipt(frozen.job(), &committed.record)?;
    let publication = receipt.publication.expect("a committed outcome");
    Ok(StatusCommit {
        accepted: committed.accepted,
        receipt: StatusReceipt {
            seq: publication.seq,
            envelope_hash: publication.envelope_hash,
            stream_id: publication.stream_id,
        },
    })
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
fn a_page_carries_work_through_actions_source_writes_and_a_new_epoch() {
    use tmt_colab::decoder::StructuredKind;
    let mut f = Fixture::new();
    let work = |f: &Fixture| page::read_work(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let act = |f: &mut Fixture,
               action: &str,
               body: serde_json::Value,
               revision: &str|
     -> tmt_colab::Result<Option<String>> {
        let prepared = page::prepare_structured_publication(
            &f.store,
            &f.key,
            PAGE,
            page::StructuredAction {
                kind: StructuredKind::Work,
                action,
                body: &body,
                expected_revision: revision,
                display_agent: Some(json!({"name":"feat-2"})),
                operation_id: None,
            },
            &mut f.decoder(),
            &|| Ok(NOW),
        )?;
        match prepared {
            page::PublicationPreparation::Write(frozen) => {
                let committed = f.commit(&frozen, NOW)?;
                Ok(Some(
                    page::publication_receipt(frozen.job(), &committed.record)?.revision,
                ))
            }
            page::PublicationPreparation::Noop { .. } => Ok(None),
        }
    };
    const A: &str = "20000000-0000-4000-8000-00000000000a";
    const B: &str = "20000000-0000-4000-8000-00000000000b";
    // A page that has had no work reads as an empty list, and a write against it succeeds. The
    // value admits any page ID; the link rules are the write owner's (`work_admission`).
    let before = work(&f);
    assert!(before.children.is_empty() && before.summary.is_none());
    let revision = act(
        &mut f,
        "subpage.attach",
        json!({"pageId":A}),
        &before.revision,
    )
    .unwrap()
    .unwrap();
    let after = work(&f);
    assert_eq!(after.revision, revision);
    assert_eq!(after.children, [A]);
    // The write owner's attribution, not the caller's, lands in the summary.
    let revision = act(
        &mut f,
        "summary.set",
        json!({"line1":"One page","line2":"Next: build"}),
        &revision,
    )
    .unwrap()
    .unwrap();
    let summary = work(&f).summary.unwrap();
    assert_eq!(summary["line1"], "One page");
    assert_eq!(
        summary["attribution"]["displayAgent"],
        json!({"name":"feat-2"})
    );
    assert!(summary["attribution"]["writerId"].is_string());
    // A stale base, an invalid action and a repeated no-op write nothing.
    let stale = act(
        &mut f,
        "subpage.detach",
        json!({"pageId":A}),
        &before.revision,
    );
    assert!(stale.is_err());
    assert!(act(&mut f, "subpage.detach", json!({"pageId":B}), &revision).is_err());
    assert!(act(&mut f, "task.add", json!({"task":{"title":"x"}}), &revision).is_err());
    assert!(
        act(&mut f, "subpage.move", json!({"pageId":A}), &revision)
            .unwrap()
            .is_none()
    );
    assert_eq!(work(&f).revision, revision);
    // The page's own source still writes, and the work stays.
    let source_receipt = f.write("a new source 🦊");
    assert_eq!(f.read().source, "a new source 🦊");
    assert_eq!(work(&f).children, [A]);
    assert_eq!(work(&f).revision, source_receipt.revision);
    // A new epoch keeps the work with the source.
    Engine::with_decoder_config(support::decoder_config(BINARY.into()))
        .unwrap()
        .advance_epoch(
            &mut f.store,
            &f.key,
            EpochAdvance {
                page: PAGE,
                operation_id: "40000000-0000-4000-8000-000000000002",
                expected_revision: 2,
            },
            NOW,
        )
        .unwrap();
    let rotated = work(&f);
    assert_eq!(rotated.epoch, "2");
    assert_eq!(rotated.children, [A]);
    assert_eq!(rotated.summary, Some(summary));
    assert_eq!(f.read().source, "a new source 🦊");
    let revision = act(
        &mut f,
        "subpage.attach",
        json!({"pageId":B,"before":A}),
        &rotated.revision,
    )
    .unwrap()
    .unwrap();
    assert_eq!(work(&f).revision, revision);
    assert_eq!(work(&f).children, [B, A]);
}
#[test]
fn a_replayed_write_reports_the_batch_it_originally_committed() {
    let mut f = Fixture::new();
    let first = f.prepare("one", None).unwrap();
    let accepted = f.commit(&first, NOW).unwrap();
    let original = page::publication_receipt(first.job(), &accepted.record).unwrap();
    assert!(original.changed && original.update.count == 1 && original.update.bytes > 0);
    assert_eq!(original.update.minimal, Some(true));
    // A later, larger write moves the page on; its batch is its own.
    let later = f.write(&"a longer replacement ".repeat(50));
    assert_ne!(later.update.bytes, original.update.bytes);
    let replay = f.commit(&first, NOW).unwrap();
    assert_eq!(replay.accepted, Accepted::Replay);
    let again = page::publication_receipt(first.job(), &replay.record).unwrap();
    assert_eq!(again.update, original.update);
    assert_eq!(again.update.count, again.publication.unwrap().count);
}
#[test]
fn a_patch_is_evaluated_on_the_base_it_commits_against_and_merges_a_concurrent_edit_elsewhere() {
    use page::patch::Patch;
    use tmt_colab_model::crypto;
    let mut f = Fixture::new();
    f.write("alpha\nbeta\ngamma\n");
    let patch = Patch::Replace(vec![("beta".into(), "BETA".into())]);
    // The patch is evaluated against the page as read...
    let base = f.read();
    let edited = patch.apply(&base.source).unwrap();
    assert_eq!(edited, "alpha\nBETA\ngamma\n");
    // ...a concurrent writer changes the page elsewhere before the edit is prepared...
    f.write("alpha\nbeta\ngamma\ndelta\n");
    let options = page::PublishOptions {
        base_source_sha256: Some(crypto::digest(base.source.as_bytes())),
        ..Default::default()
    };
    let prepare = |f: &Fixture, source: &str, options: page::PublishOptions| {
        page::prepare_publication(
            &f.store,
            &f.key,
            PAGE,
            tmt_colab::decoder::ContentEdit {
                layout: None,
                source,
                publisher_agent: None,
                attachments: None,
            },
            options,
            &mut f.decoder(),
            NOW,
        )
    };
    // ...so the edit, whose base is gone, is refused instead of overwriting that change.
    assert_eq!(
        prepare(&f, &edited, options)
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::StaleBase)
    );
    assert_eq!(f.read().source, "alpha\nbeta\ngamma\ndelta\n");
    // Evaluated again on the new base, the same patch applies and keeps both edits.
    let base = f.read();
    let edited = patch.apply(&base.source).unwrap();
    let options = page::PublishOptions {
        base_source_sha256: Some(crypto::digest(base.source.as_bytes())),
        ..Default::default()
    };
    let page::PublicationPreparation::Write(frozen) = prepare(&f, &edited, options).unwrap() else {
        panic!("the patch changes the source");
    };
    // One update for one changed word: the rest of the page is not rewritten.
    assert_eq!(frozen.job().manifest.entries.len(), 1);
    f.commit(&frozen, NOW).unwrap();
    assert_eq!(f.read().source, "alpha\nBETA\ngamma\ndelta\n");
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
        .patch_command()
        .args([
            "--file",
            "-",
            "--expected-revision",
            read["revision"].as_str().unwrap(),
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn_owned()
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
            .patch_command()
            .args(["--file", input.to_str().unwrap(), "--json"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let error: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(error["error"]["code"], code);
        assert_eq!(f.bytes(), before);
    }
    fs::write(&input, b"new").unwrap();
    let out = f
        .patch_command()
        .args([
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
        .patch_command()
        .args(["--file", input.to_str().unwrap(), "--json"])
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

const THREAD: &str = "40000000-0000-4000-8000-000000000001";
fn seed_member_thread(f: &mut Fixture) {
    seed_member_thread_with(f, &[]);
}
/// The member thread, plus `comments` as (message id, revision, deleted, body) records on it.
fn seed_member_thread_with(f: &mut Fixture, comments: &[(&str, &str, bool, &str)]) {
    let record = json!({"version":1,"kind":"thread","spaceId":f.key.space_id,
        "pageId":PAGE,"epoch":"1","senderDevice":DEVICE,"revision":"1",
        "deleted":false,"deviceName":"Page author","at":NOW.to_string(),
        "threadId":THREAD,"anchor":{"exact":"old","prefix":"","suffix":""},"resolved":false});
    let doc = Doc::new();
    for root in ["threads", "messages", "intents", "replies"] {
        doc.get_or_insert_map(root);
    }
    doc.get_or_insert_map("threads").insert(
        &mut doc.transact_mut(),
        format!("{THREAD}:1"),
        yrs::Any::from_json(&record.to_string()).unwrap(),
    );
    // A non-owner member's historical key permits its ordinary records, but not
    // a status action, even when it targets its own thread and claims an agent.
    let unauthorized = json!({"version":1,"kind":"thread-status","spaceId":f.key.space_id,
        "pageId":PAGE,"epoch":"1","senderDevice":DEVICE,"revision":"1","deleted":false,
        "deviceName":"Page author","at":NOW.to_string(),"actionId":"50000000-0000-4000-8000-000000000001",
        "thread":{"writer":DEVICE,"id":THREAD},"previous":null,"resolved":true,
        "actor":"agent","agentName":"Untrusted label","recipients":[]});
    doc.get_or_insert_map("messages").insert(
        &mut doc.transact_mut(),
        "50000000-0000-4000-8000-000000000001:thread-status",
        yrs::Any::from_json(&unauthorized.to_string()).unwrap(),
    );
    for (id, revision, deleted, body) in comments {
        let comment = json!({"version":1,"kind":"comment","spaceId":f.key.space_id,
            "pageId":PAGE,"epoch":"1","senderDevice":DEVICE,"revision":revision,
            "deleted":deleted,"deviceName":"Page author","at":NOW.to_string(),
            "messageId":id,"thread":{"writer":DEVICE,"id":THREAD},"body":body});
        doc.get_or_insert_map("messages").insert(
            &mut doc.transact_mut(),
            format!("{id}:{revision}"),
            yrs::Any::from_json(&comment.to_string()).unwrap(),
        );
    }
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let previous = object::Envelope::from_json(
        &f.store
            .payload(
                StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream: DEVICE,
                },
                1,
            )
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .hash()
    .unwrap();
    let envelope = object::seal(
        &object::Context {
            space: f.key.space_id.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: "update".into(),
            namespace: "own".into(),
            author_device: DEVICE.into(),
            membership_revision: "2".into(),
            stream_seq: "2".into(),
            prev_hash: previous,
        },
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
            namespace: Namespace::Own,
            seq: 2,
            hash: envelope.hash().unwrap(),
            previous,
            bytes: &envelope.to_json().unwrap(),
        })
        .unwrap();
}

#[test]
fn an_own_publication_commits_once_replays_exactly_and_reports_through_status() {
    use tmt_colab::{
        discussion::{self, StatusEdit},
        publication::{Outcome, PublicationKind},
    };
    let mut f = Fixture::new();
    seed_member_thread(&mut f);
    let (frozen, _) = discussion::prepare_status(
        &f.store,
        &f.key,
        PAGE,
        StatusEdit {
            thread: THREAD,
            resolved: true,
            agent_name: None,
        },
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    assert_eq!(frozen.job().manifest.kind, PublicationKind::Own);
    let before = f.read();
    let first = f.commit(&frozen, NOW).unwrap();
    assert_eq!(first.accepted, Accepted::New);
    let Outcome::Committed {
        count,
        final_position,
        ..
    } = &first.record.outcome
    else {
        panic!("expected a committed outcome");
    };
    assert_eq!((*count, final_position.seq.as_str()), (1, "1"));
    // The page's content and its revision base are not moved by an own-namespace entry.
    assert_eq!(f.read().source, before.source);
    let stored: i64 = f
        .sql()
        .query_row("SELECT count(*) FROM receipts", [], |r| r.get(0))
        .unwrap();
    // Exact replay returns the original retained bytes and appends nothing.
    let again = f.commit(&frozen, NOW).unwrap();
    assert_eq!(again.accepted, Accepted::Replay);
    assert_eq!(again.record.bytes, first.record.bytes);
    assert_eq!(
        f.sql()
            .query_row::<i64, _, _>("SELECT count(*) FROM receipts", [], |r| r.get(0))
            .unwrap(),
        stored
    );
    // A later lookup by the original key sees the same committed result, read-only.
    let original = frozen.job().key().unwrap();
    let looked_up =
        page::publication_status(&f.store, &f.key, &original, frozen.chain(), NOW).unwrap();
    assert_eq!(looked_up.bytes, first.record.bytes);
    // The frozen job is not refreshed: the base moved, so the same bytes cannot publish again
    // as a new operation, and the original result stays the retained one.
    assert_eq!(f.write("later source").publication.unwrap().seq, "2");
    assert_eq!(
        f.commit(&frozen, NOW).unwrap().record.bytes,
        first.record.bytes
    );
}

#[test]
fn agent_status_is_own_stream_causal_fenced_and_shared_with_native_reads() {
    use tmt_colab::discussion::{self, StatusEdit};
    let mut f = Fixture::new();
    seed_member_thread(&mut f);
    let initial = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert!(!initial.conversations.threads[0].resolved);
    let (prepared, action) = discussion::prepare_status(
        &f.store,
        &f.key,
        PAGE,
        StatusEdit {
            thread: THREAD,
            resolved: true,
            agent_name: Some("Review agent"),
        },
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    assert!(action.recipients.is_empty());
    assert_eq!(action.actor, "agent");
    assert!(
        !discussion::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .conversations
            .threads[0]
            .resolved
    );
    let receipt = commit_status(&mut f, &prepared).unwrap();
    assert_eq!(receipt.accepted, Accepted::New);
    assert_eq!(receipt.receipt.seq, "1");
    assert_eq!(f.read().source, "old 🐈\r\n");
    let view = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let thread = &view.conversations.threads[0];
    assert_eq!(thread.writer, DEVICE);
    assert_eq!(thread.revision, "1");
    assert!(thread.resolved);
    let status = thread.status.as_ref().unwrap();
    assert_eq!(status.action.agent_name.as_deref(), Some("Review agent"));
    assert_eq!(status.reference.id, action.action_id);
    assert!(
        view.conversations.asks.is_empty(),
        "CLI Resolve never dispatches or creates an Ask"
    );
    assert!(
        discussion::prepare_status(
            &f.store,
            &f.key,
            PAGE,
            StatusEdit {
                thread: THREAD,
                resolved: true,
                agent_name: Some("Other label")
            },
            &mut f.decoder(),
            NOW
        )
        .unwrap()
        .is_none()
    );
    // The native writer's content and own updates share a sequence/hash chain.
    assert_eq!(f.write("new source").publication.unwrap().seq, "2");
    let (reopen, action) = discussion::prepare_status(
        &f.store,
        &f.key,
        PAGE,
        StatusEdit {
            thread: THREAD,
            resolved: false,
            agent_name: None,
        },
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    assert_eq!(action.previous.as_ref().unwrap(), &status.reference);
    assert_eq!(commit_status(&mut f, &reopen).unwrap().receipt.seq, "3");
    assert!(
        !discussion::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .conversations
            .threads[0]
            .resolved
    );
    let (stale, _) = discussion::prepare_status(
        &f.store,
        &f.key,
        PAGE,
        StatusEdit {
            thread: THREAD,
            resolved: true,
            agent_name: None,
        },
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    f.write("concurrent source");
    assert_eq!(
        commit_status(&mut f, &stale)
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(&Fault::StaleBase)
    );
    assert!(
        !discussion::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .conversations
            .threads[0]
            .resolved
    );
    assert_eq!(f.read().source, "concurrent source");
    let output = f
        .command()
        .args(["threads", PAGE, "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let queried: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(queried["threads"][0]["writer"], DEVICE);
    assert_eq!(queried["threads"][0]["resolved"], false);
    assert_eq!(queried["path"], format!("/colab/p/{}", &PAGE[..8]));
    let output = f
        .command()
        .args(["threads", "resolve", PAGE, THREAD, "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let changed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(changed["changed"], true);
    values::generated_id(
        changed["operationId"]
            .as_str()
            .expect("published operation"),
    )
    .unwrap();
    assert_eq!(changed["action"]["actor"], "agent");
    assert!(
        changed["action"]["agentName"].is_null(),
        "Unbound public identity probe fails closed"
    );
    assert_eq!(changed["action"]["recipients"], json!([]));
    let output = f
        .command()
        .args(["threads", "resolve", PAGE, THREAD, "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let unchanged: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(unchanged["changed"], false);
    assert!(unchanged["action"].is_null() && unchanged["operationId"].is_null());
    let output = f.command().args(["threads", PAGE]).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("resolved") && text.contains("Local CLI") && text.contains("old"));
}

#[test]
fn bridge_own_records_remain_visible_but_cannot_resolve_a_thread() {
    use tmt_colab::discussion;
    const BRIDGE: &str = "60000000-0000-4000-8000-000000000001";
    const MESSAGE: &str = "70000000-0000-4000-8000-000000000001";
    const ACTION: &str = "50000000-0000-4000-8000-000000000002";
    let mut f = Fixture::new();
    seed_member_thread(&mut f);
    let signing = SigningKey::from_bytes(&[13; 32]);
    let encryption = wrap::RecipientKey::from_seed(&[14; 32])
        .unwrap()
        .public_key();
    f.store.owner_transaction(&f.key.space_id, &f.key.owner_public(),
        Mutation { operation_id: BRIDGE, digest: [13; 32], expected_revision: 2 }, |tx| {
            let statement = f.key.sign_statement(tx.head(), "bridge.add", &serde_json::to_vec(&json!({
                "machineId":BRIDGE,"machineSignKey":values::encode_binary(signing.verifying_key().as_bytes()),
                "encKey":values::encode_binary(&encryption),"pages":[PAGE]
            }))?)?;
            tx.append_statement(&statement)?;
            tx.put_recipient(&Recipient {kind:"bridge".into(),id:BRIDGE.into(),role:None,
                signing_key:signing.verifying_key().to_bytes(),encryption_key:encryption,pages:vec![PAGE.into()],revoked:false})?;
            Ok(Vec::new())
        }).unwrap();
    let doc = Doc::new();
    for root in ["threads", "messages", "intents", "replies"] {
        doc.get_or_insert_map(root);
    }
    let scope = json!({"version":1,"spaceId":f.key.space_id,"pageId":PAGE,"epoch":"1",
        "senderDevice":BRIDGE,"revision":"1","deleted":false,"deviceName":"Reply bridge","at":NOW.to_string()});
    let mut comment = scope.clone();
    comment["kind"] = json!("comment");
    comment["messageId"] = json!(MESSAGE);
    comment["thread"] = json!({"writer":DEVICE,"id":THREAD});
    comment["body"] = json!("An ordinary bridge comment");
    let mut status = scope;
    status["kind"] = json!("thread-status");
    status["actionId"] = json!(ACTION);
    status["thread"] = json!({"writer":DEVICE,"id":THREAD});
    status["previous"] = Value::Null;
    status["resolved"] = json!(true);
    status["actor"] = json!("agent");
    status["agentName"] = json!("Bridge label");
    status["recipients"] = json!([]);
    for (key, value) in [
        (format!("{MESSAGE}:1"), comment),
        (format!("{ACTION}:thread-status"), status),
    ] {
        doc.get_or_insert_map("messages").insert(
            &mut doc.transact_mut(),
            key,
            yrs::Any::from_json(&value.to_string()).unwrap(),
        );
    }
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let context = object::Context {
        space: f.key.space_id.clone(),
        page: PAGE.into(),
        epoch: "1".into(),
        kind: "update".into(),
        namespace: "own".into(),
        author_device: BRIDGE.into(),
        membership_revision: "3".into(),
        stream_seq: "1".into(),
        prev_hash: [0; 32],
    };
    let envelope = object::seal(&context, &[11; 32], &signing, &update).unwrap();
    f.store
        .append(&Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: BRIDGE,
            },
            namespace: Namespace::Own,
            seq: 1,
            hash: envelope.hash().unwrap(),
            previous: [0; 32],
            bytes: &envelope.to_json().unwrap(),
        })
        .unwrap();
    let view = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let thread = &view.conversations.threads[0];
    assert!(!thread.resolved);
    assert!(thread.status.is_none());
    assert_eq!(thread.comments.len(), 1);
    assert_eq!(thread.comments[0].body, "An ordinary bridge comment");
    assert_eq!(thread.comments[0].writer, BRIDGE);
}

#[test]
fn historical_owner_status_survives_revoke_only_inside_the_signed_cut() {
    use tmt_colab::discussion::{self, StatusEdit};
    use tmt_colab_model::stream_cut;
    for includes_action in [true, false] {
        let mut f = Fixture::new();
        seed_member_thread(&mut f);
        let (prepared, action) = discussion::prepare_status(
            &f.store,
            &f.key,
            PAGE,
            StatusEdit {
                thread: THREAD,
                resolved: true,
                agent_name: Some("Owner agent"),
            },
            &mut f.decoder(),
            NOW,
        )
        .unwrap()
        .unwrap();
        let receipt = commit_status(&mut f, &prepared).unwrap().receipt;
        let tail = if includes_action { 1 } else { 0 };
        let hash = if includes_action {
            values::binary(&receipt.envelope_hash, 32)
                .unwrap()
                .try_into()
                .unwrap()
        } else {
            [0; 32]
        };
        let cut = values::encode_binary(
            &stream_cut::input(&stream_cut::StreamCut {
                stream_id: &receipt.stream_id,
                namespace: "own",
                checkpoint_hash: None,
                checkpoint_seq: "0",
                tail_head_seq: &tail.to_string(),
                tail_head_hash: &hash,
            })
            .unwrap(),
        );
        f.store.owner_transaction(&f.key.space_id,&f.key.owner_public(),
            Mutation {operation_id:"80000000-0000-4000-8000-000000000001",digest:[15;32],expected_revision:2},|tx| {
                let statement = f.key.sign_statement(tx.head(),"device.revoke",&serde_json::to_vec(&json!({
                    "deviceId":receipt.stream_id,"cuts":[{"pageId":PAGE,"epoch":"1","namespace":"own","cut":cut}]
                }))?)?;
                tx.append_statement(&statement)?;
                let mut device = tx.device(&receipt.stream_id)?.unwrap();
                device.revoked = true; tx.put_device(&device)?;
                Ok(Vec::new())
            }).unwrap();
        let read = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder());
        if includes_action {
            let view = read.unwrap();
            assert!(view.conversations.threads[0].resolved);
            assert_eq!(
                view.conversations.threads[0]
                    .status
                    .as_ref()
                    .unwrap()
                    .reference
                    .id,
                action.action_id
            );
            let denied = discussion::prepare_status(
                &f.store,
                &f.key,
                PAGE,
                StatusEdit {
                    thread: THREAD,
                    resolved: false,
                    agent_name: None,
                },
                &mut f.decoder(),
                NOW,
            )
            .err()
            .unwrap();
            assert_eq!(denied.downcast_ref::<Fault>(), Some(&Fault::Denied));
        } else {
            assert!(
                read.is_err(),
                "a signed history outside the revoke cut is not admitted"
            );
        }
    }
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

/// What the stored envelopes hold, counted without the code that reports usage: AES-GCM adds a
/// 16-byte tag and no padding, so a stored object's plaintext is its ciphertext minus the tag.
fn stored_usage(f: &Fixture) -> (usize, usize, usize) {
    let plaintext = |table: &str| -> Vec<usize> {
        let db = f.sql();
        let mut statement = db
            .prepare(&format!(
                "SELECT payload FROM {table} WHERE page = ?1 AND payload IS NOT NULL"
            ))
            .unwrap();
        statement
            .query_map([PAGE], |row| row.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(|payload| {
                object::Envelope::from_json(&payload.unwrap())
                    .unwrap()
                    .ciphertext()
                    .len()
                    - 16
            })
            .collect()
    };
    let tail = plaintext("receipts");
    (
        tail.len(),
        tail.iter().sum(),
        plaintext("checkpoints").iter().sum(),
    )
}

fn request_compaction(f: &mut Fixture) -> page::compact::Report {
    let mut decoder = f.decoder();
    page::compact::request(&mut f.store, &f.key, PAGE, &mut decoder, None).unwrap()
}

fn deleted_owner_history(history: &str, public: bool) -> (Fixture, usize) {
    let mut f = Fixture::new();
    // Register the local signer before it publishes a fold of the other owner device.
    f.write("old 🐈\r\n<p>Retained source</p>");
    let mut device = f.owner_device("30000000-0000-4000-8000-000000000091", 91);
    let head = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap();
    f.store.owner_transaction(
        &f.key.space_id, &f.key.owner_public(),
        Mutation { operation_id: "40000000-0000-4000-8000-000000000092", digest: [92; 32], expected_revision: head.revision },
        |tx| {
            tx.append_statement(&f.key.sign_statement(tx.head(), "page.history",
                &serde_json::to_vec(&json!({"pageId": PAGE, "mode": history}))?)?)?;
            if public {
                tx.append_statement(&f.key.sign_statement(tx.head(), "page.share",
                    &serde_json::to_vec(&json!({"pageId": PAGE, "epoch": "1", "mode": "public", "publishedKeys": [{"epoch": "1", "key": values::encode_binary(&[11; 32])}]}))?)?)?;
            }
            Ok(Vec::new())
        },
    ).unwrap();
    let removed = "Deleted private draft ".repeat(3_000);
    let (seq, previous) = device.type_text(&mut f, &removed);
    let html = device.doc.get_or_insert_text("html");
    let deletion = {
        let mut tx = device.doc.transact_mut();
        html.remove_range(&mut tx, 0, removed.len() as u32);
        tx.encode_update_v1()
    };
    let envelope = object::seal(
        &object::Context {
            space: f.key.space_id.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: "update".into(),
            namespace: "content".into(),
            author_device: device.id.into(),
            membership_revision: "2".into(),
            stream_seq: (seq + 1).to_string(),
            prev_hash: previous,
        },
        &[11; 32],
        &device.signing,
        &deletion,
    )
    .unwrap();
    f.store
        .append(&Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: device.id,
            },
            namespace: Namespace::Content,
            seq: seq + 1,
            hash: envelope.hash().unwrap(),
            previous,
            bytes: &envelope.to_json().unwrap(),
        })
        .unwrap();
    (f, removed.len())
}

#[test]
fn history_rebuild_drops_deleted_content_without_changing_the_page_projection() {
    let (mut f, removed) = deleted_owner_history("current", false);
    seed_member_thread_with(
        &mut f,
        &[
            (
                "60000000-0000-4000-8000-000000000091",
                "1",
                false,
                "Original comment",
            ),
            (
                "60000000-0000-4000-8000-000000000091",
                "2",
                false,
                "Revised comment",
            ),
        ],
    );
    let conversations = |f: &Fixture| {
        serde_json::to_value(
            tmt_colab::discussion::read(&f.store, &f.key, PAGE, &mut f.decoder())
                .unwrap()
                .conversations,
        )
        .unwrap()
    };
    let own_payloads = |f: &Fixture| {
        f.sql()
            .prepare("SELECT payload FROM receipts WHERE namespace='own' ORDER BY stream, seq")
            .unwrap()
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let own_before = own_payloads(&f);
    let discussion_before = conversations(&f);
    let before = f.read();
    let (_, tail, checkpoints) = stored_usage(&f);
    assert!(tail + checkpoints > removed);
    let report = request_compaction(&mut f);
    assert!(report.compacted, "{report:?}");
    let after = f.read();
    assert_eq!(after.source, before.source);
    assert_eq!(after.title, before.title);
    assert_eq!(conversations(&f), discussion_before);
    assert_eq!(
        own_payloads(&f),
        own_before,
        "own revisions were rewritten or pruned"
    );
    assert_eq!(exported_source(&f), before.source.as_bytes());
    let (_, tail, checkpoints) = stored_usage(&f);
    assert!(
        tail + checkpoints < removed / 4,
        "rebuild retained deleted content: {} bytes",
        tail + checkpoints
    );
    let paired = f.bytes();
    assert!(!request_compaction(&mut f).compacted);
    assert_eq!(
        f.bytes(),
        paired,
        "an immutable paired head was rewritten without a new edit"
    );
}

#[test]
fn history_rebuild_rejects_a_changed_candidate_before_pruning_the_writer() {
    let (mut f, _) = deleted_owner_history("current", false);
    let before = f.read().source;
    let program = f.root.join("changed-history-rebuild");
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
if request.get("rebuild"):
    reply["merged"] = request["updates"][0]
sys.stdout.write(json.dumps(reply))
"#,
        binary = serde_json::to_string(BINARY).unwrap()
    );
    tmt_test_support::write_executable(&program, script.as_bytes(), 0o700).unwrap();
    let mut decoder = Decoder::with_config(support::decoder_config(program)).unwrap();
    let report = page::compact::request(&mut f.store, &f.key, PAGE, &mut decoder, None).unwrap();
    assert!(report.folded.is_empty());
    assert!(
        report
            .left
            .iter()
            .any(|left| left.stream == "30000000-0000-4000-8000-000000000091"
                && left.why == page::owner_fold::Why::Refused)
    );
    assert!(fold_rows(&f, "30000000-0000-4000-8000-000000000091").is_empty());
    assert_eq!(
        retained(&f, "30000000-0000-4000-8000-000000000091"),
        [(1, true), (2, true)]
    );
    assert_eq!(f.read().source, before);
    // The storage entry point reuses that same refusal and must not publish the bad fold.
    let retained_before = retained(&f, "30000000-0000-4000-8000-000000000091");
    let refused =
        tmt_colab::storage::reclaim(&mut f.store, &f.key, PAGE, &mut decoder, false).unwrap();
    assert_eq!(refused.skipped.as_deref(), Some("not-combined"));
    assert_eq!(refused.freed.history, Some(0));
    assert_eq!(
        retained(&f, "30000000-0000-4000-8000-000000000091"),
        retained_before
    );
    assert!(fold_rows(&f, "30000000-0000-4000-8000-000000000091").is_empty());
    assert_eq!(f.read().source, before);
    // Positive control: the real child rebuilds the same stream and can publish its fold.
    assert_eq!(request_compaction(&mut f).folded.len(), 1);
}

#[test]
fn reclaim_estimate_excludes_a_non_owner_stream_even_with_an_active_tail() {
    let mut f = Fixture::new();
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
                operation_id: "40000000-0000-4000-8000-000000000092",
                digest: [92; 32],
                expected_revision: head.revision,
            },
            |tx| {
                tx.append_statement(&f.key.sign_statement(
                    tx.head(),
                    "page.history",
                    &serde_json::to_vec(&json!({"pageId": PAGE, "mode": "current"}))?,
                )?)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    let raw: Vec<u8> = f
        .sql()
        .query_row(
            "SELECT payload FROM receipts WHERE stream=?1 AND seq=?2",
            rusqlite::params![DEVICE, format!("{:020}", 1)],
            |r| r.get(0),
        )
        .unwrap();
    let envelope = object::Envelope::from_json(&raw).unwrap();
    let header = object::Header::decode(envelope.header()).unwrap();
    let signing = SigningKey::from_bytes(&[9; 32]);
    let plain = object::open(
        &envelope,
        &header.context,
        &[11; 32],
        signing.verifying_key().as_bytes(),
    )
    .unwrap();
    let doc = Doc::new();
    doc.transact_mut()
        .apply_update(Update::decode_v1(&plain).unwrap())
        .unwrap();
    let mut device = OwnerDevice {
        id: DEVICE,
        signing,
        doc,
    };
    let scope = StreamScope {
        page: PAGE,
        epoch: 1,
        stream: DEVICE,
    };
    let (_, head) = f.store.stream_head(scope).unwrap().unwrap();
    for held in device.checkpoint_pair(&f, 1, head) {
        f.store.checkpoint(&held_envelope(scope, 1, &held)).unwrap();
    }
    f.sql()
        .execute(
            "UPDATE receipts SET payload=?1 WHERE stream=?2 AND seq=?3",
            rusqlite::params![raw, DEVICE, format!("{:020}", 1)],
        )
        .unwrap();
    device.type_text(&mut f, " member tail");
    assert!(
        f.store.eligible_history_bytes(PAGE).unwrap() > 0,
        "the raw prune selection must find a candidate"
    );
    let mut decoder = f.decoder();
    let rows = tmt_colab::storage::read(&f.store, &f.key, &f.layout, &mut decoder).unwrap();
    let row = rows.pages.iter().find(|row| row.page_id == PAGE).unwrap();
    assert!(row.history_bytes > 0);
    assert_eq!(
        row.freeable.history, 0,
        "the owner cannot fold a member device stream"
    );
    let result =
        tmt_colab::storage::reclaim(&mut f.store, &f.key, PAGE, &mut decoder, false).unwrap();
    assert_eq!(result.freed.history, Some(0));
}

#[test]
fn reclaim_estimate_includes_admitted_owner_stream_with_an_active_tail() {
    let (mut f, _) = deleted_owner_history("current", false);
    let mut device = f.owner_device("30000000-0000-4000-8000-000000000093", 93);
    let (seq, head) = device.type_text(&mut f, "owner history");
    let raw: Vec<u8> = f
        .sql()
        .query_row(
            "SELECT payload FROM receipts WHERE stream=?1 AND seq=?2",
            rusqlite::params![device.id, format!("{seq:020}")],
            |r| r.get(0),
        )
        .unwrap();
    let scope = StreamScope {
        page: PAGE,
        epoch: 1,
        stream: device.id,
    };
    for held in device.checkpoint_pair(&f, seq, head) {
        f.store
            .checkpoint(&held_envelope(scope, seq, &held))
            .unwrap();
    }
    f.sql()
        .execute(
            "UPDATE receipts SET payload=?1 WHERE stream=?2 AND seq=?3",
            rusqlite::params![raw, device.id, format!("{seq:020}")],
        )
        .unwrap();
    device.type_text(&mut f, " new owner tail");
    let mut decoder = f.decoder();
    let preview =
        tmt_colab::storage::reclaim(&mut f.store, &f.key, PAGE, &mut decoder, true).unwrap();
    assert!(
        preview.freeable.history > 0,
        "an admitted owner candidate must be included"
    );
    let result =
        tmt_colab::storage::reclaim(&mut f.store, &f.key, PAGE, &mut decoder, false).unwrap();
    assert!(result.error.is_none());
    assert!(result.freed.history.unwrap() > 0);
}

#[test]
fn reclaim_no_tail_reports_no_freeable_bytes_and_keeps_history() {
    let (mut f, _) = deleted_owner_history("current", false);
    let stream = "30000000-0000-4000-8000-000000000091";
    let payload: Vec<u8> = f
        .sql()
        .query_row(
            "SELECT payload FROM receipts WHERE stream=?1 AND seq=?2",
            rusqlite::params![stream, format!("{:020}", 1)],
            |r| r.get(0),
        )
        .unwrap();
    assert!(request_compaction(&mut f).compacted);
    // Model an older store retaining a superseded payload under a complete paired cut.
    f.sql()
        .execute(
            "UPDATE receipts SET payload=?1 WHERE stream=?2 AND seq=?3",
            rusqlite::params![payload, stream, format!("{:020}", 1)],
        )
        .unwrap();
    let before = f.store.page_storage_bytes(PAGE).unwrap();
    assert!(before.history > 0);
    let source = f.read().source;
    let mut decoder = f.decoder();
    let preview =
        tmt_colab::storage::reclaim(&mut f.store, &f.key, PAGE, &mut decoder, true).unwrap();
    assert_eq!(preview.freeable.history, 0);
    let actual =
        tmt_colab::storage::reclaim(&mut f.store, &f.key, PAGE, &mut decoder, false).unwrap();
    assert_eq!(actual.skipped.as_deref(), Some("no-tail"));
    assert_eq!(actual.freed.history, Some(0));
    assert_eq!(
        f.store.page_storage_bytes(PAGE).unwrap().history,
        before.history
    );
    assert_eq!(f.read().source, source);
}

#[test]
fn reclaim_reports_bytes_freed_before_a_later_stream_failure() {
    let (mut f, _) = deleted_owner_history("current", false);
    let mut second = f.owner_device("30000000-0000-4000-8000-000000000093", 93);
    let (seq, previous) = second.type_text(&mut f, &"Another discarded draft ".repeat(2_000));
    let text = second.doc.get_or_insert_text("html");
    let deletion = {
        let mut tx = second.doc.transact_mut();
        let len = text.len(&tx);
        text.remove_range(&mut tx, 0, len);
        tx.encode_update_v1()
    };
    let envelope = object::seal(
        &object::Context {
            space: f.key.space_id.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: "update".into(),
            namespace: "content".into(),
            author_device: second.id.into(),
            membership_revision: "2".into(),
            stream_seq: (seq + 1).to_string(),
            prev_hash: previous,
        },
        &[11; 32],
        &second.signing,
        &deletion,
    )
    .unwrap();
    f.store
        .append(&Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: second.id,
            },
            namespace: Namespace::Content,
            seq: seq + 1,
            hash: envelope.hash().unwrap(),
            previous,
            bytes: &envelope.to_json().unwrap(),
        })
        .unwrap();
    let before = f.read().source;
    let bytes = f.store.page_storage_bytes(PAGE).unwrap();
    let second_before = retained(&f, second.id);
    f.sql().execute_batch("CREATE TRIGGER fail_later_fold BEFORE INSERT ON checkpoints WHEN NEW.stream = '30000000-0000-4000-8000-000000000093' BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    let mut decoder = f.decoder();
    let result =
        tmt_colab::storage::reclaim(&mut f.store, &f.key, PAGE, &mut decoder, false).unwrap();
    let after = f.store.page_storage_bytes(PAGE).unwrap();
    let freed = (bytes.content + bytes.history).saturating_sub(after.content + after.history);
    assert!(freed > 0, "first stream must actually free bytes");
    assert_eq!(result.freed.history, Some(freed));
    assert!(result.error.is_some());
    assert_eq!(retained(&f, second.id), second_before);
    assert!(fold_rows(&f, second.id).is_empty());
    assert_eq!(f.read().source, before);
}

#[test]
fn history_rebuild_keeps_signed_old_epoch_payloads_after_a_new_current_checkpoint() {
    let (mut f, _) = deleted_owner_history("current", false);
    // The epoch seal commits this uncombined tail, so its large draft stays retained.
    let head = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap();
    Engine::with_decoder_config(support::decoder_config(BINARY.into()))
        .unwrap()
        .advance_epoch(
            &mut f.store,
            &f.key,
            EpochAdvance {
                operation_id: "40000000-0000-4000-8000-000000000094",
                expected_revision: head.revision,
                page: PAGE,
            },
            NOW,
        )
        .unwrap();
    let old_payloads = |f: &Fixture| {
        f.sql().prepare("SELECT payload FROM receipts WHERE epoch='1' AND payload IS NOT NULL ORDER BY stream, seq")
            .unwrap().query_map([], |row| row.get::<_, Vec<u8>>(0)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    let retained = old_payloads(&f);
    assert!(retained.iter().map(Vec::len).sum::<usize>() > 66_000);
    f.write("<p>Current epoch after a new edit</p>");
    assert!(request_compaction(&mut f).compacted);
    assert_eq!(
        old_payloads(&f),
        retained,
        "a new pair erased a signed old cut"
    );
    assert_eq!(f.read().source, "<p>Current epoch after a new edit</p>");
}

#[test]
fn history_rebuild_keeps_shared_history_and_public_payloads_merge_only() {
    for (history, public) in [("shared", false), ("current", true)] {
        let (mut f, removed) = deleted_owner_history(history, public);
        let before = f.read();
        assert!(request_compaction(&mut f).compacted);
        let (_, tail, checkpoints) = stored_usage(&f);
        assert!(
            tail + checkpoints > removed,
            "protected history was GC-rebuilt"
        );
        assert_eq!(f.read().source, before.source);
        assert_eq!(f.read().title, before.title);
    }
}

#[test]
fn usage_counts_what_the_page_stores_and_a_request_reports_it_before_and_after() {
    let mut f = Fixture::new();
    let mut source = String::from("old 🐈\r\n");
    let mut last = None;
    for i in 0..12 {
        source.push_str(&format!("<p>{i}</p>"));
        last = Some(f.write(&source));
    }
    let before = f.read().usage;
    let (count, bytes, checkpoints) = stored_usage(&f);
    assert_eq!(
        (before.updates, before.tail_bytes, before.checkpoint_bytes),
        (count, bytes, checkpoints),
        "usage is the stored tail and checkpoints, counted independently"
    );
    assert_eq!(
        before.other_device_updates, 1,
        "the page's creator wrote the first update"
    );
    assert_eq!(before.update_limit, 200);
    assert_eq!(before.tail_bytes_limit, 4 * 1024 * 1024);
    assert_eq!(before.gzip_limit, 5_000_000);
    assert_eq!(
        before.state_bytes,
        bytes + checkpoints,
        "no baseline on a first-epoch page"
    );
    assert_eq!(
        before.gzip_bytes, None,
        "a small state is not gzipped just to be measured"
    );
    assert_eq!(before.percent, before.updates * 100 / 200);
    assert_eq!(before.limiting, page::Limiting::Changes);

    let report = request_compaction(&mut f);
    assert!(report.compacted && report.skipped.is_none());
    assert_eq!((report.through, report.merged_objects), (12, 12));
    let last = last.unwrap();
    assert_eq!(report.by, last.publication.as_ref().unwrap().stream_id);
    assert_eq!(report.page_id, PAGE);
    assert_eq!(report.before, before);
    assert_eq!(report.after, f.read().usage);
    assert_eq!(
        (report.after.updates, report.after.other_device_updates),
        (1, 1),
        "only the other device's change is left, and this device cannot combine it"
    );
    assert!(report.after.tail_bytes < before.tail_bytes);
    assert!(report.after.checkpoint_bytes > 0);
    let (count, bytes, checkpoints) = stored_usage(&f);
    assert_eq!(
        (
            report.after.updates,
            report.after.tail_bytes,
            report.after.checkpoint_bytes
        ),
        (count, bytes, checkpoints)
    );
    // The source is byte-identical, and its hash is the one the last write reported.
    assert_eq!(f.read().source, source);
    assert_eq!(report.source_sha256, last.source_sha256);
}

#[test]
fn a_second_requested_compaction_finds_nothing_to_combine_and_writes_nothing() {
    let mut f = Fixture::new();
    let mut source = String::from("old 🐈\r\n");
    for i in 0..5 {
        source.push_str(&format!("<p>{i}</p>"));
        f.write(&source);
    }
    assert!(request_compaction(&mut f).compacted);
    let stored = f.bytes();
    let revision = f.read().revision;
    let again = request_compaction(&mut f);
    assert!(!again.compacted);
    assert_eq!(
        again.skipped,
        Some(page::compact::Skipped::NothingToCombine)
    );
    assert_eq!((again.through, again.merged_objects), (0, 0));
    assert_eq!(again.before, again.after);
    assert_eq!(
        f.bytes(),
        stored,
        "a no-op leaves the stored state byte-identical"
    );
    assert_eq!(f.read().revision, revision);
    assert_eq!(f.read().source, source);
}

const FOLDED: &str = "50000000-0000-4000-8000-000000000002";

/// The rows of one stream's retained objects: `(seq, has payload)` of its receipts.
fn retained(f: &Fixture, stream: &str) -> Vec<(u64, bool)> {
    let db = f.sql();
    let mut statement = db
        .prepare("SELECT seq, payload IS NOT NULL FROM receipts WHERE stream = ?1 ORDER BY seq")
        .unwrap();
    statement
        .query_map([stream], |row| {
            Ok((row.get::<_, String>(0)?.parse().unwrap(), row.get(1)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
/// Who signed the retained checkpoint of a stream, and its sequence.
fn fold_rows(f: &Fixture, stream: &str) -> Vec<(u64, Option<String>)> {
    let db = f.sql();
    let mut statement = db
        .prepare(
            "SELECT seq, signer FROM checkpoints WHERE stream = ?1 AND payload IS NOT NULL ORDER BY seq",
        )
        .unwrap();
    statement
        .query_map([stream], |row| {
            Ok((row.get::<_, String>(0)?.parse().unwrap(), row.get(1)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_requested_compaction_folds_an_owner_devices_stream_and_an_edit_in_flight_survives_it() {
    let mut f = Fixture::new();
    let mut other = f.owner_device(FOLDED, 21);
    let receipt = f.write("old 🐈\r\n<p>local</p>");
    for i in 0..4 {
        other.type_text(&mut f, &format!("[{i}]"));
    }
    let before = f.read();
    assert_eq!(
        before.usage.updates,
        1 + 1 + 4,
        "creator, local write, four edits"
    );
    let writer = receipt.publication.as_ref().unwrap().stream_id.clone();

    let report = request_compaction(&mut f);
    assert!(report.compacted && report.skipped.is_none());
    assert_eq!(
        report.folded,
        [page::owner_fold::Folded {
            stream: FOLDED.into(),
            through: 4,
            merged_objects: 4
        }]
    );
    // The member-issued creator is not an owner device: its change stays, and the report says so.
    assert_eq!(
        report.left,
        [page::owner_fold::Left {
            stream: DEVICE.into(),
            why: page::owner_fold::Why::NotOwnerDevice
        }]
    );
    assert_eq!(report.unfolded, 0);
    assert_eq!(report.after.updates, before.usage.updates - 4 - 1);
    // The page reads the same, the fold is the stream's checkpoint, signed by this machine's
    // writer, and only the update the fold binds is kept of the stream.
    let after = f.read();
    assert_eq!(after.source, before.source);
    assert_eq!(fold_rows(&f, FOLDED), [(4, Some(writer.clone()))]);
    assert_eq!(
        retained(&f, FOLDED),
        [(1, false), (2, false), (3, false), (4, true)]
    );

    // An edit in flight cites the stream's hash at 4: it is stored on top of the fold, kept as a
    // tail, and the page reads it.
    other.type_text(&mut f, "[in flight]");
    assert!(f.read().source.contains("[in flight]"));
    assert_eq!(f.read().usage.updates, report.after.updates + 1);
    // The next run folds again from the fold: one fold at 5 supersedes the one at 4, and the
    // anchor moves with it.
    let again = request_compaction(&mut f);
    assert_eq!(
        again.folded,
        [page::owner_fold::Folded {
            stream: FOLDED.into(),
            through: 5,
            merged_objects: 2
        }]
    );
    assert_eq!(fold_rows(&f, FOLDED), [(5, Some(writer))]);
    assert_eq!(
        retained(&f, FOLDED),
        [(1, false), (2, false), (3, false), (4, false), (5, true)]
    );
    assert!(f.read().source.contains("[in flight]"));
    // Nothing is left that this device can combine; the creator's change is named once more.
    let last = request_compaction(&mut f);
    assert!(!last.compacted);
    assert_eq!(last.skipped, Some(page::compact::Skipped::NothingToCombine));
    assert_eq!(last.folded, []);
}

/// One object of a stream as the cloud hands it to a machine that holds none of that stream.
struct Held {
    namespace: Namespace,
    hash: [u8; 32],
    previous: [u8; 32],
    bytes: Vec<u8>,
    signer: Option<String>,
}
fn held_envelope<'a>(scope: StreamScope<'a>, seq: u64, held: &'a Held) -> Envelope<'a> {
    Envelope {
        scope,
        namespace: held.namespace,
        seq,
        hash: held.hash,
        previous: held.previous,
        bytes: &held.bytes,
    }
}
/// Start `stream` on `into` from a checkpoint pair at `seq`, the way the native cloud inbound does
/// when the stream was compacted before it was first published.
fn start_stream(into: &mut Fixture, stream: &str, seq: u64, anchor: Option<&Held>, pair: &[Held]) {
    let scope = StreamScope {
        page: PAGE,
        epoch: 1,
        stream,
    };
    let anchor = anchor.map(|held| held_envelope(scope, seq, held));
    let objects: Vec<Bootstrapped> = pair
        .iter()
        .map(|held| Bootstrapped {
            envelope: held_envelope(scope, seq, held),
            signer: held.signer.as_deref(),
        })
        .collect();
    assert_eq!(
        into.store.bootstrap(anchor.as_ref(), &objects).unwrap(),
        Accepted::New
    );
}
/// The update of `stream` at `seq` that `from` holds, stored on `into` as the cloud's tail would be.
fn deliver(from: &Fixture, into: &mut Fixture, stream: &str, seq: u64) {
    let held = held_update(from, stream, seq);
    into.store
        .append(&Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream,
            },
            namespace: held.namespace,
            seq,
            hash: held.hash,
            previous: held.previous,
            bytes: &held.bytes,
        })
        .unwrap();
}
fn namespace_of(name: &str) -> Namespace {
    if name == "own" {
        Namespace::Own
    } else {
        Namespace::Content
    }
}
/// The checkpoints (or folds) of `stream` at `seq` that `from` holds, as the cloud would hold them.
fn held_checkpoints(from: &Fixture, stream: &str, seq: u64) -> Vec<Held> {
    let db = from.sql();
    let mut statement = db
        .prepare(
            "SELECT namespace, hash, head, payload, signer FROM checkpoints \
             WHERE stream = ?1 AND seq = ?2 AND payload IS NOT NULL ORDER BY namespace",
        )
        .unwrap();
    statement
        .query_map([stream, &format!("{seq:020}")], |row| {
            Ok(Held {
                namespace: namespace_of(&row.get::<_, String>(0)?),
                hash: row.get::<_, Vec<u8>>(1)?.try_into().unwrap(),
                previous: row.get::<_, Vec<u8>>(2)?.try_into().unwrap(),
                bytes: row.get(3)?,
                signer: row.get(4)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
/// The update `from` keeps at `seq` of `stream`, which a fold binds and a machine that starts from
/// the fold needs.
fn held_update(from: &Fixture, stream: &str, seq: u64) -> Held {
    let db = from.sql();
    let at = |seq: u64| {
        db.query_row(
            "SELECT namespace, hash, payload FROM receipts WHERE stream = ?1 AND seq = ?2",
            [stream, &format!("{seq:020}")],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                ))
            },
        )
        .unwrap()
    };
    let (namespace, hash, payload) = at(seq);
    Held {
        namespace: namespace_of(&namespace),
        hash: hash.try_into().unwrap(),
        previous: if seq == 1 {
            [0; 32]
        } else {
            at(seq - 1).1.try_into().unwrap()
        },
        bytes: payload.unwrap(),
        signer: None,
    }
}
/// The page source the real `export` command writes.
fn exported_source(f: &Fixture) -> Vec<u8> {
    let output = f
        .command()
        .args(["export", PAGE, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    fs::read(PathBuf::from(value["directory"].as_str().unwrap()).join("page.html")).unwrap()
}

#[test]
fn a_stream_started_from_a_checkpoint_pair_reads_exports_and_compacts_like_the_machine_that_wrote_it()
 {
    let mut a = Fixture::new();
    let mut other = a.owner_device(FOLDED, 21);
    a.write("old 🐈\r\n<p>local</p>");
    // The second machine starts here: the same authority and the same first write, none of the
    // other device's stream.
    let mut b = a.copy();
    for i in 0..4 {
        other.type_text(&mut a, &format!("[{i}]"));
    }
    let head = a
        .store
        .stream_head(StreamScope {
            page: PAGE,
            epoch: 1,
            stream: FOLDED,
        })
        .unwrap()
        .unwrap();
    assert_eq!(head.0, 4);
    // The other device compacted before its first publication: the cloud holds only the pair.
    start_stream(
        &mut b,
        FOLDED,
        4,
        None,
        &other.checkpoint_pair(&a, 4, head.1),
    );

    // Reading and exporting give what the machine that holds every update gives.
    let expected = a.read();
    assert_eq!(b.read().source, expected.source);
    assert!(expected.source.contains("[3]"));
    assert_eq!(exported_source(&b), exported_source(&a));

    // Compacting the second machine's own stream leaves the started stream as it is: it has no
    // tail to combine, so nothing is folded and nothing fails.
    let mut source = b.read().source;
    for i in 0..4 {
        source.push_str(&format!("<p>b{i}</p>"));
        b.write(&source);
    }
    let rows = fold_rows(&b, FOLDED);
    assert_eq!(rows.len(), 2, "both namespaces' checkpoints, at 4");
    let report = request_compaction(&mut b);
    assert!(report.compacted && report.skipped.is_none(), "{report:?}");
    assert_eq!((report.folded.len(), report.unfolded), (0, 0), "{report:?}");
    assert_eq!(fold_rows(&b, FOLDED), rows);
    assert_eq!(b.read().source, source);

    // The tail follows from the next sequence, and reads and exports the same again.
    other.type_text(&mut a, "[tail]");
    deliver(&a, &mut b, FOLDED, 5);
    let after = b.read().source;
    assert!(after.contains("[tail]") && after.contains("[3]") && after.contains("b3"));
    assert_eq!(exported_source(&b), after.as_bytes());
    // Now the stream has a tail, so this machine's fold combines it from the checkpoint the stream
    // started with, and the page reads and exports the same afterwards.
    let report = request_compaction(&mut b);
    assert_eq!(
        report.folded,
        [page::owner_fold::Folded {
            stream: FOLDED.into(),
            through: 5,
            merged_objects: 3
        }],
        "{report:?}"
    );
    assert_eq!(report.unfolded, 0);
    assert_eq!(b.read().source, after);
    assert_eq!(exported_source(&b), after.as_bytes());
    assert_eq!(
        retained(&b, FOLDED),
        [(5, true)],
        "only the fold's anchor is kept of a stream that started from a checkpoint"
    );
}

#[test]
fn a_stream_started_from_an_owner_fold_and_its_anchor_reads_exports_and_folds_again() {
    let mut a = Fixture::new();
    let mut other = a.owner_device(FOLDED, 21);
    let receipt = a.write("old 🐈\r\n<p>local</p>");
    let writer = receipt.publication.as_ref().unwrap().stream_id.clone();
    let mut b = a.copy();
    for i in 0..4 {
        other.type_text(&mut a, &format!("[{i}]"));
    }
    // The first machine folds the other device's stream: the fold is the cloud's checkpoint, and
    // the update it binds is the anchor.
    assert_eq!(request_compaction(&mut a).folded.len(), 1);
    let fold = held_checkpoints(&a, FOLDED, 4);
    assert!(
        !fold.is_empty()
            && fold
                .iter()
                .all(|held| held.signer.as_ref() == Some(&writer))
    );
    let anchor = held_update(&a, FOLDED, 4);
    start_stream(&mut b, FOLDED, 4, Some(&anchor), &fold);

    assert_eq!(b.read().source, a.read().source);
    assert_eq!(exported_source(&b), exported_source(&a));
    assert_eq!(fold_rows(&b, FOLDED), fold_rows(&a, FOLDED));
    assert_eq!(
        retained(&b, FOLDED),
        [(4, true)],
        "the anchor, and no update before it"
    );

    // Nothing is left to combine, so compacting this machine's own stream leaves the fold alone.
    let mut source = b.read().source;
    for i in 0..4 {
        source.push_str(&format!("<p>b{i}</p>"));
        b.write(&source);
    }
    let rows = fold_rows(&b, FOLDED);
    let report = request_compaction(&mut b);
    assert!(report.compacted && report.skipped.is_none(), "{report:?}");
    assert_eq!((report.folded.len(), report.unfolded), (0, 0), "{report:?}");
    assert_eq!(fold_rows(&b, FOLDED), rows);
    assert_eq!(b.read().source, source);

    // The stream's next edit cites the fold's head: it follows, and the next fold supersedes the
    // first one and moves the anchor, as on the machine that made the first.
    other.type_text(&mut a, "[tail]");
    deliver(&a, &mut b, FOLDED, 5);
    let after = b.read().source;
    assert!(after.contains("[tail]") && after.contains("b3"));
    let report = request_compaction(&mut b);
    assert_eq!(report.folded.len(), 1, "{report:?}");
    assert_eq!(fold_rows(&b, FOLDED), [(5, Some(writer))]);
    assert_eq!(retained(&b, FOLDED), [(4, false), (5, true)]);
    assert_eq!(b.read().source, after);
    assert_eq!(exported_source(&b), after.as_bytes());
}

#[test]
fn revoking_the_device_that_signed_a_fold_lists_the_fold_it_signed() {
    let mut f = Fixture::new();
    let mut other = f.owner_device(FOLDED, 21);
    let receipt = f.write("old 🐈\r\n<p>local</p>");
    let writer = receipt.publication.as_ref().unwrap().stream_id.clone();
    other.type_text(&mut f, "[a]");
    other.type_text(&mut f, "[b]");
    let revoke = |f: &mut Fixture, device: &str, operation: &str| {
        let head = f
            .store
            .owner_head(&f.key.space_id, &f.key.owner_public())
            .unwrap()
            .unwrap();
        let outcome = Engine::with_decoder_config(support::decoder_config(BINARY.into()))
            .unwrap()
            .revoke_device(
                &mut f.store,
                &f.key,
                DeviceRevoke {
                    operation_id: operation,
                    expected_revision: head.revision,
                    device_id: device,
                    grant_revision: 1,
                },
                NOW,
            )
            .unwrap();
        let wire: Value = serde_json::from_slice(&outcome).unwrap();
        let statement = &wire["statements"][0];
        let payload = values::binary(
            statement["payload"].as_str().unwrap(),
            tmt_colab_model::payload::MAX_BYTES,
        )
        .unwrap();
        serde_json::from_slice::<Value>(&payload).unwrap()
    };

    // This machine's writer folds the paired device's stream.
    let report = request_compaction(&mut f);
    assert_eq!(report.folded.len(), 1);
    // The paired device signed no fold: its revoke carries no list, as before folds existed.
    let plain = revoke(&mut f, FOLDED, "40000000-0000-4000-8000-0000000000a1");
    assert_eq!(plain["deviceId"], FOLDED);
    assert!(plain.get("folds").is_none());
    // Revoking the signer lists exactly the folds the store retains, sorted as a reader compares them.
    let mut hashes: Vec<Vec<u8>> = f
        .sql()
        .prepare("SELECT hash FROM checkpoints WHERE signer=?1")
        .unwrap()
        .query_map([&writer], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        hashes.len(),
        1,
        "the stream wrote the content namespace only"
    );
    hashes.sort();
    let listed = revoke(&mut f, &writer, "40000000-0000-4000-8000-0000000000a2");
    assert_eq!(
        listed["folds"],
        json!(
            hashes
                .iter()
                .map(|hash| values::encode_binary(hash))
                .collect::<Vec<_>>()
        )
    );
}

#[test]
fn a_device_this_machine_publishes_can_be_folded_after_publication() {
    let mut f = Fixture::new();
    let mut paired = f.owner_device(FOLDED, 21);
    f.write("old 🐈\r\n<p>local</p>");
    paired.type_text(&mut f, "[a]");
    paired.type_text(&mut f, "[b]");
    // A browser paired through this machine's Remote has a registration row.
    f.sql()
        .execute(
            "INSERT INTO device_registrations VALUES (?1, NULL, 0, '1')",
            [FOLDED],
        )
        .unwrap();
    // The page has never been published: nothing says the cloud will read it as a fold, so it folds.
    let report = request_compaction(&mut f);
    assert_eq!(report.folded.len(), 1);
    // Publication evidence does not prevent the next fold: its anchor travels to the cloud.
    paired.type_text(&mut f, "[c]");
    f.sql()
        .execute(
            "INSERT INTO cloud_publications VALUES (?1, '1', ?2, ?3, 'owner', '00000000000000000002', '00000000000000000002')",
            rusqlite::params![PAGE, FOLDED, [1u8; 32].as_slice()],
        )
        .unwrap();
    let report = request_compaction(&mut f);
    assert!(report.compacted);
    assert_eq!(report.folded.len(), 1);
    assert!(!report.left.iter().any(|left| left.stream == FOLDED));
    assert!(f.read().source.contains("[c]"));
}

#[test]
fn a_requested_compaction_past_its_deadline_says_it_could_not_combine_and_writes_nothing() {
    let mut f = Fixture::new();
    let mut source = String::from("old 🐈\r\n");
    for i in 0..4 {
        source.push_str(&format!("<p>{i}</p>"));
        f.write(&source);
    }
    let late = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(1))
        .unwrap();
    let stored = f.bytes();
    let mut decoder = f.decoder();
    let report =
        page::compact::request(&mut f.store, &f.key, PAGE, &mut decoder, Some(late)).unwrap();
    assert!(!report.compacted);
    assert_eq!(report.skipped, Some(page::compact::Skipped::NotCombined));
    assert_eq!((report.through, report.merged_objects), (0, 0));
    assert_eq!(report.before, report.after);
    assert_eq!(f.bytes(), stored, "a combine that gave up changed nothing");
    // The same request with time left combines, so the refusal was the deadline and nothing else.
    assert!(request_compaction(&mut f).compacted);
    assert_eq!(f.read().source, source);
}

#[test]
fn a_fold_run_past_its_deadline_counts_the_devices_it_did_not_reach_and_writes_nothing() {
    let mut f = Fixture::new();
    let mut other = f.owner_device(FOLDED, 21);
    f.write("old 🐈\r\n<p>local</p>");
    other.type_text(&mut f, "[a]");
    other.type_text(&mut f, "[b]");
    let late = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(1))
        .unwrap();
    let stored = f.bytes();
    let mut decoder = f.decoder();
    let report =
        page::compact::request(&mut f.store, &f.key, PAGE, &mut decoder, Some(late)).unwrap();
    // The person is told to run it again: the other device's changes are counted, not dropped.
    assert!(!report.compacted);
    assert_eq!(report.skipped, Some(page::compact::Skipped::NotCombined));
    assert_eq!(report.unfolded, 1);
    assert_eq!(report.folded, []);
    assert_eq!(f.bytes(), stored, "a run that gave up changed nothing");
    // With time left the same request folds it, so the deadline was the only reason.
    let report = request_compaction(&mut f);
    assert_eq!(report.folded.len(), 1);
    assert_eq!(report.unfolded, 0);
}

#[test]
fn a_write_after_a_requested_compaction_lands_in_the_tail_and_the_next_one_keeps_it() {
    let mut f = Fixture::new();
    let mut source = String::from("old 🐈\r\n");
    for i in 0..4 {
        source.push_str(&format!("<p>{i}</p>"));
        f.write(&source);
    }
    assert!(request_compaction(&mut f).compacted);
    source.push_str("<p>after</p>");
    f.write(&source);
    let report = request_compaction(&mut f);
    assert!(report.compacted);
    assert_eq!(
        report.merged_objects, 2,
        "the earlier checkpoint and the one new change"
    );
    assert_eq!(f.read().source, source);
}

#[test]
fn writes_racing_requested_compactions_are_never_lost() {
    use std::sync::mpsc;
    use tmt_colab::publication::{Outcome, Rejection};

    // One round also covers the valid schedule with fewer than three nonempty compactions.
    // Timed channel barriers bound coordination even if the other connection fails.
    for rounds in [1, 10] {
        let mut f = Fixture::new();
        let mut source = String::from("old 🐈\r\n");
        for i in 0..3 {
            source.push_str(&format!("<p>{i}</p>"));
            f.write(&source);
        }
        let layout = Layout::existing(&f.root).unwrap().unwrap();
        let (prepared_tx, prepared_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (compacted_tx, compacted_rx) = mpsc::channel();
        let (written_tx, written_rx) = mpsc::channel();
        let mut combined = 0;
        let last = std::thread::scope(|scope| {
            let writer = scope.spawn(move || {
                let key = Keyring::read(&layout).unwrap();
                let mut store = Store::write_existing(&layout).unwrap();
                let mut decoder =
                    Decoder::with_config(support::decoder_config(BINARY.into())).unwrap();
                for i in 0..rounds {
                    source.push_str(&format!("<b>{i}</b>"));
                    let mut committed = false;
                    // Only this round's one compaction can stale the first preparation.
                    for attempt in 0..2 {
                        let prepared = page::prepare_publication(
                            &store,
                            &key,
                            PAGE,
                            tmt_colab::decoder::ContentEdit {
                                layout: None,
                                source: &source,
                                publisher_agent: None,
                                attachments: None,
                            },
                            None,
                            &mut decoder,
                            NOW,
                        )
                        .unwrap();
                        let page::PublicationPreparation::Write(frozen) = prepared else {
                            panic!("each write changes the source")
                        };
                        if attempt == 0 {
                            prepared_tx.send(()).unwrap();
                            release_rx.recv_timeout(support::DECODER_DEADLINE).unwrap();
                        }
                        let result = page::commit_publication(
                            &mut store,
                            &key,
                            frozen.job(),
                            frozen.packet(),
                            frozen.chain(),
                            NOW,
                        )
                        .unwrap();
                        if attempt == 0 {
                            compacted_rx
                                .recv_timeout(support::DECODER_DEADLINE)
                                .unwrap();
                        }
                        match result.record.outcome {
                            Outcome::Committed { .. } => {
                                committed = true;
                                break;
                            }
                            Outcome::Rejected {
                                code: Rejection::StaleBase,
                                ..
                            } if attempt == 0 => {}
                            outcome => panic!("unexpected write outcome: {outcome:?}"),
                        }
                    }
                    assert!(
                        committed,
                        "the write commits after the one competing compaction"
                    );
                    written_tx.send(source.clone()).unwrap();
                }
                source
            });
            for _ in 0..rounds {
                prepared_rx.recv_timeout(support::DECODER_DEADLINE).unwrap();
                // Release the prepared write and request a compaction on another connection.
                release_tx.send(()).unwrap();
                combined += usize::from(request_compaction(&mut f).compacted);
                compacted_tx.send(()).unwrap();
                let committed_source = written_rx.recv_timeout(support::DECODER_DEADLINE).unwrap();
                assert_eq!(f.read().source, committed_source);
            }
            writer.join().unwrap()
        });
        combined += usize::from(request_compaction(&mut f).compacted);
        assert!(combined > 0, "at least one request combined stored changes");
        if rounds == 1 {
            assert!(
                combined < 3,
                "completion does not require three nonempty compactions"
            );
        }
        assert_eq!(f.read().source, last);
        assert!(!request_compaction(&mut f).compacted);
    }
}

/// A measurement, not a gate: how large the checkpoint grows when a page is rewritten over and
/// over and its device combines each batch. Run it with
/// `cargo test -p tmt-colab --test page churn -- --ignored --nocapture`.
#[test]
#[ignore = "measurement: several hundred rewrites with a decoder run each"]
fn churn_measurement_records_checkpoint_growth_after_merge() {
    // Pseudo-random words so a rewrite shares almost nothing with the page it replaces.
    let rewrite = |seed: u64, bytes: usize| -> String {
        let mut state = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let mut text = String::with_capacity(bytes + 16);
        while text.len() < bytes {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            text.push_str(&format!("<p>{:x}</p>", state >> 20));
        }
        text
    };
    let rows = std::env::var("CHURN_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20 * 1024);
    let cycles: u64 = std::env::var("CHURN_CYCLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(300);
    let mut f = Fixture::new();
    println!(
        "cycle, source bytes, checkpoint bytes, tail updates, tail bytes, state bytes, gzip bytes, percent"
    );
    for cycle in 1..=cycles {
        let source = rewrite(cycle, rows);
        f.write(&source);
        if cycle % 10 == 0 {
            let report = request_compaction(&mut f);
            if cycle % 50 == 0 || cycle == 10 {
                println!(
                    "{cycle}, {}, {}, {}, {}, {}, {:?}, {}",
                    source.len(),
                    report.after.checkpoint_bytes,
                    report.after.updates,
                    report.after.tail_bytes,
                    report.after.state_bytes,
                    report.after.gzip_bytes,
                    report.after.percent
                );
            }
        }
    }
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

#[test]
fn original_author_survives_native_edits_compaction_reload_epoch_and_export() {
    let mut f = Fixture::with_author(Some("original-author"));
    for publisher in [Some("latest-agent"), None] {
        let page::PublicationPreparation::Write(frozen) = page::prepare_publication(
            &f.store,
            &f.key,
            PAGE,
            tmt_colab::decoder::ContentEdit {
                layout: None,
                source: "updated",
                publisher_agent: publisher,
                attachments: None,
            },
            None,
            &mut f.decoder(),
            NOW,
        )
        .unwrap() else {
            panic!("the source or publisher changed, so a write is expected")
        };
        f.commit(&frozen, NOW).unwrap();
        assert_eq!(f.read().original_author.as_deref(), Some("original-author"));
        assert_eq!(f.read().publisher_agent.as_deref(), publisher);
    }
    assert!(compact_now(&mut f, 1).is_some());
    let reopened = Store::read(&f.layout).unwrap();
    assert_eq!(
        page::read(&reopened, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .original_author
            .as_deref(),
        Some("original-author")
    );
    reopened.close().unwrap();
    let mut engine = Engine::with_decoder_config(support::decoder_config(BINARY.into())).unwrap();
    let revision = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap()
        .revision;
    engine
        .advance_epoch(
            &mut f.store,
            &f.key,
            EpochAdvance {
                operation_id: "40000000-0000-4000-8000-000000000099",
                expected_revision: revision,
                page: PAGE,
            },
            NOW,
        )
        .unwrap();
    assert_eq!(f.read().original_author.as_deref(), Some("original-author"));
    let bundle = tmt_colab::export::Bundle::capture(
        &f.store,
        &f.key,
        PAGE,
        &mut f.decoder(),
        NOW,
        &tmt_colab::export::attachments::Unserved,
    )
    .unwrap();
    let exported = bundle.publish(&f.root).unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(exported.directory.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["originalAuthor"], "original-author");
    assert!(manifest.get("publisherAgent").is_none());
}

/// The layout the authenticated fold reports for a page, read from its authenticated fold.
fn layout_in(store: &Store, f: &Fixture) -> Option<tmt_colab::decoder::PageLayout> {
    let mut page = json!({"pageId": PAGE, "archived": false, "warnings": []});
    tmt_colab::inspection::title(store, &f.key, &mut page, BINARY.into()).unwrap();
    let kind = tmt_colab::decoder::LayoutKind::parse(page["layout"].as_str()?)?;
    Some(tmt_colab::decoder::PageLayout {
        kind,
        reason: page["layoutReason"].as_str().map(str::to_owned),
    })
}
fn layout_of(f: &Fixture) -> Option<tmt_colab::decoder::PageLayout> {
    layout_in(&f.store, f)
}
fn write_with_layout(
    f: &mut Fixture,
    source: &str,
    layout: Option<&tmt_colab::decoder::PageLayout>,
) {
    let page::PublicationPreparation::Write(frozen) = page::prepare_publication(
        &f.store,
        &f.key,
        PAGE,
        tmt_colab::decoder::ContentEdit {
            layout,
            source,
            publisher_agent: None,
            attachments: None,
        },
        None,
        &mut f.decoder(),
        NOW,
    )
    .unwrap() else {
        panic!("the source or layout changed, so a write is expected")
    };
    f.commit(&frozen, NOW).unwrap();
}

#[test]
fn layout_is_page_metadata_that_survives_edits_compaction_reload_and_epoch() {
    use tmt_colab::decoder::{LayoutKind, PageLayout};
    let mut f = Fixture::with_author(None);
    // A page seeded without layout metadata is freeform: an absence, never a stored value.
    assert_eq!(layout_of(&f), None);
    let canvas = PageLayout {
        kind: LayoutKind::Canvas,
        reason: Some("A status board draws its own cards".into()),
    };
    // A metadata-only change publishes: the source is the one the page already has.
    let source = f.read().source;
    write_with_layout(&mut f, &source, Some(&canvas));
    assert_eq!(layout_of(&f), Some(canvas.clone()));
    // Later source writes without a layout keep it.
    write_with_layout(&mut f, "updated", None);
    assert_eq!(f.read().source, "updated");
    assert_eq!(layout_of(&f), Some(canvas.clone()));
    assert!(compact_now(&mut f, 1).is_some());
    let reopened = Store::read(&f.layout).unwrap();
    assert_eq!(layout_in(&reopened, &f), Some(canvas.clone()));
    reopened.close().unwrap();
    let mut engine = Engine::with_decoder_config(support::decoder_config(BINARY.into())).unwrap();
    let revision = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap()
        .revision;
    engine
        .advance_epoch(
            &mut f.store,
            &f.key,
            EpochAdvance {
                operation_id: "40000000-0000-4000-8000-000000000098",
                expected_revision: revision,
                page: PAGE,
            },
            NOW,
        )
        .unwrap();
    assert_eq!(layout_of(&f), Some(canvas));
    // Changing to a column drops the reason; an invalid layout is refused before anything is written.
    let column = PageLayout {
        kind: LayoutKind::Column,
        reason: None,
    };
    write_with_layout(&mut f, "updated", Some(&column));
    assert_eq!(layout_of(&f), Some(column));
    let invalid = PageLayout {
        kind: LayoutKind::Canvas,
        reason: None,
    };
    assert!(
        page::prepare_publication(
            &f.store,
            &f.key,
            PAGE,
            tmt_colab::decoder::ContentEdit {
                layout: Some(&invalid),
                source: "updated",
                publisher_agent: None,
                attachments: None,
            },
            None,
            &mut f.decoder(),
            NOW,
        )
        .is_err()
    );
}

#[test]
fn authenticated_status_survives_own_compaction_and_reopens_from_the_same_causal_action() {
    use tmt_colab::discussion::{self, StatusEdit};
    let mut f = Fixture::new();
    seed_member_thread(&mut f);
    let (prepared, action) = discussion::prepare_status(
        &f.store,
        &f.key,
        PAGE,
        StatusEdit {
            thread: THREAD,
            resolved: true,
            agent_name: Some("Review agent"),
        },
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    commit_status(&mut f, &prepared).unwrap();
    f.write("source before checkpoint");
    let before = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let compacted = compact_now(&mut f, 1).unwrap();
    assert_eq!(compacted.namespaces, 2);
    let after = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert_eq!(
        serde_json::to_value(&after.conversations.threads).unwrap(),
        serde_json::to_value(&before.conversations.threads).unwrap()
    );
    assert_eq!(f.read().source, "source before checkpoint");
    let (reopen, next) = discussion::prepare_status(
        &f.store,
        &f.key,
        PAGE,
        StatusEdit {
            thread: THREAD,
            resolved: false,
            agent_name: Some("Review agent"),
        },
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    assert_eq!(next.previous.as_ref().unwrap().id, action.action_id);
    commit_status(&mut f, &reopen).unwrap();
    let final_view = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert!(!final_view.conversations.threads[0].resolved);
    assert_eq!(
        final_view.conversations.threads[0]
            .status
            .as_ref()
            .unwrap()
            .depth,
        2
    );
    assert!(final_view.conversations.asks.is_empty());
}

#[test]
fn frozen_creation_preference_survives_compaction_epoch_reload_and_export() {
    let recipient = tmt_colab::decoder::CreationRecipient {
        machine_id: "40000000-0000-4000-8000-000000000001".into(),
        agent_id: "50000000-0000-1000-8000-000000000001".into(),
    };
    for hint in [Some(&recipient), None] {
        let mut f = Fixture::with_creation_recipient(hint);
        assert_eq!(f.read().creation_recipient.as_ref(), hint);
        for i in 0..4 {
            f.write(&format!("later source {i}"));
        }
        assert!(compact_now(&mut f, 2).is_some());
        assert_eq!(f.read().creation_recipient.as_ref(), hint);
        let reopened = Store::read(&f.layout).unwrap();
        assert_eq!(
            page::read(&reopened, &f.key, PAGE, &mut f.decoder())
                .unwrap()
                .creation_recipient
                .as_ref(),
            hint
        );
        reopened.close().unwrap();
        Engine::with_decoder_config(support::decoder_config(BINARY.into()))
            .unwrap()
            .advance_epoch(
                &mut f.store,
                &f.key,
                EpochAdvance {
                    page: PAGE,
                    operation_id: "40000000-0000-4000-8000-000000000088",
                    expected_revision: 2,
                },
                NOW,
            )
            .unwrap();
        f.write("after epoch");
        assert_eq!(f.read().creation_recipient.as_ref(), hint);
        let bundle = tmt_colab::export::Bundle::capture(
            &f.store,
            &f.key,
            PAGE,
            &mut f.decoder(),
            NOW,
            &tmt_colab::export::attachments::Unserved,
        )
        .unwrap();
        let parent = f.root.join("exports");
        fs::create_dir(&parent).unwrap();
        let published = bundle.publish(&parent).unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(published.directory.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest.get("creationRecipient"),
            hint.map(|v| serde_json::to_value(v).unwrap()).as_ref()
        );
    }
}

#[test]
fn creation_observes_each_public_snapshot_once_and_absence_never_blocks_creation() {
    let identity =
        json!({"identity":{"name":"creator","id":"50000000-0000-1000-8000-000000000001"}});
    let running = json!({"running":true,"origin":"http://127.0.0.1:53253","path":"/r/abcdefghijkl2345","machineId":"40000000-0000-4000-8000-000000000001"});
    for (caller, descriptor, success, expected) in [
        (identity.clone(), running.clone(), true, true),
        (
            identity.clone(),
            json!({"running":true,"origin":"http://127.0.0.1:53253","path":"/r/abcdefghijkl2345"}),
            true,
            false,
        ),
        (
            identity.clone(),
            json!({"running":false,"lastPort":null}),
            true,
            false,
        ),
        (
            identity.clone(),
            json!({"running":false,"unexpected":1}),
            true,
            false,
        ),
        (
            identity.clone(),
            json!({"error":{"code":"REMOTE_SERVE_OUTDATED"}}),
            false,
            false,
        ),
        (
            json!({"identity":{"name":"creator"}}),
            running.clone(),
            true,
            false,
        ),
        (
            json!({"error":{"code":"IDENTITY_REQUIRED"}}),
            running.clone(),
            true,
            false,
        ),
    ] {
        let f = Fixture::new();
        let script = format!(
            r#"#!/usr/bin/python3
import json, sys
from pathlib import Path
root = Path({root})
args = sys.argv[1:]
with (root / 'observations').open('a') as log:
    log.write(' '.join(args) + '\n')
if args == ['api']:
    json.load(sys.stdin)
    print(json.dumps({{'dataRoot': str(root)}}))
elif args == ['identity', 'show', '--json']:
    print({caller})
elif args == ['remote', 'status', '--machine', '--json']:
    print({descriptor})
    sys.exit({exit})
elif args == ['remote', 'devices', '--json']:
    print('{{"devices":[]}}')
else:
    sys.exit(97)
"#,
            root = serde_json::to_string(f.root.to_str().unwrap()).unwrap(),
            caller = serde_json::to_string(&caller.to_string()).unwrap(),
            descriptor = serde_json::to_string(&descriptor.to_string()).unwrap(),
            exit = if success { 0 } else { 1 },
        );
        let core = f.root.join("creation-core");
        tmt_test_support::write_executable(&core, script.as_bytes(), 0o700).unwrap();
        let output = Command::new(BINARY)
            .current_dir(&f.root)
            .env_clear()
            .env("TMT_EXECUTABLE", &core)
            .env("HOME", &f.root)
            // Creation opens the new page; opening is not under test here, and no real opener is reachable.
            .env("TMT_AGENT", "1")
            .env("PATH", core_fixture::opener_guard(&f.root))
            .args([
                "page", "create", "--title", "Created", "--layout", "column", "--json",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        let page = page::read(
            &f.store,
            &f.key,
            result["pageId"].as_str().unwrap(),
            &mut f.decoder(),
        )
        .unwrap();
        assert_eq!(page.creation_recipient.is_some(), expected);
        if let Some(hint) = page.creation_recipient {
            assert_eq!(hint.machine_id, running["machineId"].as_str().unwrap());
            assert_eq!(hint.agent_id, identity["identity"]["id"].as_str().unwrap());
        }
        let observed = fs::read_to_string(f.root.join("observations")).unwrap();
        assert_eq!(
            observed
                .lines()
                .filter(|line| *line == "identity show --json")
                .count(),
            1
        );
        assert_eq!(
            observed
                .lines()
                .filter(|line| *line == "remote status --machine --json")
                .count(),
            1
        );
        assert!(!observed.lines().any(|line| line == "remote status --json"));
        assert!(
            !observed.contains("serve") && !observed.contains("pair") && !observed.contains("talk")
        );
    }
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
                layout: None,
                source,
                publisher_agent: publisher,
                attachments: None,
            },
            expected,
            // Admission is semantic, not a production-deadline benchmark. Match the legacy
            // preparation path's fixture budget, including its large own-stream checkpoint.
            &mut f.decoder(),
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
                    layout: None,
                    source: &text,
                    publisher_agent: None,
                    attachments: None,
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

    #[test]
    fn an_insufficient_deferred_combine_keeps_the_original_capacity_refusal() {
        let mut f = Fixture::new();
        let first = "a".repeat(1_536 * 1024);
        f.write(&first);
        // Another valid writer retains three MiB of history without changing the final
        // source. The local writer may reclaim its own tail, never this foreign tail.
        let doc = content(&f);
        let html = doc.get_or_insert_text("html");
        let block = "x".repeat(64 * 1024);
        for _ in 0..48 {
            let delta = {
                let mut tx = doc.transact_mut();
                html.insert(&mut tx, 0, &block);
                tx.encode_update_v1()
            };
            append(&mut f, false, Namespace::Content, &delta);
            let delta = {
                let mut tx = doc.transact_mut();
                html.remove_range(&mut tx, 0, block.len() as u32);
                tx.encode_update_v1()
            };
            append(&mut f, false, Namespace::Content, &delta);
        }
        assert_eq!(f.read().source, first);
        let replacement = "b".repeat(first.len());
        let original = f.prepare(&replacement, None).err().unwrap();
        assert!(
            matches!(original.downcast_ref::<tmt_colab::store::owner::OwnerFault>(),
            Some(tmt_colab::store::owner::OwnerFault::PageCapacity(capacity)) if capacity.compactable)
        );
        let file = f.root.join("replace.html");
        fs::write(&file, &replacement).unwrap();
        let output = f
            .patch_command()
            .args(["--file", file.to_str().unwrap(), "--json"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let failure: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(failure["error"]["code"], "COLAB_CAPACITY");
        // These are separate preparations: fresh Yjs client IDs can change encoded sizes.
        // Keep the limit and refusal text exact, but compare each requested size structurally.
        let normalize = |message: &str| {
            let (before, after) = message.split_once(" bytes), more than the ").unwrap();
            let (prefix, requested) = before.rsplit_once('(').unwrap();
            let requested = requested.parse::<usize>().unwrap();
            let (limit, _) = after.split_once(" bytes) one page can hold").unwrap();
            let limit = limit.rsplit_once('(').unwrap().1.parse::<usize>().unwrap();
            assert_eq!(limit, tmt_colab::decoder::WRITE_TAIL_BYTES);
            assert!(requested > limit, "{message}");
            format!("{prefix}(<requested> bytes), more than the {after}")
        };
        assert_eq!(
            normalize(failure["error"]["message"].as_str().unwrap()),
            normalize(&original.to_string())
        );
        let after = f.read();
        assert_eq!(after.source, first);
        assert!(
            after.usage.checkpoint_bytes > 0,
            "the own combine did not commit"
        );
        assert_eq!(after.usage.updates, after.usage.other_device_updates);
        assert!(after.usage.tail_bytes > 3 * 1024 * 1024);
        // Failed replacement cannot record a new publication, despite legitimate maintenance.
        let publications: i64 = f
            .sql()
            .query_row(
                "SELECT count(*) FROM owner_operations WHERE publication_kind IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(publications, 1);
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
            "INSERT INTO checkpoints(page,epoch,stream,namespace,seq,hash,digest,head,payload,pinned) VALUES(?,?,?,?,?,?,?,?,?,0)",
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
                layout: None,
                source: "new",
                publisher_agent: None,
                attachments: None,
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
                    layout: None,
                    source: "old 🐈\r\n",
                    publisher_agent: None,
                    attachments: None,
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
                layout: None,
                source: "new",
                publisher_agent: None,
                attachments: None,
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
                    layout: None,
                    source: "old 🐈\r\n",
                    publisher_agent: None,
                    attachments: None,
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

#[test]
fn proposals_are_complete_before_placement_and_recover_by_id_without_notifications() {
    use tmt_colab::{
        discussion,
        threads::{Proposal, Proposer},
    };
    let mut f = Fixture::new();
    let proposal = Proposal {
        proposal_id: "90000000-0000-4000-8000-000000000001".into(),
        title: "Review 🐈".into(),
        body: "<script>display text</script>".into(),
        proposer: Proposer {
            machine_id: "40000000-0000-4000-8000-000000000001".into(),
            agent_id: "50000000-0000-1000-8000-000000000001".into(),
            label: "Proposer".into(),
        },
    };
    let source = f.read().source;
    let frozen =
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &proposal, &mut f.decoder(), NOW)
            .unwrap()
            .unwrap();
    f.commit(&frozen, NOW).unwrap();
    assert_eq!(
        f.read().source,
        source,
        "record-first publication leaves source alone"
    );
    let view = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert_eq!(
        view.conversations.threads[0].proposal.as_ref(),
        Some(&proposal)
    );
    assert!(!view.conversations.threads[0].resolved);
    assert!(view.conversations.threads[0].decision.is_none());
    assert!(
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &proposal, &mut f.decoder(), NOW)
            .unwrap()
            .is_none()
    );
    let mut changed = proposal.clone();
    changed.body = "Changed".into();
    assert!(
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &changed, &mut f.decoder(), NOW)
            .is_err()
    );
    // Existing-ID recovery uses retained provenance, even without a running Remote.
    let run = |f: &Fixture| {
        f.command()
            .args([
                "proposal",
                "add",
                PAGE,
                "--id",
                &proposal.proposal_id,
                "--title",
                &proposal.title,
                "--body",
                &proposal.body,
                "--json",
            ])
            .output()
            .unwrap()
    };
    let first = run(&f);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&first.stdout).unwrap()["placed"],
        true
    );
    let human = f
        .command()
        .args([
            "proposal",
            "add",
            PAGE,
            "--id",
            "90000000",
            "--title",
            &proposal.title,
            "--body",
            &proposal.body,
        ])
        .output()
        .unwrap();
    assert!(human.status.success());
    let shown = String::from_utf8(human.stdout).unwrap();
    assert!(
        shown.contains(&proposal.proposal_id) && shown.contains("placed") && shown.contains("yes")
    );
    assert!(!shown.contains("spaceId") && !shown.contains("{\""));
    let after = f.read();
    let second = run(&f);
    assert!(second.status.success());
    assert_eq!(
        f.read().revision,
        after.revision,
        "already placed recovery publishes nothing"
    );
    assert_eq!(f.read().source.matches("<tmt-proposal ").count(), 1);
    let listed = f
        .command()
        .args(["proposal", "ls", PAGE, "--json"])
        .output()
        .unwrap();
    assert!(listed.status.success());
    let rows: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(rows["proposals"].as_array().unwrap().len(), 1);
    let human = f.command().args(["proposal", "ls", PAGE]).output().unwrap();
    assert!(human.status.success());
    let shown = String::from_utf8(human.stdout).unwrap();
    assert!(
        shown.contains("90000000")
            && shown.contains(&proposal.title)
            && shown.contains("open")
            && shown.contains("not resolved")
    );
    for expected in [true, false] {
        let output = f
            .command()
            .args(["proposal", "resolve", PAGE, "90000000", "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["changed"],
            expected
        );
    }
    let human = f
        .command()
        .args(["proposal", "resolve", PAGE, &proposal.proposal_id])
        .output()
        .unwrap();
    assert!(human.status.success());
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .contains("already resolved")
    );
    let view = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert!(view.conversations.threads[0].resolved);
    assert!(
        view.conversations.threads[0]
            .status
            .as_ref()
            .unwrap()
            .action
            .recipients
            .is_empty()
    );
    let duplicate = format!(
        "{}\n<tmt-proposal data-id=\"{}\"></tmt-proposal>",
        f.read().source,
        proposal.proposal_id
    );
    f.write(&duplicate);
    let output = run(&f);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["placed"],
        false
    );
    assert_eq!(
        f.read().source,
        duplicate,
        "duplicate recovery never guesses a placement"
    );
    let human = f
        .command()
        .args([
            "proposal",
            "add",
            PAGE,
            "--id",
            "90000000",
            "--title",
            &proposal.title,
            "--body",
            &proposal.body,
        ])
        .output()
        .unwrap();
    assert!(human.status.success());
    let shown = String::from_utf8(human.stdout).unwrap();
    assert!(shown.contains(&format!(
        "tmt colab proposal add {PAGE} --id {}",
        proposal.proposal_id
    )));
    let mut other = proposal.clone();
    other.proposal_id = "90000000-1000-4000-8000-000000000002".into();
    let frozen =
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &other, &mut f.decoder(), NOW)
            .unwrap()
            .unwrap();
    f.commit(&frozen, NOW).unwrap();
    let before = f.read().revision;
    for prefix in ["90000000", "deadbeef"] {
        let resolve = f
            .command()
            .args(["proposal", "resolve", PAGE, prefix, "--json"])
            .output()
            .unwrap();
        let add = f
            .command()
            .args([
                "proposal",
                "add",
                PAGE,
                "--id",
                prefix,
                "--title",
                &proposal.title,
                "--body",
                &proposal.body,
                "--json",
            ])
            .output()
            .unwrap();
        for output in [resolve, add] {
            assert!(!output.status.success());
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            let message = value["error"]["message"].as_str().unwrap();
            assert!(
                message.contains(if prefix == "90000000" {
                    "matches more than one"
                } else {
                    "No proposal matches"
                }),
                "{message}"
            );
        }
        assert_eq!(
            f.read().revision,
            before,
            "ambiguous/missing prefix never publishes"
        );
    }
}

#[test]
fn the_owner_summary_comes_from_the_real_fold_and_follows_each_proposal_state() {
    use tmt_colab::{
        discussion,
        summary::summarize,
        threads::{Proposal, Proposer},
    };
    let mut f = Fixture::with_metadata(
        Some("Studio agent"),
        Some(&tmt_colab::decoder::CreationRecipient {
            machine_id: "40000000-0000-4000-8000-000000000001".into(),
            agent_id: "50000000-0000-1000-8000-000000000001".into(),
        }),
    );
    let root = f.root.clone();
    let summarize_now = || {
        let mut source = tmt_colab::page::save::open_source(
            &root,
            support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
        )
        .unwrap();
        summarize(&mut source, PAGE).unwrap()
    };
    // A page with no discussion: nothing counted, nothing waiting, the creation label named.
    let empty = summarize_now();
    assert_eq!((empty.comments, empty.waiting), (0, false));
    assert_eq!(empty.author.as_deref(), Some("Studio agent"));
    let proposal = Proposal {
        proposal_id: "90000000-0000-4000-8000-000000000001".into(),
        title: "Review".into(),
        body: "Review body".into(),
        proposer: Proposer {
            machine_id: "40000000-0000-4000-8000-000000000001".into(),
            agent_id: "50000000-0000-1000-8000-000000000001".into(),
            label: "Proposer".into(),
        },
    };
    let frozen =
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &proposal, &mut f.decoder(), NOW)
            .unwrap()
            .unwrap();
    f.commit(&frozen, NOW).unwrap();
    // An undecided proposal awaits the owner.
    assert!(summarize_now().waiting);
    let view = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let reference = tmt_colab::threads::status::Reference {
        writer: view.conversations.threads[0].writer.clone(),
        id: proposal.proposal_id.clone(),
    };
    // Resolving the proposal thread clears the mark, reopening it brings it back.
    for (resolved, waiting) in [(true, false), (false, true)] {
        let (frozen, _) = discussion::prepare_status(
            &f.store,
            &f.key,
            PAGE,
            discussion::StatusEdit {
                thread: &proposal.proposal_id,
                resolved,
                agent_name: None,
            },
            &mut f.decoder(),
            NOW,
        )
        .unwrap()
        .unwrap();
        f.commit(&frozen, NOW).unwrap();
        assert_eq!(summarize_now().waiting, waiting, "resolved: {resolved}");
    }
    // A final decision clears it for good.
    let frozen = discussion::prepare_decision(
        &f.store,
        &f.key,
        PAGE,
        discussion::DecisionEdit {
            thread: &reference,
            decision: "approved",
        },
        &mut f.decoder(),
        NOW,
    )
    .unwrap();
    f.commit(&frozen, NOW).unwrap();
    let decided = summarize_now();
    assert!(!decided.waiting);
    // A proposal thread is not an annotation thread: it adds no comment to the count.
    assert_eq!(decided.comments, 0);
    // A page without a creation label has no author.
    let bare = Fixture::new();
    let mut source = tmt_colab::page::save::open_source(
        &bare.root,
        support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
    )
    .unwrap();
    assert_eq!(summarize(&mut source, PAGE).unwrap().author, None);
}

#[test]
fn the_owner_summary_counts_live_comments_of_annotation_threads_from_the_real_fold() {
    let mut f = Fixture::new();
    seed_member_thread_with(
        &mut f,
        &[
            ("60000000-0000-4000-8000-000000000001", "1", false, "first"),
            ("60000000-0000-4000-8000-000000000002", "1", false, "second"),
            // A deleted comment, and a question answer, are not comments about the page.
            ("60000000-0000-4000-8000-000000000003", "1", true, ""),
            (
                "60000000-0000-4000-8000-000000000004",
                "1",
                false,
                "Answer to \u{201c}Which?\u{201d}: this one",
            ),
        ],
    );
    let mut source = tmt_colab::page::save::open_source(
        &f.root,
        support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
    )
    .unwrap();
    let summary = tmt_colab::summary::summarize(&mut source, PAGE).unwrap();
    assert_eq!(summary.comments, 2);
    assert!(!summary.waiting);
}

#[test]
fn the_owner_list_serves_fresh_summaries_only_and_refreshes_a_changed_page_in_the_background() {
    use tmt_colab::{
        discussion,
        registration::Registration,
        threads::{Proposal, Proposer},
    };
    let mut f = Fixture::with_metadata(
        Some("Studio agent"),
        Some(&tmt_colab::decoder::CreationRecipient {
            machine_id: "40000000-0000-4000-8000-000000000001".into(),
            agent_id: "50000000-0000-1000-8000-000000000001".into(),
        }),
    );
    let root = f.root.clone();
    let context = json!({"deviceId":DEVICE,"kind":"browser","origin":"http://127.0.0.1:1","name":"Laptop",
        "publicKey":values::encode_binary(SigningKey::from_bytes(&[9; 32]).verifying_key().as_bytes()),
        "owner":true,"grantRevision":1})
    .to_string();
    let registration = Registration::with_decoder_config(
        Store::open(&f.layout).unwrap(),
        Keyring::read(&f.layout).unwrap(),
        support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
    )
    .unwrap()
    .with_summary_source(std::sync::Arc::new(move || {
        tmt_colab::page::save::open_source(
            &root,
            support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
        )
    }));
    let entry = || -> Value {
        let list: Value =
            serde_json::from_slice(&registration.pages(Some(&context)).unwrap()).unwrap();
        list["pages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|page| page["pageId"] == PAGE)
            .unwrap()
            .clone()
    };
    let settled = |expected_waiting: bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let page = entry();
            if page["summary"]["waiting"] == expected_waiting && page.get("summary").is_some() {
                return page;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no fresh summary: {page}"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    };
    // The first read replies at once without a summary and schedules the refresh.
    assert!(entry().get("summary").is_none());
    let fresh = settled(false);
    assert_eq!(
        fresh["summary"],
        json!({"comments":0,"waiting":false,"needsYou":0,"author":"Studio agent"})
    );
    // An accepted write changes the page's fingerprint: the old summary is stale, so it is no
    // longer served, and the next refresh brings the new state.
    let proposal = Proposal {
        proposal_id: "90000000-0000-4000-8000-000000000001".into(),
        title: "Review".into(),
        body: "Review body".into(),
        proposer: Proposer {
            machine_id: "40000000-0000-4000-8000-000000000001".into(),
            agent_id: "50000000-0000-1000-8000-000000000001".into(),
            label: "Proposer".into(),
        },
    };
    let frozen =
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &proposal, &mut f.decoder(), NOW)
            .unwrap()
            .unwrap();
    f.commit(&frozen, NOW).unwrap();
    assert!(
        entry().get("summary").is_none(),
        "a page changed since its summary was computed must not serve it"
    );
    let pending = settled(true);
    assert_eq!(pending["summary"]["waiting"], true);
    assert_eq!(pending["summary"]["needsYou"], 1);
}

/// A registration that folds the fixture's page on its own summary worker, and the owner context a
/// list read needs.
fn title_registration(f: &Fixture) -> (tmt_colab::registration::Registration, String) {
    let root = f.root.clone();
    let context = json!({"deviceId":DEVICE,"kind":"browser","origin":"http://127.0.0.1:1","name":"Laptop",
        "publicKey":values::encode_binary(SigningKey::from_bytes(&[9; 32]).verifying_key().as_bytes()),
        "owner":true,"grantRevision":1})
    .to_string();
    let registration = tmt_colab::registration::Registration::with_decoder_config(
        Store::open(&f.layout).unwrap(),
        Keyring::read(&f.layout).unwrap(),
        support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
    )
    .unwrap()
    .with_summary_source(std::sync::Arc::new(move || {
        tmt_colab::page::save::open_source(
            &root,
            support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
        )
    }));
    (registration, context)
}

/// The stored title row as (title, progress key it was folded at).
fn stored_title(f: &Fixture) -> Option<(String, String)> {
    use rusqlite::OptionalExtension;
    f.sql()
        .query_row(
            "SELECT title, progress FROM page_titles WHERE page=?",
            [PAGE],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap()
}

/// Polls `row` until it yields a title row other than `previous`, within a bounded time.
fn title_row_after(
    f: &Fixture,
    previous: Option<&(String, String)>,
    mut kick: impl FnMut(),
) -> (String, String) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        kick();
        let row = stored_title(f);
        if let Some(found) = row.as_ref().filter(|found| Some(*found) != previous) {
            return found.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no title row: {row:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn the_summary_worker_records_the_verified_title_it_folds_and_a_changed_page_replaces_it() {
    let mut f = Fixture::new();
    let (registration, context) = title_registration(&f);
    // A list read schedules the drain; the row appears (or changes) once the worker has folded.
    let list = || {
        registration.pages(Some(&context)).unwrap();
    };
    // The catalog entry the browser reads: its optional `title` is the projection's, only while fresh.
    let listed = || -> Option<String> {
        let list: Value =
            serde_json::from_slice(&registration.pages(Some(&context)).unwrap()).unwrap();
        let page = list["pages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|page| page["pageId"] == PAGE)
            .unwrap()
            .clone();
        page["title"].as_str().map(str::to_owned)
    };
    assert_eq!(stored_title(&f), None);
    assert_eq!(listed(), None);
    let first = title_row_after(&f, None, list);
    assert_eq!(first.0, "Exact title 🐈");
    assert_eq!(listed().as_deref(), Some("Exact title 🐈"));
    // A changed page keeps its old row until the worker folds the new state, and the replaced row
    // carries the new progress key; the catalog never serves the stale one.
    f.write("new 🐈\r\n");
    assert_eq!(stored_title(&f).as_ref(), Some(&first));
    assert_eq!(listed(), None, "a changed page serves no stale title");
    let second = title_row_after(&f, Some(&first), list);
    assert_eq!(second.0, "Exact title 🐈");
    assert_ne!(second.1, first.1);
    assert_eq!(listed().as_deref(), Some("Exact title 🐈"));
}

#[test]
fn starting_the_summaries_fills_a_store_without_titles_and_no_list_read() {
    let f = Fixture::new();
    let (registration, _context) = title_registration(&f);
    assert_eq!(stored_title(&f), None);
    // The serve's start: the one drain every page without a title waits for after an upgrade.
    registration.start_summaries();
    assert_eq!(
        title_row_after(&f, None, || ()).0,
        "Exact title 🐈",
        "the start drain records the title"
    );
}

#[test]
fn proposal_decision_is_final_and_resolution_remains_independent() {
    use tmt_colab::{
        discussion,
        threads::{Proposal, Proposer},
    };
    let mut f = Fixture::new();
    let proposal = Proposal {
        proposal_id: "90000000-0000-4000-8000-000000000001".into(),
        title: "Review".into(),
        body: "Review body".into(),
        proposer: Proposer {
            machine_id: "40000000-0000-4000-8000-000000000001".into(),
            agent_id: "50000000-0000-1000-8000-000000000001".into(),
            label: "Proposer".into(),
        },
    };
    let frozen =
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &proposal, &mut f.decoder(), NOW)
            .unwrap()
            .unwrap();
    f.commit(&frozen, NOW).unwrap();
    let view = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let reference = tmt_colab::threads::status::Reference {
        writer: view.conversations.threads[0].writer.clone(),
        id: proposal.proposal_id.clone(),
    };
    let frozen = discussion::prepare_decision(
        &f.store,
        &f.key,
        PAGE,
        discussion::DecisionEdit {
            thread: &reference,
            decision: "approved",
        },
        &mut f.decoder(),
        NOW,
    )
    .unwrap();
    f.commit(&frozen, NOW).unwrap();
    assert!(
        discussion::prepare_decision(
            &f.store,
            &f.key,
            PAGE,
            discussion::DecisionEdit {
                thread: &reference,
                decision: "declined"
            },
            &mut f.decoder(),
            NOW
        )
        .is_err()
    );
    for resolved in [true, false] {
        let (frozen, _) = discussion::prepare_status(
            &f.store,
            &f.key,
            PAGE,
            discussion::StatusEdit {
                thread: &proposal.proposal_id,
                resolved,
                agent_name: None,
            },
            &mut f.decoder(),
            NOW,
        )
        .unwrap()
        .unwrap();
        f.commit(&frozen, NOW).unwrap();
        let view = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
        assert_eq!(
            view.conversations.threads[0]
                .decision
                .as_ref()
                .unwrap()
                .decision,
            "approved"
        );
        assert_eq!(view.conversations.threads[0].resolved, resolved);
    }
}

#[test]
fn proposal_cli_creates_from_public_provenance_and_refuses_unavailable_provenance() {
    let f = Fixture::new();
    let core = f.root.join("proposal-core");
    let reply = json!({"dataRoot":f.root}).to_string();
    tmt_test_support::write_executable(&core,format!(r#"#!/bin/sh
case "$*" in
'api') printf '%s\n' '{reply}';;
'identity show --json') printf '%s\n' '{{"identity":{{"name":"Proposer","id":"50000000-0000-1000-8000-000000000001"}}}}';;
'remote status --machine --json') printf '%s\n' '{{"running":true,"origin":"http://127.0.0.1:53253","path":"/r/abcdefghijkl2345","machineId":"40000000-0000-4000-8000-000000000001"}}';;
*) exit 9;;
esac
"#).as_bytes(),0o700).unwrap();
    let output = f
        .command()
        .env("TMT_EXECUTABLE", &core)
        .args([
            "proposal",
            "add",
            PAGE,
            "--title",
            "New proposal",
            "--body",
            "Review body",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = result["proposalId"].as_str().unwrap();
    values::generated_id(id).unwrap();
    assert_eq!(result["placed"], true);
    let view = tmt_colab::discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let proposal = view.conversations.threads[0].proposal.as_ref().unwrap();
    assert_eq!(
        proposal.proposer.machine_id,
        "40000000-0000-4000-8000-000000000001"
    );
    assert_eq!(
        proposal.proposer.agent_id,
        "50000000-0000-1000-8000-000000000001"
    );
    let before = f.read();
    let missing = f
        .command()
        .args([
            "proposal",
            "add",
            PAGE,
            "--title",
            "Unavailable",
            "--body",
            "Body",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let error: Value = serde_json::from_slice(&missing.stdout).unwrap();
    values::generated_id(error["proposalId"].as_str().unwrap()).unwrap();
    assert_eq!(error["placed"], false);
    assert_eq!(f.read().revision, before.revision);
    assert_eq!(
        tmt_colab::discussion::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .conversations
            .threads
            .len(),
        1
    );
}

#[test]
fn proposal_capacity_refuses_a_new_record_before_publication() {
    use tmt_colab::{
        decoder::OwnRecord,
        discussion,
        threads::{Proposal, Proposer},
    };
    let mut f = Fixture::new();
    let mut proposal = Proposal {
        proposal_id: "90000000-0000-4000-8000-000000000001".into(),
        title: "Review".into(),
        body: "Review body".into(),
        proposer: Proposer {
            machine_id: "40000000-0000-4000-8000-000000000001".into(),
            agent_id: "50000000-0000-1000-8000-000000000001".into(),
            label: "Proposer".into(),
        },
    };
    let frozen =
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &proposal, &mut f.decoder(), NOW)
            .unwrap()
            .unwrap();
    f.commit(&frozen, NOW).unwrap();
    let view = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let writer = &view.conversations.threads[0].writer;
    let records:Vec<_>=(2..=200).map(|i| {
        let id=format!("90000000-0000-4000-8000-{i:012}");
        let mut p=proposal.clone();p.proposal_id=id.clone();
        OwnRecord {root:"threads".into(),key:format!("{id}:1"),value:json!({"version":1,"kind":"thread","spaceId":f.key.space_id,"pageId":PAGE,"epoch":view.conversations.epoch,"senderDevice":writer,"revision":"1","deleted":false,"deviceName":"Local CLI","at":NOW.to_string(),"threadId":id,"anchor":null,"resolved":false,"proposal":p})}
    }).collect();
    for batch in records.chunks(tmt_colab::limits::OWN_RECORDS) {
        let frozen =
            page::prepare_own_records(&f.store, &f.key, PAGE, batch, &mut f.decoder(), NOW)
                .unwrap();
        f.commit(&frozen, NOW).unwrap();
    }
    assert_eq!(
        discussion::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .conversations
            .threads
            .len(),
        200
    );
    let before = f.read().revision;
    proposal.proposal_id = "90000000-0000-4000-8000-000000000201".into();
    let fault =
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &proposal, &mut f.decoder(), NOW)
            .err()
            .unwrap();
    assert!(
        matches!(fault.downcast_ref::<tmt_colab::store::owner::OwnerFault>(),Some(tmt_colab::store::owner::OwnerFault::PageCapacity(c)) if c.edit)
    );
    assert!(fault.to_string().contains("maximum 200 proposals"));
    assert_eq!(f.read().revision, before);
    // Raw own publications do not interpret discussion policy; the read fold
    // still rejects authenticated state beyond the same page limit.
    let mut over = records[0].clone();
    over.key = format!("{}:1", proposal.proposal_id);
    over.value["threadId"] = json!(proposal.proposal_id);
    over.value["proposal"] = serde_json::to_value(&proposal).unwrap();
    let frozen =
        page::prepare_own_records(&f.store, &f.key, PAGE, &[over], &mut f.decoder(), NOW).unwrap();
    f.commit(&frozen, NOW).unwrap();
    let output = f
        .command()
        .args(["proposal", "ls", PAGE, "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "COLAB_CAPACITY");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("maximum 200")
    );
}

#[test]
fn owner_chapter_details_are_complete_at_one_fingerprint_and_reject_nonowners() {
    use tmt_colab::registration::{Code, Registration};
    let mut f = Fixture::with_author(Some("Agent 1"));
    let source = "<h1>Page</h1><tmt-read data-id=\"20000000-0000-4000-8000-000000000001\" data-agent=\"Agent 1\" data-why=\"Before deciding\"><h2 data-title>First passage</h2><p>Text.</p></tmt-read><tmt-read data-id=\"20000000-0000-4000-8000-000000000002\" data-agent=\"Agent 2\" data-why=\"For context\"><h2 data-title>Second passage</h2><p>More.</p></tmt-read>";
    f.write(source);
    let context = json!({"deviceId":DEVICE,"kind":"browser","origin":"http://127.0.0.1:1","name":"Laptop",
        "publicKey":values::encode_binary(SigningKey::from_bytes(&[9;32]).verifying_key().as_bytes()),
        "owner":true,"grantRevision":1}).to_string();
    let root = f.root.clone();
    let registration = Registration::with_decoder_config(
        Store::open(&f.layout).unwrap(),
        Keyring::read(&f.layout).unwrap(),
        support::decoder_config(BINARY.into()),
    )
    .unwrap()
    .with_summary_source(std::sync::Arc::new(move || {
        tmt_colab::page::save::open_source(&root, support::decoder_config(BINARY.into()))
    }));
    let read = || -> Value {
        serde_json::from_slice(
            &registration
                .page_summary_details(Some(&context), PAGE)
                .unwrap(),
        )
        .unwrap()
    };
    let settled = || -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let value = read();
            if value["state"] == "ready" {
                return value;
            }
            assert_eq!(
                value["state"], "pending",
                "a failed projection is not readiness: {value}"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "details not ready: {value}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    assert_eq!(read(), json!({"pageId":PAGE,"state":"pending"}));
    let first = settled();
    assert_eq!(first["summary"]["needsYou"], 2);
    assert_eq!(
        first["needs"],
        json!([
            {"kind":"reading","id":"20000000-0000-4000-8000-000000000001","title":"First passage","agent":"Agent 1","why":"Before deciding"},
            {"kind":"reading","id":"20000000-0000-4000-8000-000000000002","title":"Second passage","agent":"Agent 2","why":"For context"}
        ]),
        "the pending passages, in page order, from the same fold as the count"
    );
    assert!(first.get("latest").is_none(), "no comment, no excerpt");
    assert_eq!(
        first["readings"],
        json!([
            {"id":"20000000-0000-4000-8000-000000000001","title":"First passage","agent":"Agent 1","why":"Before deciding","state":"unread"},
            {"id":"20000000-0000-4000-8000-000000000002","title":"Second passage","agent":"Agent 2","why":"For context","state":"unread"}
        ])
    );
    assert!(
        first["fingerprint"]
            .as_str()
            .unwrap()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    assert_eq!(first["fingerprint"].as_str().unwrap().len(), 64);
    let mut denied: Value = serde_json::from_str(&context).unwrap();
    denied["owner"] = json!(false);
    assert_eq!(
        registration
            .page_summary_details(Some(&denied.to_string()), PAGE)
            .unwrap_err(),
        Code::Denied
    );
    assert_eq!(
        registration.page_summary_details(None, PAGE).unwrap_err(),
        Code::Denied
    );
    assert_eq!(
        registration
            .page_summary_details(Some(&context), "not-a-page")
            .unwrap_err(),
        Code::Invalid
    );
    assert_eq!(
        registration
            .page_summary_details(Some(&context), "20000000-0000-4000-8000-000000000099")
            .unwrap_err(),
        Code::Denied
    );
    let one = source
        .split("<tmt-read data-id=\"20000000-0000-4000-8000-000000000002\"")
        .next()
        .unwrap();
    f.write(one);
    assert_eq!(
        read(),
        json!({"pageId":PAGE,"state":"pending"}),
        "old descriptors never survive the new fingerprint"
    );
    let second = settled();
    assert_ne!(first["fingerprint"], second["fingerprint"]);
    assert_eq!(second["summary"]["needsYou"], 1);
    assert_eq!(second["readings"].as_array().unwrap().len(), 1);
    assert_eq!(
        second["needs"].as_array().unwrap().len(),
        1,
        "the passages follow the new fingerprint with the count"
    );
    let catalog: Value =
        serde_json::from_slice(&registration.pages(Some(&context)).unwrap()).unwrap();
    assert_eq!(catalog["pages"][0]["summary"], second["summary"]);
    let many = (0..50).map(|index| format!(
        "<tmt-read data-id=\"20000000-0000-4000-8000-{index:012}\" data-agent=\"{}\" data-why=\"{}\"><h2 data-title>{index:03}{}</h2><p>Passage.</p></tmt-read>",
        "🦉".repeat(64), "🌍".repeat(200), "🐈".repeat(197),
    )).collect::<String>();
    f.write(&many);
    assert_eq!(read()["state"], "pending");
    let largest = settled();
    assert_eq!(largest["summary"]["needsYou"], 50);
    assert_eq!(largest["readings"].as_array().unwrap().len(), 50);
    assert_eq!(
        largest["needs"].as_array().unwrap().len(),
        tmt_colab::limits::NEEDS_PASSAGES,
        "the first twenty by rank; the count stays whole"
    );
    assert!(
        serde_json::to_vec(&largest).unwrap().len() <= tmt_colab::limits::SUMMARY_DETAILS_BYTES
    );
    registration.close().unwrap();
}

#[test]
fn a_summary_refresh_never_labels_a_new_fold_with_a_stale_scheduled_fingerprint() {
    use tmt_colab::summary::{CachedDetails, SummaryCache};
    let mut f = Fixture::new();
    f.write("<h1>A changed page</h1>");
    let root = f.root.clone();
    let open: tmt_colab::page::save::SourceOpener = std::sync::Arc::new(move || {
        tmt_colab::page::save::open_source(&root, support::decoder_config(BINARY.into()))
    });
    let cache = std::sync::Arc::new(SummaryCache::default());
    let stale = "a fingerprint scheduled before the page changed";
    tmt_colab::summary::refresh(&cache, &open, vec![(PAGE.to_owned(), stale.to_owned())]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match cache.details(PAGE, stale) {
            CachedDetails::Unavailable => break,
            CachedDetails::Ready(_) => {
                panic!("a fold from a different fingerprint was labelled complete")
            }
            CachedDetails::Pending => {}
        }
        assert!(
            std::time::Instant::now() < deadline,
            "stale refresh did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn edit_locks_publish_in_own_stream_replace_and_release_without_changing_content() {
    use tmt_colab::{
        edit_lock::{Holder, Target, Whole},
        edit_locks::{self, Change},
    };
    let mut f = Fixture::new();
    let holder = Holder {
        id: "50000000-0000-7000-8000-000000000001".into(),
        name: "Lock holder".into(),
    };
    let source = f.read().source;
    let initial = edit_locks::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let acquire = [Change::Acquire {
        holder: holder.clone(),
        target: Target::Whole(Whole { whole: true }),
    }];
    let (first, actions) = edit_locks::prepare(
        &f.store,
        &f.key,
        PAGE,
        &initial.revision,
        &acquire,
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    assert!(
        edit_locks::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .locks
            .is_empty()
    );
    f.commit(&first, NOW).unwrap();
    assert_eq!(f.read().source, source);
    assert_eq!(f.read().edit_locks[0].lock_id, actions[0].lock_id);
    let before = edit_locks::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert!(
        edit_locks::prepare(
            &f.store,
            &f.key,
            PAGE,
            &initial.revision,
            &acquire,
            &mut f.decoder(),
            NOW
        )
        .is_err()
    );
    let (replace, new) = edit_locks::prepare(
        &f.store,
        &f.key,
        PAGE,
        &before.revision,
        &acquire,
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    f.commit(&replace, NOW).unwrap();
    let current = edit_locks::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert_eq!(current.locks.len(), 1);
    assert_eq!(current.locks[0].revision, "2");
    assert_ne!(actions[0].lock_id, new[0].lock_id);
    let release = [Change::Release {
        holder_id: holder.id,
        lock_id: new[0].lock_id.clone(),
    }];
    let (release, _) = edit_locks::prepare(
        &f.store,
        &f.key,
        PAGE,
        &current.revision,
        &release,
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    f.commit(&release, NOW).unwrap();
    let ended = edit_locks::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert!(ended.locks[0].released);
    assert!(f.read().edit_locks.is_empty());
    assert_eq!(f.read().source, source);
}

#[test]
fn pre_lock_history_retains_threads_statuses_proposals_and_questions_across_own_compaction() {
    use tmt_colab::{
        discussion,
        threads::{Proposal, Proposer},
    };
    fn folded_bytes(value: &impl serde::Serialize) -> Vec<u8> {
        // Y.Map iteration order is not a wire invariant; compare canonical JSON bytes.
        let mut value = serde_json::to_value(value).unwrap();
        value.sort_all_objects();
        serde_json::to_vec(&value).unwrap()
    }
    let mut f = Fixture::new();
    // This signed member history uses the pre-lock literal thread/comment format.
    seed_member_thread_with(
        &mut f,
        &[(
            "60000000-0000-4000-8000-000000000001",
            "1",
            false,
            "Historical comment",
        )],
    );
    let question_id = "00000000-0000-0000-0000-000000000001";
    let revision = f.read().revision;
    let body = json!({"id":question_id,"title":"Historical question"});
    let page::PublicationPreparation::Write(question) = page::prepare_structured_publication(
        &f.store,
        &f.key,
        PAGE,
        page::StructuredAction {
            kind: tmt_colab::decoder::StructuredKind::Questions,
            action: "question.add",
            body: &body,
            expected_revision: &revision,
            display_agent: None,
            operation_id: None,
        },
        &mut f.decoder(),
        &|| Ok(NOW),
    )
    .unwrap() else {
        panic!("question metadata must change");
    };
    f.commit(&question, NOW).unwrap();
    let proposal = Proposal {
        proposal_id: "90000000-0000-4000-8000-000000000001".into(),
        title: "Historical proposal".into(),
        body: "Keep this proposal".into(),
        proposer: Proposer {
            machine_id: "40000000-0000-4000-8000-000000000001".into(),
            agent_id: "50000000-0000-1000-8000-000000000001".into(),
            label: "Historical author".into(),
        },
    };
    let frozen =
        discussion::prepare_proposal(&f.store, &f.key, PAGE, &proposal, &mut f.decoder(), NOW)
            .unwrap()
            .unwrap();
    f.commit(&frozen, NOW).unwrap();
    let (status, _) = discussion::prepare_status(
        &f.store,
        &f.key,
        PAGE,
        discussion::StatusEdit {
            thread: THREAD,
            resolved: true,
            agent_name: Some("Review agent"),
        },
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    f.commit(&status, NOW).unwrap();
    let local = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let writer = &local
        .conversations
        .threads
        .iter()
        .find(|thread| thread.proposal.is_some())
        .unwrap()
        .writer;
    let comment_id = "60000000-0000-4000-8000-000000000002";
    let comment = tmt_colab::decoder::OwnRecord {
        root: "messages".into(),
        key: format!("{comment_id}:1"),
        value: json!({"version":1,"kind":"comment","spaceId":f.key.space_id,"pageId":PAGE,"epoch":"1","senderDevice":writer,"revision":"1","deleted":false,"deviceName":"Historical agent","at":NOW.to_string(),"messageId":comment_id,"thread":{"writer":DEVICE,"id":THREAD},"body":"Historical question answer","questionId":question_id}),
    };
    let comment =
        page::prepare_own_records(&f.store, &f.key, PAGE, &[comment], &mut f.decoder(), NOW)
            .unwrap();
    f.commit(&comment, NOW).unwrap();
    let before = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let before_threads = folded_bytes(&before.conversations.threads);
    let before_page = f.read();
    assert_eq!(before.conversations.threads.len(), 2);
    assert!(
        before
            .conversations
            .threads
            .iter()
            .any(|thread| thread.proposal.is_some())
    );
    assert!(
        before
            .conversations
            .threads
            .iter()
            .any(|thread| thread.status.is_some())
    );
    assert!(
        !before_page
            .questions
            .as_ref()
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(before_page.edit_locks.is_empty());
    let writer = before
        .conversations
        .threads
        .iter()
        .find(|thread| thread.proposal.is_some())
        .unwrap()
        .writer
        .clone();
    let compacted = compact_now(&mut f, 1).unwrap();
    assert_eq!(compacted.namespaces, 2);
    let after = discussion::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert_eq!(folded_bytes(&after.conversations.threads), before_threads);
    let after_page = f.read();
    assert_eq!(
        folded_bytes(&after_page.questions),
        folded_bytes(&before_page.questions)
    );
    assert_eq!(
        folded_bytes(&after_page.question_states),
        folded_bytes(&before_page.question_states)
    );
    assert_eq!(after_page.source, before_page.source);
    // Both generated checkpoint envelopes still authenticate under the pre-existing writer chain.
    let checkpoints = held_checkpoints(&f, &writer, compacted.through);
    assert_eq!(checkpoints.len(), 2);
    let bytes: Vec<u8> = f
        .sql()
        .query_row("SELECT record FROM devices WHERE id=?", [&writer], |row| {
            row.get(0)
        })
        .unwrap();
    let device: Device = serde_json::from_slice(&bytes).unwrap();
    let chain = certificate::Chain::from_json(&device.chain).unwrap();
    let signing_key = chain.certificate().unwrap().signing_key;
    for held in checkpoints {
        let envelope = object::Envelope::from_json(&held.bytes).unwrap();
        let header = object::Header::decode(envelope.header()).unwrap();
        assert!(object::open(&envelope, &header.context, &[11; 32], signing_key).is_ok());
        assert!(object::open(&envelope, &header.context, &[11; 32], &[0; 32]).is_err());
    }
}

#[test]
fn a_writer_refuses_a_ninth_live_lock_atomically_and_release_makes_room() {
    use tmt_colab::{
        edit_lock::{Holder, Target, Whole},
        edit_locks::{self, Change},
    };
    let mut f = Fixture::new();
    let changes = (1..=9)
        .map(|n| Change::Acquire {
            holder: Holder {
                id: format!("50000000-0000-7000-8000-{n:012x}"),
                name: format!("Holder {n}"),
            },
            target: Target::Whole(Whole { whole: true }),
        })
        .collect::<Vec<_>>();
    let initial = edit_locks::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert!(
        edit_locks::prepare(
            &f.store,
            &f.key,
            PAGE,
            &initial.revision,
            &changes,
            &mut f.decoder(),
            NOW
        )
        .is_err()
    );
    assert!(
        edit_locks::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .locks
            .is_empty()
    );
    let (batch, _) = edit_locks::prepare(
        &f.store,
        &f.key,
        PAGE,
        &initial.revision,
        &changes[..8],
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    f.commit(&batch, NOW).unwrap();
    let full = edit_locks::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert_eq!(full.locks.len(), 8);
    assert!(
        edit_locks::prepare(
            &f.store,
            &f.key,
            PAGE,
            &full.revision,
            &changes[8..],
            &mut f.decoder(),
            NOW
        )
        .is_err()
    );
    let mut replacement = vec![Change::Release {
        holder_id: full.locks[0].holder.id.clone(),
        lock_id: full.locks[0].lock_id.clone(),
    }];
    replacement.push(changes[8].clone());
    let (batch, _) = edit_locks::prepare(
        &f.store,
        &f.key,
        PAGE,
        &full.revision,
        &replacement,
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    .unwrap();
    f.commit(&batch, NOW).unwrap();
    let final_view = edit_locks::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert_eq!(final_view.locks.len(), 9);
    assert_eq!(
        final_view
            .locks
            .iter()
            .filter(|lock| !lock.released)
            .count(),
        8
    );
}

#[test]
fn manual_page_state_reaches_a_separate_owner_replica_through_an_encrypted_update() {
    use tmt_colab::decoder::StructuredKind;
    let mut owner = Fixture::new();
    owner.write("<p>replica</p>");
    let mut replica = owner.copy();
    assert_eq!(replica.read().state.as_deref(), Some("todo"));
    let current = owner.read();
    let prepared = page::prepare_structured_publication(
        &owner.store,
        &owner.key,
        PAGE,
        page::StructuredAction {
            kind: StructuredKind::PageState,
            action: "page.state",
            body: &json!({"state":"done"}),
            expected_revision: &current.revision,
            display_agent: None,
            operation_id: None,
        },
        &mut owner.decoder(),
        &|| Ok(NOW),
    )
    .unwrap();
    let page::PublicationPreparation::Write(frozen) = prepared else {
        panic!("expected state publication")
    };
    let stream = frozen.job().manifest.stream_id.clone();
    let sequences: Vec<u64> = frozen
        .job()
        .manifest
        .entries
        .iter()
        .map(|entry| entry.seq.parse().unwrap())
        .collect();
    owner.commit(&frozen, NOW).unwrap();
    assert_eq!(owner.read().state.as_deref(), Some("done"));
    assert_eq!(replica.read().state.as_deref(), Some("todo"));
    for seq in sequences {
        deliver(&owner, &mut replica, &stream, seq);
    }
    assert_eq!(replica.read().state.as_deref(), Some("done"));
    assert_eq!(replica.read().source, current.source);
    owner.write("<p>After state change</p>");
    assert_eq!(owner.read().state.as_deref(), Some("done"));
    Engine::with_decoder_config(support::decoder_config(BINARY.into()))
        .unwrap()
        .advance_epoch(
            &mut owner.store,
            &owner.key,
            EpochAdvance {
                page: PAGE,
                operation_id: "40000000-0000-4000-8000-000000000002",
                expected_revision: 2,
            },
            NOW,
        )
        .unwrap();
    let rotated = owner.read();
    assert_eq!(rotated.epoch, "2");
    assert_eq!(rotated.state.as_deref(), Some("done"));
    assert_eq!(rotated.source, "<p>After state change</p>");
}
