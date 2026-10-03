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
        "DROP TABLE baselines; DROP TABLE device_registrations; PRAGMA user_version=2;",
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
        4
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
