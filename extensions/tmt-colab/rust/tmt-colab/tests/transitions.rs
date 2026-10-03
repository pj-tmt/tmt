mod support;
use ed25519_dalek::{Signer, SigningKey};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    keyring::{Keyring, Layout},
    store::{
        Envelope, Namespace, Store, StreamScope,
        owner::{Device, Mutation, Recipient},
    },
    transitions::{Code, Engine, EpochAdvance},
};
use tmt_colab_model::{certificate, crypto, object, statement, values, wrap};
use yrs::{
    Doc, GetString, Map, ReadTxn, StateVector, Text, Transact, Update, updates::decoder::Decode,
};
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const MEMBER: &str = "20000000-0000-4000-8000-000000000001";
const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
const OP: &str = "40000000-0000-4000-8000-000000000002";
struct Fixture {
    root: PathBuf,
    layout: Layout,
    key: Keyring,
    store: Store,
    engine: Engine,
}
impl Fixture {
    fn new() -> Self {
        Self::with_role("editor")
    }
    fn with_role(role: &str) -> Self {
        Self::with_devices(role, false)
    }
    fn with_devices(role: &str, second: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-1157-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let mut store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        store.owner_transaction(&key.space_id,&key.owner_public(),Mutation {operation_id:"40000000-0000-4000-8000-000000000001",digest:[7;32],expected_revision:0},|tx| {
            let owner=key.management_member()?;
            let r=Recipient {kind:"member".into(),id:owner.id,role:Some("editor".into()),signing_key:owner.signing_key,encryption_key:owner.encryption_key,pages:vec![],revoked:false};
            let add=|r:&Recipient| serde_json::to_vec(&json!({"memberId":r.id,"role":r.role,"signKey":values::encode_binary(&r.signing_key),
                "encKey":values::encode_binary(&r.encryption_key),"pages":r.pages}));
            tx.append_statement(&key.sign_statement(tx.head(),"member.add",&add(&r)?)?)?;tx.put_recipient(&r)?;
            let r=Recipient {kind:"member".into(),id:MEMBER.into(),role:Some(role.into()),signing_key:signer(7).verifying_key().to_bytes(),
                encryption_key:wrap::RecipientKey::from_seed(&[8;32])?.public_key(),pages:vec![PAGE.into()],revoked:false};
            let issuer=key.sign_statement(tx.head(),"member.add",&add(&r)?)?;
            tx.append_statement(&issuer)?;tx.put_recipient(&r)?;
            let devsign=signer(9).verifying_key().to_bytes();let devenc=wrap::RecipientKey::from_seed(&[10;32])?.public_key();
            let cert=certificate::input(&certificate::Certificate {space:&key.space_id,issuer_kind:"member",issuer_id:MEMBER,device_id:DEVICE,
                signing_key:&devsign,encryption_key:&devenc,membership_revision:"2",issued_at:1,expires_at:100000})?;
            tx.put_device(&Device {revoked:false,chain:serde_json::to_vec(&json!({"version":1,"issuerStatement":values::encode_binary(&issuer.hash()?),
                "deviceCertificate":values::encode_binary(&cert),"issuerSignature":values::encode_binary(&signer(7).sign(&cert).to_bytes())}))?})?;
            if second {
                let devsign=signer(12).verifying_key().to_bytes();let devenc=wrap::RecipientKey::from_seed(&[13;32])?.public_key();
                let cert=certificate::input(&certificate::Certificate {space:&key.space_id,issuer_kind:"member",issuer_id:MEMBER,device_id:"30000000-0000-4000-8000-000000000002",
                    signing_key:&devsign,encryption_key:&devenc,membership_revision:"2",issued_at:1,expires_at:100000})?;
                tx.put_device(&Device {revoked:false,chain:serde_json::to_vec(&json!({"version":1,"issuerStatement":values::encode_binary(&issuer.hash()?),
                    "deviceCertificate":values::encode_binary(&cert),"issuerSignature":values::encode_binary(&signer(7).sign(&cert).to_bytes())}))?})?;
            }
            tx.put_epoch_secret(PAGE,1,&[11;32])?;Ok(b"genesis".to_vec())
        }).unwrap();
        Self {
            root,
            layout,
            key,
            store,
            engine: Engine::with_decoder_config(support::decoder_config(
                env!("CARGO_BIN_EXE_tmt-colab").into(),
            ))
            .unwrap(),
        }
    }
    fn db(&self) -> Connection {
        Connection::open(self.layout.directory.join("space.db")).unwrap()
    }
    fn advance(
        &mut self,
        id: &str,
        revision: u64,
    ) -> Result<Vec<u8>, tmt_colab::transitions::TransitionError> {
        self.engine.advance_epoch(
            &mut self.store,
            &self.key,
            EpochAdvance {
                operation_id: id,
                expected_revision: revision,
                page: PAGE,
            },
            100,
        )
    }
    fn object(
        &self,
        seq: u64,
        previous: [u8; 32],
        kind: &str,
        namespace: &str,
        update: &[u8],
    ) -> object::Envelope {
        object::seal(
            &object::Context {
                space: self.key.space_id.clone(),
                page: PAGE.into(),
                epoch: "1".into(),
                kind: kind.into(),
                namespace: namespace.into(),
                author_device: DEVICE.into(),
                membership_revision: "2".into(),
                stream_seq: seq.to_string(),
                prev_hash: previous,
            },
            &[11; 32],
            &signer(9),
            update,
        )
        .unwrap()
    }
    fn append(&mut self, object: &object::Envelope) {
        let context = object::Header::decode(object.header()).unwrap().context;
        self.append_for(object, 1, &context.author_device);
    }
    fn append_for(&mut self, object: &object::Envelope, epoch: u64, stream: &str) {
        let h = object::Header::decode(object.header()).unwrap();
        let c = &h.context;
        let bytes = object.to_json().unwrap();
        let envelope = Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch,
                stream,
            },
            namespace: if c.namespace == "content" {
                Namespace::Content
            } else {
                Namespace::Own
            },
            seq: c.stream_seq.parse().unwrap(),
            hash: object.hash().unwrap(),
            previous: c.prev_hash,
            bytes: &bytes,
        };
        if c.kind == "checkpoint" {
            self.store.checkpoint(&envelope).unwrap();
        } else {
            self.store.append(&envelope).unwrap();
        }
    }
    fn counts(&self) -> Vec<i64> {
        let db = self.db();
        [
            "membership_log",
            "epoch_secrets",
            "baselines",
            "wraps",
            "owner_operations",
        ]
        .iter()
        .map(|table| {
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        })
        .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn signer(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}
fn source(text: &str) -> Vec<u8> {
    let doc = Doc::with_client_id(57);
    doc.get_or_insert_text("html")
        .insert(&mut doc.transact_mut(), 0, text);
    doc.get_or_insert_map("meta")
        .insert(&mut doc.transact_mut(), "title", "owner view");
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
}
fn baseline_source(f: &Fixture, epoch: u64) -> String {
    let saved = f.store.baseline(PAGE, epoch).unwrap().unwrap();
    let d: Value = serde_json::from_slice(&saved.descriptor).unwrap();
    let envelope = object::Envelope::from_json(&saved.envelope).unwrap();
    assert_eq!(
        values::binary(d["objectEnvelopeHash"].as_str().unwrap(), 32).unwrap(),
        envelope.hash().unwrap()
    );
    let h = object::Header::decode(envelope.header()).unwrap();
    assert_eq!(h.context.kind, "html");
    assert_eq!(h.context.stream_seq, "0");
    let secret: Vec<u8> = f
        .db()
        .query_row(
            "SELECT secret FROM epoch_secrets WHERE page=? AND epoch=?",
            params![PAGE, format!("{epoch:020}")],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        object::open(
            &envelope,
            &h.context,
            &[11; 32],
            &f.key.management_member().unwrap().signing_key
        )
        .is_err()
    );
    let body: Value = serde_json::from_slice(
        &object::open(
            &envelope,
            &h.context,
            &secret.try_into().unwrap(),
            &f.key.management_member().unwrap().signing_key,
        )
        .unwrap(),
    )
    .unwrap();
    let text = body["source"].as_str().unwrap();
    assert_eq!(
        values::binary(d["sourceDigest"].as_str().unwrap(), 32).unwrap(),
        crypto::digest(text.as_bytes())
    );
    let update = values::binary(body["update"].as_str().unwrap(), 3 * 1024 * 1024).unwrap();
    let doc = Doc::new();
    let html = doc.get_or_insert_text("html");
    doc.transact_mut()
        .apply_update(Update::decode_v1(&update).unwrap())
        .unwrap();
    assert_eq!(html.get_string(&doc.transact()), text);
    text.into()
}
#[test]
fn authenticated_fold_rotates_exact_view_wraps_and_replays_after_reopen() {
    let mut f = Fixture::new();
    let update = f.object(1, [0; 32], "update", "content", &source("exact\r\nsource"));
    f.append(&update);
    let previous = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap();
    let outcome = f.advance(OP, 2).unwrap();
    let wire: Value = serde_json::from_slice(&outcome).unwrap();
    let stmt = statement::Envelope::from_json(&serde_json::to_vec(&wire["statements"][0]).unwrap())
        .unwrap();
    let verified = stmt
        .verify_next(&f.key.space_id, &f.key.owner_public(), Some(&previous))
        .unwrap();
    assert_eq!(verified.head.revision, 3);
    assert_eq!(baseline_source(&f, 2), "exact\r\nsource");
    let wraps: Vec<wrap::Envelope> = serde_json::from_value(wire["wrapLists"][0].clone()).unwrap();
    assert_eq!(wraps.len(), 3);
    let named = wraps
        .iter()
        .find(|w| w.header().unwrap().recipient_id == MEMBER)
        .unwrap();
    let opened = wrap::open(
        named,
        &named.header().unwrap(),
        &wrap::RecipientKey::from_seed(&[8; 32]).unwrap(),
        &f.key.owner_public(),
    )
    .unwrap();
    let stored: Vec<u8> = f
        .db()
        .query_row(
            "SELECT secret FROM epoch_secrets WHERE epoch='00000000000000000002'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(opened.as_slice(), stored);
    let before = f.counts();
    f.store = Store::open(&f.layout).unwrap();
    assert_eq!(f.advance(OP, 2).unwrap(), outcome);
    assert_eq!(f.counts(), before);
    f.advance("40000000-0000-4000-8000-000000000003", 3)
        .unwrap();
    assert_eq!(baseline_source(&f, 3), "exact\r\nsource");
    let bytes = update.to_json().unwrap();
    assert!(matches!(
        f.store.append(&Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: DEVICE
            },
            namespace: Namespace::Content,
            seq: 1,
            hash: update.hash().unwrap(),
            previous: [0; 32],
            bytes: &bytes
        }),
        Err(tmt_colab::store::Fault::StaleEpoch)
    ));
}
#[test]
fn checkpoint_prefix_is_authenticated_and_pinned_for_both_namespaces() {
    let mut f = Fixture::new();
    let first = f.object(1, [0; 32], "update", "content", &source("checkpoint"));
    f.append(&first);
    let own = f.object(2, first.hash().unwrap(), "update", "own", &[0, 0]);
    f.append(&own);
    let cp = f.object(
        2,
        own.hash().unwrap(),
        "checkpoint",
        "content",
        &source("checkpoint"),
    );
    f.append(&cp);
    let own_cp = f.object(2, own.hash().unwrap(), "checkpoint", "own", &[0, 0]);
    f.append(&own_cp);
    let head = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap();
    let result = f.advance(OP, 2).unwrap();
    assert_eq!(baseline_source(&f, 2), "checkpoint");
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM checkpoints WHERE pinned=1", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let wire: Value = serde_json::from_slice(&result).unwrap();
    let statement =
        statement::Envelope::from_json(&serde_json::to_vec(&wire["statements"][0]).unwrap())
            .unwrap();
    let verified = statement
        .verify_next(&f.key.space_id, &f.key.owner_public(), Some(&head))
        .unwrap();
    let tmt_colab_model::payload::Payload::EpochAdvance(p) = verified.payload else {
        panic!()
    };
    assert_eq!(p.cuts.as_slice().len(), 2);
}
#[test]
fn failed_baseline_or_wrap_write_rolls_back_every_authority_effect() {
    for table in ["baselines", "wraps"] {
        let mut f = Fixture::new();
        let first = f.object(1, [0; 32], "update", "content", &source("unchanged"));
        f.append(&first);
        let before = f.counts();
        f.db().execute_batch(&format!("CREATE TRIGGER fail_insert BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'injected'); END;")).unwrap();
        assert_eq!(f.advance(OP, 2).unwrap_err().code, Code::Unavailable);
        assert_eq!(f.counts(), before);
        assert_eq!(
            f.db()
                .query_row("SELECT epoch FROM pages WHERE page=?", [PAGE], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "1"
        );
        f.db().execute_batch("DROP TRIGGER fail_insert").unwrap();
        f.advance(OP, 2).unwrap();
        assert_eq!(baseline_source(&f, 2), "unchanged");
    }
}
#[test]
fn forged_signature_or_device_chain_cannot_be_folded_or_signed() {
    for target in ["object", "chain"] {
        let mut f = Fixture::new();
        let good = f.object(1, [0; 32], "update", "content", &source("verified"));
        if target == "object" {
            let mut wire: Value = serde_json::from_slice(&good.to_json().unwrap()).unwrap();
            wire["signature"] = json!(values::encode_binary(&[0; 64]));
            let forged = object::Envelope::from_json(&serde_json::to_vec(&wire).unwrap()).unwrap();
            f.append(&forged);
        } else {
            f.append(&good);
            let db = f.db();
            let bytes: Vec<u8> = db
                .query_row("SELECT record FROM devices WHERE id=?", [DEVICE], |r| {
                    r.get(0)
                })
                .unwrap();
            let mut device: Device = serde_json::from_slice(&bytes).unwrap();
            let mut chain: Value = serde_json::from_slice(&device.chain).unwrap();
            chain["issuerSignature"] = json!(values::encode_binary(&[0; 64]));
            device.chain = serde_json::to_vec(&chain).unwrap();
            db.execute(
                "UPDATE devices SET record=? WHERE id=?",
                params![serde_json::to_vec(&device).unwrap(), DEVICE],
            )
            .unwrap();
        }
        let before = f.counts();
        assert_eq!(f.advance(OP, 2).unwrap_err().code, Code::Invalid);
        assert_eq!(f.counts(), before);
    }
}
#[test]
fn baseline_tampering_and_stale_heads_apply_nothing() {
    let mut f = Fixture::new();
    let first = f.object(1, [0; 32], "update", "content", &source("committed"));
    f.append(&first);
    assert_eq!(f.advance(OP, 1).unwrap_err().code, Code::StaleHead);
    f.advance(OP, 2).unwrap();
    let before = f.counts();
    let mut saved = f.store.baseline(PAGE, 2).unwrap().unwrap();
    let mut d: Value = serde_json::from_slice(&saved.descriptor).unwrap();
    d["title"] = json!("substituted");
    saved.descriptor = serde_json::to_vec(&d).unwrap();
    f.db()
        .execute(
            "UPDATE baselines SET descriptor=? WHERE page=?",
            params![saved.descriptor, PAGE],
        )
        .unwrap();
    assert_eq!(
        f.advance("40000000-0000-4000-8000-000000000003", 3)
            .unwrap_err()
            .code,
        Code::Invalid
    );
    assert_eq!(f.counts(), before);
    assert_eq!(f.advance(OP, 3).unwrap_err().code, Code::Conflict);
}
#[test]
fn revoked_and_expired_devices_do_not_receive_new_epoch_wraps() {
    for revoked in [true, false] {
        let mut f = Fixture::new();
        let db = f.db();
        let bytes: Vec<u8> = db
            .query_row("SELECT record FROM devices WHERE id=?", [DEVICE], |r| {
                r.get(0)
            })
            .unwrap();
        let mut d: Device = serde_json::from_slice(&bytes).unwrap();
        d.revoked = revoked;
        db.execute(
            "UPDATE devices SET record=? WHERE id=?",
            params![serde_json::to_vec(&d).unwrap(), DEVICE],
        )
        .unwrap();
        let now = if revoked { 100 } else { 100001 };
        let outcome = f
            .engine
            .advance_epoch(
                &mut f.store,
                &f.key,
                EpochAdvance {
                    operation_id: OP,
                    expected_revision: 2,
                    page: PAGE,
                },
                now,
            )
            .unwrap();
        let wire: Value = serde_json::from_slice(&outcome).unwrap();
        let wraps: Vec<wrap::Envelope> =
            serde_json::from_value(wire["wrapLists"][0].clone()).unwrap();
        assert_eq!(wraps.len(), 2);
        assert!(
            wraps
                .iter()
                .all(|w| w.header().unwrap().recipient_id != DEVICE)
        );
    }
}

#[test]
fn concurrent_appends_during_decoder_work_retry_without_holding_writer_lock() {
    use nix::{
        poll::{PollFd, PollFlags, PollTimeout, poll},
        sys::stat::Mode,
        unistd::mkfifo,
    };
    use std::{io::Write, os::fd::AsFd};
    for changes in [1, 3] {
        let mut f = Fixture::new();
        let first = f.object(1, [0; 32], "update", "content", &source("raced view"));
        f.append(&first);
        let signal = f.root.join("signal");
        let release = f.root.join("release");
        for path in [&signal, &release] {
            mkfifo(path, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
        }
        let ready = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&signal)
            .unwrap();
        let mut go = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&release)
            .unwrap();
        let proxy = f.root.join("decoder");
        let script = format!(
            "#!/bin/sh\nif [ \"$2\" = baseline ]; then\n n=0; [ ! -f '{0}/count' ] || n=$(cat '{0}/count')\n if [ \"$n\" -lt {1} ]; then\n  n=$((n+1)); printf '%s' \"$n\" > '{0}/count'\n  printf x > '{0}/signal'; read line < '{0}/release'\n fi\nfi\nexec '{2}' \"$@\"\n",
            f.root.display(),
            changes,
            env!("CARGO_BIN_EXE_tmt-colab")
        );
        let mut writer = std::process::Command::new("/bin/sh")
            .args(["-c", "cat > \"$1\" && chmod 700 \"$1\"", "sh"])
            .arg(&proxy)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        writer
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        assert!(writer.wait().unwrap().success());
        let root = f.root.clone();
        let result = std::thread::scope(|scope| {
            let thread = scope.spawn(move || {
                let layout = Layout::existing(&root).unwrap().unwrap();
                let key = Keyring::read(&layout).unwrap();
                let mut store = Store::open(&layout).unwrap();
                Engine::with_decoder_config(support::decoder_config(proxy))
                    .unwrap()
                    .advance_epoch(
                        &mut store,
                        &key,
                        EpochAdvance {
                            operation_id: OP,
                            expected_revision: 2,
                            page: PAGE,
                        },
                        100,
                    )
            });
            let mut previous = first.hash().unwrap();
            for seq in 2..=changes + 1 {
                let mut events = [PollFd::new(ready.as_fd(), PollFlags::POLLIN)];
                let signaled = poll(&mut events, PollTimeout::try_from(30_000).unwrap()).unwrap();
                if signaled != 1 {
                    go.write_all(b"cancel\n").unwrap();
                    let result = thread.join().unwrap();
                    panic!("decoder signal absent: {result:?}");
                }
                let mut byte = [0];
                std::io::Read::read_exact(&mut &ready, &mut byte).unwrap();
                let next = f.object(seq as u64, previous, "update", "own", &[0, 0]);
                f.append(&next);
                previous = next.hash().unwrap();
                go.write_all(b"continue\n").unwrap();
            }
            thread.join().unwrap()
        });
        if changes == 1 {
            result.unwrap();
            assert_eq!(baseline_source(&f, 2), "raced view");
        } else {
            assert_eq!(result.unwrap_err().code, Code::StaleHead);
            assert_eq!(f.counts(), vec![2, 1, 0, 0, 1]);
        }
    }
}

