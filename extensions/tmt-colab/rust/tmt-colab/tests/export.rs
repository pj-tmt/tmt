mod support;

use ed25519_dalek::{Signer, SigningKey};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    decoder::Decoder,
    export::{Bundle, DISCLOSURE, Fault},
    keyring::{Keyring, Layout},
    store::{
        Envelope, Namespace, Store, StreamScope,
        owner::{Device, Mutation, Recipient},
    },
    transitions::{Engine, EpochAdvance},
};
use tmt_colab_model::{certificate, object, values, wrap};
use yrs::{Doc, Map, ReadTxn, StateVector, Text, Transact};
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
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
            "tmt-export-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let mut store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        store.owner_transaction(&key.space_id, &key.owner_public(), Mutation {
            operation_id: "40000000-0000-4000-8000-000000000001", digest: [7;32], expected_revision: 0,
        }, |tx| {
            let owner = key.management_member()?;
            let recipient = Recipient { kind: "member".into(), id: owner.id.clone(), role: Some("editor".into()), signing_key: owner.signing_key, encryption_key: owner.encryption_key, pages: vec![], revoked: false };
            let issuer = key.sign_statement(tx.head(), "member.add", &serde_json::to_vec(&json!({"memberId": owner.id, "role":"editor", "signKey":values::encode_binary(&owner.signing_key), "encKey":values::encode_binary(&owner.encryption_key), "pages":[]}))?)?;
            tx.append_statement(&issuer)?;
            tx.put_recipient(&recipient)?;
            // A fixture-owned member signs its device, avoiding any private Keyring export.
            let member = "20000000-0000-4000-8000-000000000001";
            let recipient = Recipient { kind: "member".into(), id: member.into(), role: Some("editor".into()), signing_key: signer(7).verifying_key().to_bytes(), encryption_key: wrap::RecipientKey::from_seed(&[8;32])?.public_key(), pages: vec![PAGE.into()], revoked: false };
            let issuer = key.sign_statement(tx.head(), "member.add", &serde_json::to_vec(&json!({"memberId": member,"role":"editor","signKey":values::encode_binary(&recipient.signing_key),"encKey":values::encode_binary(&recipient.encryption_key),"pages":[PAGE]}))?)?;
            tx.append_statement(&issuer)?;
            tx.put_recipient(&recipient)?;
            let cert = certificate::input(&certificate::Certificate { space:&key.space_id, issuer_kind:"member", issuer_id:member, device_id:DEVICE, signing_key:&signer(9).verifying_key().to_bytes(), encryption_key:&wrap::RecipientKey::from_seed(&[10;32])?.public_key(), membership_revision:"2", issued_at:1, expires_at:100000 })?;
            tx.put_device(&Device {revoked:false, chain:serde_json::to_vec(&json!({"version":1,"issuerStatement":values::encode_binary(&issuer.hash()?),"deviceCertificate":values::encode_binary(&cert),"issuerSignature":values::encode_binary(&signer(7).sign(&cert).to_bytes())}))?})?;
            tx.put_epoch_secret(PAGE,1,&[11;32])?;
            Ok(b"genesis".to_vec())
        }).unwrap();
        Self {
            root,
            layout,
            key,
            store,
        }
    }
    fn append(&mut self, source: &str, title: &str) {
        self.append_at(source, title, 1, [0; 32]);
    }
    fn append_at(&mut self, source: &str, title: &str, seq: u64, previous: [u8; 32]) -> [u8; 32] {
        let doc = Doc::with_client_id(57);
        doc.get_or_insert_text("html")
            .insert(&mut doc.transact_mut(), 0, source);
        doc.get_or_insert_map("meta")
            .insert(&mut doc.transact_mut(), "title", title);
        let bytes = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let envelope = object::seal(
            &object::Context {
                space: self.key.space_id.clone(),
                page: PAGE.into(),
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
            &bytes,
        )
        .unwrap();
        let bytes = envelope.to_json().unwrap();
        self.store
            .append(&Envelope {
                scope: StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream: DEVICE,
                },
                namespace: Namespace::Content,
                seq,
                hash: envelope.hash().unwrap(),
                previous,
                bytes: &bytes,
            })
            .unwrap();
        envelope.hash().unwrap()
    }
    fn capture(&self) -> tmt_colab::Result<Bundle> {
        Bundle::capture(
            &self.store,
            &self.key,
            PAGE,
            &mut Decoder::with_config(support::decoder_config(BINARY.into())).unwrap(),
            1234,
        )
    }
    fn db(&self) -> Connection {
        Connection::open(self.layout.directory.join("space.db")).unwrap()
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
        let mut command = Command::new(BINARY);
        command.env("TMT_EXECUTABLE", core).current_dir(&self.root);
        command
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
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn verify(
    directory: &std::path::Path,
    source: &str,
    title: &str,
    f: &Fixture,
    epoch: &str,
    revision: &str,
) {
    assert_eq!(
        fs::read(directory.join("page.html")).unwrap(),
        source.as_bytes()
    );
    let bytes = fs::read(directory.join("manifest.json")).unwrap();
    let m: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(m["format"], "tmt-colab-page-export");
    assert_eq!(m["version"], 1);
    assert_eq!(m["spaceId"], f.key.space_id);
    assert_eq!(m["pageId"], PAGE);
    assert_eq!(m["title"], title);
    assert_eq!(m["epoch"], epoch);
    assert_eq!(m["plaintext"], true);
    assert_eq!(m["discussions"], "not-included");
    assert_eq!(m["membershipHead"]["revision"], revision);
    let head = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap();
    assert_eq!(
        m["membershipHead"]["statementHash"],
        head.hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        m["files"],
        json!([{"name":"page.html","sizeBytes":source.len(),"sha256":hash(source.as_bytes())}])
    );
    assert_eq!(fs::read_dir(directory).unwrap().count(), 2);
    for key in ["secret", "wraps", "ownerKey", "seed", "session"] {
        assert!(m.get(key).is_none());
    }
}
#[test]
fn exact_authenticated_bytes_survive_export_reopen_and_baseline_rotation() {
    for (source, title) in [
        ("<script>parent.pwn()</script>\r\n<p>雪🦀\0</p>", "雪\r\n\0"),
        ("", ""),
    ] {
        let mut f = Fixture::new();
        f.append(source, title);
        let original = fs::read(f.layout.directory.join("space.db")).unwrap();
        let bundle = f.capture().unwrap();
        let published = bundle.publish(&f.root).unwrap();
        verify(&published.directory, source, title, &f, "1", "2");
        let m: Value =
            serde_json::from_slice(&fs::read(published.directory.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(m["exportedAtMs"], 1234);
        assert_eq!(
            fs::read(f.layout.directory.join("space.db")).unwrap(),
            original,
            "export wrote database"
        );
        let read = Store::read(&f.layout).unwrap();
        let reopened = Bundle::capture(
            &read,
            &Keyring::read(&f.layout).unwrap(),
            PAGE,
            &mut Decoder::with_config(support::decoder_config(BINARY.into())).unwrap(),
            1234,
        )
        .unwrap()
        .publish(&f.root)
        .unwrap();
        for name in ["page.html", "manifest.json"] {
            assert_eq!(
                fs::read(reopened.directory.join(name)).unwrap(),
                fs::read(published.directory.join(name)).unwrap()
            );
        }
        read.close().unwrap();
        Engine::with_decoder_config(support::decoder_config(BINARY.into()))
            .unwrap()
            .advance_epoch(
                &mut f.store,
                &f.key,
                EpochAdvance {
                    operation_id: "40000000-0000-4000-8000-000000000002",
                    expected_revision: 2,
                    page: PAGE,
                },
                100,
            )
            .unwrap();
        let rotated = f.capture().unwrap().publish(&f.root).unwrap();
        verify(&rotated.directory, source, title, &f, "2", "3");
        // A completed capture remains immutable after a later owner transition.
        let frozen = bundle.publish(&f.root).unwrap();
        assert_eq!(
            fs::read(frozen.directory.join("manifest.json")).unwrap(),
            fs::read(published.directory.join("manifest.json")).unwrap()
        );
    }
}
#[test]
fn signature_key_baseline_and_capacity_failures_publish_nothing() {
    for defect in ["signature", "key", "baseline", "capacity"] {
        let mut f = Fixture::new();
        f.append("positive control", "title");
        f.capture().unwrap();
        if defect == "capacity" {
            let mut previous = f
                .db()
                .query_row(
                    "SELECT hash FROM receipts WHERE seq=?",
                    [format!("{:020}", 1)],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .unwrap()
                .try_into()
                .unwrap();
            for seq in 2..=201 {
                previous = f.append_at("positive control", "title", seq, previous);
            }
        }
        match defect {
            "signature" => {
                f.db()
                    .execute(
                        "UPDATE membership_log SET envelope=? WHERE revision=?",
                        params![b"{}", format!("{:020}", 2)],
                    )
                    .unwrap();
            }
            "key" => {
                f.db().execute("DELETE FROM epoch_secrets", []).unwrap();
            }
            "baseline" => {
                Engine::with_decoder_config(support::decoder_config(BINARY.into()))
                    .unwrap()
                    .advance_epoch(
                        &mut f.store,
                        &f.key,
                        EpochAdvance {
                            operation_id: "40000000-0000-4000-8000-000000000002",
                            expected_revision: 2,
                            page: PAGE,
                        },
                        100,
                    )
                    .unwrap();
                f.capture().unwrap();
                f.db()
                    .execute("UPDATE baselines SET descriptor=?", [b"{}".as_slice()])
                    .unwrap();
            }
            _ => {}
        }
        assert!(f.capture().is_err(), "accepted {defect}");
        assert_eq!(
            fs::read_dir(&f.root).unwrap().count(),
            1,
            "created export after {defect}"
        );
    }
}
#[test]
fn inactive_and_unknown_pages_are_denied_without_changing_state() {
    for operation in ["page.archive", "page.delete"] {
        let mut f = Fixture::new();
        f.append("positive control", "title");
        f.capture().unwrap();
        f.store
            .owner_transaction(
                &f.key.space_id,
                &f.key.owner_public(),
                Mutation {
                    operation_id: "40000000-0000-4000-8000-000000000002",
                    digest: [9; 32],
                    expected_revision: 2,
                },
                |tx| {
                    tx.append_statement(&f.key.sign_statement(
                        tx.head(),
                        operation,
                        &serde_json::to_vec(&json!({"pageId":PAGE}))?,
                    )?)?;
                    Ok(b"inactive".to_vec())
                },
            )
            .unwrap();
        let before = fs::read(f.layout.directory.join("space.db")).unwrap();
        let error = f.capture().err().unwrap();
        assert!(matches!(
            error.downcast_ref::<Fault>(),
            Some(Fault::Inactive)
        ));
        assert_eq!(
            error.to_string(),
            "archived or deleted pages cannot be exported yet"
        );
        assert_eq!(
            fs::read(f.layout.directory.join("space.db")).unwrap(),
            before
        );
    }
    let f = Fixture::new();
    assert!(
        Bundle::capture(
            &f.store,
            &f.key,
            "10000000-0000-4000-8000-000000000099",
            &mut Decoder::with_config(support::decoder_config(BINARY.into())).unwrap(),
            1
        )
        .is_err()
    );
}
#[test]
fn cli_help_defaults_disclosure_json_and_read_only_failures() {
    let mut f = Fixture::new();
    f.append("<p>CLI\r\n雪\0</p>", "CLI title");
    let help = f.command().args(["export", "--help"]).output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains(DISCLOSURE));
    let output = f
        .command()
        .args(["export", PAGE, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["disclosure"], DISCLOSURE);
    let directory = PathBuf::from(result["directory"].as_str().unwrap());
    assert_eq!(directory.parent(), Some(f.root.as_path()));
    values::generated_id(directory.file_name().unwrap().to_str().unwrap()).unwrap();
    verify(&directory, "<p>CLI\r\n雪\0</p>", "CLI title", &f, "1", "2");
    for info in result["files"].as_array().unwrap() {
        let bytes = fs::read(directory.join(info["name"].as_str().unwrap())).unwrap();
        assert_eq!(info["sizeBytes"], bytes.len());
        assert_eq!(info["sha256"], hash(&bytes));
    }
    let human = f.command().args(["export", PAGE]).output().unwrap();
    assert!(human.status.success());
    assert!(
        String::from_utf8(human.stderr)
            .unwrap()
            .contains(DISCLOSURE)
    );
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .contains("PAGE EXPORTED")
    );
    let target = f.root.join("destination");
    fs::create_dir(&target).unwrap();
    let explicit = f
        .command()
        .args(["export", PAGE, "--dir", target.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(explicit.status.success());
    let value: Value = serde_json::from_slice(&explicit.stdout).unwrap();
    assert_eq!(
        PathBuf::from(value["directory"].as_str().unwrap()).parent(),
        Some(target.as_path())
    );
    for args in [
        vec!["export", "invalid", "--json"],
        vec!["export", PAGE, "--overwrite", "--json"],
        vec!["export", PAGE, "--dir", "missing", "--json"],
    ] {
        let output = f.command().args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stderr.is_empty());
        assert!(
            serde_json::from_slice::<Value>(&output.stdout)
                .unwrap()
                .get("error")
                .is_some()
        );
    }
    let database = f.layout.directory.join("space.db");
    let before = fs::read(&database).unwrap();
    f.db().pragma_update(None, "user_version", 999).unwrap();
    let future = fs::read(&database).unwrap();
    let output = f
        .command()
        .args(["export", PAGE, "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "COLAB_SCHEMA_UNSUPPORTED"
    );
    assert_eq!(fs::read(&database).unwrap(), future);
    fs::write(&database, &before).unwrap();
    fs::remove_file(&database).unwrap();
    assert!(Store::read(&f.layout).is_err());
    assert!(!database.exists());
    symlink(f.root.join("foreign"), &database).unwrap();
    fs::write(f.root.join("foreign"), b"keep").unwrap();
    assert!(Store::read(&f.layout).is_err());
    assert_eq!(fs::read(f.root.join("foreign")).unwrap(), b"keep");
}

#[test]
fn missing_legacy_and_unsafe_state_are_never_initialized_or_migrated() {
    let f = Fixture::new();
    let database = f.layout.directory.join("space.db");
    let command = f.command();
    drop(command);
    fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).unwrap();
    let error = Store::read(&f.layout).err().unwrap();
    assert!(matches!(
        error.downcast_ref::<tmt_colab::keyring::StateFault>(),
        Some(tmt_colab::keyring::StateFault::UnsafeFile)
    ));
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(&database).unwrap();
    let legacy = Connection::open(&database).unwrap();
    legacy.execute_batch("CREATE TABLE pages(page TEXT PRIMARY KEY, epoch TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
    drop(legacy);
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    let before = fs::read(&database).unwrap();
    let legacy = Store::read(&f.layout).unwrap();
    assert!(
        Bundle::capture(
            &legacy,
            &f.key,
            PAGE,
            &mut Decoder::with_config(support::decoder_config(BINARY.into())).unwrap(),
            1
        )
        .is_err()
    );
    legacy.close().unwrap();
    assert_eq!(fs::read(&database).unwrap(), before);
    let output = f
        .command()
        .args(["export", PAGE, "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read(&database).unwrap(), before);
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    fs::remove_dir_all(&f.layout.directory).unwrap();
    let output = f
        .command()
        .args(["export", PAGE, "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "COLAB_EXPORT_STATE_MISSING"
    );
    assert!(!f.layout.directory.exists());
    assert_eq!(
        fs::read_dir(&f.root).unwrap().count(),
        1,
        "missing-state export created files"
    );
}
