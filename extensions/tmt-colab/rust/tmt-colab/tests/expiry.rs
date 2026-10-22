//! Durable server time and expiry through real creation/write/projections.
mod support;
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use tmt_colab::{
    decoder::Decoder,
    inspection,
    keyring::{Keyring, Layout},
    page,
    registration::Registration,
    store::{Accepted, Store},
    transitions::{Engine, OwnerAction, OwnerRequest},
};
use tmt_colab_model::values;
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const NOW: u64 = 1_790_000_000_000;
const DAY: u64 = 86_400_000;
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
struct Fixture {
    root: PathBuf,
    layout: Layout,
    key: Keyring,
    store: Store,
    clock: Arc<AtomicU64>,
    engine: Engine,
    next: u64,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "colab-expiry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let clock = Arc::new(AtomicU64::new(NOW));
        let captured = clock.clone();
        let store = Store::open(&layout)
            .unwrap()
            .with_clock(move || Ok(captured.load(Ordering::SeqCst)));
        Self {
            root,
            layout,
            key,
            store,
            clock,
            engine: Engine::with_decoder_config(support::decoder_config(BINARY.into())).unwrap(),
            next: 1,
        }
    }
    fn apply(&mut self, action: OwnerAction<'_>) {
        let id = format!("20000000-0000-4000-8000-{:012}", self.next);
        self.next += 1;
        let expected_revision = self
            .store
            .owner_head(&self.key.space_id, &self.key.owner_public())
            .unwrap()
            .map_or(0, |h| h.revision);
        self.engine
            .apply(
                &mut self.store,
                &self.key,
                OwnerRequest {
                    operation_id: &id,
                    expected_revision,
                    action,
                    transport_digest: None,
                    scope: None,
                },
                NOW,
            )
            .unwrap();
    }
    fn create(&mut self) {
        self.apply(OwnerAction::Create {
            layout: None,
            creation_recipient: None,
            page: PAGE,
            title: "Expiry",
            source: "<p>Initial</p>",
            publisher_agent: None,
        });
    }
    fn sql(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.layout.directory.join("space.db")).unwrap()
    }
    fn projection(&self) -> Value {
        let cli = inspection::catalog(&self.store, &self.key).unwrap();
        let clock = self.clock.clone();
        let api = Registration::with_decoder_config(
            Store::read(&self.layout)
                .unwrap()
                .with_clock(move || Ok(clock.load(Ordering::SeqCst))),
            Keyring::read(&self.layout).unwrap(),
            support::decoder_config(BINARY.into()),
        )
        .unwrap();
        let context = json!({"deviceId":"30000000-0000-4000-8000-000000000001", "kind":"browser", "origin":"http://127.0.0.1:1", "name":"Owner", "owner":true, "grantRevision":1,
            "publicKey":values::encode_binary(SigningKey::from_bytes(&[9;32]).verifying_key().as_bytes())}).to_string();
        let api_pages: Value = serde_json::from_slice(&api.pages(Some(&context)).unwrap()).unwrap();
        // The CLI catalog also says when each page is archived (#2600); the browser's does not, and
        // everything else is the same verified projection.
        let mut shared = cli["pages"].clone();
        for page in shared.as_array_mut().unwrap() {
            for field in ["lastActivityAtMs", "archiveAfterDays", "archiveAtMs"] {
                assert!(
                    page.as_object_mut().unwrap().remove(field).is_some(),
                    "{field}"
                );
            }
        }
        assert_eq!(shared, api_pages["pages"]);
        api.close().unwrap();
        cli["pages"][0].clone()
    }
    fn decoder(&self) -> Decoder {
        Decoder::with_config(support::decoder_config(BINARY.into())).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn verified_projection_reports_the_cloud_expiry_the_last_confirmed_page_row_made() {
    let mut f = Fixture::new();
    f.create();
    // Nothing was ever confirmed in the cloud: the page is local only, with nothing to warn of.
    let p = f.projection();
    assert_eq!(p["lastUpdateAtMs"], NOW);
    assert_eq!(p["retentionDays"], 30);
    assert!(p["expiresAtMs"].is_null());
    assert_eq!(p["warnings"], json!([]));
    f.store.confirm_page_row(PAGE, 1, NOW + 30 * DAY).unwrap();
    let p = f.projection();
    assert_eq!(p["expiresAtMs"], NOW + 30 * DAY);
    assert_eq!(p["warnings"], json!([]));
    f.clock.store(NOW + 23 * DAY - 1, Ordering::SeqCst);
    assert_eq!(f.projection()["warnings"], json!([]));
    f.clock.store(NOW + 23 * DAY, Ordering::SeqCst);
    assert_eq!(f.projection()["warnings"], json!(["expires-soon"]));
    f.clock.store(NOW + 30 * DAY - 1, Ordering::SeqCst);
    assert_eq!(f.projection()["warnings"], json!(["expires-soon"]));
    f.clock.store(NOW + 30 * DAY, Ordering::SeqCst);
    let p = f.projection();
    assert_eq!(p["expiresAtMs"], NOW + 30 * DAY);
    assert_eq!(p["warnings"], json!(["not-in-cloud"]));
    // Expiry leaves readable content and edit authority in place.
    assert_eq!(
        page::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .source,
        "<p>Initial</p>"
    );
    // A retention override alone moves nothing: only a confirmed write of the row does.
    f.apply(OwnerAction::Retention {
        page: PAGE,
        days: Some(60),
    });
    let p = f.projection();
    assert_eq!(p["expiresAtMs"], NOW + 30 * DAY);
    assert_eq!(p["warnings"], json!(["not-in-cloud"]));
    f.store.confirm_page_row(PAGE, 1, NOW + 90 * DAY).unwrap();
    let p = f.projection();
    assert_eq!(p["expiresAtMs"], NOW + 90 * DAY);
    assert_eq!(p["warnings"], json!([]));
    // Local edits never move it, and an unknown last-update time does not either.
    f.sql()
        .execute("UPDATE pages SET last_update_at_ms=NULL", [])
        .unwrap();
    let p = f.projection();
    assert_eq!(p["expiresAtMs"], NOW + 90 * DAY);
    assert_eq!(p["warnings"], json!([]));
    // Kept forever locally, the cloud copy still has the deadline its confirmed row made.
    f.apply(OwnerAction::Retention {
        page: PAGE,
        days: None,
    });
    let p = f.projection();
    assert_eq!(p["expiresAtMs"], NOW + 90 * DAY);
    assert_eq!(p["warnings"], json!([]));
    f.sql()
        .execute(
            "UPDATE membership_log SET envelope=x'00' WHERE revision='00000000000000000001'",
            [],
        )
        .unwrap();
    assert!(inspection::catalog(&f.store, &f.key).is_err());
}
#[test]
fn a_locally_forever_page_shows_its_confirmed_cloud_deadline_and_the_same_warnings() {
    const OTHER: &str = "10000000-0000-4000-8000-000000000002";
    let mut f = Fixture::new();
    f.create();
    f.apply(OwnerAction::Create {
        layout: None,
        creation_recipient: None,
        page: OTHER,
        title: "Other",
        source: "<p>Other</p>",
        publisher_agent: None,
    });
    f.apply(OwnerAction::Retention {
        page: PAGE,
        days: None,
    });
    // No confirmed row for a forever page: only kept forever, even once this machine has a
    // cloud copy of another page.
    f.store.confirm_page_row(OTHER, 1, NOW + 30 * DAY).unwrap();
    let p = f.projection();
    assert!(p["retentionDays"].is_null());
    assert!(p["expiresAtMs"].is_null());
    assert_eq!(p["warnings"], json!([]));
    f.store.confirm_page_row(PAGE, 1, NOW + 365 * DAY).unwrap();
    let p = f.projection();
    assert!(p["retentionDays"].is_null());
    assert_eq!(p["expiresAtMs"], NOW + 365 * DAY);
    assert_eq!(p["warnings"], json!([]));
    f.clock.store(NOW + 358 * DAY, Ordering::SeqCst);
    assert_eq!(f.projection()["warnings"], json!(["expires-soon"]));
    f.clock.store(NOW + 365 * DAY, Ordering::SeqCst);
    let p = f.projection();
    assert_eq!(p["expiresAtMs"], NOW + 365 * DAY);
    assert_eq!(p["warnings"], json!(["not-in-cloud"]));
}
#[test]
fn a_confirmed_row_belongs_to_its_epoch_and_goes_with_the_page() {
    let mut f = Fixture::new();
    f.create();
    f.store.confirm_page_row(PAGE, 1, NOW + 30 * DAY).unwrap();
    f.apply(OwnerAction::EpochAdvance { page: PAGE });
    // The row confirmed for epoch 1 says nothing about epoch 2, which is not in the cloud yet.
    let p = f.projection();
    assert_eq!(p["epoch"], "2");
    assert!(p["expiresAtMs"].is_null());
    assert_eq!(p["warnings"], json!(["not-in-cloud"]));
    // A confirmation that finished after the epoch moved changes nothing.
    f.store.confirm_page_row(PAGE, 1, NOW + 60 * DAY).unwrap();
    assert!(f.projection()["expiresAtMs"].is_null());
    f.store.confirm_page_row(PAGE, 2, NOW + 40 * DAY).unwrap();
    let p = f.projection();
    assert_eq!(p["expiresAtMs"], NOW + 40 * DAY);
    assert_eq!(p["warnings"], json!([]));
    let rows = |f: &Fixture| -> i64 {
        f.sql()
            .query_row("SELECT count(*) FROM cloud_page_rows", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(rows(&f), 1);
    f.apply(OwnerAction::Delete { page: PAGE });
    assert_eq!(rows(&f), 0);
}
#[test]
fn a_page_never_confirmed_is_not_in_the_cloud_only_once_this_machine_has_a_cloud_copy() {
    const OTHER: &str = "10000000-0000-4000-8000-000000000002";
    let mut f = Fixture::new();
    f.create();
    f.apply(OwnerAction::Create {
        layout: None,
        creation_recipient: None,
        page: OTHER,
        title: "Other",
        source: "<p>Other</p>",
        publisher_agent: None,
    });
    let warnings = |f: &Fixture| -> Vec<Value> {
        inspection::catalog(&f.store, &f.key).unwrap()["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["warnings"].clone())
            .collect()
    };
    assert_eq!(warnings(&f), [json!([]), json!([])]);
    f.store.confirm_page_row(PAGE, 1, NOW + 30 * DAY).unwrap();
    assert_eq!(warnings(&f), [json!([]), json!(["not-in-cloud"])]);
}
#[test]
fn content_samples_append_clock_and_failed_late_receipt_rolls_back_time() {
    let mut f = Fixture::new();
    f.create();
    let p = match page::prepare_publication(
        &f.store,
        &f.key,
        PAGE,
        tmt_colab::decoder::ContentEdit {
            layout: None,
            source: "<p>New</p>",
            publisher_agent: None,
            attachments: None,
        },
        None,
        &mut f.decoder(),
        NOW,
    )
    .unwrap()
    {
        page::PublicationPreparation::Write(frozen) => frozen,
        page::PublicationPreparation::Noop { .. } => panic!("expected a write"),
    };
    let commit = |f: &mut Fixture| {
        page::commit_publication(&mut f.store, &f.key, p.job(), p.packet(), p.chain(), NOW)
    };
    f.clock.store(NOW + 40 * DAY, Ordering::SeqCst);
    f.sql().execute_batch("CREATE TRIGGER deny_receipt BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(FAIL,'late rollback'); END;").unwrap();
    assert!(commit(&mut f).is_err());
    assert_eq!(f.projection()["lastUpdateAtMs"], NOW);
    assert_eq!(
        page::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .source,
        "<p>Initial</p>"
    );
    f.sql().execute_batch("DROP TRIGGER deny_receipt").unwrap();
    assert_eq!(commit(&mut f).unwrap().accepted, Accepted::New);
    assert_eq!(f.projection()["lastUpdateAtMs"], NOW + 40 * DAY);
    f.clock.store(NOW + 50 * DAY, Ordering::SeqCst);
    assert_eq!(commit(&mut f).unwrap().accepted, Accepted::Replay);
    assert_eq!(f.projection()["lastUpdateAtMs"], NOW + 40 * DAY);
    f.apply(OwnerAction::EpochAdvance { page: PAGE });
    assert_eq!(f.projection()["lastUpdateAtMs"], NOW + 40 * DAY);
    f.apply(OwnerAction::Archive { page: PAGE });
    let p = f.projection();
    assert_eq!(p["lastUpdateAtMs"], NOW + 40 * DAY);
    assert_eq!(p["archived"], true);
}

#[test]
fn restore_restarts_activity_atomically_without_changing_keys_content_or_replaying_activity() {
    let mut f = Fixture::new();
    f.create();
    let activity = |f: &Fixture| {
        f.sql()
            .query_row(
                "SELECT last_activity_at_ms FROM pages WHERE page=?",
                [PAGE],
                |row| row.get::<_, i64>(0),
            )
            .unwrap() as u64
    };
    let before = page::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    let retained = |f: &Fixture| {
        let db = f.sql();
        [
            "SELECT secret FROM epoch_secrets ORDER BY page,epoch",
            "SELECT envelope FROM wraps ORDER BY page,epoch,kind,recipient,revision",
            "SELECT record FROM recipients ORDER BY kind,id",
            "SELECT record FROM devices ORDER BY id",
            "SELECT envelope FROM baselines ORDER BY page,epoch",
        ]
        .map(|sql| {
            let mut statement = db.prepare(sql).unwrap();
            statement
                .query_map([], |row| row.get::<_, Vec<u8>>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        })
    };
    let original = retained(&f);
    f.apply(OwnerAction::Archive { page: PAGE });
    let head = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap();
    let restore = |f: &mut Fixture| {
        f.engine.apply(
            &mut f.store,
            &f.key,
            OwnerRequest {
                operation_id: "90000000-0000-4000-8000-000000000001",
                expected_revision: head.revision,
                action: OwnerAction::Unarchive { page: PAGE },
                transport_digest: None,
                scope: None,
            },
            NOW,
        )
    };
    f.clock.store(NOW + 40 * DAY, Ordering::SeqCst);
    // This returned-error fault is after both statement and activity writes, before their receipt.
    f.sql().execute_batch("CREATE TRIGGER deny_restore_receipt BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(FAIL,'restore rollback'); END;").unwrap();
    assert!(restore(&mut f).is_err());
    assert_eq!(
        f.store
            .owner_head(&f.key.space_id, &f.key.owner_public())
            .unwrap(),
        Some(head.clone())
    );
    let unchanged = f.projection();
    assert_eq!(unchanged["archived"], true);
    assert_eq!(unchanged["lastUpdateAtMs"], NOW);
    assert_eq!(activity(&f), NOW);
    assert_eq!(retained(&f), original);
    f.sql()
        .execute_batch("DROP TRIGGER deny_restore_receipt")
        .unwrap();
    let committed = restore(&mut f).unwrap();
    assert!(!committed.replayed);
    let restored = f.projection();
    assert_eq!(restored["archived"], false);
    assert_eq!(
        restored["lastUpdateAtMs"], NOW,
        "restore is not a content edit"
    );
    assert_eq!(activity(&f), NOW + 40 * DAY);
    assert_eq!(
        f.store.last_activity_ms(PAGE).unwrap(),
        Some(NOW + 40 * DAY)
    );
    assert_eq!(restored["epoch"], before.epoch);
    assert_eq!(retained(&f), original);
    let after = page::read(&f.store, &f.key, PAGE, &mut f.decoder()).unwrap();
    assert_eq!(after.source, before.source);
    assert_eq!(after.title, before.title);
    assert_eq!(after.epoch, before.epoch);
    f.clock.store(NOW + 50 * DAY, Ordering::SeqCst);
    let replay = restore(&mut f).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    assert_eq!(replay.head, committed.head);
    assert_eq!(f.projection()["lastUpdateAtMs"], NOW);
    assert_eq!(activity(&f), NOW + 40 * DAY);
    f.apply(OwnerAction::Archive { page: PAGE });
    f.clock.store(NOW + 39 * DAY, Ordering::SeqCst);
    f.apply(OwnerAction::Unarchive { page: PAGE });
    assert_eq!(
        activity(&f),
        NOW + 40 * DAY,
        "page activity never moves backwards"
    );
}

#[test]
fn archiving_drops_a_pin_and_unarchiving_does_not_bring_it_back() {
    let mut f = Fixture::new();
    f.create();
    let pins = |f: &mut Fixture| -> Vec<String> {
        let key = &f.key;
        f.store
            .project_transaction(&key.space_id, &key.owner_public(), |tx| tx.pins())
            .unwrap()
    };
    let pin = |f: &mut Fixture, pinned: bool| {
        let key = &f.key;
        f.store
            .project_transaction(&key.space_id, &key.owner_public(), |tx| {
                tx.set_pinned(PAGE, pinned)
            })
            .unwrap();
    };
    pin(&mut f, true);
    assert_eq!(pins(&mut f), [PAGE]);
    assert_eq!(f.projection()["pinned"], true);
    f.apply(OwnerAction::Archive { page: PAGE });
    assert!(pins(&mut f).is_empty());
    assert!(f.projection().get("pinned").is_none());
    f.apply(OwnerAction::Unarchive { page: PAGE });
    assert!(pins(&mut f).is_empty());
    assert!(f.projection().get("pinned").is_none());
}

#[test]
fn restore_refuses_active_and_deleted_pages_without_recording_activity_or_an_operation() {
    let mut f = Fixture::new();
    f.create();
    for deleted in [false, true] {
        if deleted {
            f.apply(OwnerAction::Archive { page: PAGE });
            f.apply(OwnerAction::Delete { page: PAGE });
        }
        let head = f
            .store
            .owner_head(&f.key.space_id, &f.key.owner_public())
            .unwrap()
            .unwrap();
        let count: i64 = f
            .sql()
            .query_row("SELECT count(*) FROM owner_operations", [], |row| {
                row.get(0)
            })
            .unwrap();
        let error = f
            .engine
            .apply(
                &mut f.store,
                &f.key,
                OwnerRequest {
                    operation_id: if deleted {
                        "90000000-0000-4000-8000-000000000003"
                    } else {
                        "90000000-0000-4000-8000-000000000002"
                    },
                    expected_revision: head.revision,
                    action: OwnerAction::Unarchive { page: PAGE },
                    transport_digest: None,
                    scope: None,
                },
                NOW,
            )
            .expect_err("restore must be refused");
        assert_eq!(error.code, tmt_colab::transitions::Code::Denied);
        assert_eq!(
            f.store
                .owner_head(&f.key.space_id, &f.key.owner_public())
                .unwrap(),
            Some(head.clone())
        );
        assert_eq!(
            f.sql()
                .query_row("SELECT count(*) FROM owner_operations", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            count
        );
        assert_eq!(f.store.last_activity_ms(PAGE).unwrap(), Some(NOW));
        assert_eq!(
            f.sql()
                .query_row(
                    "SELECT last_update_at_ms FROM pages WHERE page=?",
                    [PAGE],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            NOW as i64
        );
    }
}