#[test]
fn valid_signatures_do_not_authorize_wrong_epoch_role_or_namespace_roots() {
    for target in ["epoch", "role", "roots"] {
        let mut f = Fixture::with_role(if target == "role" {
            "commenter"
        } else {
            "editor"
        });
        let mut envelope = f.object(
            1,
            [0; 32],
            "update",
            if target == "roots" { "own" } else { "content" },
            &source("source"),
        );
        if target == "epoch" {
            let mut context = object::Header::decode(envelope.header()).unwrap().context;
            context.epoch = "2".into();
            envelope = object::seal(&context, &[11; 32], &signer(9), &source("source")).unwrap();
        }
        f.append(&envelope);
        let before = f.counts();
        assert_eq!(f.advance(OP, 2).unwrap_err().code, Code::Invalid);
        assert_eq!(f.counts(), before);
    }
}

#[test]
fn retained_baseline_ciphertext_counts_toward_the_page_quota() {
    let mut f = Fixture::new();
    f.advance(OP, 2).unwrap();
    f.db()
        .execute(
            "UPDATE baselines SET envelope=zeroblob(?) WHERE page=?",
            params![tmt_colab::limits::PAGE_BYTES as i64, PAGE],
        )
        .unwrap();
    let before = f.counts();
    let result = f.store.append(&Envelope {
        scope: StreamScope {
            page: PAGE,
            epoch: 2,
            stream: DEVICE,
        },
        namespace: Namespace::Content,
        seq: 1,
        hash: [3; 32],
        previous: [0; 32],
        bytes: b"x",
    });
    assert!(matches!(result, Err(tmt_colab::store::Fault::Capacity)));
    assert_eq!(f.counts(), before);
}

#[test]
fn reduction_cut_rejects_signed_forks_and_uncommitted_checkpoint_replacements() {
    for target in ["committed", "fork", "checkpoint"] {
        let mut f = Fixture::new();
        let first = f.object(1, [0; 32], "update", "content", &source("committed cut"));
        f.append(&first);
        let cuts=[("content","1",first.hash().unwrap()),("own","0",[0;32])].into_iter().map(|(namespace,seq,hash)| {
            json!({"pageId":PAGE,"epoch":"1","namespace":namespace,"cut":values::encode_binary(&tmt_colab_model::stream_cut::input(
                &tmt_colab_model::stream_cut::StreamCut {stream_id:DEVICE,namespace,checkpoint_hash:None,checkpoint_seq:"0",tail_head_seq:seq,tail_head_hash:&hash}).unwrap())})
        }).collect::<Vec<_>>();
        f.store
            .owner_transaction(
                &f.key.space_id,
                &f.key.owner_public(),
                Mutation {
                    operation_id: "40000000-0000-4000-8000-000000000004",
                    digest: [8; 32],
                    expected_revision: 2,
                },
                |tx| {
                    let statement = f.key.sign_statement(
                        tx.head(),
                        "member.role",
                        &serde_json::to_vec(
                            &json!({"memberId":MEMBER,"role":"commenter","cuts":cuts}),
                        )?,
                    )?;
                    tx.append_statement(&statement)?;
                    Ok(statement.to_json()?)
                },
            )
            .unwrap();
        if target == "fork" {
            let alternative = f.object(1, [0; 32], "update", "content", &source("valid fork"));
            let bytes = alternative.to_json().unwrap();
            f.db()
                .execute(
                    "UPDATE receipts SET hash=?,digest=?,payload=? WHERE stream=?",
                    params![
                        alternative.hash().unwrap().as_slice(),
                        crypto::digest(&bytes).as_slice(),
                        bytes,
                        DEVICE
                    ],
                )
                .unwrap();
        }
        if target == "checkpoint" {
            let replacement = f.object(
                1,
                first.hash().unwrap(),
                "checkpoint",
                "content",
                &source("replacement"),
            );
            f.append(&replacement);
        }
        let before = f.counts();
        let result = f.advance(OP, 3);
        if target != "committed" {
            assert_eq!(result.unwrap_err().code, Code::Invalid);
            assert_eq!(f.counts(), before);
        } else {
            result.unwrap();
            assert_eq!(baseline_source(&f, 2), "committed cut");
        }
    }
}

