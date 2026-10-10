//! Real SQLite/keyring and mount socket acceptance for owner-key registration.
mod support;
use ed25519_dalek::{Signer, SigningKey};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};
use tmt_colab::{
    keyring::{Keyring, Layout},
    registration::{CERTIFICATE_MS, Code, FRESHNESS_MS, RENEWAL_MS, Registration},
    socket::{MountSocket, Tunnels},
    store::Store,
};
use tmt_colab_model::{certificate, framing, statement, values};
const DEVICE: &str = "10000000-0000-4000-8000-000000000001";
const OTHER: &str = "10000000-0000-4000-8000-000000000002";
const NOW: u64 = 1790000000000;
struct Fixture {
    root: PathBuf,
    layout: Layout,
    service: Option<Registration>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = PathBuf::from(format!(
            "/tmp/tmt-1162-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let store = Store::open(&layout).unwrap();
        Self {
            root,
            layout,
            service: Some(
                Registration::with_decoder_config(
                    store,
                    key,
                    support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
                )
                .unwrap(),
            ),
        }
    }
    fn service(&mut self) -> &mut Registration {
        self.service.as_mut().unwrap()
    }
    fn oracle(&self) -> Connection {
        Connection::open(self.layout.directory.join("space.db")).unwrap()
    }
    fn reopen(&mut self) {
        self.service.take().unwrap().close().unwrap();
        self.service = Some(
            Registration::with_decoder_config(
                Store::open(&self.layout).unwrap(),
                Keyring::read(&self.layout).unwrap(),
                support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
            )
            .unwrap(),
        );
    }
    fn rows(&self, table: &str) -> i64 {
        self.oracle()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(s) = self.service.take() {
            s.close().unwrap();
        }
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn remote() -> SigningKey {
    SigningKey::from_bytes(&[9; 32])
}
fn context(id: &str, revision: u64) -> String {
    json!({"deviceId":id,"kind":"browser","origin":"http://127.0.0.1:1","name":"Browser",
        "publicKey":values::encode_binary(&remote().verifying_key().to_bytes()),"owner":true,"grantRevision":revision}).to_string()
}
fn key_cert(purpose: &str, key: &[u8; 32], issued: u64) -> Value {
    // Independent field list, matching remote's canonical ext_cert contract.
    let input = framing::frame(&[
        b"tmt-ext-cert-v1",
        b"colab",
        purpose.as_bytes(),
        key,
        issued.to_string().as_bytes(),
    ])
    .unwrap();
    json!({"publicKey":values::encode_binary(key),"issuedAtMs":issued,"signature":values::encode_binary(&remote().sign(&input).to_bytes())})
}
fn request(id: &str, issued: u64) -> Value {
    let sign = SigningKey::from_bytes(&[10; 32]).verifying_key().to_bytes();
    let enc = tmt_colab_model::wrap::RecipientKey::from_seed(&[11; 32])
        .unwrap()
        .public_key();
    json!({"deviceId":id,"sign":key_cert("sign",&sign,issued),"enc":key_cert("enc",&enc,issued)})
}
fn register(f: &mut Fixture, c: Option<&str>, body: &Value, now: u64) -> Result<Vec<u8>, Code> {
    f.service()
        .register(c, &serde_json::to_vec(body).unwrap(), now)
}
fn chain(outcome: &[u8]) -> certificate::Chain {
    let v: Value = serde_json::from_slice(outcome).unwrap();
    certificate::Chain::from_json(&serde_json::to_vec(&v["chain"]).unwrap()).unwrap()
}
fn assert_browser_registration_anchor(f: &Fixture, outcome: &[u8]) {
    let key = Keyring::read(&f.layout).unwrap();
    let v: Value = serde_json::from_slice(outcome).unwrap();
    let issuer =
        statement::Envelope::from_json(&serde_json::to_vec(&v["issuerStatement"]).unwrap())
            .unwrap();
    let verified = issuer
        .verify_next(&key.space_id, &key.owner_public(), None)
        .unwrap();
    let chain = chain(outcome);
    let cert = chain.certificate().unwrap();
    // Browser verifyRegistration authenticates the genesis issuer before sync.
    assert_eq!(cert.space, key.space_id);
    assert_eq!(cert.issuer_kind, "member");
    assert_eq!(cert.issuer_id, verified.head.owner_member.id);
    assert_eq!(cert.membership_revision, "1");
    assert_eq!(verified.head.revision, 1);
    chain
        .verify(
            &verified.head.hash,
            &cert,
            &verified.head.owner_member.signing_key,
        )
        .unwrap();
}
#[test]
fn independent_python_management_keys_and_remote_ext_cert_bytes() {
    let v: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/management-key-v1.json"
    ))
    .unwrap();
    let decode = |name: &str| -> Vec<u8> {
        v[name]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks(2)
            .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
            .collect()
    };
    let f = Fixture::new();
    fs::remove_file(f.layout.directory.join("owner.key")).unwrap();
    f.layout
        .file("owner.key")
        .unwrap()
        .write_all(&decode("ownerSeed"))
        .unwrap();
    let key = Keyring::read(&f.layout).unwrap();
    let member = key.management_member().unwrap();
    assert_eq!(member.id, v["memberId"]);
    assert_eq!(member.signing_key.as_slice(), decode("signingPublic"));
    assert_eq!(member.encryption_key.as_slice(), decode("encryptionPublic"));
    let request = request(DEVICE, NOW);
    assert_eq!(
        values::binary(request["sign"]["signature"].as_str().unwrap(), 64).unwrap(),
        decode("extCertSignature")
    );
    let input = framing::frame(&[
        b"tmt-ext-cert-v1",
        b"colab",
        b"sign",
        &decode("extensionPublic"),
        NOW.to_string().as_bytes(),
    ])
    .unwrap();
    assert_eq!(input, decode("extCertInput"));
    tmt_colab_model::crypto::verify_signature(
        &decode("remotePublic"),
        &input,
        &decode("extCertSignature"),
    )
    .unwrap();
}
#[test]
fn certificate_persists_retries_exactly_and_renews_without_changing_membership() {
    let mut f = Fixture::new();
    let c = context(DEVICE, 1);
    let first = register(&mut f, Some(&c), &request(DEVICE, NOW), NOW).unwrap();
    let key = Keyring::read(&f.layout).unwrap();
    let v: Value = serde_json::from_slice(&first).unwrap();
    let issuer =
        statement::Envelope::from_json(&serde_json::to_vec(&v["issuerStatement"]).unwrap())
            .unwrap();
    let verified = issuer
        .verify_next(&key.space_id, &key.owner_public(), None)
        .unwrap();
    let chain = chain(&first);
    let cert = chain.certificate().unwrap();
    assert_eq!(cert.device_id, DEVICE);
    assert_eq!(cert.issuer_id, verified.head.owner_member.id);
    assert_eq!(cert.membership_revision, "1");
    assert_eq!(cert.expires_at, NOW + CERTIFICATE_MS);
    chain
        .verify(
            &issuer.hash().unwrap(),
            &cert,
            &verified.head.owner_member.signing_key,
        )
        .unwrap();
    f.reopen();
    assert_eq!(
        register(&mut f, Some(&c), &request(DEVICE, NOW + 1), NOW + 1).unwrap(),
        first
    );
    assert_eq!(
        f.service().active_device(Some(&c), NOW).unwrap(),
        *cert.signing_key
    );
    let renewal = NOW + CERTIFICATE_MS - RENEWAL_MS + 1;
    let next = register(&mut f, Some(&c), &request(DEVICE, renewal), renewal).unwrap();
    assert_ne!(next, first);
    assert_eq!(chain_of_expiry(&next), renewal + CERTIFICATE_MS);
    assert_eq!(f.rows("membership_log"), 1);
    assert_eq!(f.rows("devices"), 1);
}
fn chain_of_expiry(bytes: &[u8]) -> u64 {
    chain(bytes).certificate().unwrap().expires_at
}
#[test]
fn every_admission_negative_has_valid_keys_and_no_persisted_effect() {
    for case in [
        "cookie",
        "nonowner",
        "missingkey",
        "wrongkey",
        "wrongid",
        "sign",
        "enc",
        "stale",
        "future",
        "purpose",
        "extension",
        "extra",
        "duplicate",
    ] {
        let mut f = Fixture::new();
        let mut c: Value = serde_json::from_str(&context(DEVICE, 1)).unwrap();
        let mut b = request(DEVICE, NOW);
        let mut expected = Code::Denied;
        match case {
            "nonowner" => c["owner"] = json!(false),
            "missingkey" => {
                c.as_object_mut().unwrap().remove("publicKey");
            }
            "wrongkey" => {
                c["publicKey"] = json!(values::encode_binary(
                    &SigningKey::from_bytes(&[12; 32]).verifying_key().to_bytes()
                ))
            }
            "wrongid" => b["deviceId"] = json!(OTHER),
            "sign" => b["sign"]["signature"] = json!(values::encode_binary(&[0; 64])),
            "enc" => b["enc"]["signature"] = json!(values::encode_binary(&[0; 64])),
            "stale" => {
                b = request(DEVICE, NOW - FRESHNESS_MS - 1);
                expected = Code::Expired;
            }
            "future" => {
                b = request(DEVICE, NOW + 1);
                expected = Code::Expired;
            }
            "purpose" => {
                let key = values::binary(b["sign"]["publicKey"].as_str().unwrap(), 32)
                    .unwrap()
                    .try_into()
                    .unwrap();
                b["sign"] = key_cert("enc", &key, NOW);
            }
            "extension" => {
                let key = values::binary(b["sign"]["publicKey"].as_str().unwrap(), 32).unwrap();
                let input = framing::frame(&[
                    b"tmt-ext-cert-v1",
                    b"other",
                    b"sign",
                    &key,
                    NOW.to_string().as_bytes(),
                ])
                .unwrap();
                b["sign"]["signature"] =
                    json!(values::encode_binary(&remote().sign(&input).to_bytes()));
            }
            "extra" => {
                b["unknown"] = json!(1);
                expected = Code::Invalid;
            }
            "duplicate" => expected = Code::Invalid,
            _ => {}
        }
        let c = c.to_string();
        let mut bytes = b.to_string();
        if case == "duplicate" {
            bytes = bytes.replacen(
                '{',
                "{\"deviceId\":\"10000000-0000-4000-8000-000000000001\",",
                1,
            );
        }
        assert_eq!(
            f.service()
                .register(
                    if case == "cookie" { None } else { Some(&c) },
                    bytes.as_bytes(),
                    NOW
                )
                .unwrap_err(),
            expected,
            "{case}"
        );
        assert_eq!(f.rows("devices"), 0, "{case}");
        assert_eq!(f.rows("membership_log"), 0, "{case}");
    }
}
#[test]
fn changed_binding_conflicts_and_ordered_revocation_survives_restart() {
    let mut f = Fixture::new();
    let c = context(DEVICE, 5);
    let b = request(DEVICE, NOW);
    let first = register(&mut f, Some(&c), &b, NOW).unwrap();
    let mut changed = b.clone();
    changed["enc"] = key_cert("enc", &[25; 32], NOW);
    assert_eq!(
        register(&mut f, Some(&c), &changed, NOW).unwrap_err(),
        Code::Conflict
    );
    f.service()
        .active_device(Some(&context(DEVICE, 5)), NOW)
        .unwrap();
    f.service().revoke(DEVICE, 4).unwrap(); // Older event cannot undo a newer admitted context.
    assert_eq!(register(&mut f, Some(&c), &b, NOW).unwrap(), first);
    f.service()
        .active_device(Some(&context(DEVICE, 7)), NOW)
        .unwrap();
    f.service().revoke(DEVICE, 6).unwrap();
    f.service()
        .active_device(Some(&context(DEVICE, 7)), NOW)
        .unwrap();
    assert!(f.service().revoke(DEVICE, 8).unwrap());
    assert_eq!(f.rows("membership_log"), 2);
    let db = f.oracle();
    db.execute_batch("CREATE TRIGGER no_replay_insert BEFORE INSERT ON device_registrations BEGIN SELECT RAISE(ABORT,'replay writes registration'); END;
        CREATE TRIGGER no_replay_update BEFORE UPDATE ON devices BEGIN SELECT RAISE(ABORT,'replay writes device'); END;").unwrap();
    let before = fs::read(f.layout.directory.join("space.db")).unwrap();
    assert!(!f.service().revoke(DEVICE, 8).unwrap());
    assert!(!f.service().revoke(DEVICE, 9).unwrap());
    assert_eq!(
        fs::read(f.layout.directory.join("space.db")).unwrap(),
        before,
        "equal revision must not write"
    );
    db.execute_batch("DROP TRIGGER no_replay_insert; DROP TRIGGER no_replay_update;")
        .unwrap();
    f.reopen();
    assert_eq!(
        register(&mut f, Some(&context(DEVICE, 9)), &b, NOW).unwrap_err(),
        Code::Denied
    );
    assert_eq!(
        f.service().active_device(Some(&c), NOW).unwrap_err(),
        Code::Denied
    );
    f.service().revoke(OTHER, 2).unwrap();
    f.reopen();
    assert_eq!(
        register(&mut f, Some(&context(OTHER, 3)), &request(OTHER, NOW), NOW).unwrap_err(),
        Code::Denied
    );
    let record: Vec<u8> = f
        .oracle()
        .query_row("SELECT record FROM devices WHERE id=?", [DEVICE], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&record).unwrap()["revoked"],
        true
    );
    let binding: Option<Vec<u8>> = f
        .oracle()
        .query_row(
            "SELECT binding FROM device_registrations WHERE device_id=?",
            [DEVICE],
            |r| r.get(0),
        )
        .unwrap();
    assert!(binding.is_none());
}
#[test]
fn failed_registration_write_rolls_back_device_and_response() {
    for table in ["devices", "device_registrations"] {
        let mut f = Fixture::new();
        f.oracle().execute_batch(&format!("CREATE TRIGGER injected BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'injected'); END;")).unwrap();
        assert_eq!(
            register(
                &mut f,
                Some(&context(DEVICE, 1)),
                &request(DEVICE, NOW),
                NOW
            )
            .unwrap_err(),
            Code::Unavailable
        );
        assert_eq!(f.rows("devices"), 0);
        assert_eq!(f.rows("device_registrations"), 0);
        // Genesis is a separate, valid existing owner transaction.
        assert_eq!(f.rows("membership_log"), 1);
    }
}
#[test]
fn mounted_endpoint_rejects_cookie_only_and_accepts_certified_owner_twice() {
    for _ in 0..2 {
        let mut f = Fixture::new();
        let service = Arc::new(Mutex::new(f.service.take().unwrap()));
        let key = Keyring::read(&f.layout).unwrap();
        let socket = MountSocket::bind(&f.layout, &key.space_id, Tunnels::PRODUCT)
            .unwrap()
            .with_registration(&f.layout, Arc::clone(&service))
            .unwrap();
        let path = socket.path.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let worker = thread::spawn(move || socket.run(&flag).unwrap());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let body = request(DEVICE, now).to_string();
        let call = |headers: &str| {
            let mut peer = UnixStream::connect(&path).unwrap();
            peer.set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            write!(
                peer,
                "POST /api/devices/register HTTP/1.1\r\nContent-Length: {}\r\n{headers}\r\n{body}",
                body.len()
            )
            .unwrap();
            peer.shutdown(std::net::Shutdown::Write).unwrap();
            let mut reply = String::new();
            peer.read_to_string(&mut reply).unwrap();
            reply
        };
        assert!(call("Cookie: tmt_door=forged\r\n").starts_with("HTTP/1.1 403"));
        assert_eq!(f.rows("devices"), 0);
        let reply = call(&format!("tmt-device-context: {}\r\n", context(DEVICE, 1)));
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(reply.contains("Content-Type: application/json"));
        assert_eq!(
            call(&format!("tmt-device-context: {}\r\n", context(DEVICE, 1))),
            reply
        );
        stop.store(true, Ordering::Release);
        worker.join().unwrap();
        assert!(!path.exists());
        f.service = Some(Arc::try_unwrap(service).ok().unwrap().into_inner().unwrap());
    }
}

#[test]
fn schema_two_authority_rows_survive_registration_migration() {
    let mut f = Fixture::new();
    register(
        &mut f,
        Some(&context(DEVICE, 1)),
        &request(DEVICE, NOW),
        NOW,
    )
    .unwrap();
    f.service.take().unwrap().close().unwrap();
    let db = f.oracle();
    let before: Vec<u8> = db
        .query_row("SELECT record FROM devices WHERE id=?", [DEVICE], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute_batch(
        "ALTER TABLE owner_operations RENAME TO current_owner_operations;
        CREATE TABLE owner_operations(id TEXT PRIMARY KEY, digest BLOB NOT NULL, outcome BLOB NOT NULL);
        INSERT INTO owner_operations(id,digest,outcome) SELECT id,digest,outcome FROM current_owner_operations;
        DROP TABLE current_owner_operations;
        ALTER TABLE pages DROP COLUMN last_update_at_ms; DROP TABLE baselines; DROP TABLE device_registrations; PRAGMA user_version=2;",
    )
    .unwrap();
    let store = Store::open(&f.layout).unwrap();
    let after: Vec<u8> = db
        .query_row("SELECT record FROM devices WHERE id=?", [DEVICE], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(after, before);
    assert_eq!(f.rows("membership_log"), 1);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        6
    );
    store.close().unwrap();
}

#[test]
fn unknown_revoke_before_genesis_is_tombstone_only_and_never_signs_on_replay() {
    let mut f = Fixture::new();
    assert!(f.service().revoke(OTHER, 2).unwrap());
    assert_eq!(f.rows("device_registrations"), 1);
    assert_eq!(f.rows("devices"), 0);
    assert_eq!(f.rows("membership_log"), 0);
    f.oracle().execute_batch("CREATE TRIGGER deny_replay BEFORE INSERT ON device_registrations BEGIN SELECT RAISE(ABORT,'replay wrote'); END;").unwrap();
    for revision in [1, 2, 3] {
        assert!(!f.service().revoke(OTHER, revision).unwrap());
    }
    assert_eq!(f.rows("membership_log"), 0);
    f.reopen();
    assert_eq!(
        register(&mut f, Some(&context(OTHER, 4)), &request(OTHER, NOW), NOW).unwrap_err(),
        Code::Denied
    );
}

#[test]
fn archive_preserves_reads_and_delete_denies_admission_catchup_and_queued_delivery() {
    use tmt_colab::{
        registration::OwnerAdmission,
        store::owner::Mutation,
        sync::{Access, Admission, Progress, Server, SyncScope},
        transitions::{Engine, OwnerAction, OwnerRequest},
    };
    use tmt_colab_model::object;
    use tungstenite::{Message, WebSocket, protocol::Role};
    let mut f = Fixture::new();
    register(
        &mut f,
        Some(&context(DEVICE, 1)),
        &request(DEVICE, NOW),
        NOW,
    )
    .unwrap();
    let key = Keyring::read(&f.layout).unwrap();
    let mut store = Store::open(&f.layout).unwrap();
    store.create_page(OTHER).unwrap();
    store
        .owner_transaction(
            &key.space_id,
            &key.owner_public(),
            Mutation {
                operation_id: OTHER,
                digest: [116; 32],
                expected_revision: 1,
            },
            |tx| {
                tx.put_epoch_secret(OTHER, 1, &[12; 32])?;
                tx.append_statement(&key.sign_statement(
                    tx.head(),
                    "page.history",
                    &serde_json::to_vec(&json!({"pageId":OTHER,"mode":"shared"}))?,
                )?)?;
                Ok(vec![])
            },
        )
        .unwrap();
    let registration = Arc::new(Mutex::new(f.service.take().unwrap()));
    let admission = OwnerAdmission(Arc::clone(&registration));
    let scope = SyncScope {
        space: key.space_id.clone(),
        page: OTHER.into(),
        epoch: "1".into(),
    };
    let mut header = object::Context {
        space: key.space_id.clone(),
        page: OTHER.into(),
        epoch: "1".into(),
        kind: "update".into(),
        namespace: "content".into(),
        author_device: DEVICE.into(),
        membership_revision: "2".into(),
        stream_seq: "1".into(),
        prev_hash: [0; 32],
    };
    assert!(admission.authorize(DEVICE, &scope, Access::Read).is_ok());
    assert!(
        admission
            .authorize(DEVICE, &scope, Access::Append(&header))
            .is_ok()
    );
    assert!(admission.catchup_context(DEVICE, &scope, &store).is_ok());
    let server = Server::new(
        Store::open(&f.layout).unwrap(),
        OwnerAdmission(Arc::clone(&registration)),
    );
    let pair = || {
        let (client, socket) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        socket.set_nonblocking(true).unwrap();
        (
            WebSocket::from_raw_socket(client, Role::Client, None),
            server.connect(socket, DEVICE.into()).unwrap(),
        )
    };
    let mut reader = pair();
    let mut sender = pair();
    let frame = |kind: &str, fields: Value| {
        let mut value =
            json!({"type":kind,"version":1,"space":key.space_id,"page":OTHER,"epoch":"1"});
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        value
    };
    reader
        .0
        .send(Message::Text(
            frame("subscribe", json!({"cursors":[]})).to_string().into(),
        ))
        .unwrap();
    assert_eq!(reader.1.poll(), Progress::Advanced);
    assert_eq!(reader.1.poll(), Progress::Pending);
    let mut sent = Vec::new();
    for seq in 1..=2 {
        header.stream_seq = seq.to_string();
        let envelope = object::seal(
            &header,
            &[12; 32],
            &SigningKey::from_bytes(&[10; 32]),
            b"queued ciphertext",
        )
        .unwrap();
        let hash = envelope.hash().unwrap();
        let bytes = envelope.to_json().unwrap();
        sender.0.send(Message::Text(frame("append",json!({"streamId":DEVICE,"seq":seq.to_string(),"envelopeHash":values::encode_binary(&hash),"envelope":values::encode_binary(&bytes)})).to_string().into())).unwrap();
        assert_eq!(sender.1.poll(), Progress::Advanced);
        let ack: Value = serde_json::from_str(sender.0.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(ack["type"], "receipt");
        sent.push(bytes);
        header.prev_hash = hash;
    }
    let mut engine = Engine::new(env!("CARGO_BIN_EXE_tmt-colab").into()).unwrap();
    let mut archived = None;
    server
        .update_admission(|a| {
            let _registration = a.0.lock().unwrap();
            archived = Some(
                engine
                    .apply(
                        &mut store,
                        &key,
                        OwnerRequest {
                            operation_id: "40000000-0000-4000-8000-000000000151",
                            expected_revision: 2,
                            action: OwnerAction::Archive { page: OTHER },
                            transport_digest: None,
                            scope: None,
                        },
                        NOW,
                    )
                    .unwrap(),
            );
        })
        .unwrap();
    header.membership_revision = archived.as_ref().unwrap().head.revision.to_string();
    assert!(admission.authorize(DEVICE, &scope, Access::Read).is_ok());
    assert_eq!(
        admission
            .authorize(DEVICE, &scope, Access::Append(&header))
            .unwrap_err(),
        tmt_colab::sync::Code::Denied
    );
    assert!(admission.catchup_context(DEVICE, &scope, &store).is_ok());
    assert_eq!(reader.1.poll(), Progress::Advanced);
    let delivered: Value =
        serde_json::from_str(reader.0.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(delivered["type"], "broadcast");
    assert_eq!(
        values::binary(delivered["envelope"].as_str().unwrap(), 65536).unwrap(),
        sent[0]
    );
    header.stream_seq = "3".into();
    let denied = object::seal(
        &header,
        &[12; 32],
        &SigningKey::from_bytes(&[10; 32]),
        b"must not persist",
    )
    .unwrap();
    sender.0.send(Message::Text(frame("append",json!({"streamId":DEVICE,"seq":"3","envelopeHash":values::encode_binary(&denied.hash().unwrap()),"envelope":values::encode_binary(&denied.to_json().unwrap())})).to_string().into())).unwrap();
    assert_eq!(sender.1.poll(), Progress::Advanced);
    let error: Value = serde_json::from_str(sender.0.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(error["code"], "DENIED");
    assert!(
        store
            .payload(
                tmt_colab::store::StreamScope {
                    page: OTHER,
                    epoch: 1,
                    stream: DEVICE
                },
                3
            )
            .unwrap()
            .is_none()
    );
    server
        .update_admission(|a| {
            let _registration = a.0.lock().unwrap();
            engine
                .apply(
                    &mut store,
                    &key,
                    OwnerRequest {
                        operation_id: "40000000-0000-4000-8000-000000000152",
                        expected_revision: 3,
                        action: OwnerAction::Delete { page: OTHER },
                        transport_digest: None,
                        scope: None,
                    },
                    NOW,
                )
                .unwrap();
        })
        .unwrap();
    assert_eq!(
        admission
            .authorize(DEVICE, &scope, Access::Read)
            .unwrap_err(),
        tmt_colab::sync::Code::Denied
    );
    assert_eq!(
        admission
            .authorize(DEVICE, &scope, Access::Append(&header))
            .unwrap_err(),
        tmt_colab::sync::Code::Denied
    );
    assert!(matches!(
        admission.catchup_context(DEVICE, &scope, &store),
        Err(tmt_colab::sync::Code::Denied)
    ));
    assert_eq!(reader.1.poll(), Progress::Closed);
    let Message::Close(Some(close)) = reader.0.read().unwrap() else {
        panic!("deleted page delivered pending ciphertext")
    };
    assert_eq!(close.reason, "DENIED");
    assert!(store.create_page(OTHER).is_err());
    drop(reader);
    drop(sender);
    drop(server);
    drop(admission);
    Arc::try_unwrap(registration)
        .ok()
        .unwrap()
        .into_inner()
        .unwrap()
        .close()
        .unwrap();
    let reopened = OwnerAdmission(Arc::new(Mutex::new(
        Registration::new(
            Store::open(&f.layout).unwrap(),
            Keyring::read(&f.layout).unwrap(),
            env!("CARGO_BIN_EXE_tmt-colab").into(),
        )
        .unwrap(),
    )));
    assert_eq!(
        reopened
            .authorize(DEVICE, &scope, Access::Read)
            .unwrap_err(),
        tmt_colab::sync::Code::Denied
    );
    drop(reopened);
    store.close().unwrap();
}

fn create_page(
    f: &mut Fixture,
    page: &str,
    operation: &str,
    expected: u64,
) -> tmt_colab::transitions::Applied {
    f.service()
        .apply_owner(
            tmt_colab::transitions::OwnerRequest {
                operation_id: operation,
                expected_revision: expected,
                action: tmt_colab::transitions::OwnerAction::Create {
                    creation_recipient: None,
                    page,
                    title: "Created title",
                    source: "<h1>Created source</h1>",
                    publisher_agent: None,
                },
                transport_digest: None,
                scope: None,
            },
            NOW,
        )
        .unwrap()
}
fn browser_reads_created_page(f: &Fixture, page: &str, device: &str) {
    use tmt_colab_model::{object, wrap};
    use yrs::{Doc, GetString, Map, ReadTxn, Transact, Update, updates::decoder::Decode};
    let key = Keyring::read(&f.layout).unwrap();
    let bytes: Vec<u8> = f.oracle().query_row(
        "SELECT envelope FROM wraps WHERE page=? AND kind='device' AND recipient=? ORDER BY revision DESC LIMIT 1",
        [page,device], |r| r.get(0),
    ).unwrap();
    let wrapped = wrap::Envelope::from_json(&bytes).unwrap();
    let secret = wrap::open(
        &wrapped,
        &wrapped.header().unwrap(),
        &wrap::RecipientKey::from_seed(&[11; 32]).unwrap(),
        &key.owner_public(),
    )
    .unwrap();
    let (bytes,writer): (Vec<u8>,String) = f.oracle().query_row(
        "SELECT payload,stream FROM receipts WHERE page=? AND namespace='content' ORDER BY seq LIMIT 1", [page],
        |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    let record: Vec<u8> = f
        .oracle()
        .query_row("SELECT record FROM devices WHERE id=?", [&writer], |r| {
            r.get(0)
        })
        .unwrap();
    let registered: tmt_colab::store::owner::Device = serde_json::from_slice(&record).unwrap();
    let chain = certificate::Chain::from_json(&registered.chain).unwrap();
    let object = object::Envelope::from_json(&bytes).unwrap();
    let header = object::Header::decode(object.header()).unwrap();
    let update = object::open(
        &object,
        &header.context,
        &secret,
        chain.certificate().unwrap().signing_key,
    )
    .unwrap();
    let doc = Doc::new();
    doc.transact_mut()
        .apply_update(Update::decode_v1(&update).unwrap())
        .unwrap();
    let tx = doc.transact();
    assert_eq!(
        tx.get_text("html").unwrap().get_string(&tx),
        "<h1>Created source</h1>"
    );
    assert_eq!(
        tx.get_map("meta")
            .unwrap()
            .get(&tx, "title")
            .unwrap()
            .to_string(&tx),
        "Created title"
    );
}
#[test]
fn creation_wraps_existing_owner_and_late_registration_opens_only_active_pages_without_retry_growth()
 {
    use tmt_colab::transitions::{OwnerAction, OwnerRequest};
    const PAGE_A: &str = "50000000-0000-4000-8000-000000000001";
    const PAGE_B: &str = "50000000-0000-4000-8000-000000000002";
    let mut f = Fixture::new();
    register(
        &mut f,
        Some(&context(DEVICE, 1)),
        &request(DEVICE, NOW),
        NOW,
    )
    .unwrap();
    create_page(&mut f, PAGE_A, PAGE_A, 1);
    browser_reads_created_page(&f, PAGE_A, DEVICE);
    create_page(&mut f, PAGE_B, PAGE_B, 2);
    f.service()
        .apply_owner(
            OwnerRequest {
                operation_id: "50000000-0000-4000-8000-000000000003",
                expected_revision: 3,
                action: OwnerAction::Archive { page: PAGE_A },
                transport_digest: None,
                scope: None,
            },
            NOW,
        )
        .unwrap();
    let devices = f.rows("devices");
    let bindings = f.rows("device_registrations");
    let wraps = f.rows("wraps");
    f.oracle().execute_batch("CREATE TRIGGER deny_forward_wrap BEFORE INSERT ON wraps BEGIN SELECT RAISE(FAIL,'forced rollback'); END;").unwrap();
    assert_eq!(
        register(&mut f, Some(&context(OTHER, 1)), &request(OTHER, NOW), NOW).unwrap_err(),
        Code::Unavailable
    );
    assert_eq!(f.rows("devices"), devices);
    assert_eq!(f.rows("device_registrations"), bindings);
    assert_eq!(f.rows("wraps"), wraps);
    f.oracle()
        .execute_batch("DROP TRIGGER deny_forward_wrap")
        .unwrap();
    let first = register(&mut f, Some(&context(OTHER, 1)), &request(OTHER, NOW), NOW).unwrap();
    assert_browser_registration_anchor(&f, &first);
    browser_reads_created_page(&f, PAGE_B, OTHER);
    let count: i64 = f
        .oracle()
        .query_row(
            "SELECT count(*) FROM wraps WHERE page=? AND kind='device' AND recipient=?",
            [PAGE_A, OTHER],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let count = f.rows("wraps");
    let log = f.rows("membership_log");
    f.reopen();
    assert_eq!(
        register(&mut f, Some(&context(OTHER, 1)), &request(OTHER, NOW), NOW).unwrap(),
        first
    );
    assert_eq!(f.rows("wraps"), count);
    assert_eq!(f.rows("membership_log"), log);
    f.service().revoke(OTHER, 2).unwrap();
    let count = f.rows("wraps");
    assert_eq!(
        register(&mut f, Some(&context(OTHER, 3)), &request(OTHER, NOW), NOW).unwrap_err(),
        Code::Denied
    );
    assert_eq!(f.rows("wraps"), count);
}
#[test]
fn create_first_registration_and_saved_head_certificate_retry_use_the_genesis_anchor() {
    const PAGE: &str = "50000000-0000-4000-8000-000000000005";
    let mut f = Fixture::new();
    create_page(&mut f, PAGE, PAGE, 0);
    let c = context(DEVICE, 1);
    let first = register(&mut f, Some(&c), &request(DEVICE, NOW), NOW).unwrap();
    assert_browser_registration_anchor(&f, &first);
    browser_reads_created_page(&f, PAGE, DEVICE);

    // Reproduce a durable response issued by the previous native implementation:
    // genuine member signature, same keys and issuer, but the current head (2).
    let key = Keyring::read(&f.layout).unwrap();
    let first_chain = chain(&first);
    let mut old_cert = first_chain.certificate().unwrap();
    old_cert.membership_revision = "2";
    let mut old_response: Value = serde_json::from_slice(&first).unwrap();
    let mut root: [u8; 32] = fs::read(f.layout.directory.join("owner.key"))
        .unwrap()
        .try_into()
        .unwrap();
    let info = framing::frame(&[
        b"tmt-colab-management-signing-seed-v1",
        key.space_id.as_bytes(),
    ])
    .unwrap();
    let mut seed = tmt_colab_model::crypto::derive_key(&root, &[], &info);
    root.fill(0);
    let signer = SigningKey::from_bytes(&seed);
    seed.fill(0);
    assert_eq!(
        signer.verifying_key().to_bytes(),
        key.management_member().unwrap().signing_key
    );
    old_response["chain"]["deviceCertificate"] = json!(values::encode_binary(
        &certificate::input(&old_cert).unwrap()
    ));
    old_response["chain"]["issuerSignature"] = json!(values::encode_binary(
        &signer
            .sign(&certificate::input(&old_cert).unwrap())
            .to_bytes()
    ));
    let old_chain_bytes = serde_json::to_vec(&old_response["chain"]).unwrap();
    let old_response = serde_json::to_vec(&old_response).unwrap();
    let old_chain = chain(&old_response);
    assert_eq!(old_chain.certificate().unwrap().membership_revision, "2");
    let device = tmt_colab::store::owner::Device {
        chain: old_chain_bytes,
        revoked: false,
    };
    let db = f.oracle();
    let binding: Vec<u8> = db
        .query_row(
            "SELECT binding FROM device_registrations WHERE device_id=?",
            [DEVICE],
            |r| r.get(0),
        )
        .unwrap();
    let mut binding: Value = serde_json::from_slice(&binding).unwrap();
    binding["outcome"] = json!(old_response);
    db.execute(
        "UPDATE devices SET record=? WHERE id=?",
        rusqlite::params![serde_json::to_vec(&device).unwrap(), DEVICE],
    )
    .unwrap();
    db.execute(
        "UPDATE device_registrations SET binding=? WHERE device_id=?",
        rusqlite::params![serde_json::to_vec(&binding).unwrap(), DEVICE],
    )
    .unwrap();
    drop(db);
    f.reopen();
    assert_eq!(
        f.service().active_device(Some(&c), NOW).unwrap(),
        *old_cert.signing_key
    );
    let wraps = f.rows("wraps");
    let next = register(&mut f, Some(&c), &request(DEVICE, NOW + 1), NOW + 1).unwrap();
    assert_browser_registration_anchor(&f, &next);
    assert_eq!(
        chain(&next).certificate().unwrap().signing_key,
        old_cert.signing_key
    );
    assert_eq!(
        chain(&next).certificate().unwrap().encryption_key,
        old_cert.encryption_key
    );
    assert_eq!(f.rows("membership_log"), 2);
    assert_eq!(f.rows("devices"), 2); // local page author plus the browser
    assert_eq!(f.rows("device_registrations"), 1);
    assert_eq!(f.rows("wraps"), wraps);
    browser_reads_created_page(&f, PAGE, DEVICE);
    f.reopen();
    assert_eq!(
        register(&mut f, Some(&c), &request(DEVICE, NOW + 2), NOW + 2).unwrap(),
        next
    );
}
#[test]
fn fresh_creation_is_atomic_replayable_and_conflicting_selections_never_create_another_page() {
    use tmt_colab::transitions::{OwnerAction, OwnerRequest};
    const PAGE: &str = "50000000-0000-4000-8000-000000000004";
    let recipient = tmt_colab::decoder::CreationRecipient {
        machine_id: "40000000-0000-4000-8000-000000000001".into(),
        agent_id: "50000000-0000-1000-8000-000000000001".into(),
    };
    let mut f = Fixture::new();
    f.oracle().execute_batch("CREATE TRIGGER deny_create_receipt BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(FAIL,'forced rollback'); END;").unwrap();
    let apply =
        |f: &mut Fixture, title: &str, hint: Option<&tmt_colab::decoder::CreationRecipient>| {
            f.service().apply_owner(
                OwnerRequest {
                    operation_id: PAGE,
                    expected_revision: 0,
                    action: OwnerAction::Create {
                        creation_recipient: hint,
                        page: PAGE,
                        title,
                        source: "<h1>Created source</h1>",
                        publisher_agent: Some("original-author"),
                    },
                    transport_digest: None,
                    scope: None,
                },
                NOW,
            )
        };
    assert!(apply(&mut f, "Created title", Some(&recipient)).is_err());
    for table in [
        "pages",
        "membership_log",
        "epoch_secrets",
        "wraps",
        "receipts",
        "devices",
        "owner_operations",
    ] {
        assert_eq!(f.rows(table), 0, "{table} was partially committed");
    }
    f.oracle()
        .execute_batch("DROP TRIGGER deny_create_receipt")
        .unwrap();
    let first = apply(&mut f, "Created title", Some(&recipient)).unwrap();
    assert_eq!(first.head.revision, 2);
    f.reopen();
    let again = apply(&mut f, "Created title", Some(&recipient)).unwrap();
    assert!(again.replayed);
    assert_eq!(first.outcome, again.outcome);
    assert_eq!(f.rows("pages"), 1);
    assert_eq!(f.rows("receipts"), 1);
    assert_eq!(
        apply(&mut f, "Changed", Some(&recipient)).unwrap_err().code,
        tmt_colab::transitions::Code::Conflict
    );
    assert_eq!(f.rows("pages"), 1);
    assert_eq!(
        apply(&mut f, "Created title", None).unwrap_err().code,
        tmt_colab::transitions::Code::Conflict
    );
    let mut different = recipient.clone();
    different.agent_id = "50000000-0000-1000-8000-000000000002".into();
    assert_eq!(
        apply(&mut f, "Created title", Some(&different))
            .unwrap_err()
            .code,
        tmt_colab::transitions::Code::Conflict
    );
    let key = Keyring::read(&f.layout).unwrap();
    let store = Store::read(&f.layout).unwrap();
    let mut decoder = tmt_colab::decoder::Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    let page = tmt_colab::page::read(&store, &key, PAGE, &mut decoder).unwrap();
    assert_eq!(page.source, "<h1>Created source</h1>");
    assert_eq!(page.title, "Created title");
    assert_eq!(page.original_author.as_deref(), Some("original-author"));
    assert_eq!(page.publisher_agent.as_deref(), Some("original-author"));
    assert_eq!(page.creation_recipient, Some(recipient));
    store.close().unwrap();
}

/// Attachment scenarios use the real owner registration, signed streams and
/// isolated decoder. SQLite inspection supplies fixture asset bytes, never admission.
#[test]
fn authenticated_attachment_reads_survive_rotation_archive_and_reject_stale_disclosure() {
    use tmt_colab::{
        attachments::{self, CommittedObjectVerifier},
        decoder::Decoder,
        page,
        store::{Envelope as StoredEnvelope, Namespace, StreamScope},
        transitions::{OwnerAction, OwnerRequest},
    };
    use tmt_colab_model::{
        attachment::{AttachmentPublication, AttachmentSelector, Descriptor, Source},
        crypto, object,
    };
    use yrs::{Any, Doc, Map, ReadTxn, StateVector, Transact};
    const PAGE: &str = "20000000-0000-4000-8000-000000000091";
    const ATTACHMENT: &str = "20000000-0000-4000-8000-000000000092";
    const MESSAGE: &str = "20000000-0000-4000-8000-000000000093";
    fn hex(b: &[u8]) -> String {
        b.iter().map(|b| format!("{b:02x}")).collect()
    }
    struct Committed {
        namespace: [u8; 32],
        key: [u8; 32],
        raw: Vec<u8>,
        complete: bool,
    }
    impl CommittedObjectVerifier for Committed {
        fn read_committed(
            &self,
            namespace: &[u8; 32],
            key: &[u8; 32],
            _: std::time::Instant,
        ) -> tmt_colab::Result<Vec<u8>> {
            assert_eq!(*namespace, self.namespace);
            assert_eq!(*key, self.key);
            if !self.complete {
                return Err(page::Fault::Unavailable.into());
            }
            Ok(self.raw.clone())
        }
    }
    let mut f = Fixture::new();
    register(
        &mut f,
        Some(&context(DEVICE, 1)),
        &request(DEVICE, NOW),
        NOW,
    )
    .unwrap();
    let created = create_page(&mut f, PAGE, PAGE, 1);
    register(&mut f, Some(&context(OTHER, 1)), &request(OTHER, NOW), NOW).unwrap();
    let key = Keyring::read(&f.layout).unwrap();
    let mut store = Store::open(&f.layout).unwrap();
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    let base = page::read(&store, &key, PAGE, &mut decoder).unwrap();
    let secret: Vec<u8> = f
        .oracle()
        .query_row(
            "SELECT secret FROM epoch_secrets WHERE page=? AND CAST(epoch AS INTEGER)=1",
            [PAGE],
            |r| r.get(0),
        )
        .unwrap();
    let secret: [u8; 32] = secret.try_into().unwrap();
    let source = Source::Document {
        source_digest: hex(&crypto::digest(base.source.as_bytes())),
    };
    let asset_context = object::Context {
        space: key.space_id.clone(),
        page: PAGE.into(),
        epoch: "1".into(),
        kind: "asset".into(),
        namespace: "content".into(),
        author_device: DEVICE.into(),
        membership_revision: created.head.revision.to_string(),
        stream_seq: "0".into(),
        prev_hash: [0; 32],
    };
    let asset = object::seal(
        &asset_context,
        &secret,
        &SigningKey::from_bytes(&[10; 32]),
        b"original attachment bytes",
    )
    .unwrap();
    let raw = asset.to_json().unwrap();
    let descriptor = Descriptor {
        version: 1,
        attachment_id: ATTACHMENT.into(),
        space: key.space_id.clone(),
        page: PAGE.into(),
        epoch: "1".into(),
        namespace: "content".into(),
        object_id: object::Header::decode(asset.header()).unwrap().object_id,
        author_device: DEVICE.into(),
        membership_revision: created.head.revision.to_string(),
        source,
        envelope_hash: hex(&asset.hash().unwrap()),
        signature: values::encode_binary(asset.signature()),
        payload_sha256: hex(&crypto::digest(&raw)),
        payload_bytes: raw.len().to_string(),
        plaintext_bytes: "25".into(),
        filename: "original.txt".into(),
        media_type: "text/plain".into(),
    };
    assert_eq!(b"original attachment bytes".len(), 25);
    descriptor.validate().unwrap();
    let proof = AttachmentPublication {
        version: 1,
        kind: "attachment-publication".into(),
        space_id: key.space_id.clone(),
        page_id: PAGE.into(),
        epoch: "1".into(),
        sender_device: DEVICE.into(),
        membership_revision: created.head.revision.to_string(),
        attachment_id: ATTACHMENT.into(),
        descriptor_hash: hex(&descriptor.hash().unwrap()),
        source: descriptor.source.clone(),
        base_revision: base.revision,
    };
    let own = Doc::with_client_id(18531);
    own.get_or_insert_map("intents").insert(
        &mut own.transact_mut(),
        ATTACHMENT,
        Any::from_json(&serde_json::to_string(&proof).unwrap()).unwrap(),
    );
    // An immutable Chat/annotation revision can reference an existing document asset.
    let comment = json!({"version":1,"kind":"comment","spaceId":key.space_id,"pageId":PAGE,"epoch":"1","senderDevice":DEVICE,"revision":"1","deleted":false,
        "deviceName":"Browser","at":NOW.to_string(),"messageId":MESSAGE,"thread":{"writer":DEVICE,"id":DEVICE},"body":"Original revision","attachments":[descriptor]});
    own.get_or_insert_map("messages").insert(
        &mut own.transact_mut(),
        format!("{MESSAGE}:1"),
        Any::from_json(&comment.to_string()).unwrap(),
    );
    let content = Doc::with_client_id(18532);
    content.get_or_insert_map("meta").insert(
        &mut content.transact_mut(),
        "attachments",
        Any::from_json(&json!([descriptor]).to_string()).unwrap(),
    );
    let mut previous = [0; 32];
    for (index, (namespace, doc)) in [("own", &own), ("content", &content)]
        .into_iter()
        .enumerate()
    {
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let context = object::Context {
            kind: "update".into(),
            namespace: namespace.into(),
            stream_seq: (index + 1).to_string(),
            prev_hash: previous,
            ..asset_context.clone()
        };
        let envelope = object::seal(
            &context,
            &secret,
            &SigningKey::from_bytes(&[10; 32]),
            &update,
        )
        .unwrap();
        store
            .append(&StoredEnvelope {
                scope: StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream: DEVICE,
                },
                namespace: if namespace == "own" {
                    Namespace::Own
                } else {
                    Namespace::Content
                },
                seq: (index + 1) as u64,
                previous,
                hash: envelope.hash().unwrap(),
                bytes: &envelope.to_json().unwrap(),
            })
            .unwrap();
        previous = envelope.hash().unwrap();
    }
    // The typed document change reaches this real page. A reference without its creator's
    // own-stream proof, or bound to another epoch, never prepares; an exact repeat of a proven
    // one changes nothing; removing it is an ordinary content publication.
    {
        use tmt_colab::{decoder::ContentEdit, page::PublishOptions};
        use tmt_colab_model::attachment::DocumentChange;
        let change = |set: Vec<Descriptor>, remove: Vec<&str>| -> DocumentChange {
            serde_json::from_value(json!({
                "set": set,
                "remove": remove,
            }))
            .unwrap()
        };
        let prepare = |change: &DocumentChange, store: &Store, decoder: &mut Decoder| {
            page::prepare_publication(
                store,
                &key,
                PAGE,
                ContentEdit {
                    source: &base.source,
                    publisher_agent: None,
                    attachments: Some(change),
                },
                PublishOptions::default(),
                decoder,
                NOW,
            )
        };
        let unproven = Descriptor {
            attachment_id: "20000000-0000-4000-8000-0000000000a1".into(),
            ..descriptor.clone()
        };
        assert!(
            prepare(
                &change(vec![unproven.clone()], vec![]),
                &store,
                &mut decoder
            )
            .is_err()
        );
        let other_epoch = Descriptor {
            epoch: "2".into(),
            ..descriptor.clone()
        };
        assert!(prepare(&change(vec![other_epoch], vec![]), &store, &mut decoder).is_err());
        let wrong_source = Descriptor {
            source: Source::Document {
                source_digest: hex(&crypto::digest(b"another source")),
            },
            ..descriptor.clone()
        };
        assert!(prepare(&change(vec![wrong_source], vec![]), &store, &mut decoder).is_err());
        assert!(matches!(
            prepare(
                &change(vec![descriptor.clone()], vec![]),
                &store,
                &mut decoder
            )
            .unwrap(),
            page::PublicationPreparation::Noop { .. }
        ));
        assert!(matches!(
            prepare(&change(vec![], vec![ATTACHMENT]), &store, &mut decoder).unwrap(),
            page::PublicationPreparation::Write(_)
        ));
        assert!(
            prepare(
                &change(vec![], vec!["20000000-0000-4000-8000-0000000000a2"]),
                &store,
                &mut decoder
            )
            .is_err()
        );
    }
    let deadline = || std::time::Instant::now() + std::time::Duration::from_secs(10);
    let selector = || AttachmentSelector::DocumentCurrent {
        attachment_id: ATTACHMENT.into(),
        descriptor_hash: hex(&descriptor.hash().unwrap()),
        content_revision: page::revision(&store, &key, PAGE).unwrap(),
    };
    let object_key: [u8; 32] = descriptor
        .object_id
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let mut objects = Committed {
        namespace: attachments::namespace(&key.space_id, PAGE).unwrap(),
        key: object_key,
        raw,
        complete: true,
    };
    // Fixture-only oracle derives the actual local writer's key. Production
    // preparation still admits that writer through the Keyring/fold owners.
    let seed = fs::read(f.layout.directory.join("owner.key")).unwrap();
    let derive = |label: &[u8]| {
        crypto::derive_key(
            &seed,
            &[],
            &framing::frame(&[label, key.space_id.as_bytes()]).unwrap(),
        )
    };
    let mut id = derive(b"tmt-colab-cli-device-id-v1");
    id[6] = (id[6] & 15) | 64;
    id[8] = (id[8] & 63) | 128;
    let h = hex(&id[..16]);
    let local_id = format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    );
    let local_signer = SigningKey::from_bytes(&derive(b"tmt-colab-cli-signing-seed-v1"));
    let local_context = object::Context {
        author_device: local_id.clone(),
        ..asset_context.clone()
    };
    let local_asset = object::seal(
        &local_context,
        &secret,
        &local_signer,
        b"original attachment bytes",
    )
    .unwrap();
    let local_raw = local_asset.to_json().unwrap();
    let local_descriptor = Descriptor {
        author_device: local_id,
        object_id: object::Header::decode(local_asset.header())
            .unwrap()
            .object_id,
        envelope_hash: hex(&local_asset.hash().unwrap()),
        signature: values::encode_binary(local_asset.signature()),
        payload_sha256: hex(&crypto::digest(&local_raw)),
        payload_bytes: local_raw.len().to_string(),
        ..descriptor.clone()
    };
    let local_object_key = local_descriptor
        .object_id
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let mut committed = Committed {
        namespace: objects.namespace,
        key: local_object_key,
        raw: local_raw,
        complete: false,
    };
    let current = page::revision(&store, &key, PAGE).unwrap();
    let intent = || attachments::PublicationIntent {
        descriptor: &local_descriptor,
        base: &current,
    };
    assert!(
        attachments::prepare_publication(
            &store,
            &key,
            intent(),
            &committed,
            &mut decoder,
            deadline(),
            NOW
        )
        .is_err()
    );
    committed.complete = true;
    let frozen = attachments::prepare_publication(
        &store,
        &key,
        intent(),
        &committed,
        &mut decoder,
        deadline(),
        NOW,
    )
    .unwrap();
    assert_eq!(page::revision(&store, &key, PAGE).unwrap(), current);
    let entries = frozen
        .job()
        .verify_packet(frozen.packet(), &local_signer.verifying_key().to_bytes())
        .unwrap();
    let updates = entries
        .iter()
        .map(|e| {
            object::open(
                &e.envelope,
                &e.header.context,
                &secret,
                &local_signer.verifying_key().to_bytes(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let result = decoder
        .decode(
            tmt_colab::decoder::UpdateBatch {
                namespace: tmt_colab::decoder::Namespace::Own,
                baseline: &[],
                updates: &refs,
            },
            tmt_colab::decoder::Role::Commenter,
            None,
        )
        .unwrap();
    let prepared_proof = AttachmentPublication::from_json(
        &serde_json::to_vec(&result.projection["intents"][ATTACHMENT]).unwrap(),
    )
    .unwrap();
    prepared_proof
        .matches_descriptor(&local_descriptor)
        .unwrap();
    assert_eq!(prepared_proof.base_revision, current);
    let stale = attachments::PublicationIntent {
        descriptor: &local_descriptor,
        base: &format!("v1:{}", "00".repeat(32)),
    };
    assert!(
        attachments::prepare_publication(
            &store,
            &key,
            stale,
            &committed,
            &mut decoder,
            deadline(),
            NOW
        )
        .is_err()
    );
    // A message-source attachment is fenced by the membership head and epoch it was sealed
    // under, not by the page revision: a foreign write while it uploads leaves it valid, and
    // a document-source attachment (bound to the page revision) goes stale on the same write.
    let own_context = object::Context {
        namespace: "own".into(),
        author_device: local_descriptor.author_device.clone(),
        ..asset_context.clone()
    };
    let message_asset = object::seal(
        &own_context,
        &secret,
        &local_signer,
        b"message attachment bytes",
    )
    .unwrap();
    let message_raw = message_asset.to_json().unwrap();
    let message_descriptor = Descriptor {
        namespace: "own".into(),
        object_id: object::Header::decode(message_asset.header())
            .unwrap()
            .object_id,
        source: Source::Message {
            writer_id: local_descriptor.author_device.clone(),
            message_id: MESSAGE.into(),
            message_revision: "1".into(),
        },
        envelope_hash: hex(&message_asset.hash().unwrap()),
        signature: values::encode_binary(message_asset.signature()),
        payload_sha256: hex(&crypto::digest(&message_raw)),
        payload_bytes: message_raw.len().to_string(),
        plaintext_bytes: "24".into(),
        attachment_id: "20000000-0000-4000-8000-000000000094".into(),
        ..local_descriptor.clone()
    };
    message_descriptor.validate().unwrap();
    let message_committed = Committed {
        namespace: objects.namespace,
        key: message_descriptor
            .object_id
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap(),
        raw: message_raw,
        complete: true,
    };
    let fence = || {
        let head = store
            .owner_head(&key.space_id, &key.owner_public())
            .unwrap()
            .unwrap();
        tmt_colab_model::attachment::message_fence(
            &key.space_id,
            PAGE,
            "1",
            &head.revision.to_string(),
            &head.hash,
            &message_descriptor.author_device,
        )
        .unwrap()
    };
    let mut prepare_message = |base: &str| {
        attachments::prepare_publication(
            &store,
            &key,
            attachments::PublicationIntent {
                descriptor: &message_descriptor,
                base,
            },
            &message_committed,
            &mut decoder,
            deadline(),
            NOW,
        )
    };
    let sealed_fence = fence();
    assert!(
        prepare_message(&current).is_err(),
        "a page revision is never a fence"
    );
    prepare_message(&sealed_fence).unwrap();
    // A foreign write: the page revision moves, the fence does not.
    let moving = Doc::with_client_id(18533);
    moving.get_or_insert_map("meta").insert(
        &mut moving.transact_mut(),
        "title",
        Any::from_json("\"Moved by another writer\"").unwrap(),
    );
    let update = moving
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let write = object::seal(
        &object::Context {
            kind: "update".into(),
            namespace: "content".into(),
            stream_seq: "3".into(),
            prev_hash: previous,
            ..asset_context.clone()
        },
        &secret,
        &SigningKey::from_bytes(&[10; 32]),
        &update,
    )
    .unwrap();
    let mut writer = Store::open(&f.layout).unwrap();
    writer
        .append(&StoredEnvelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: DEVICE,
            },
            namespace: Namespace::Content,
            seq: 3,
            previous,
            hash: write.hash().unwrap(),
            bytes: &write.to_json().unwrap(),
        })
        .unwrap();
    assert_ne!(page::revision(&store, &key, PAGE).unwrap(), current);
    assert_eq!(fence(), sealed_fence);
    prepare_message(&sealed_fence).unwrap();
    assert!(
        attachments::prepare_publication(
            &store,
            &key,
            intent(),
            &committed,
            &mut decoder,
            deadline(),
            NOW
        )
        .is_err(),
        "a document attachment is stale once the page moved"
    );
    let read =
        attachments::capture_root_local(&store, &key, PAGE, &selector(), &mut decoder, deadline())
            .unwrap();
    assert_eq!(
        read.disclose(&store, &key, &objects).unwrap(),
        b"original attachment bytes"
    );
    objects.complete = false;
    assert!(read.disclose(&store, &key, &objects).is_err());
    objects.complete = true;
    objects.raw[0] ^= 1;
    assert!(read.disclose(&store, &key, &objects).is_err());
    objects.raw[0] ^= 1;
    // Actual revoked creator: original own proof is pinned in the old epoch;
    // the baseline carries the document descriptor unchanged into the new epoch.
    assert!(f.service().revoke(DEVICE, 2).unwrap());
    // A membership change moves the message fence too.
    assert_ne!(fence(), sealed_fence);
    assert!(read.disclose(&store, &key, &objects).is_err());
    let rotated =
        attachments::capture_root_local(&store, &key, PAGE, &selector(), &mut decoder, deadline())
            .unwrap();
    assert_eq!(rotated.descriptor(), &descriptor);
    assert_eq!(
        rotated.disclose(&store, &key, &objects).unwrap(),
        b"original attachment bytes"
    );
    let message = AttachmentSelector::Message {
        writer_id: DEVICE.into(),
        message_id: MESSAGE.into(),
        message_revision: "1".into(),
        attachment_id: ATTACHMENT.into(),
        descriptor_hash: hex(&descriptor.hash().unwrap()),
    };
    assert_eq!(
        attachments::capture_root_local(&store, &key, PAGE, &message, &mut decoder, deadline())
            .unwrap()
            .disclose(&store, &key, &objects)
            .unwrap(),
        b"original attachment bytes"
    );
    // The same page names the attachment by an ID prefix: the exact reference the read takes.
    let by_prefix = attachments::resolve(
        &store,
        &key,
        PAGE,
        &ATTACHMENT[..8],
        &mut decoder,
        deadline(),
    )
    .unwrap();
    assert_eq!(by_prefix, selector());
    for refused in [&ATTACHMENT[..7], "zzzzzzzz", "ffffffff"] {
        let code = attachments::resolve(&store, &key, PAGE, refused, &mut decoder, deadline())
            .unwrap_err()
            .downcast_ref::<page::Fault>()
            .map(page::Fault::code);
        assert_eq!(
            code,
            Some(if refused == "ffffffff" {
                "COLAB_STATE_MISSING"
            } else {
                "COLAB_INPUT_INVALID"
            }),
            "{refused}"
        );
    }
    let head = store
        .owner_head(&key.space_id, &key.owner_public())
        .unwrap()
        .unwrap();
    f.service()
        .apply_owner(
            OwnerRequest {
                operation_id: MESSAGE,
                expected_revision: head.revision,
                action: OwnerAction::Archive { page: PAGE },
                transport_digest: None,
                scope: None,
            },
            NOW,
        )
        .unwrap();
    let archived =
        attachments::capture_root_local(&store, &key, PAGE, &selector(), &mut decoder, deadline())
            .unwrap();
    assert_eq!(
        archived.disclose(&store, &key, &objects).unwrap(),
        b"original attachment bytes"
    );
    // A retention long past expiry is a warning only: archived attachment reads still open.
    let head = store
        .owner_head(&key.space_id, &key.owner_public())
        .unwrap()
        .unwrap();
    f.service()
        .apply_owner(
            OwnerRequest {
                operation_id: "20000000-0000-4000-8000-000000000094",
                expected_revision: head.revision,
                action: OwnerAction::Retention {
                    page: PAGE,
                    days: Some(1),
                },
                transport_digest: None,
                scope: None,
            },
            NOW + 400 * 86_400_000,
        )
        .unwrap();
    let expired =
        attachments::capture_root_local(&store, &key, PAGE, &selector(), &mut decoder, deadline())
            .unwrap();
    assert_eq!(
        expired.disclose(&store, &key, &objects).unwrap(),
        b"original attachment bytes"
    );
    assert!(page::prepare_own_records(&store, &key, PAGE, &[], &mut decoder, NOW).is_err());
    let admission =
        tmt_colab::registration::OwnerAdmission(Arc::new(Mutex::new(f.service.take().unwrap())));
    let scope = tmt_colab::sync::SyncScope {
        space: key.space_id.clone(),
        page: PAGE.into(),
        epoch: page::read(&store, &key, PAGE, &mut decoder).unwrap().epoch,
    };
    assert!(admission.attachment_read_owner(DEVICE, &scope).is_err());
    let owner = admission.attachment_read_owner(OTHER, &scope).unwrap();
    let scoped =
        attachments::capture_session(&store, &key, &owner, &message, &mut decoder, deadline())
            .unwrap();
    assert_eq!(
        scoped.disclose(&store, &key, &objects).unwrap(),
        b"original attachment bytes"
    );

    let head = store
        .owner_head(&key.space_id, &key.owner_public())
        .unwrap()
        .unwrap();
    admission
        .0
        .lock()
        .unwrap()
        .apply_owner(
            OwnerRequest {
                operation_id: ATTACHMENT,
                expected_revision: head.revision,
                action: OwnerAction::Delete { page: PAGE },
                transport_digest: None,
                scope: None,
            },
            NOW,
        )
        .unwrap();
    assert!(archived.disclose(&store, &key, &objects).is_err());
    assert!(scoped.disclose(&store, &key, &objects).is_err());
    assert!(
        attachments::capture_root_local(&store, &key, PAGE, &message, &mut decoder, deadline())
            .is_err()
    );
    drop(store);
    drop(key);
}
