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
