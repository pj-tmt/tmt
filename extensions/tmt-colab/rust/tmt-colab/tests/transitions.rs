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
            tx.put_epoch_secret(PAGE,1,&[11;32])?;Ok(b"genesis".to_vec())
        }).unwrap();
        Self {
            root,
            layout,
            key,
            store,
            engine: Engine::new(env!("CARGO_BIN_EXE_tmt-colab").into()).unwrap(),
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
        let h = object::Header::decode(object.header()).unwrap();
        let c = &h.context;
        let bytes = object.to_json().unwrap();
        let envelope = Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: DEVICE,
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
        let thread = std::thread::spawn(move || {
            let layout = Layout::existing(&root).unwrap().unwrap();
            let key = Keyring::read(&layout).unwrap();
            let mut store = Store::open(&layout).unwrap();
            Engine::new(proxy).unwrap().advance_epoch(
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
            let signaled = poll(&mut events, PollTimeout::try_from(5000).unwrap()).unwrap();
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
        let result = thread.join().unwrap();
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