const JOINER: &str = "20000000-0000-4000-8000-000000000002";
fn operation(n: u64) -> String {
    format!("40000000-0000-4000-8000-{n:012}")
}
fn joiner(pages: Vec<String>) -> Recipient {
    Recipient {
        kind: "member".into(),
        id: JOINER.into(),
        role: Some("editor".into()),
        signing_key: signer(21).verifying_key().to_bytes(),
        encryption_key: wrap::RecipientKey::from_seed(&[22; 32])
            .unwrap()
            .public_key(),
        pages,
        revoked: false,
    }
}
fn change(
    f: &mut Fixture,
    n: u64,
    revision: u64,
    action: tmt_colab::transitions::MemberAction,
) -> Result<Vec<u8>, tmt_colab::transitions::TransitionError> {
    f.engine.member(
        &mut f.store,
        &f.key,
        tmt_colab::transitions::MemberRequest {
            operation_id: &operation(n),
            expected_revision: revision,
            action,
        },
        50,
    )
}
fn response_wraps(outcome: &[u8]) -> Vec<wrap::Envelope> {
    let v: Value = serde_json::from_slice(outcome).unwrap();
    v["wrapLists"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|l| l.as_array().unwrap())
        .map(|w| wrap::Envelope::from_json(&serde_json::to_vec(w).unwrap()).unwrap())
        .collect()
}
fn wire_statement(outcome: &[u8], index: usize) -> statement::Envelope {
    let v: Value = serde_json::from_slice(outcome).unwrap();
    statement::Envelope::from_json(&serde_json::to_vec(&v["statements"][index]).unwrap()).unwrap()
}
fn history(f: &mut Fixture, mode: &str) {
    f.store
        .owner_transaction(
            &f.key.space_id,
            &f.key.owner_public(),
            Mutation {
                operation_id: &operation(40),
                digest: [40; 32],
                expected_revision: 2,
            },
            |tx| {
                tx.append_statement(&f.key.sign_statement(
                    tx.head(),
                    "page.history",
                    &serde_json::to_vec(&json!({"pageId":PAGE,"mode":mode}))?,
                )?)?;
                Ok(b"history".to_vec())
            },
        )
        .unwrap();
}
#[test]
fn shared_join_wraps_current_key_and_replays_exactly_after_reopen() {
    use tmt_colab::transitions::MemberAction;
    let mut f = Fixture::new();
    let result = change(&mut f, 50, 2, MemberAction::Add(joiner(vec![PAGE.into()]))).unwrap();
    let wraps = response_wraps(&result);
    assert_eq!(wraps.len(), 1);
    let h = wraps[0].header().unwrap();
    assert_eq!(h.epoch, "1");
    assert_eq!(h.membership_revision, "3");
    assert_eq!(open_join_wrap(&wraps[0], &f.key), [11; 32]);
    let counts = f.counts();
    let reopened = Store::open(&f.layout).unwrap();
    std::mem::replace(&mut f.store, reopened).close().unwrap();
    assert_eq!(
        change(&mut f, 50, 2, MemberAction::Add(joiner(vec![PAGE.into()]))).unwrap(),
        result
    );
    assert_eq!(f.counts(), counts);
    let mut altered = joiner(vec![PAGE.into()]);
    altered.role = Some("viewer".into());
    assert_eq!(
        change(&mut f, 50, 2, MemberAction::Add(altered))
            .unwrap_err()
            .code,
        Code::Conflict
    );
}
#[test]
fn current_join_rotates_with_exact_baseline_and_never_wraps_earlier_keys() {
    use tmt_colab::transitions::MemberAction;
    let mut f = Fixture::new();
    let object = f.object(1, [0; 32], "update", "content", &source("current source"));
    f.append(&object);
    history(&mut f, "current");
    let result = change(&mut f, 51, 3, MemberAction::Add(joiner(vec![PAGE.into()]))).unwrap();
    assert_eq!(baseline_source(&f, 2), "current source");
    let wraps = response_wraps(&result);
    assert!(wraps.iter().all(|w| w.header().unwrap().epoch == "2"));
    let new = wraps
        .iter()
        .find(|w| w.header().unwrap().recipient_id == JOINER)
        .unwrap();
    assert_ne!(open_join_wrap(new, &f.key), [11; 32]);
    let v: Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(v["statements"].as_array().unwrap().len(), 2);
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM wraps WHERE recipient=? AND epoch=?",
                params![JOINER, format!("{:020}", 1)],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn member_removal_rotates_excludes_member_and_devices_and_rolls_back_on_failure() {
    use tmt_colab::transitions::MemberAction;
    for fail in [true, false] {
        let mut f = Fixture::new();
        let update = f.object(1, [0; 32], "update", "content", &source("kept source"));
        f.append(&update);
        let before = f.counts();
        if fail {
            f.db().execute_batch("CREATE TRIGGER deny_wrap BEFORE INSERT ON wraps BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        }
        let result = change(
            &mut f,
            52,
            2,
            MemberAction::Remove {
                member_id: MEMBER.into(),
            },
        );
        if fail {
            assert_eq!(result.unwrap_err().code, Code::Unavailable);
            assert_eq!(f.counts(), before);
            let d: Vec<u8> = f
                .db()
                .query_row("SELECT record FROM devices WHERE id=?", [DEVICE], |r| {
                    r.get(0)
                })
                .unwrap();
            assert!(!serde_json::from_slice::<Device>(&d).unwrap().revoked);
            continue;
        }
        let result = result.unwrap();
        assert_eq!(baseline_source(&f, 2), "kept source");
        let wraps = response_wraps(&result);
        assert!(!wraps.is_empty());
        assert!(wraps.iter().all(|w| {
            let h = w.header().unwrap();
            h.recipient_id != MEMBER && h.recipient_id != DEVICE
        }));
        let first = wire_statement(&result, 0);
        let payload = statement_payload(&first);
        assert_eq!(payload["cuts"].as_array().unwrap().len(), 2);
        let record: Vec<u8> = f
            .db()
            .query_row(
                "SELECT record FROM recipients WHERE kind='member' AND id=?",
                [MEMBER],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            serde_json::from_slice::<Recipient>(&record)
                .unwrap()
                .revoked
        );
    }
}
#[test]
fn role_reduction_commits_both_namespaces_without_rotation_and_promotion_has_no_cuts() {
    use tmt_colab::transitions::MemberAction;
    let mut f = Fixture::new();
    let content = f.object(1, [0; 32], "update", "content", &source("cut source"));
    f.append(&content);
    let own = f.object(2, content.hash().unwrap(), "update", "own", &[0, 0]);
    f.append(&own);
    let cp = f.object(
        2,
        own.hash().unwrap(),
        "checkpoint",
        "content",
        &source("cut source"),
    );
    f.append(&cp);
    let own_cp = f.object(2, own.hash().unwrap(), "checkpoint", "own", &[0, 0]);
    f.append(&own_cp);
    let result = change(
        &mut f,
        53,
        2,
        MemberAction::Role {
            member_id: MEMBER.into(),
            role: "viewer".into(),
        },
    )
    .unwrap();
    let statement = wire_statement(&result, 0);
    let p = statement_payload(&statement);
    assert_eq!(p["cuts"].as_array().unwrap().len(), 2);
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM checkpoints WHERE pinned=1", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        f.db()
            .query_row("SELECT epoch FROM pages WHERE page=?", [PAGE], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "1"
    );
    let promoted = change(
        &mut f,
        54,
        3,
        MemberAction::Role {
            member_id: MEMBER.into(),
            role: "editor".into(),
        },
    )
    .unwrap();
    let s = wire_statement(&promoted, 0);
    let p = statement_payload(&s);
    assert!(p["cuts"].as_array().unwrap().is_empty());
    f.advance(&operation(55), 4).unwrap();
    assert_eq!(baseline_source(&f, 2), "cut source");
}
#[test]
fn pinned_owner_is_denied_for_every_member_role_and_removal() {
    use tmt_colab::transitions::MemberAction;
    let mut f = Fixture::new();
    let owner = f.key.management_member().unwrap().id;
    let before = f.counts();
    for role in ["editor", "commenter", "viewer"] {
        assert_eq!(
            change(
                &mut f,
                56,
                2,
                MemberAction::Role {
                    member_id: owner.clone(),
                    role: role.into()
                }
            )
            .unwrap_err()
            .code,
            Code::Denied
        );
    }
    assert_eq!(
        change(&mut f, 57, 2, MemberAction::Remove { member_id: owner })
            .unwrap_err()
            .code,
        Code::Denied
    );
    assert_eq!(f.counts(), before);
}
#[test]
fn known_device_revoke_is_atomic_and_fresh_operation_replay_never_rotates_twice() {
    use tmt_colab::transitions::DeviceRevoke;
    for fail in [true, false] {
        let mut f = Fixture::new();
        let update = f.object(1, [0; 32], "update", "content", &source("survives revoke"));
        f.append(&update);
        if fail {
            f.db().execute_batch("CREATE TRIGGER deny_tombstone BEFORE INSERT ON device_registrations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        }
        let before = f.counts();
        let result = f.engine.revoke_device(
            &mut f.store,
            &f.key,
            DeviceRevoke {
                operation_id: &operation(58),
                expected_revision: 2,
                device_id: DEVICE,
                grant_revision: 7,
            },
            50,
        );
        if fail {
            assert_eq!(result.unwrap_err().code, Code::Unavailable);
            assert_eq!(f.counts(), before);
            assert_eq!(
                f.db()
                    .query_row("SELECT count(*) FROM device_registrations", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            continue;
        }
        let result = result.unwrap();
        assert_eq!(baseline_source(&f, 2), "survives revoke");
        let wraps = response_wraps(&result);
        assert!(
            wraps
                .iter()
                .any(|w| w.header().unwrap().recipient_id == MEMBER)
        );
        assert!(
            wraps
                .iter()
                .all(|w| w.header().unwrap().recipient_id != DEVICE)
        );
        let p = statement_payload(&wire_statement(&result, 0));
        assert_eq!(p["cuts"].as_array().unwrap().len(), 2);
        let before = f.counts();
        f.db().execute_batch("CREATE TRIGGER deny_replay BEFORE INSERT ON membership_log BEGIN SELECT RAISE(ABORT,'replay signed'); END;").unwrap();
        for (n, grant) in [(59, 7), (60, 6), (61, 8)] {
            let out = f
                .engine
                .revoke_device(
                    &mut f.store,
                    &f.key,
                    DeviceRevoke {
                        operation_id: &operation(n),
                        expected_revision: 2,
                        device_id: DEVICE,
                        grant_revision: grant,
                    },
                    50,
                )
                .unwrap();
            let v: Value = serde_json::from_slice(&out).unwrap();
            assert!(v["statements"].as_array().unwrap().is_empty());
            assert_eq!(f.counts(), before);
        }
    }
}
// Seed real signed/encrypted baseline history without invoking hundreds of child
// processes: each retained epoch has a fresh Y.Doc identity and matching descriptor.
fn retained_history(f: &mut Fixture, pages: &[String], through: u64) {
    use tmt_colab_model::framing;
    for page in pages {
        if page != PAGE {
            f.store.create_page(page).unwrap();
        }
    }
    let mut root: [u8; 32] = fs::read(f.layout.directory.join("owner.key"))
        .unwrap()
        .try_into()
        .unwrap();
    let info = framing::frame(&[
        b"tmt-colab-management-signing-seed-v1",
        f.key.space_id.as_bytes(),
    ])
    .unwrap();
    let mut seed = crypto::derive_key(&root, &[], &info);
    root.fill(0);
    let management = SigningKey::from_bytes(&seed);
    seed.fill(0);
    assert_eq!(
        management.verifying_key().to_bytes(),
        f.key.management_member().unwrap().signing_key
    );
    let mut baselines = Vec::new();
    f.store.owner_transaction(&f.key.space_id,&f.key.owner_public(),Mutation{operation_id:&operation(70),digest:[70;32],expected_revision:2},|tx| {
        for (index,page) in pages.iter().enumerate() {
            if page!=PAGE {tx.put_epoch_secret(page,1,&[1;32])?;}
            for epoch in 2..=through {
                let revision=tx.head().unwrap().revision+1;let secret=[epoch as u8;32];
                let doc=Doc::with_client_id(1000+index as u64*100+epoch);doc.get_or_insert_text("html").insert(&mut doc.transact_mut(),0,"retained source");
                doc.get_or_insert_map("meta").insert(&mut doc.transact_mut(),"title","retained title");
                let update=doc.transact().encode_state_as_update_v1(&StateVector::default());
                let body=serde_json::to_vec(&json!({"source":"retained source","update":values::encode_binary(&update)}))?;
                let object=object::seal(&object::Context{space:f.key.space_id.clone(),page:page.clone(),epoch:epoch.to_string(),kind:"html".into(),namespace:"content".into(),
                    author_device:f.key.management_member()?.id,membership_revision:revision.to_string(),stream_seq:"0".into(),prev_hash:[0;32]},&secret,&management,&body)?;
                let descriptor=json!({"pageId":page,"epoch":epoch.to_string(),"sourceDigest":values::encode_binary(&crypto::digest(b"retained source")),
                    "baselineCommitment":values::encode_binary(&crypto::digest(&framing::frame(&[b"tmt-colab-baseline-v1",b"1",b"retained source",&update])?)),
                    "title":"retained title","objectEnvelopeHash":values::encode_binary(&object.hash()?),"membershipRevision":revision.to_string()});
                tx.append_statement(&f.key.sign_statement(tx.head(),"epoch.advance",&serde_json::to_vec(&json!({"pageId":page,"epoch":epoch.to_string(),"cuts":[],"baseline":descriptor,"wraps":[]}))?)?)?;
                tx.put_epoch_secret(page,epoch,&secret)?;tx.advance_epoch(page,epoch-1)?;
                baselines.push((page.clone(),epoch,serde_json::to_vec(&descriptor)?,object.to_json()?));
            }
        }
        Ok(b"retained history".to_vec())
    }).unwrap();
    let mut db = f.db();
    let tx = db.transaction().unwrap();
    for (page, epoch, descriptor, envelope) in baselines {
        tx.execute(
            "INSERT INTO baselines VALUES (?,?,?,?)",
            params![page, format!("{epoch:020}"), descriptor, envelope],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}
#[test]
fn shared_join_delivers_576_ordered_wraps_at_epoch_cap_or_none() {
    use tmt_colab::transitions::MemberAction;
    let mut f = Fixture::new();
    let pages = (1..=9)
        .map(|n| format!("10000000-0000-4000-8000-{n:012}"))
        .collect::<Vec<_>>();
    retained_history(&mut f, &pages, 65);
    let revision = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap()
        .revision;
    let before = f.counts();
    f.db().execute_batch("CREATE TRIGGER deny_last_wrap BEFORE INSERT ON wraps WHEN NEW.page='10000000-0000-4000-8000-000000000009' BEGIN SELECT RAISE(ABORT,'last page'); END;").unwrap();
    assert_eq!(
        change(
            &mut f,
            71,
            revision,
            MemberAction::Add(joiner(pages.clone()))
        )
        .unwrap_err()
        .code,
        Code::Unavailable
    );
    assert_eq!(f.counts(), before);
    f.db()
        .execute_batch("DROP TRIGGER deny_last_wrap;")
        .unwrap();
    let result = change(
        &mut f,
        71,
        revision,
        MemberAction::Add(joiner(pages.clone())),
    )
    .unwrap();
    let v: Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(v["wrapLists"][0].as_array().unwrap().len(), 512);
    assert_eq!(v["wrapLists"][1].as_array().unwrap().len(), 64);
    let wraps = response_wraps(&result);
    assert_eq!(wraps.len(), 576);
    let mut previous = None;
    for wrapped in wraps {
        let h = wrapped.header().unwrap();
        let epoch = h.epoch.parse::<u64>().unwrap();
        assert!((2..=65).contains(&epoch));
        let order = (
            h.page.clone(),
            epoch,
            h.recipient_kind.clone(),
            h.recipient_id.clone(),
        );
        assert!(previous.as_ref().is_none_or(|p| p < &order));
        previous = Some(order);
        assert_eq!(open_join_wrap(&wrapped, &f.key), [epoch as u8; 32]);
    }
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM wraps WHERE recipient=? AND epoch=?",
                params![JOINER, format!("{:020}", 1)],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

fn statement_payload(s: &statement::Envelope) -> Value {
    let wire: Value = serde_json::from_slice(&s.to_json().unwrap()).unwrap();
    serde_json::from_slice(
        &values::binary(
            wire["payload"].as_str().unwrap(),
            tmt_colab_model::payload::MAX_BYTES,
        )
        .unwrap(),
    )
    .unwrap()
}

fn open_join_wrap(w: &wrap::Envelope, key: &Keyring) -> [u8; 32] {
    wrap::open(
        w,
        &w.header().unwrap(),
        &wrap::RecipientKey::from_seed(&[22; 32]).unwrap(),
        &key.owner_public(),
    )
    .unwrap()
}

const LINK: &str = "50000000-0000-4000-8000-000000000001";
const REPLACEMENT: &str = "50000000-0000-4000-8000-000000000002";
const LINK_DEVICE: &str = "60000000-0000-4000-8000-000000000001";
const LINK_SEED: [u8; 32] = [31; 32];
const REPLACEMENT_SEED: [u8; 32] = [32; 32];
fn link_spec<'a>(
    id: &'a str,
    seed: &'a [u8; 32],
    pages: Vec<String>,
) -> tmt_colab::transitions::LinkSpec<'a> {
    tmt_colab::transitions::LinkSpec {
        id,
        seed,
        role: "editor",
        pages,
    }
}
fn link_change(
    f: &mut Fixture,
    op: u64,
    revision: u64,
    action: tmt_colab::transitions::LinkAction<'_>,
) -> Result<Vec<u8>, tmt_colab::transitions::TransitionError> {
    f.engine.link(
        &mut f.store,
        &f.key,
        tmt_colab::transitions::LinkRequest {
            operation_id: &operation(op),
            expected_revision: revision,
            action,
        },
        50,
    )
}
fn revision(f: &Fixture) -> u64 {
    f.store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap()
        .revision
}
fn link_policy(f: &mut Fixture, pages: &[String], current: bool) {
    let expected = revision(f);
    let epochs = pages
        .iter()
        .map(|page| {
            f.db()
                .query_row("SELECT epoch FROM pages WHERE page=?", [page], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap()
        })
        .collect::<Vec<_>>();
    f.store
        .owner_transaction(
            &f.key.space_id,
            &f.key.owner_public(),
            Mutation {
                operation_id: &operation(80),
                digest: [80; 32],
                expected_revision: expected,
            },
            |tx| {
                for (page, epoch) in pages.iter().zip(&epochs) {
                    tx.append_statement(&f.key.sign_statement(
                        tx.head(),
                        "page.share",
                        &serde_json::to_vec(&json!({"pageId":page,"epoch":epoch,"mode":"link"}))?,
                    )?)?;
                    if current {
                        tx.append_statement(&f.key.sign_statement(
                            tx.head(),
                            "page.history",
                            &serde_json::to_vec(&json!({"pageId":page,"mode":"current"}))?,
                        )?)?;
                    }
                }
                Ok(b"link policy".to_vec())
            },
        )
        .unwrap();
}
fn open_link_wrap(f: &Fixture, wrapped: &wrap::Envelope, id: &str, seed: &[u8; 32]) -> [u8; 32] {
    let keys = tmt_colab_model::link::Keys::derive(seed, &f.key.space_id, id).unwrap();
    wrap::open(
        wrapped,
        &wrapped.header().unwrap(),
        keys.recipient(),
        &f.key.owner_public(),
    )
    .unwrap()
}
fn add_link_device(
    f: &Fixture,
    id: &str,
    issuer: &statement::Envelope,
    seed: &[u8; 32],
    link_id: &str,
) {
    let keys = tmt_colab_model::link::Keys::derive(seed, &f.key.space_id, link_id).unwrap();
    let signing = signer(33).verifying_key().to_bytes();
    let encryption = wrap::RecipientKey::from_seed(&[34; 32])
        .unwrap()
        .public_key();
    let wire: Value = serde_json::from_slice(&issuer.to_json().unwrap()).unwrap();
    let framed = values::binary(wire["statement"].as_str().unwrap(), 1024).unwrap();
    let rev = statement::decode(&framed).unwrap().revision;
    let cert = certificate::Certificate {
        space: &f.key.space_id,
        issuer_kind: "link",
        issuer_id: link_id,
        device_id: id,
        signing_key: &signing,
        encryption_key: &encryption,
        membership_revision: rev,
        issued_at: 1,
        expires_at: 100000,
    };
    let device = Device {
        revoked: false,
        chain: serde_json::to_vec(&json!({"version":1,
        "issuerStatement":values::encode_binary(&issuer.hash().unwrap()),
        "deviceCertificate":values::encode_binary(&certificate::input(&cert).unwrap()),
        "issuerSignature":values::encode_binary(&keys.certify(&cert).unwrap())}))
        .unwrap(),
    };
    // Registration's device-only transaction is crate-private; seed its exact certified projection.
    f.db()
        .execute(
            "INSERT INTO devices VALUES (?,?)",
            params![id, serde_json::to_vec(&device).unwrap()],
        )
        .unwrap();
}
fn link_object(
    f: &Fixture,
    id: &str,
    epoch: u64,
    tail: (u64, [u8; 32]),
    namespace: &str,
    secret: &[u8; 32],
    rev: u64,
) -> object::Envelope {
    let update = if namespace == "content" {
        source("link source")
    } else {
        vec![0, 0]
    };
    object::seal(
        &object::Context {
            space: f.key.space_id.clone(),
            page: PAGE.into(),
            epoch: epoch.to_string(),
            kind: "update".into(),
            namespace: namespace.into(),
            author_device: id.into(),
            membership_revision: rev.to_string(),
            stream_seq: tail.0.to_string(),
            prev_hash: tail.1,
        },
        secret,
        &signer(33),
        &update,
    )
    .unwrap()
}
fn link_fixture(current: bool) -> (Fixture, Vec<u8>, u64) {
    use tmt_colab::transitions::LinkAction;
    let mut f = Fixture::new();
    link_policy(&mut f, &[PAGE.into()], current);
    let rev = revision(&f);
    let added = link_change(
        &mut f,
        81,
        rev,
        LinkAction::Add(link_spec(LINK, &LINK_SEED, vec![PAGE.into()])),
    )
    .unwrap();
    let issuer = wire_statement(&added, 0);
    add_link_device(&f, LINK_DEVICE, &issuer, &LINK_SEED, LINK);
    let rev = revision(&f);
    let content = link_object(&f, LINK_DEVICE, 1, (1, [0; 32]), "content", &[11; 32], rev);
    f.append_for(&content, 1, LINK_DEVICE);
    let own = link_object(
        &f,
        LINK_DEVICE,
        1,
        (2, content.hash().unwrap()),
        "own",
        &[11; 32],
        rev,
    );
    f.append_for(&own, 1, LINK_DEVICE);
    (f, added, rev)
}
fn assert_no_seed(f: &Fixture, returned: &[u8], seed: &[u8; 32]) {
    let needles = [
        seed.to_vec(),
        values::encode_binary(seed).into_bytes(),
        seed.iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            .into_bytes(),
    ];
    let mut rows = vec![returned.to_vec()];
    for sql in [
        "SELECT envelope FROM membership_log",
        "SELECT digest FROM owner_operations",
        "SELECT outcome FROM owner_operations",
        "SELECT record FROM recipients",
        "SELECT envelope FROM wraps",
    ] {
        let db = f.db();
        let mut query = db.prepare(sql).unwrap();
        rows.extend(
            query
                .query_map([], |r| r.get::<_, Vec<u8>>(0))
                .unwrap()
                .map(Result::unwrap),
        );
    }
    // Statements encode payload JSON; inspect decoded payloads as well as exact stored wire bytes.
    let db = f.db();
    let mut query = db.prepare("SELECT envelope FROM membership_log").unwrap();
    for row in query.query_map([], |r| r.get::<_, Vec<u8>>(0)).unwrap() {
        rows.push(
            serde_json::to_vec(&statement_payload(
                &statement::Envelope::from_json(&row.unwrap()).unwrap(),
            ))
            .unwrap(),
        );
    }
    for row in rows {
        for needle in &needles {
            assert!(
                !row.windows(needle.len()).any(|w| w == needle),
                "seed leaked to durable/public bytes"
            );
        }
    }
}
#[test]
fn shared_link_join_wraps_576_keys_in_numeric_order_or_rolls_back() {
    use tmt_colab::transitions::LinkAction;
    let mut f = Fixture::new();
    let pages = (1..=9)
        .map(|n| format!("10000000-0000-4000-8000-{n:012}"))
        .collect::<Vec<_>>();
    retained_history(&mut f, &pages, 65);
    link_policy(&mut f, &pages, false);
    let rev = revision(&f);
    let before = f.counts();
    f.db().execute_batch("CREATE TRIGGER deny_link_wrap BEFORE INSERT ON wraps WHEN NEW.page='10000000-0000-4000-8000-000000000009' BEGIN SELECT RAISE(ABORT,'late wrap'); END;").unwrap();
    assert_eq!(
        link_change(
            &mut f,
            82,
            rev,
            LinkAction::Add(link_spec(LINK, &LINK_SEED, pages.clone()))
        )
        .unwrap_err()
        .code,
        Code::Unavailable
    );
    assert_eq!(f.counts(), before);
    f.db()
        .execute_batch("DROP TRIGGER deny_link_wrap;")
        .unwrap();
    let added = link_change(
        &mut f,
        82,
        rev,
        LinkAction::Add(link_spec(LINK, &LINK_SEED, pages)),
    )
    .unwrap();
    let v: Value = serde_json::from_slice(&added).unwrap();
    assert_eq!(v["wrapLists"][0].as_array().unwrap().len(), 512);
    assert_eq!(v["wrapLists"][1].as_array().unwrap().len(), 64);
    let mut previous = None;
    for w in response_wraps(&added) {
        w.verify_owner(&f.key.owner_public()).unwrap();
        let h = w.header().unwrap();
        let epoch = h.epoch.parse::<u64>().unwrap();
        assert!((2..=65).contains(&epoch));
        let order = (
            h.page.clone(),
            epoch,
            h.recipient_kind.clone(),
            h.recipient_id.clone(),
        );
        assert!(previous.as_ref().is_none_or(|p| p < &order));
        previous = Some(order);
        assert_eq!(open_link_wrap(&f, &w, LINK, &LINK_SEED), [epoch as u8; 32]);
    }
    assert_no_seed(&f, &added, &LINK_SEED);
}
#[test]
fn current_link_join_wraps_only_existing_current_epoch_without_advance() {
    use tmt_colab::transitions::LinkAction;
    let mut f = Fixture::new();
    retained_history(&mut f, &[PAGE.into()], 3);
    link_policy(&mut f, &[PAGE.into()], true);
    let rev = revision(&f);
    let before = f.counts();
    let added = link_change(
        &mut f,
        83,
        rev,
        LinkAction::Add(link_spec(LINK, &LINK_SEED, vec![PAGE.into()])),
    )
    .unwrap();
    let wraps = response_wraps(&added);
    assert_eq!(wraps.len(), 1);
    assert_eq!(wraps[0].header().unwrap().epoch, "3");
    assert_eq!(open_link_wrap(&f, &wraps[0], LINK, &LINK_SEED), [3; 32]);
    let after = f.counts();
    assert_eq!(after[1], before[1]);
    assert_eq!(after[2], before[2]);
    assert_eq!(
        serde_json::from_slice::<Value>(&added).unwrap()["statements"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM wraps WHERE recipient=? AND epoch<?",
                params![LINK, format!("{:020}", 3)],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn reset_link_removes_rotates_then_adds_and_never_persists_replacement_seed() {
    use tmt_colab::transitions::LinkAction;
    for current in [false, true] {
        let (mut f, old, rev) = link_fixture(current);
        let reset = link_change(
            &mut f,
            84,
            rev,
            LinkAction::Reset {
                link_id: LINK,
                replacement: Some(link_spec(REPLACEMENT, &REPLACEMENT_SEED, vec![PAGE.into()])),
            },
        )
        .unwrap();
        assert_eq!(baseline_source(&f, 2), "link source");
        let v: Value = serde_json::from_slice(&reset).unwrap();
        assert_eq!(v["statements"].as_array().unwrap().len(), 3);
        let statements = (0..3)
            .map(|i| wire_statement(&reset, i))
            .collect::<Vec<_>>();
        // Verify the complete retained chain, not a synthetic post-reset head.
        let db = f.db();
        let mut query = db
            .prepare("SELECT envelope FROM membership_log ORDER BY revision")
            .unwrap();
        let mut verified_head = None;
        let mut operations = Vec::new();
        for bytes in query.query_map([], |r| r.get::<_, Vec<u8>>(0)).unwrap() {
            let s = statement::Envelope::from_json(&bytes.unwrap()).unwrap();
            let verified = s
                .verify_next(
                    &f.key.space_id,
                    &f.key.owner_public(),
                    verified_head.as_ref(),
                )
                .unwrap();
            operations.push(verified.header.operation.to_owned());
            verified_head = Some(verified.head);
        }
        assert_eq!(
            &operations[operations.len() - 3..],
            &["link.remove", "epoch.advance", "link.add"]
        );
        let removal = statement_payload(&statements[0]);
        assert_eq!(removal["cuts"].as_array().unwrap().len(), 2);
        for cut in removal["cuts"].as_array().unwrap() {
            let raw = values::binary(cut["cut"].as_str().unwrap(), 2048).unwrap();
            let c = tmt_colab_model::stream_cut::decode(&raw).unwrap();
            assert_eq!(c.stream_id, LINK_DEVICE);
        }
        let wraps = response_wraps(&reset);
        assert!(wraps.iter().all(|w| {
            let h = w.header().unwrap();
            h.recipient_id != LINK && h.recipient_id != LINK_DEVICE
        }));
        assert!(
            wraps
                .iter()
                .any(|w| w.header().unwrap().recipient_id == MEMBER)
        );
        let replacements = wraps
            .iter()
            .filter(|w| w.header().unwrap().recipient_id == REPLACEMENT)
            .collect::<Vec<_>>();
        assert_eq!(replacements.len(), if current { 1 } else { 2 });
        let new = replacements
            .iter()
            .find(|w| w.header().unwrap().epoch == "2")
            .unwrap();
        let secret = open_link_wrap(&f, new, REPLACEMENT, &REPLACEMENT_SEED);
        assert_ne!(secret, [11; 32]);
        let old_keys =
            tmt_colab_model::link::Keys::derive(&LINK_SEED, &f.key.space_id, REPLACEMENT).unwrap();
        assert!(
            wrap::open(
                new,
                &new.header().unwrap(),
                old_keys.recipient(),
                &f.key.owner_public()
            )
            .is_err()
        );
        for w in wraps {
            w.verify_owner(&f.key.owner_public()).unwrap();
        }
        let record: Vec<u8> = f
            .db()
            .query_row(
                "SELECT record FROM devices WHERE id=?",
                [LINK_DEVICE],
                |r| r.get(0),
            )
            .unwrap();
        assert!(serde_json::from_slice::<Device>(&record).unwrap().revoked);
        assert_no_seed(&f, &reset, &REPLACEMENT_SEED);
        let counts = f.counts();
        let reopened = Store::open(&f.layout).unwrap();
        std::mem::replace(&mut f.store, reopened).close().unwrap();
        assert_eq!(
            link_change(
                &mut f,
                84,
                rev,
                LinkAction::Reset {
                    link_id: LINK,
                    replacement: Some(link_spec(REPLACEMENT, &REPLACEMENT_SEED, vec![PAGE.into()]))
                }
            )
            .unwrap(),
            reset
        );
        assert_eq!(f.counts(), counts);
        assert_eq!(
            link_change(
                &mut f,
                84,
                rev,
                LinkAction::Reset {
                    link_id: LINK,
                    replacement: None
                }
            )
            .unwrap_err()
            .code,
            Code::Conflict
        );
        // A retained old seed can certify a new identity, but the revoked issuer cannot regain authority.
        let fresh = "60000000-0000-4000-8000-000000000002";
        add_link_device(&f, fresh, &wire_statement(&old, 0), &LINK_SEED, LINK);
        let object = link_object(&f, fresh, 2, (1, [0; 32]), "content", &secret, revision(&f));
        f.append_for(&object, 2, fresh);
        let expected = revision(&f);
        assert_eq!(
            f.advance(&operation(85), expected).unwrap_err().code,
            Code::Invalid
        );
    }
}
#[test]
fn link_remove_and_reset_without_replacement_rotate_once_and_revoke_every_device() {
    use tmt_colab::transitions::LinkAction;
    for reset in [false, true] {
        let (mut f, old, rev) = link_fixture(false);
        add_link_device(
            &f,
            "60000000-0000-4000-8000-000000000003",
            &wire_statement(&old, 0),
            &LINK_SEED,
            LINK,
        );
        let action = if reset {
            LinkAction::Reset {
                link_id: LINK,
                replacement: None,
            }
        } else {
            LinkAction::Remove { link_id: LINK }
        };
        let removed = link_change(&mut f, 86, rev, action).unwrap();
        assert_eq!(baseline_source(&f, 2), "link source");
        assert_eq!(
            serde_json::from_slice::<Value>(&removed).unwrap()["statements"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let db = f.db();
        let mut query = db
            .prepare("SELECT record FROM devices WHERE id LIKE '60000000-%'")
            .unwrap();
        for row in query.query_map([], |r| r.get::<_, Vec<u8>>(0)).unwrap() {
            assert!(
                serde_json::from_slice::<Device>(&row.unwrap())
                    .unwrap()
                    .revoked
            );
        }
        let counts = f.counts();
        let rev = revision(&f);
        assert_eq!(
            link_change(
                &mut f,
                87,
                rev,
                LinkAction::Add(link_spec(LINK, &REPLACEMENT_SEED, vec![PAGE.into()]))
            )
            .unwrap_err()
            .code,
            Code::Conflict
        );
        assert_eq!(f.counts(), counts);
    }
}
#[test]
fn reset_rejects_seed_equivalence_reused_id_and_rolls_back_late_receipt_failure() {
    use tmt_colab::transitions::LinkAction;
    let (mut f, _, rev) = link_fixture(false);
    let before = f.counts();
    assert_eq!(
        link_change(
            &mut f,
            88,
            rev,
            LinkAction::Reset {
                link_id: LINK,
                replacement: Some(link_spec(REPLACEMENT, &LINK_SEED, vec![PAGE.into()]))
            }
        )
        .unwrap_err()
        .code,
        Code::Denied
    );
    assert_eq!(
        link_change(
            &mut f,
            88,
            rev,
            LinkAction::Reset {
                link_id: LINK,
                replacement: Some(link_spec(LINK, &REPLACEMENT_SEED, vec![PAGE.into()]))
            }
        )
        .unwrap_err()
        .code,
        Code::Conflict
    );
    assert_eq!(f.counts(), before);
    f.db().execute_batch("CREATE TRIGGER deny_reset_receipt BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(ABORT,'late receipt'); END;").unwrap();
    assert_eq!(
        link_change(
            &mut f,
            88,
            rev,
            LinkAction::Reset {
                link_id: LINK,
                replacement: Some(link_spec(REPLACEMENT, &REPLACEMENT_SEED, vec![PAGE.into()]))
            }
        )
        .unwrap_err()
        .code,
        Code::Unavailable
    );
    assert_eq!(f.counts(), before);
    assert!(f.store.baseline(PAGE, 2).unwrap().is_none());
    let record: Vec<u8> = f
        .db()
        .query_row(
            "SELECT record FROM recipients WHERE kind='link' AND id=?",
            [LINK],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !serde_json::from_slice::<Recipient>(&record)
            .unwrap()
            .revoked
    );
    let record: Vec<u8> = f
        .db()
        .query_row(
            "SELECT record FROM devices WHERE id=?",
            [LINK_DEVICE],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!serde_json::from_slice::<Device>(&record).unwrap().revoked);
}
#[test]
fn device_only_revoke_preserves_surviving_link_bearer_capability() {
    let (mut f, old, rev) = link_fixture(false);
    let result = f
        .engine
        .revoke_device(
            &mut f.store,
            &f.key,
            tmt_colab::transitions::DeviceRevoke {
                operation_id: &operation(89),
                expected_revision: rev,
                device_id: LINK_DEVICE,
                grant_revision: 1,
            },
            50,
        )
        .unwrap();
    let wraps = response_wraps(&result);
    let w = wraps
        .iter()
        .find(|w| w.header().unwrap().recipient_id == LINK)
        .unwrap();
    let secret = open_link_wrap(&f, w, LINK, &LINK_SEED);
    assert_ne!(secret, [11; 32]);
    assert!(
        !wraps
            .iter()
            .any(|w| w.header().unwrap().recipient_id == LINK_DEVICE)
    );
    let record: Vec<u8> = f
        .db()
        .query_row(
            "SELECT record FROM recipients WHERE kind='link' AND id=?",
            [LINK],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !serde_json::from_slice::<Recipient>(&record)
            .unwrap()
            .revoked
    );
    let fresh = "60000000-0000-4000-8000-000000000004";
    add_link_device(&f, fresh, &wire_statement(&old, 0), &LINK_SEED, LINK);
    let object = link_object(&f, fresh, 2, (1, [0; 32]), "content", &secret, revision(&f));
    f.append_for(&object, 2, fresh);
    let rev = revision(&f);
    f.advance(&operation(93), rev).unwrap();
    assert_eq!(baseline_source(&f, 3), "link sourcelink source");
}

#[test]
fn reset_rotates_old_scope_and_joins_replacement_scope_without_extra_advances() {
    use tmt_colab::transitions::LinkAction;
    let mut f = Fixture::new();
    let pages = (1..=3)
        .map(|n| format!("10000000-0000-4000-8000-{n:012}"))
        .collect::<Vec<_>>();
    retained_history(&mut f, &pages, 2);
    link_policy(&mut f, &pages, true);
    let rev = revision(&f);
    link_change(
        &mut f,
        90,
        rev,
        LinkAction::Add(link_spec(LINK, &LINK_SEED, pages[..2].to_vec())),
    )
    .unwrap();
    let rev = revision(&f);
    let reset = link_change(
        &mut f,
        91,
        rev,
        LinkAction::Reset {
            link_id: LINK,
            replacement: Some(link_spec(
                REPLACEMENT,
                &REPLACEMENT_SEED,
                pages[1..].to_vec(),
            )),
        },
    )
    .unwrap();
    let v: Value = serde_json::from_slice(&reset).unwrap();
    assert_eq!(v["statements"].as_array().unwrap().len(), 4);
    let first = statement_payload(&wire_statement(&reset, 1));
    let second = statement_payload(&wire_statement(&reset, 2));
    assert_eq!(first["pageId"], pages[0]);
    assert_eq!(second["pageId"], pages[1]);
    assert_eq!(first["epoch"], "3");
    assert_eq!(second["epoch"], "3");
    assert!(f.store.baseline(&pages[2], 3).unwrap().is_none());
    let wraps = response_wraps(&reset);
    let replacement = wraps
        .iter()
        .filter(|w| w.header().unwrap().recipient_id == REPLACEMENT)
        .collect::<Vec<_>>();
    assert_eq!(replacement.len(), 2);
    for w in replacement {
        let h = w.header().unwrap();
        assert_eq!(h.epoch, if h.page == pages[1] { "3" } else { "2" });
        assert_eq!(h.membership_revision, (rev + 4).to_string());
        open_link_wrap(&f, w, REPLACEMENT, &REPLACEMENT_SEED);
    }
}
#[test]
fn link_add_rejects_private_pages_stale_head_and_invalid_role_or_pages_without_writes() {
    use tmt_colab::transitions::LinkAction;
    let mut f = Fixture::new();
    let before = f.counts();
    assert_eq!(
        link_change(
            &mut f,
            92,
            2,
            LinkAction::Add(link_spec(LINK, &LINK_SEED, vec![PAGE.into()]))
        )
        .unwrap_err()
        .code,
        Code::Denied
    );
    assert_eq!(f.counts(), before);
    link_policy(&mut f, &[PAGE.into()], false);
    let rev = revision(&f);
    let before = f.counts();
    assert_eq!(
        link_change(
            &mut f,
            92,
            rev - 1,
            LinkAction::Add(link_spec(LINK, &LINK_SEED, vec![PAGE.into()]))
        )
        .unwrap_err()
        .code,
        Code::StaleHead
    );
    for (role, pages) in [
        ("owner", vec![PAGE.into()]),
        ("editor", vec![PAGE.into(), PAGE.into()]),
    ] {
        let spec = tmt_colab::transitions::LinkSpec {
            id: LINK,
            seed: &LINK_SEED,
            role,
            pages,
        };
        assert_eq!(
            link_change(&mut f, 92, rev, LinkAction::Add(spec))
                .unwrap_err()
                .code,
            Code::Invalid
        );
    }
    assert_eq!(f.counts(), before);
}

#[test]
fn rotated_epoch_bootstrap_delivers_the_real_stored_baseline_inline_and_chunked() {
    use std::{io::ErrorKind, os::unix::net::UnixStream};
    use tmt_colab::sync::{Access, Admission, CatchupContext, Progress, Server, SyncScope};
    use tungstenite::{Message, WebSocket, protocol::Role};
    struct Policy {
        space: String,
        owner: [u8; 32],
    }
    impl Admission for Policy {
        fn authorize(
            &self,
            principal: &str,
            scope: &SyncScope,
            _: Access<'_>,
        ) -> Result<[u8; 32], tmt_colab::sync::Code> {
            if principal != DEVICE
                || scope.space != self.space
                || scope.page != PAGE
                || scope.epoch != "2"
            {
                return Err(tmt_colab::sync::Code::Denied);
            }
            Ok(signer(9).verifying_key().to_bytes())
        }
        fn catchup_context(
            &self,
            _: &str,
            scope: &SyncScope,
            store: &Store,
        ) -> Result<CatchupContext, tmt_colab::sync::Code> {
            Ok(CatchupContext {
                owner_key: self.owner,
                membership_head: store
                    .owner_head(&scope.space, &self.owner)
                    .unwrap()
                    .unwrap(),
                baseline: Some(store.baseline(PAGE, 2).unwrap().unwrap().descriptor),
            })
        }
    }
    for size in [4, 120 * 1024] {
        let mut f = Fixture::new();
        let text = "x".repeat(size);
        let update = f.object(1, [0; 32], "update", "content", &source(&text));
        f.append(&update);
        f.advance(OP, 2).unwrap();
        let saved = f.store.baseline(PAGE, 2).unwrap().unwrap();
        assert_eq!(baseline_source(&f, 2), text);
        let expected = object::Envelope::from_json(&saved.envelope).unwrap();
        crypto::verify_signature(
            &f.key.management_member().unwrap().signing_key,
            &expected.signature_input().unwrap(),
            expected.signature(),
        )
        .unwrap();
        let server = Server::new(
            Store::open(&f.layout).unwrap(),
            Policy {
                space: f.key.space_id.clone(),
                owner: f.key.owner_public(),
            },
        );
        let (client, remote) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        remote.set_nonblocking(true).unwrap();
        let mut client = WebSocket::from_raw_socket(client, Role::Client, None);
        let mut connection = server.connect(remote, DEVICE.into()).unwrap();
        let scope = json!({"version":1,"space":f.key.space_id,"page":PAGE,"epoch":"2"});
        let mut hello = scope.clone();
        hello.as_object_mut().unwrap().extend(
            json!({"type":"hello","device":DEVICE,"membershipRevision":"0","cursors":[]})
                .as_object()
                .unwrap()
                .clone(),
        );
        client
            .send(Message::Text(hello.to_string().into()))
            .unwrap();
        let mut first = true;
        let mut bytes = Vec::new();
        let mut chunks = 0;
        let mut done = false;
        for _ in 0..2048 {
            let frame: Value = match client.read() {
                Ok(message) => serde_json::from_str(message.to_text().unwrap()).unwrap(),
                Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                    assert_ne!(connection.poll(), Progress::Closed);
                    continue;
                }
                Err(e) => panic!("baseline socket: {e}"),
            };
            assert!(frame.to_string().len() <= tmt_colab::limits::WS_FRAME_BYTES);
            let mut ack = scope.clone();
            ack.as_object_mut().unwrap().extend(
                json!({"type":"ack","cursors":[]})
                    .as_object()
                    .unwrap()
                    .clone(),
            );
            client.send(Message::Text(ack.to_string().into())).unwrap();
            if first {
                first = false;
                assert_eq!(frame["type"], "catchup");
                assert_eq!(
                    values::binary(frame["baseline"].as_str().unwrap(), 8192).unwrap(),
                    saved.descriptor
                );
                assert_eq!(
                    frame["baselineObject"]["envelopeHash"],
                    values::encode_binary(&expected.hash().unwrap())
                );
                if size == 4 {
                    bytes = values::binary(
                        frame["baselineObject"]["envelope"].as_str().unwrap(),
                        65536,
                    )
                    .unwrap();
                } else {
                    assert_eq!(
                        frame["baselineObject"]["envelope"]["objectId"],
                        object::Header::decode(expected.header()).unwrap().object_id
                    );
                }
            } else if frame["type"] == "chunk" {
                assert_eq!(frame["index"], chunks);
                assert_eq!(
                    frame["count"],
                    saved
                        .envelope
                        .len()
                        .div_ceil(tmt_colab::limits::CHUNK_BYTES)
                );
                assert_eq!(
                    frame["envelopeHash"],
                    values::encode_binary(&expected.hash().unwrap())
                );
                bytes.extend(
                    values::binary(
                        frame["bytes"].as_str().unwrap(),
                        tmt_colab::limits::CHUNK_BYTES,
                    )
                    .unwrap(),
                );
                chunks += 1;
            } else {
                assert_eq!(bytes, saved.envelope); // No control frame interrupts the transfer.
                assert!(frame.get("baselineObject").is_none());
                assert_eq!(frame["type"], "catchup");
                if frame["more"] == false {
                    done = true;
                    break;
                }
            }
        }
        assert!(done);
        assert_eq!(bytes, saved.envelope);
        assert_eq!(chunks == 0, size == 4);
        if size > 4 {
            assert!(chunks > 8);
        }
    }
}

// Runner admission uses the same real fixtures as membership/links/epochs.
use tmt_colab::transitions::{MemberAction, OwnerAction, OwnerRequest, RequestScope};
fn request_scope(pages: Vec<String>) -> RequestScope {
    RequestScope {
        initiating_page: pages[0].clone(),
        affected_pages: pages,
    }
}
#[test]
fn runner_replays_transport_scope_and_original_head_after_later_writes() {
    let mut f = Fixture::new();
    let update = f.object(
        1,
        [0; 32],
        "update",
        "content",
        &source("frozen runner view"),
    );
    f.append(&update);
    let id = operation(100);
    let request = || OwnerRequest {
        operation_id: &id,
        expected_revision: 2,
        action: OwnerAction::EpochAdvance { page: PAGE },
        transport_digest: Some([100; 32]),
        scope: Some(request_scope(vec![PAGE.into()])),
    };
    let first = f
        .engine
        .apply(&mut f.store, &f.key, request(), 100)
        .unwrap();
    assert!(!first.replayed);
    assert_eq!(first.head.revision, 3);
    assert_eq!(baseline_source(&f, 2), "frozen runner view");
    f.advance(&operation(101), 3).unwrap();
    let reopened = Store::open(&f.layout).unwrap();
    std::mem::replace(&mut f.store, reopened).close().unwrap();
    let before = f.counts();
    let replay = f
        .engine
        .apply(&mut f.store, &f.key, request(), 101)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.head, first.head);
    assert_eq!(replay.outcome, first.outcome);
    assert_eq!(revision(&f), 4);
    assert_eq!(f.counts(), before);
    for changed in 0..4 {
        let mut r = request();
        match changed {
            0 => r.transport_digest = Some([101; 32]),
            1 => r.transport_digest = None,
            2 => {
                r.scope = Some(request_scope(vec![
                    "10000000-0000-4000-8000-000000000002".into(),
                ]))
            }
            _ => {
                r.action = OwnerAction::Member(MemberAction::Remove {
                    member_id: MEMBER.into(),
                })
            }
        }
        assert_eq!(
            f.engine
                .apply(&mut f.store, &f.key, r, 100)
                .err()
                .unwrap()
                .code,
            Code::Conflict
        );
        assert_eq!(f.counts(), before);
    }
}
#[test]
fn scoped_member_and_link_reductions_fence_assignments_and_replay_revoked_targets() {
    use tmt_colab::transitions::LinkAction;
    for action in 0..4 {
        let (mut f, expected) = if action < 2 {
            (Fixture::new(), 2)
        } else {
            let (f, _, rev) = link_fixture(false);
            (f, rev)
        };
        let id = operation(102 + action);
        let selected = || match action {
            0 => OwnerAction::Member(MemberAction::Remove {
                member_id: MEMBER.into(),
            }),
            1 => OwnerAction::Member(MemberAction::Role {
                member_id: MEMBER.into(),
                role: "viewer".into(),
            }),
            2 => OwnerAction::Link(LinkAction::Remove { link_id: LINK }),
            _ => OwnerAction::Link(LinkAction::Reset {
                link_id: LINK,
                replacement: None,
            }),
        };
        let before = f.counts();
        for pages in [
            vec!["10000000-0000-4000-8000-000000000002".into()],
            vec![PAGE.into(), "10000000-0000-4000-8000-000000000002".into()],
        ] {
            let r = OwnerRequest {
                operation_id: &id,
                expected_revision: expected,
                action: selected(),
                transport_digest: Some([102; 32]),
                scope: Some(request_scope(pages)),
            };
            assert_eq!(
                f.engine
                    .apply(&mut f.store, &f.key, r, 50)
                    .err()
                    .unwrap()
                    .code,
                Code::StaleHead
            );
            assert_eq!(f.counts(), before);
        }
        let request = || OwnerRequest {
            operation_id: &id,
            expected_revision: expected,
            action: selected(),
            transport_digest: Some([102; 32]),
            scope: Some(request_scope(vec![PAGE.into()])),
        };
        let first = f.engine.apply(&mut f.store, &f.key, request(), 50).unwrap();
        assert!(!first.replayed);
        let replay = f.engine.apply(&mut f.store, &f.key, request(), 50).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.head, first.head);
        assert_eq!(replay.outcome, first.outcome);
        let mut changed = request();
        changed.scope = None;
        assert_eq!(
            f.engine
                .apply(&mut f.store, &f.key, changed, 50)
                .err()
                .unwrap()
                .code,
            Code::Conflict
        );
    }
}
#[test]
fn unscoped_root_wrappers_preserve_legacy_digests_and_share_receipts_with_apply() {
    use tmt_colab_model::framing;
    let mut f = Fixture::new();
    let id = operation(108);
    let expected = crypto::digest(
        &framing::frame(&[
            b"tmt-colab-local-epoch-request-v1",
            f.key.space_id.as_bytes(),
            id.as_bytes(),
            b"2",
            PAGE.as_bytes(),
        ])
        .unwrap(),
    );
    let first = f.advance(&id, 2).unwrap();
    let saved: Vec<u8> = f
        .db()
        .query_row(
            "SELECT digest FROM owner_operations WHERE id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, expected);
    let replay = f
        .engine
        .apply(
            &mut f.store,
            &f.key,
            OwnerRequest {
                operation_id: &id,
                expected_revision: 2,
                action: OwnerAction::EpochAdvance { page: PAGE },
                transport_digest: None,
                scope: None,
            },
            100,
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, first);
    let mut f = Fixture::new();
    let id = operation(109);
    let recipient = joiner(vec![PAGE.into()]);
    let selected = json!({"memberId":recipient.id,"role":recipient.role,"signKey":values::encode_binary(&recipient.signing_key),"encKey":values::encode_binary(&recipient.encryption_key),"pages":recipient.pages});
    let expected = crypto::digest(
        &framing::frame(&[
            b"tmt-colab-local-transition-v1",
            f.key.space_id.as_bytes(),
            id.as_bytes(),
            b"2",
            b"member.add",
            &serde_json::to_vec(&selected).unwrap(),
        ])
        .unwrap(),
    );
    let first = change(&mut f, 109, 2, MemberAction::Add(recipient)).unwrap();
    let saved: Vec<u8> = f
        .db()
        .query_row(
            "SELECT digest FROM owner_operations WHERE id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, expected);
    let replay = f
        .engine
        .apply(
            &mut f.store,
            &f.key,
            OwnerRequest {
                operation_id: &id,
                expected_revision: 2,
                action: OwnerAction::Member(MemberAction::Add(joiner(vec![PAGE.into()]))),
                transport_digest: None,
                scope: None,
            },
            100,
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, first);
    let before = f.counts();
    let mismatch = OwnerRequest {
        operation_id: &operation(110),
        expected_revision: revision(&f),
        action: OwnerAction::EpochAdvance { page: PAGE },
        transport_digest: None,
        scope: Some(RequestScope {
            initiating_page: "10000000-0000-4000-8000-000000000002".into(),
            affected_pages: vec![PAGE.into()],
        }),
    };
    assert_eq!(
        f.engine
            .apply(&mut f.store, &f.key, mismatch, 100)
            .err()
            .unwrap()
            .code,
        Code::StaleHead
    );
    assert_eq!(f.counts(), before);
}

#[test]
fn scope_is_rechecked_inside_writer_after_baseline_preparation() {
    use nix::{
        poll::{PollFd, PollFlags, PollTimeout, poll},
        sys::stat::Mode,
        unistd::mkfifo,
    };
    use std::{
        io::{Read, Write},
        os::fd::AsFd,
    };
    let f = Fixture::new();
    let signal = f.root.join("signal");
    let release = f.root.join("release");
    for path in [&signal, &release] {
        mkfifo(path, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    }
    let mut ready = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&signal)
        .unwrap();
    let mut go = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&release)
        .unwrap();
    let proxy = f.root.join("decoder");
    let script = format!(
        "#!/bin/sh\nif [ \"$2\" = baseline ] && [ ! -f '{0}/paused' ]; then\n touch '{0}/paused'; printf x > '{0}/signal'; read line < '{0}/release'\nfi\nexec '{1}' \"$@\"\n",
        f.root.display(),
        env!("CARGO_BIN_EXE_tmt-colab")
    );
    tmt_test_support::write_executable(&proxy, script.as_bytes(), 0o700).unwrap();
    let root = f.root.clone();
    std::thread::scope(|scope| {
        let thread = scope.spawn(move || {
            let layout = Layout::existing(&root).unwrap().unwrap();
            let key = Keyring::read(&layout).unwrap();
            let mut store = Store::open(&layout).unwrap();
            Engine::with_decoder_config(support::decoder_config(proxy))
                .unwrap()
                .apply(
                    &mut store,
                    &key,
                    OwnerRequest {
                        operation_id: &operation(111),
                        expected_revision: 2,
                        action: OwnerAction::Member(MemberAction::Remove {
                            member_id: MEMBER.into(),
                        }),
                        transport_digest: Some([111; 32]),
                        scope: Some(request_scope(vec![PAGE.into()])),
                    },
                    100,
                )
        });
        let mut events = [PollFd::new(ready.as_fd(), PollFlags::POLLIN)];
        if poll(&mut events, PollTimeout::try_from(30_000).unwrap()).unwrap() != 1 {
            go.write_all(b"cancel\n").unwrap();
            panic!("baseline barrier absent: {:?}", thread.join().unwrap());
        }
        let mut byte = [0];
        ready.read_exact(&mut byte).unwrap();
        let db = f.db();
        let bytes: Vec<u8> = db
            .query_row(
                "SELECT record FROM recipients WHERE kind='member' AND id=?",
                [MEMBER],
                |r| r.get(0),
            )
            .unwrap();
        let mut recipient: Recipient = serde_json::from_slice(&bytes).unwrap();
        recipient.pages = vec!["10000000-0000-4000-8000-000000000002".into()];
        db.execute(
            "UPDATE recipients SET record=? WHERE kind='member' AND id=?",
            params![serde_json::to_vec(&recipient).unwrap(), MEMBER],
        )
        .unwrap();
        go.write_all(b"continue\n").unwrap();
        assert_eq!(thread.join().unwrap().unwrap_err().code, Code::StaleHead);
        assert_eq!(revision(&f), 2);
        assert_eq!(f.counts(), vec![2, 1, 0, 0, 1]);
        let bytes: Vec<u8> = db
            .query_row(
                "SELECT record FROM recipients WHERE kind='member' AND id=?",
                [MEMBER],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!serde_json::from_slice::<Recipient>(&bytes).unwrap().revoked);
    });
}

#[test]
fn own_thread_limit_is_summed_across_authenticated_writers_before_rotation() {
    let v: Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/own-v1.json")).unwrap();
    let batch = values::binary(v["threadBatch"].as_str().unwrap(), 256 * 1024).unwrap();
    let extra = values::binary(v["extraThread"].as_str().unwrap(), 256 * 1024).unwrap();
    const OTHER: &str = "30000000-0000-4000-8000-000000000002";
    for over in [false, true] {
        let mut f = Fixture::with_devices("editor", true);
        let first = f.object(1, [0; 32], "update", "own", &batch);
        f.append(&first);
        let mut context = object::Header::decode(first.header()).unwrap().context;
        context.author_device = OTHER.into();
        let second = object::seal(&context, &[11; 32], &signer(12), &batch).unwrap();
        f.append(&second);
        if over {
            context.stream_seq = "2".into();
            context.prev_hash = second.hash().unwrap();
            let last = object::seal(&context, &[11; 32], &signer(12), &extra).unwrap();
            f.append(&last);
        }
        let before = f.counts();
        let result = f.advance(OP, 2);
        if over {
            assert_eq!(result.unwrap_err().code, Code::Capacity);
            assert_eq!(f.counts(), before);
            assert!(f.store.baseline(PAGE, 2).unwrap().is_none());
        } else {
            result.unwrap();
            assert_eq!(baseline_source(&f, 2), "");
        }
    }
}

use tmt_colab::transitions::{Applied, HistoryMode, Publication, ShareMode};
fn policy(
    f: &mut Fixture,
    op: u64,
    expected: u64,
    action: OwnerAction<'_>,
) -> Result<Applied, tmt_colab::transitions::TransitionError> {
    let page = match &action {
        OwnerAction::Share { page, .. }
        | OwnerAction::History { page, .. }
        | OwnerAction::Retention { page, .. }
        | OwnerAction::Archive { page }
        | OwnerAction::Delete { page } => *page,
        _ => panic!("page action required"),
    };
    f.engine.apply(
        &mut f.store,
        &f.key,
        OwnerRequest {
            operation_id: &operation(op),
            expected_revision: expected,
            action,
            transport_digest: Some([116; 32]),
            scope: Some(request_scope(vec![page.into()])),
        },
        50,
    )
}
fn share(page: &str, mode: ShareMode) -> OwnerAction<'_> {
    OwnerAction::Share {
        page,
        mode,
        publication: Publication::Loopback,
    }
}
fn wire_operations(outcome: &[u8]) -> Vec<String> {
    let wire: Value = serde_json::from_slice(outcome).unwrap();
    wire["statements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let bytes = values::binary(v["statement"].as_str().unwrap(), 1024).unwrap();
            statement::decode(&bytes).unwrap().operation.to_owned()
        })
        .collect()
}
fn page_epoch(f: &Fixture, page: &str) -> u64 {
    f.db()
        .query_row("SELECT epoch FROM pages WHERE page=?", [page], |r| {
            r.get::<_, String>(0)
        })
        .unwrap()
        .parse()
        .unwrap()
}
fn page_secret(f: &Fixture, page: &str, epoch: u64) -> [u8; 32] {
    f.db()
        .query_row(
            "SELECT secret FROM epoch_secrets WHERE page=? AND epoch=?",
            params![page, format!("{epoch:020}")],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .unwrap()
        .try_into()
        .unwrap()
}
#[test]
fn page_publication_is_loopback_only_rotates_and_tracks_later_rotations() {
    let mut f = Fixture::new();
    let update = f.object(1, [0; 32], "update", "content", &source("public source"));
    f.append(&update);
    let before = f.counts();
    assert_eq!(
        f.engine
            .apply(
                &mut f.store,
                &f.key,
                OwnerRequest {
                    operation_id: &operation(120),
                    expected_revision: 2,
                    action: share(PAGE, ShareMode::Public),
                    transport_digest: Some([116; 32]),
                    scope: Some(request_scope(vec![
                        "10000000-0000-4000-8000-000000000002".into()
                    ])),
                },
                50
            )
            .unwrap_err()
            .code,
        Code::StaleHead
    );
    assert_eq!(f.counts(), before);
    assert_eq!(
        policy(
            &mut f,
            120,
            2,
            OwnerAction::Share {
                page: PAGE,
                mode: ShareMode::Public,
                publication: Publication::Cloud
            }
        )
        .unwrap_err()
        .code,
        Code::Denied
    );
    assert_eq!(f.counts(), before);
    let published = policy(&mut f, 120, 2, share(PAGE, ShareMode::Public)).unwrap();
    assert_eq!(
        wire_operations(&published.outcome),
        ["epoch.advance", "page.share"]
    );
    assert_eq!(page_epoch(&f, PAGE), 2);
    assert_eq!(baseline_source(&f, 2), "public source");
    let payload = statement_payload(&wire_statement(&published.outcome, 1));
    assert_eq!(payload["publishedKeys"].as_array().unwrap().len(), 2);
    let key = values::binary(payload["publishedKeys"][1]["key"].as_str().unwrap(), 32).unwrap();
    assert_eq!(key, page_secret(&f, PAGE, 2));
    let rev = revision(&f);
    let advanced = f.advance(&operation(121), rev).unwrap();
    assert_eq!(wire_operations(&advanced), ["epoch.advance", "page.share"]);
    let current = statement_payload(&wire_statement(&advanced, 1));
    assert_eq!(current["epoch"], "3");
    assert_eq!(
        values::binary(current["publishedKeys"][2]["key"].as_str().unwrap(), 32).unwrap(),
        page_secret(&f, PAGE, 3)
    );
    let counts = f.counts();
    let reopened = Store::open(&f.layout).unwrap();
    std::mem::replace(&mut f.store, reopened).close().unwrap();
    let replay = policy(&mut f, 120, 2, share(PAGE, ShareMode::Public)).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.head, published.head);
    assert_eq!(replay.outcome, published.outcome);
    assert_eq!(f.counts(), counts);
    assert_eq!(
        policy(
            &mut f,
            120,
            2,
            OwnerAction::Share {
                page: PAGE,
                mode: ShareMode::Public,
                publication: Publication::Cloud
            }
        )
        .unwrap_err()
        .code,
        Code::Conflict
    );
    assert_eq!(
        policy(&mut f, 120, 2, share(PAGE, ShareMode::Link))
            .unwrap_err()
            .code,
        Code::Conflict
    );
}
#[test]
fn every_narrowing_pair_revokes_links_devices_and_rotates_without_leaking_new_keys() {
    for (old, new) in [
        (ShareMode::Link, ShareMode::Private),
        (ShareMode::Public, ShareMode::Link),
        (ShareMode::Public, ShareMode::Private),
    ] {
        let (mut f, _, _) = link_fixture(false);
        if old == ShareMode::Public {
            let rev = revision(&f);
            policy(&mut f, 122, rev, share(PAGE, old)).unwrap();
        }
        let epoch = page_epoch(&f, PAGE);
        let rev = revision(&f);
        let narrowed = policy(&mut f, 123, rev, share(PAGE, new)).unwrap();
        assert_eq!(
            wire_operations(&narrowed.outcome),
            ["link.remove", "epoch.advance", "page.share"]
        );
        assert_eq!(page_epoch(&f, PAGE), epoch + 1);
        assert_eq!(baseline_source(&f, epoch + 1), "link source");
        let removal = statement_payload(&wire_statement(&narrowed.outcome, 0));
        assert_eq!(removal["linkId"], LINK);
        if old == ShareMode::Link {
            assert_eq!(removal["cuts"].as_array().unwrap().len(), 2);
        }
        let payload = statement_payload(&wire_statement(&narrowed.outcome, 2));
        assert_eq!(
            payload["mode"],
            match new {
                ShareMode::Private => "private",
                ShareMode::Link => "link",
                ShareMode::Public => "public",
            }
        );
        assert_eq!(payload["publishedKeys"], json!([]));
        let record: Vec<u8> = f
            .db()
            .query_row(
                "SELECT record FROM devices WHERE id=?",
                [LINK_DEVICE],
                |r| r.get(0),
            )
            .unwrap();
        assert!(serde_json::from_slice::<Device>(&record).unwrap().revoked);
        let recipient: Vec<u8> = f
            .db()
            .query_row(
                "SELECT record FROM recipients WHERE kind='link' AND id=?",
                [LINK],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            serde_json::from_slice::<Recipient>(&recipient)
                .unwrap()
                .revoked
        );
        let wraps = response_wraps(&narrowed.outcome);
        assert!(!wraps.is_empty());
        assert!(
            wraps
                .iter()
                .all(|w| ![LINK, LINK_DEVICE].contains(&w.header().unwrap().recipient_id.as_str()))
        );
        let now = revision(&f);
        assert_eq!(
            link_change(
                &mut f,
                124,
                now,
                tmt_colab::transitions::LinkAction::Add(link_spec(
                    LINK,
                    &LINK_SEED,
                    vec![PAGE.into()]
                ))
            )
            .unwrap_err()
            .code,
            Code::Conflict
        );
        let counts = f.counts();
        assert_eq!(
            policy(&mut f, 123, rev, share(PAGE, new)).unwrap().outcome,
            narrowed.outcome
        );
        assert_eq!(f.counts(), counts);
    }
}
#[test]
fn narrowing_revokes_a_multi_page_link_globally_and_republishes_other_public_pages() {
    const OTHER_PAGE: &str = "10000000-0000-4000-8000-000000000002";
    let mut f = Fixture::new();
    f.store.create_page(OTHER_PAGE).unwrap();
    f.store
        .owner_transaction(
            &f.key.space_id,
            &f.key.owner_public(),
            Mutation {
                operation_id: &operation(125),
                digest: [125; 32],
                expected_revision: 2,
            },
            |tx| {
                tx.put_epoch_secret(OTHER_PAGE, 1, &[12; 32])?;
                tx.append_statement(&f.key.sign_statement(
                    tx.head(),
                    "page.history",
                    &serde_json::to_vec(&json!({"pageId":OTHER_PAGE,"mode":"shared"}))?,
                )?)?;
                Ok(vec![])
            },
        )
        .unwrap();
    let rev = revision(&f);
    policy(&mut f, 126, rev, share(PAGE, ShareMode::Link)).unwrap();
    let rev = revision(&f);
    policy(&mut f, 127, rev, share(OTHER_PAGE, ShareMode::Link)).unwrap();
    let rev = revision(&f);
    let added = link_change(
        &mut f,
        128,
        rev,
        tmt_colab::transitions::LinkAction::Add(link_spec(
            LINK,
            &LINK_SEED,
            vec![PAGE.into(), OTHER_PAGE.into()],
        )),
    )
    .unwrap();
    add_link_device(
        &f,
        LINK_DEVICE,
        &wire_statement(&added, 0),
        &LINK_SEED,
        LINK,
    );
    let rev = revision(&f);
    policy(&mut f, 129, rev, share(OTHER_PAGE, ShareMode::Public)).unwrap();
    let rev = revision(&f);
    let narrowed = policy(&mut f, 130, rev, share(PAGE, ShareMode::Private)).unwrap();
    assert_eq!(
        wire_operations(&narrowed.outcome),
        [
            "link.remove",
            "epoch.advance",
            "epoch.advance",
            "page.share",
            "page.share"
        ]
    );
    assert_eq!(page_epoch(&f, PAGE), 2);
    assert_eq!(page_epoch(&f, OTHER_PAGE), 3);
    let other = statement_payload(&wire_statement(&narrowed.outcome, 3));
    assert_eq!(other["pageId"], OTHER_PAGE);
    assert_eq!(other["epoch"], "3");
    assert_eq!(
        values::binary(other["publishedKeys"][2]["key"].as_str().unwrap(), 32).unwrap(),
        page_secret(&f, OTHER_PAGE, 3)
    );
    assert!(
        response_wraps(&narrowed.outcome)
            .iter()
            .all(|w| ![LINK, LINK_DEVICE].contains(&w.header().unwrap().recipient_id.as_str()))
    );
    // Consecutive public rotations must account for each publication in descriptor revisions.
    let rev = revision(&f);
    policy(&mut f, 148, rev, share(PAGE, ShareMode::Public)).unwrap();
    let recipient = joiner(vec![PAGE.into(), OTHER_PAGE.into()]);
    let rev = revision(&f);
    change(&mut f, 149, rev, MemberAction::Add(recipient.clone())).unwrap();
    let rev = revision(&f);
    let removed = change(
        &mut f,
        131,
        rev,
        MemberAction::Remove {
            member_id: recipient.id,
        },
    )
    .unwrap();
    assert_eq!(
        wire_operations(&removed),
        [
            "member.remove",
            "epoch.advance",
            "page.share",
            "epoch.advance",
            "page.share"
        ]
    );
}
#[test]
fn page_history_changes_future_joins_caps_public_history_and_rejects_earlier_current_wraps() {
    for current in [false, true] {
        let mut f = Fixture::new();
        retained_history(&mut f, &[PAGE.into()], 65);
        let rev = revision(&f);
        let changed = policy(
            &mut f,
            132,
            rev,
            OwnerAction::History {
                page: PAGE,
                mode: if current {
                    HistoryMode::Current
                } else {
                    HistoryMode::Shared
                },
            },
        )
        .unwrap();
        assert_eq!(wire_operations(&changed.outcome), ["page.history"]);
        assert_eq!(page_epoch(&f, PAGE), 65);
        let rev = revision(&f);
        let added = change(
            &mut f,
            133,
            rev,
            MemberAction::Add(joiner(vec![PAGE.into()])),
        )
        .unwrap();
        let wraps = response_wraps(&added);
        let joined = wraps
            .iter()
            .filter(|w| w.header().unwrap().recipient_id == joiner(vec![]).id)
            .collect::<Vec<_>>();
        assert_eq!(joined.len(), if current { 1 } else { 64 });
        let epoch = page_epoch(&f, PAGE);
        let join_revision = revision(&f);
        if current {
            assert_eq!(epoch, 66);
            let forbidden = f
                .key
                .seal_wrap(
                    &wrap::Header {
                        space: f.key.space_id.clone(),
                        page: PAGE.into(),
                        epoch: "65".into(),
                        recipient_kind: "member".into(),
                        recipient_id: joiner(vec![]).id,
                        recipient_key: joiner(vec![]).encryption_key,
                        signer_key: f.key.owner_public(),
                        membership_revision: join_revision.to_string(),
                    },
                    &[65; 32],
                )
                .unwrap();
            let before = f.counts();
            let result = f.store.owner_transaction(
                &f.key.space_id,
                &f.key.owner_public(),
                Mutation {
                    operation_id: &operation(134),
                    digest: [134; 32],
                    expected_revision: join_revision,
                },
                |tx| {
                    tx.append_statement(&f.key.sign_statement(
                        tx.head(),
                        "retention.set",
                        &serde_json::to_vec(&json!({"pageId":PAGE,"days":30}))?,
                    )?)?;
                    tx.put_wrap(&forbidden)?;
                    Ok(vec![])
                },
            );
            assert!(result.is_err());
            assert_eq!(f.counts(), before);
        } else {
            assert_eq!(epoch, 65);
            assert_eq!(joined[0].header().unwrap().epoch, "2");
        }
        let rev = revision(&f);
        let public = policy(&mut f, 135, rev, share(PAGE, ShareMode::Public)).unwrap();
        let last = wire_operations(&public.outcome).len() - 1;
        let payload = statement_payload(&wire_statement(&public.outcome, last));
        let keys = payload["publishedKeys"].as_array().unwrap();
        assert_eq!(keys.len(), if current { 1 } else { 64 });
        assert_eq!(
            keys.last().unwrap()["epoch"],
            page_epoch(&f, PAGE).to_string()
        );
        if !current {
            assert_eq!(keys[0]["epoch"], "3");
        }
    }
}
#[test]
fn sharing_and_deletion_failures_roll_back_every_effect_and_allow_exact_retry() {
    for table in ["baselines", "wraps", "owner_operations"] {
        let (mut f, _, rev) = link_fixture(false);
        let before = f.counts();
        f.db().execute_batch(&format!("CREATE TRIGGER policy_failure BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'injected'); END;")).unwrap();
        assert_eq!(
            policy(&mut f, 136, rev, share(PAGE, ShareMode::Private))
                .unwrap_err()
                .code,
            Code::Unavailable
        );
        assert_eq!(f.counts(), before);
        assert_eq!(page_epoch(&f, PAGE), 1);
        let record: Vec<u8> = f
            .db()
            .query_row(
                "SELECT record FROM devices WHERE id=?",
                [LINK_DEVICE],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!serde_json::from_slice::<Device>(&record).unwrap().revoked);
        f.db().execute_batch("DROP TRIGGER policy_failure").unwrap();
        policy(&mut f, 136, rev, share(PAGE, ShareMode::Private)).unwrap();
    }
    let mut f = Fixture::new();
    let update = f.object(
        1,
        [0; 32],
        "update",
        "content",
        &source("retained until delete"),
    );
    f.append(&update);
    let rev = revision(&f);
    let advanced = f.advance(&operation(137), rev).unwrap();
    assert!(!response_wraps(&advanced).is_empty());
    let rev = revision(&f);
    let before = f.counts();
    f.db().execute_batch("CREATE TRIGGER delete_failure BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert_eq!(
        policy(&mut f, 138, rev, OwnerAction::Delete { page: PAGE })
            .unwrap_err()
            .code,
        Code::Unavailable
    );
    assert_eq!(f.counts(), before);
    let retained: Vec<u8> = f
        .db()
        .query_row(
            "SELECT payload FROM receipts WHERE page=? AND stream=?",
            params![PAGE, DEVICE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, update.to_json().unwrap());
    f.db().execute_batch("DROP TRIGGER delete_failure").unwrap();
    let deleted = policy(&mut f, 138, rev, OwnerAction::Delete { page: PAGE }).unwrap();
    for table in [
        "receipts",
        "checkpoints",
        "streams",
        "baselines",
        "wraps",
        "epoch_secrets",
    ] {
        let rows: i64 = f
            .db()
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE page=?"),
                [PAGE],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
    assert!(f.store.create_page(PAGE).is_err());
    let counts = f.counts();
    let reopened = Store::open(&f.layout).unwrap();
    std::mem::replace(&mut f.store, reopened).close().unwrap();
    let replay = policy(&mut f, 138, rev, OwnerAction::Delete { page: PAGE }).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.head, deleted.head);
    assert_eq!(replay.outcome, deleted.outcome);
    assert_eq!(f.counts(), counts);
    let rev = revision(&f);
    assert_eq!(
        policy(&mut f, 139, rev, share(PAGE, ShareMode::Link))
            .unwrap_err()
            .code,
        Code::Denied
    );
    assert_eq!(f.counts(), counts);
}
#[test]
fn retention_and_archive_are_signed_replayable_policies_without_rotation() {
    let mut f = Fixture::new();
    for (op, days) in [(140, Some(7)), (141, None)] {
        let rev = revision(&f);
        let applied = policy(&mut f, op, rev, OwnerAction::Retention { page: PAGE, days }).unwrap();
        assert_eq!(wire_operations(&applied.outcome), ["retention.set"]);
        assert_eq!(page_epoch(&f, PAGE), 1);
        assert_eq!(
            statement_payload(&wire_statement(&applied.outcome, 0))["days"],
            json!(days)
        );
        assert_eq!(
            policy(&mut f, op, rev, OwnerAction::Retention { page: PAGE, days })
                .unwrap()
                .outcome,
            applied.outcome
        );
    }
    let counts = f.counts();
    let rev = revision(&f);
    assert_eq!(
        policy(
            &mut f,
            142,
            rev,
            OwnerAction::Retention {
                page: PAGE,
                days: Some(0)
            }
        )
        .unwrap_err()
        .code,
        Code::Invalid
    );
    assert_eq!(f.counts(), counts);
    let rev = revision(&f);
    let archived = policy(&mut f, 143, rev, OwnerAction::Archive { page: PAGE }).unwrap();
    assert_eq!(wire_operations(&archived.outcome), ["page.archive"]);
    let counts = f.counts();
    let now = revision(&f);
    assert_eq!(
        policy(&mut f, 144, now, share(PAGE, ShareMode::Public))
            .unwrap_err()
            .code,
        Code::Denied
    );
    assert!(f.advance(&operation(145), now).is_err());
    assert_eq!(f.counts(), counts);
    assert_eq!(
        policy(&mut f, 143, rev, OwnerAction::Archive { page: PAGE })
            .unwrap()
            .outcome,
        archived.outcome
    );
    policy(
        &mut f,
        146,
        now,
        OwnerAction::Retention {
            page: PAGE,
            days: None,
        },
    )
    .unwrap();
    let now = revision(&f);
    policy(&mut f, 147, now, OwnerAction::Delete { page: PAGE }).unwrap();
}
