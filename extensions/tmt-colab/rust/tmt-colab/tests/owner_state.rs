use ed25519_dalek::{Signer, SigningKey};
use rusqlite::Connection;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    Result,
    keyring::{Keyring, Layout},
    store::{
        Fault, Store,
        owner::{Device, Mutation, OwnerFault, OwnerTransaction, Recipient},
    },
};
use tmt_colab_model::{certificate, statement, values, wrap};

const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const MEMBER: &str = "20000000-0000-4000-8000-000000000001";
const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
const OP: &str = "40000000-0000-4000-8000-000000000001";
const NEXT_OP: &str = "40000000-0000-4000-8000-000000000002";

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
            "tmt-1158-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        Self {
            root,
            layout,
            key,
            store,
        }
    }
    fn oracle(&self) -> Connection {
        Connection::open(self.layout.directory.join("space.db")).unwrap()
    }
    fn bootstrap(&mut self) -> Vec<u8> {
        let key = &self.key;
        self.store
            .owner_transaction(&key.space_id, &key.owner_public(), mutation(OP, 0), |tx| {
                populate(tx, key)
            })
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn mutation(id: &str, revision: u64) -> Mutation<'_> {
    Mutation {
        operation_id: id,
        digest: [7; 32],
        expected_revision: revision,
    }
}
fn recipient() -> Recipient {
    Recipient {
        kind: "member".into(),
        id: MEMBER.into(),
        role: Some("editor".into()),
        signing_key: SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes(),
        encryption_key: wrap::RecipientKey::from_seed(&[8; 32])
            .unwrap()
            .public_key(),
        pages: vec![PAGE.into()],
        revoked: false,
    }
}
fn initial(key: &Keyring) -> statement::Envelope {
    let r = recipient();
    key.sign_statement(
        None,
        "member.add",
        &serde_json::to_vec(&serde_json::json!({
            "memberId":MEMBER,"role":"editor","signKey":values::encode_binary(&r.signing_key),
            "encKey":values::encode_binary(&r.encryption_key),"pages":[PAGE]
        }))
        .unwrap(),
    )
    .unwrap()
}
fn device(key: &Keyring, issuer: [u8; 32]) -> Device {
    let signing = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
    let enc = wrap::RecipientKey::from_seed(&[10; 32])
        .unwrap()
        .public_key();
    let cert = certificate::input(&certificate::Certificate {
        space: &key.space_id,
        issuer_kind: "member",
        issuer_id: MEMBER,
        device_id: DEVICE,
        signing_key: &signing,
        encryption_key: &enc,
        membership_revision: "1",
        issued_at: 1,
        expires_at: 100,
    })
    .unwrap();
    let signature = SigningKey::from_bytes(&[7; 32]).sign(&cert).to_bytes();
    Device { chain:serde_json::to_vec(&serde_json::json!({"version":1,
        "issuerStatement":values::encode_binary(&issuer),"deviceCertificate":values::encode_binary(&cert),
        "issuerSignature":values::encode_binary(&signature)})).unwrap(), revoked:false }
}
fn header(key: &Keyring, epoch: u64, revision: u64) -> wrap::Header {
    wrap::Header {
        space: key.space_id.clone(),
        page: PAGE.into(),
        epoch: epoch.to_string(),
        recipient_kind: "member".into(),
        recipient_id: MEMBER.into(),
        recipient_key: recipient().encryption_key,
        signer_key: key.owner_public(),
        membership_revision: revision.to_string(),
    }
}
fn populate(tx: &mut OwnerTransaction<'_>, key: &Keyring) -> Result<Vec<u8>> {
    let statement = initial(key);
    tx.append_statement(&statement)?;
    tx.put_recipient(&recipient())?;
    tx.put_device(&device(key, statement.hash()?))?;
    tx.put_epoch_secret(PAGE, 1, &[11; 32])?;
    let wrapped = key.seal_wrap(&header(key, 1, 1), &[11; 32])?;
    tx.put_wrap(&wrapped)?;
    // A real result containing exact signed bytes, not a success marker.
    Ok(serde_json::to_vec(
        &serde_json::json!({"statement":values::encode_binary(&statement.to_json()?),"wrap":wrapped}),
    )?)
}
fn append_retention(tx: &mut OwnerTransaction<'_>, key: &Keyring) -> Result<()> {
    tx.append_statement(&key.sign_statement(
        tx.head(),
        "retention.set",
        &serde_json::to_vec(&serde_json::json!({"pageId":PAGE,"days":30}))?,
    )?)
}
fn counts(db: &Connection) -> Vec<i64> {
    [
        "membership_log",
        "owner_state",
        "recipients",
        "devices",
        "epoch_secrets",
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
fn is_owner(error: &(dyn std::error::Error + Send + Sync + 'static), expected: OwnerFault) -> bool {
    error.downcast_ref::<OwnerFault>() == Some(&expected)
}

#[test]
fn signed_authority_round_trips_and_exact_replay_survives_reopen() {
    let mut f = Fixture::new();
    let outcome = f.bootstrap();
    assert_eq!(counts(&f.oracle()), vec![1; 7]);
    let key = &f.key;
    f.store = Store::open(&f.layout).unwrap();
    let replay = f
        .store
        .owner_transaction(&key.space_id, &key.owner_public(), mutation(OP, 0), |_| {
            panic!("replay re-signed")
        })
        .unwrap();
    assert_eq!(replay, outcome);
    f.store
        .owner_transaction(
            &key.space_id,
            &key.owner_public(),
            mutation(NEXT_OP, 1),
            |tx| {
                let first = tx.statement(1)?.unwrap();
                let verified = first.verify_next(&key.space_id, &key.owner_public(), None)?;
                assert_eq!(tx.head(), Some(&verified.head));
                let stored = tx.recipient("member", MEMBER)?.unwrap();
                assert_eq!(stored.encryption_key, recipient().encryption_key);
                assert_eq!(stored.pages, vec![PAGE]);
                assert!(!stored.revoked);
                let stored = tx.device(DEVICE)?.unwrap();
                assert!(!stored.revoked);
                let chain = certificate::Chain::from_json(&stored.chain)?;
                chain.verify(
                    &first.hash()?,
                    &chain.certificate()?,
                    &recipient().signing_key,
                )?;
                assert_eq!(tx.epoch_secret(PAGE, 1)?, Some([11; 32]));
                let h = header(key, 1, 1);
                let wrapped = tx.wrap(&h)?.unwrap();
                assert_eq!(
                    wrap::open(
                        &wrapped,
                        &h,
                        &wrap::RecipientKey::from_seed(&[8; 32])?,
                        &key.owner_public()
                    )?,
                    [11; 32]
                );
                append_retention(tx, key)?;
                Ok(b"second exact outcome".to_vec())
            },
        )
        .unwrap();
    assert_eq!(counts(&f.oracle()), vec![2, 1, 1, 1, 1, 1, 2]);
}

#[test]
fn every_partial_write_and_receipt_failure_rolls_back_the_entire_mutation() {
    for table in [
        "membership_log",
        "owner_state",
        "recipients",
        "devices",
        "epoch_secrets",
        "wraps",
        "owner_operations",
    ] {
        let mut f = Fixture::new();
        let db = f.oracle();
        db.execute_batch(&format!("CREATE TRIGGER injected_failure BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'injected owner failure'); END;")).unwrap();
        let error = f
            .store
            .owner_transaction(
                &f.key.space_id,
                &f.key.owner_public(),
                mutation(OP, 0),
                |tx| populate(tx, &f.key),
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("injected owner failure"),
            "{table}: {error}"
        );
        assert_eq!(counts(&db), vec![0; 7], "{table}");
        db.execute_batch("DROP TRIGGER injected_failure;").unwrap();
        // The same operation can succeed after rollback: no poisoned replay fence.
        f.bootstrap();
        assert_eq!(counts(&db), vec![1; 7]);
    }
}

#[test]
fn returned_error_rolls_back_epoch_fence_and_projection_changes() {
    let mut f = Fixture::new();
    f.bootstrap();
    let before = counts(&f.oracle());
    let key = &f.key;
    let error = f
        .store
        .owner_transaction(
            &key.space_id,
            &key.owner_public(),
            mutation(NEXT_OP, 1),
            |tx| {
                append_retention(tx, key)?;
                let mut r = tx.recipient("member", MEMBER)?.unwrap();
                r.revoked = true;
                tx.put_recipient(&r)?;
                let mut d = tx.device(DEVICE)?.unwrap();
                d.revoked = true;
                tx.put_device(&d)?;
                tx.put_epoch_secret(PAGE, 2, &[12; 32])?;
                tx.put_wrap(&key.seal_wrap(&header(key, 2, 2), &[12; 32])?)?;
                tx.advance_epoch(PAGE, 1)?;
                Err(OwnerFault::Invalid.into())
            },
        )
        .unwrap_err();
    assert!(is_owner(&*error, OwnerFault::Invalid));
    let db = f.oracle();
    assert_eq!(counts(&db), before);
    assert_eq!(
        db.query_row("SELECT epoch FROM pages WHERE page=?", [PAGE], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "1"
    );
    f.store
        .owner_transaction(
            &key.space_id,
            &key.owner_public(),
            mutation(NEXT_OP, 1),
            |tx| {
                assert!(!tx.recipient("member", MEMBER)?.unwrap().revoked);
                assert!(!tx.device(DEVICE)?.unwrap().revoked);
                assert!(tx.epoch_secret(PAGE, 2)?.is_none());
                append_retention(tx, key)?;
                tx.put_epoch_secret(PAGE, 2, &[12; 32])?;
                tx.advance_epoch(PAGE, 1)?;
                Ok(vec![])
            },
        )
        .unwrap();
    assert_eq!(
        db.query_row("SELECT epoch FROM pages WHERE page=?", [PAGE], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "2"
    );
}

#[test]
fn stale_revision_conflicting_digest_and_wrong_owner_never_invoke_the_closure() {
    let mut f = Fixture::new();
    let outcome = f.bootstrap();
    let mut conflict = mutation(OP, 1);
    conflict.digest = [8; 32];
    let error = f
        .store
        .owner_transaction(&f.key.space_id, &f.key.owner_public(), conflict, |_| {
            panic!("conflict applied")
        })
        .unwrap_err();
    assert!(is_owner(&*error, OwnerFault::Conflict));
    let error = f
        .store
        .owner_transaction(
            &f.key.space_id,
            &f.key.owner_public(),
            mutation(NEXT_OP, 0),
            |_| panic!("stale applied"),
        )
        .unwrap_err();
    assert!(is_owner(&*error, OwnerFault::StaleHead));
    let other = Fixture::new();
    let error = f
        .store
        .owner_transaction(
            &other.key.space_id,
            &other.key.owner_public(),
            mutation(OP, 0),
            |_| panic!("wrong owner replayed"),
        )
        .unwrap_err();
    assert!(is_owner(&*error, OwnerFault::WrongOwner));
    assert_eq!(
        f.store
            .owner_transaction(
                &f.key.space_id,
                &f.key.owner_public(),
                mutation(OP, 0),
                |_| panic!("replay applied")
            )
            .unwrap(),
        outcome
    );
    assert_eq!(counts(&f.oracle()), vec![1; 7]);
}

#[test]
fn log_forks_invalid_owner_statements_and_pinned_member_changes_are_denied() {
    let mut f = Fixture::new();
    let other = Fixture::new();
    assert!(
        f.store
            .owner_transaction(
                &f.key.space_id,
                &f.key.owner_public(),
                mutation(OP, 0),
                |tx| {
                    tx.append_statement(&initial(&other.key))?;
                    Ok(vec![])
                }
            )
            .is_err()
    );
    assert_eq!(counts(&f.oracle()), vec![0; 7]);
    f.bootstrap();
    let key = &f.key;
    assert!(
        f.store
            .owner_transaction(
                &key.space_id,
                &key.owner_public(),
                mutation(NEXT_OP, 1),
                |tx| {
                    tx.append_statement(&initial(key))?;
                    Ok(vec![])
                }
            )
            .is_err()
    );
    f.store
        .owner_transaction(
            &key.space_id,
            &key.owner_public(),
            mutation(NEXT_OP, 1),
            |tx| {
                for (op, payload) in [
                    (
                        "member.remove",
                        serde_json::json!({"memberId":MEMBER,"cuts":[]}),
                    ),
                    (
                        "member.role",
                        serde_json::json!({"memberId":MEMBER,"role":"viewer","cuts":[]}),
                    ),
                ] {
                    assert!(
                        key.sign_statement(tx.head(), op, &serde_json::to_vec(&payload)?)
                            .is_err()
                    );
                }
                append_retention(tx, key)?;
                Ok(vec![])
            },
        )
        .unwrap();
    assert_eq!(counts(&f.oracle()), vec![2, 1, 1, 1, 1, 1, 2]);
}

#[test]
fn secrets_and_wraps_are_create_only_and_identical_republication_is_harmless() {
    let mut f = Fixture::new();
    f.bootstrap();
    let key = &f.key;
    for change_secret in [true, false] {
        let error = f
            .store
            .owner_transaction(
                &key.space_id,
                &key.owner_public(),
                mutation(NEXT_OP, 1),
                |tx| {
                    append_retention(tx, key)?;
                    tx.put_epoch_secret(PAGE, 1, &[11; 32])?;
                    let h = header(key, 1, 1);
                    let same = tx.wrap(&h)?.unwrap();
                    tx.put_wrap(&same)?;
                    if change_secret {
                        tx.put_epoch_secret(PAGE, 1, &[12; 32])?;
                    } else {
                        tx.put_wrap(&key.seal_wrap(&h, &[11; 32])?)?;
                    }
                    Ok(vec![])
                },
            )
            .unwrap_err();
        assert!(is_owner(&*error, OwnerFault::Conflict));
        assert_eq!(counts(&f.oracle()), vec![1; 7]);
    }
}

#[test]
fn schema_one_migration_preserves_all_ciphertext_and_refuses_newer_schema_untouched() {
    let f = Fixture::new();
    let db = f.oracle();
    // Independent schema-1 fixture: all original columns carry non-default data.
    db.execute_batch("DROP TABLE owner_operations; DROP TABLE wraps; DROP TABLE epoch_secrets;
        DROP TABLE devices; DROP TABLE recipients; DROP TABLE membership_log; DROP TABLE owner_state;
        DROP TABLE checkpoints; DROP TABLE receipts; DROP TABLE streams; DROP TABLE pages;
        CREATE TABLE pages(page TEXT PRIMARY KEY, epoch TEXT NOT NULL);
        CREATE TABLE streams(page TEXT, epoch TEXT, stream TEXT, frozen INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(page,epoch,stream), FOREIGN KEY(page) REFERENCES pages(page));
        CREATE TABLE receipts(page TEXT, epoch TEXT, stream TEXT, seq TEXT, namespace TEXT NOT NULL,
            hash BLOB NOT NULL, digest BLOB NOT NULL, payload BLOB,
            PRIMARY KEY(page,epoch,stream,seq), FOREIGN KEY(page,epoch,stream) REFERENCES streams(page,epoch,stream));
        CREATE TABLE checkpoints(page TEXT, epoch TEXT, stream TEXT, namespace TEXT, seq TEXT,
            hash BLOB NOT NULL, digest BLOB NOT NULL, head BLOB NOT NULL, payload BLOB, pinned INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(page,epoch,stream,namespace,seq), FOREIGN KEY(page,epoch,stream) REFERENCES streams(page,epoch,stream));
        PRAGMA user_version=1;
        INSERT INTO pages VALUES ('legacy','7');
        INSERT INTO streams VALUES ('legacy','7','writer',1);
        INSERT INTO receipts VALUES ('legacy','7','writer','00000000000000000003','own',X'01',X'02',X'0304');
        INSERT INTO checkpoints VALUES ('legacy','7','writer','content','00000000000000000002',X'05',X'06',X'07',X'0809',1);").unwrap();
    let legacy_rows = |db: &Connection| -> Vec<String> {
        ["pages", "streams", "receipts", "checkpoints"]
            .iter()
            .map(|table| {
                let mut s = db
                    .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                    .unwrap();
                let cols = s.column_count();
                let rows = s
                    .query_map([], |r| {
                        Ok((0..cols)
                            .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                            .collect::<Vec<_>>()
                            .join("|"))
                    })
                    .unwrap();
                rows.map(|r| r.unwrap()).collect::<Vec<_>>().join("\n")
            })
            .collect()
    };
    let before = legacy_rows(&db);
    let reopened = Store::open(&f.layout).unwrap();
    reopened.close().unwrap();
    assert_eq!(legacy_rows(&db), before);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(counts(&db), vec![0; 7]);
    db.pragma_update(None, "user_version", 3).unwrap();
    let path = f.layout.directory.join("space.db");
    let bytes = fs::read(&path).unwrap();
    let error = Store::open(&f.layout).err().unwrap();
    assert!(matches!(
        error.downcast_ref::<Fault>(),
        Some(Fault::UnsupportedSchema(3))
    ));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(legacy_rows(&db), before);
}

#[test]
fn competing_connections_check_the_head_under_the_writer_lock() {
    let mut f = Fixture::new();
    f.bootstrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = [NEXT_OP, "40000000-0000-4000-8000-000000000003"]
        .into_iter()
        .map(|id| {
            let layout = Layout::existing(&f.root).unwrap().unwrap();
            let key = Keyring::read(&layout).unwrap();
            let mut store = Store::open(&layout).unwrap();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.owner_transaction(&key.space_id, &key.owner_public(), mutation(id, 1), |tx| {
                    assert_eq!(tx.head().unwrap().revision, 1);
                    append_retention(tx, &key)?;
                    Ok(b"winner".to_vec())
                })
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| r
                .as_ref()
                .is_err_and(|e| is_owner(&**e, OwnerFault::StaleHead)))
            .count(),
        1
    );
    assert_eq!(counts(&f.oracle()), vec![2, 1, 1, 1, 1, 1, 2]);
}

#[test]
fn empty_mutation_and_oversized_outcome_do_not_leave_authority_or_receipts() {
    let mut f = Fixture::new();
    let error = f
        .store
        .owner_transaction(
            &f.key.space_id,
            &f.key.owner_public(),
            mutation(OP, 0),
            |_| Ok(vec![]),
        )
        .unwrap_err();
    assert!(is_owner(&*error, OwnerFault::Invalid));
    let error = f
        .store
        .owner_transaction(
            &f.key.space_id,
            &f.key.owner_public(),
            mutation(OP, 0),
            |tx| {
                populate(tx, &f.key)?;
                Ok(vec![0; tmt_colab::store::owner::MAX_OUTCOME_BYTES + 1])
            },
        )
        .unwrap_err();
    assert!(is_owner(&*error, OwnerFault::Capacity));
    assert_eq!(counts(&f.oracle()), vec![0; 7]);
}
